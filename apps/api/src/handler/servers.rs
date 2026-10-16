use axum::{
    Extension, Json,
    extract::{Path, State},
    response::IntoResponse,
};
use chrono::Utc;
use serde_json::from_str;
use std::sync::LazyLock;
use uuid::Uuid;

use crate::security::authz::extract::{RequireOrg, perm};
use crate::security::middleware::org_scope::OrgScope;
use crate::{
    dtos::{
        response::{AppError, accepted, created, ok},
        server::{
            CapacityDto, ConfirmHostKeyRequest, CreateServerRequest, RemoteServerCapacityDto,
            RenameServerRequest, ServerAssignedApplicationDto, ServerDto, ServerErrorDto,
            ServerFactsDto, ServerGeoDto, ServerJobDto, ServerMaintenanceResultDto,
            ServerPreflightDto, ServerPublicKeyDto, ServerRegistrationDto, ServerSummaryDto,
            ServerTestDeployDto,
        },
    },
    security::middleware::auth::AuthUser,
    servers::{
        geoip::{self, ServerGeo, Unlocated},
        model::{IngressMode, PodmanMode, ServerState, SshTarget},
        repository::{self, ServerJobRecord, ServerRecord},
        setup::{self, ServerFacts},
        ssh::{SshExecutor, validate_imported_private_key},
    },
    state::state::SharedState,
    utils::security::{encrypt_data, get_encryption_key},
};

#[cfg(any(target_os = "linux", test))]
fn memory_bytes_from_meminfo(meminfo: &str) -> Option<u64> {
    meminfo
        .lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))?
        .split_whitespace()
        .next()?
        .parse::<u64>()
        .ok()?
        .checked_mul(1024)
}

#[cfg(target_os = "linux")]
fn local_memory_bytes() -> Option<u64> {
    memory_bytes_from_meminfo(&std::fs::read_to_string("/proc/meminfo").ok()?)
}

#[cfg(target_os = "macos")]
fn local_memory_bytes() -> Option<u64> {
    std::process::Command::new("sysctl")
        .args(["-n", "hw.memsize"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|bytes| bytes.trim().parse().ok())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn local_memory_bytes() -> Option<u64> {
    None
}

fn dto_with_count(
    state: &SharedState,
    server: ServerRecord,
    assigned_application_count: u64,
) -> ServerDto {
    let is_local = server.kind == "local";
    let mut facts = from_str::<ServerFacts>(&server.facts_json).unwrap_or_default();
    if is_local {
        facts.cpu_cores = std::thread::available_parallelism()
            .ok()
            .map(|count| count.get() as f64);
        facts.memory_bytes = local_memory_bytes();
    }
    ServerDto {
        id: server.id,
        kind: server.kind,
        // On cloud the local server is the control plane host; the stock name would mislead.
        name: if is_local && state.is_cloud && server.name == "Local server" {
            "Cloud server".into()
        } else {
            server.name
        },
        ingress_mode: server.ingress_mode,
        podman_mode: server.podman_mode,
        state: server.state,
        public_address: if is_local {
            Some(state.public_ip.clone())
        } else {
            server.public_address
        },
        facts: ServerFactsDto {
            swap_recommended: setup::swap_recommended(&facts),
            os: facts.os,
            version: facts.version,
            architecture: facts.architecture,
            podman_version: facts.podman_version,
            caddy_version: facts.caddy_version,
            swap_bytes: facts.swap_bytes,
            capacity: CapacityDto {
                cpu_cores: facts.cpu_cores,
                memory_bytes: facts.memory_bytes,
                disk_free_bytes: facts.disk_free_bytes,
            },
        },
        last_error: server
            .last_error_code
            .zip(server.last_error_message)
            .map(|(code, message)| ServerErrorDto { code, message }),
        assigned_application_count,
        ssh_host: server.ssh_host,
        ssh_port: server.ssh_port,
        ssh_user: server.ssh_user,
        host_key_fingerprint: server.host_key_fingerprint,
        organization_id: server.organization_id,
    }
}

async fn dto(state: &SharedState, server: ServerRecord) -> Result<ServerDto, AppError> {
    let count = repository::count_assigned_applications(state, &server.id).await?;
    Ok(dto_with_count(state, server, count))
}

fn job_dto(job: ServerJobRecord) -> ServerJobDto {
    ServerJobDto {
        id: job.id,
        server_id: job.server_id,
        kind: job.kind,
        status: job.status,
        phase: job.phase,
        log: job.log,
        error: job
            .error_code
            .zip(job.error_message)
            .map(|(code, message)| ServerErrorDto { code, message }),
        created_at: job.created_at,
        started_at: job.started_at,
        finished_at: job.finished_at,
    }
}

fn summary(server: ServerRecord) -> ServerSummaryDto {
    let facts = from_str::<ServerFacts>(&server.facts_json).unwrap_or_default();
    crate::dtos::server::server_summary(&server, &facts)
}

pub async fn list_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerRead>,
) -> Result<impl IntoResponse, AppError> {
    let include_local = org.user.is_sysadmin;
    let organization_id = org.organization_id;
    let counts = repository::assigned_counts(&state).await?;
    let servers = repository::list_for_organization(&state, &organization_id, include_local)
        .await?
        .into_iter()
        .map(|server| {
            let count = counts.get(&server.id).copied().unwrap_or(0);
            dto_with_count(&state, server, count)
        })
        .collect::<Vec<_>>();
    Ok(ok(servers))
}

pub async fn get_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerRead>,
    // The extractor covers `organization_id`.
    Path((_, server_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let organization_id = org.organization_id;
    Ok(ok(dto(
        &state,
        repository::get_for_server_route(&state, &server_id, &organization_id)
            .await?
            .ok_or(AppError::ResourceNotFound)?,
    )
    .await?))
}

/// Recomputes the public key so a sysadmin can install it on the host after the one-time
/// registration response is gone. The private key never leaves the API.
pub async fn public_key_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerManage>,
    // The extractor covers `organization_id`.
    Path((_, server_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let organization_id = org.organization_id;
    let server = repository::get_for_server_route(&state, &server_id, &organization_id)
        .await?
        .ok_or(AppError::ResourceNotFound)?;
    if server.kind != "ssh" {
        return Err(AppError::ValidationError(
            "The local server has no SSH key".into(),
        ));
    }
    let encrypted = server
        .private_key_encrypted
        .ok_or(AppError::ResourceNotFound)?;
    let public_key = SshExecutor::public_key_from_encrypted(&encrypted).await?;
    if public_key.contains('\'') || public_key.contains('\n') {
        return Err(AppError::InternalError(
            "The derived SSH public key is malformed".into(),
        ));
    }
    Ok(ok(ServerPublicKeyDto {
        authorized_keys_command: format!(
            "mkdir -p ~/.ssh && chmod 700 ~/.ssh && echo '{public_key}' >> ~/.ssh/authorized_keys && chmod 600 ~/.ssh/authorized_keys"
        ),
        public_key,
    }))
}

pub async fn delete_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerManage>,
    // The extractor covers `organization_id`.
    Path((_, server_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let organization_id = org.organization_id;
    let server = repository::get_for_server_route(&state, &server_id, &organization_id)
        .await?
        .ok_or(AppError::ResourceNotFound)?;
    if server.kind != "ssh" {
        return Err(AppError::ValidationError(
            "The local server cannot be removed".into(),
        ));
    }
    if repository::has_active_setup_job(&state, &server_id).await? {
        return Err(AppError::ResourceInUse(
            "A Setup or Repair job is still running for this server".into(),
        ));
    }
    let assigned = repository::count_assigned_applications(&state, &server_id).await?;
    if assigned > 0 {
        return Err(AppError::ResourceInUse(format!(
            "{assigned} application(s) are still assigned to this server. Move or delete them first."
        )));
    }
    repository::delete(&state, &server_id).await?;
    crate::git::repositories::audit_log::record(
        &state,
        &organization_id,
        &org.user,
        "server.deleted",
        Some("server"),
        Some(&server_id),
        Some(&server.name),
        None,
        None,
        None,
        None,
    )
    .await
    .ok();
    Ok(ok(()))
}

pub async fn rename_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerManage>,
    // The extractor covers `organization_id`.
    Path((_, server_id)): Path<(String, String)>,
    Json(request): Json<RenameServerRequest>,
) -> Result<impl IntoResponse, AppError> {
    let organization_id = org.organization_id;
    let server = repository::get_for_server_route(&state, &server_id, &organization_id)
        .await?
        .ok_or(AppError::ResourceNotFound)?;
    if server.kind != "ssh" && !(server.kind == "local" && org.user.is_sysadmin) {
        return Err(AppError::ValidationError(
            "Only a system administrator can rename the local server".into(),
        ));
    }
    if request.name.trim().is_empty() || request.name.trim().len() > 100 {
        return Err(AppError::ValidationError(
            "Server name must be between 1 and 100 characters".into(),
        ));
    }
    let previous_name = server.name.clone();
    let updated = repository::update_name(&state, &server_id, request.name.trim()).await?;
    crate::git::repositories::audit_log::record(
        &state,
        &organization_id,
        &org.user,
        "server.renamed",
        Some("server"),
        Some(&server_id),
        Some(request.name.trim()),
        Some(&serde_json::json!({ "from": previous_name, "to": request.name.trim() }).to_string()),
        None,
        None,
        None,
    )
    .await
    .ok();
    Ok(ok(dto(&state, updated).await?))
}

/// Backs the removal dialog: a server that still hosts applications cannot be removed, so the
/// user is told which ones to move or delete rather than just how many.
pub async fn assigned_applications_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerRead>,
    // The extractor covers `organization_id`.
    Path((_, server_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let organization_id = org.organization_id;
    repository::get_for_server_route(&state, &server_id, &organization_id)
        .await?
        .ok_or(AppError::ResourceNotFound)?;
    Ok(ok(repository::assigned_applications(&state, &server_id)
        .await?
        .into_iter()
        .map(|application| ServerAssignedApplicationDto {
            id: application.id,
            name: application.name,
            environment_id: application.environment_id,
            environment_name: application.environment_name,
            project_id: application.project_id,
            project_name: application.project_name,
            deploy_type: application.deploy_type,
            template_kind: application.template_kind,
            icon_url: application.icon_url,
            catalog_icon_url: application.catalog_icon_url,
        })
        .collect::<Vec<_>>()))
}

/// Every host port an application on this server publishes, from every source
/// litepod knows about (the app's own Compose `ports:`, the Ports-section
/// overrides, and litepod's own managed loopback endpoint) - so a collision
/// between two apps is visible before it becomes a Podman deploy failure.
pub async fn ports_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerRead>,
    // The extractor covers `organization_id`.
    Path((_, server_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let organization_id = org.organization_id;
    repository::get_for_server_route(&state, &server_id, &organization_id)
        .await?
        .ok_or(AppError::ResourceNotFound)?;
    Ok(ok(crate::servers::ports::list_for_server(
        &state, &server_id,
    )
    .await?))
}

/// Placement choices for this organization: no SSH host, key fingerprint, key
/// material, or Caddy details, and never a server another organization owns.
pub async fn available_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerRead>,
) -> Result<impl IntoResponse, AppError> {
    let organization_id = org.organization_id;
    Ok(ok(repository::list_ready_for_organization(
        &state,
        &organization_id,
    )
    .await?
    .into_iter()
    .map(summary)
    .collect::<Vec<_>>()))
}

/// A located server is not going to move; a lookup that produced nothing is
/// worth retrying sooner, in case the address or the upstream data changed.
const LOCATED_TTL_MS: i64 = 30 * 24 * 60 * 60 * 1000;
const UNLOCATED_TTL_MS: i64 = 24 * 60 * 60 * 1000;

/// The address the fleet map should geolocate a remote server by. `public_address`
/// is the one the server advertises; `ssh_host` is how the control plane reaches it.
fn geo_address(server: &ServerRecord) -> Option<&str> {
    server
        .public_address
        .as_deref()
        .or(server.ssh_host.as_deref())
}

/// Why a server has no coordinates, decided without touching the network. Used
/// for cached failures, where the reason was never stored.
fn offline_reason(state: &SharedState, server: &ServerRecord) -> Unlocated {
    let address = if server.kind == "local" {
        Some(state.public_ip.as_str())
    } else {
        geo_address(server)
    };
    match address {
        None => Unlocated::NoAddress,
        Some(address) => match address.parse::<std::net::IpAddr>() {
            Ok(ip) if !crate::security::ssrf::is_public(ip) => Unlocated::PrivateAddress,
            _ => Unlocated::LookupFailed,
        },
    }
}

/// A stored lookup that is still worth trusting, if there is one.
fn cached_geo(
    state: &SharedState,
    server: &ServerRecord,
    now: i64,
) -> Option<Result<ServerGeo, Unlocated>> {
    let age = now - server.geo_resolved_at?;
    match (server.latitude, server.longitude) {
        // Country-only answers (no city) are retried like failures, in case a
        // provider now knows more.
        (Some(_), Some(_)) if server.geo_city.is_none() && age >= UNLOCATED_TTL_MS => None,
        (Some(latitude), Some(longitude)) if age < LOCATED_TTL_MS => Some(Ok(ServerGeo {
            latitude,
            longitude,
            city: server.geo_city.clone(),
            country: server.geo_country.clone(),
            timezone: server.geo_timezone.clone(),
            asn: server.geo_asn.clone(),
        })),
        (Some(_), Some(_)) => None,
        _ if age < UNLOCATED_TTL_MS => Some(Err(offline_reason(state, server))),
        _ => None,
    }
}

async fn resolve_geo(
    state: &SharedState,
    server: &ServerRecord,
    now: i64,
) -> Result<ServerGeo, Unlocated> {
    if let Some(cached) = cached_geo(state, server, now) {
        return cached;
    }
    let outcome = if server.kind == "local" {
        match geoip::for_host(&state.public_ip).await {
            // `LITEPOD_PUBLIC_IP` is a placeholder on a dev box or behind NAT.
            // With no path, ip.guide answers about the caller, which is this host.
            Err(Unlocated::PrivateAddress | Unlocated::NoAddress) => geoip::for_self().await,
            other => other,
        }
    } else {
        match geo_address(server) {
            Some(address) => geoip::for_host(address).await,
            None => Err(Unlocated::NoAddress),
        }
    };
    // Recorded either way: a failed lookup that is not remembered is repeated on
    // every request. A write failure only costs a redundant lookup next time.
    let _ = repository::set_geo(state, &server.id, outcome.as_ref().ok()).await;
    outcome
}

fn geo_dto(server: &ServerRecord, outcome: Result<ServerGeo, Unlocated>) -> ServerGeoDto {
    let located = outcome.as_ref().ok();
    ServerGeoDto {
        server_id: server.id.clone(),
        name: server.name.clone(),
        kind: server.kind.clone(),
        state: server.state,
        ingress_mode: server.ingress_mode,
        is_main: server.kind == "local",
        latitude: located.map(|geo| geo.latitude),
        longitude: located.map(|geo| geo.longitude),
        city: located.and_then(|geo| geo.city.clone()),
        country: located.and_then(|geo| geo.country.clone()),
        timezone: located.and_then(|geo| geo.timezone.clone()),
        asn: located.and_then(|geo| geo.asn.clone()),
        unlocated_reason: outcome.err().map(|reason| reason.as_str().to_string()),
    }
}

/// The litepod control plane has no `servers` row to cache a lookup in, so its
/// location is memoized in-process for the same 30-day window a server's stored
/// lookup gets (`LOCATED_TTL_MS`).
static CLOUD_GEO_CACHE: LazyLock<std::sync::Mutex<Option<(i64, ServerGeo)>>> =
    LazyLock::new(|| std::sync::Mutex::new(None));

/// The control plane's own location, resolved once per long window. `None` when
/// it cannot be placed, in which case cloud fleets simply have no anchor pin.
async fn resolve_cloud_geo() -> Option<ServerGeo> {
    let now = Utc::now().timestamp_millis();
    if let Ok(cache) = CLOUD_GEO_CACHE.lock() {
        if let Some((resolved_at, geo)) = cache.as_ref() {
            if now - *resolved_at < LOCATED_TTL_MS {
                return Some(geo.clone());
            }
        }
    }
    let geo = geoip::for_self().await.ok()?;
    if let Ok(mut cache) = CLOUD_GEO_CACHE.lock() {
        *cache = Some((now, geo.clone()));
    }
    Some(geo)
}

/// The anchor pin of a cloud fleet. Cloud mode has no `local` server to be
/// `is_main`, so the control plane itself fills that role: every signed-up
/// organization's globe shows one fixed "Litepod Cloud" hub where litepod runs,
/// with the org's own servers around it.
fn cloud_geo_dto(geo: ServerGeo) -> ServerGeoDto {
    ServerGeoDto {
        server_id: "cloud".into(),
        name: "Litepod Cloud".into(),
        kind: "cloud".into(),
        state: ServerState::Ready,
        ingress_mode: IngressMode::Central,
        is_main: true,
        latitude: Some(geo.latitude),
        longitude: Some(geo.longitude),
        city: geo.city,
        country: geo.country,
        timezone: geo.timezone,
        asn: geo.asn,
        unlocated_reason: None,
    }
}

/// Every server this organization can see, placed on the map. Geolocation never
/// fails the request: a server that cannot be placed comes back with null
/// coordinates and a reason, so the fleet list stays complete.
pub async fn geo_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerRead>,
) -> Result<impl IntoResponse, AppError> {
    let servers = repository::list_for_organization(&state, &org.organization_id, false).await?;
    let now = Utc::now().timestamp_millis();
    let outcomes = futures_util::future::join_all(
        servers
            .iter()
            .map(|server| resolve_geo(&state, server, now)),
    )
    .await;
    let mut dtos = servers
        .iter()
        .zip(outcomes)
        .map(|(server, outcome)| geo_dto(server, outcome))
        .collect::<Vec<_>>();
    // Cloud mode has no `local` server to anchor the fleet map, so the control
    // plane itself becomes the anchor: one fixed "Litepod Cloud" hub in the same
    // spot for every organization, with that organization's own servers around it.
    if state.is_cloud {
        if let Some(geo) = resolve_cloud_geo().await {
            dtos.insert(0, cloud_geo_dto(geo));
        }
    }
    Ok(ok(dtos))
}

/// Cloud capacity is owned by the organization payer, not by the member viewing
/// this page. Creation repeats this check as the authoritative guard.
pub async fn remote_capacity_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerManage>,
) -> Result<impl IntoResponse, AppError> {
    let result: Result<(), AppError> = if crate::billing::entitlement::requires_server_entitlement(
        state.is_cloud,
        org.user.is_sysadmin,
    ) {
        crate::billing::entitlement::require_active_subscription_for_server(
            &state,
            &org.organization_id,
        )
        .await
        .map(|_| ())
    } else {
        Ok(())
    };
    Ok(ok(match result {
        Ok(_) => RemoteServerCapacityDto {
            available: true,
            reason: None,
        },
        Err(error) => RemoteServerCapacityDto {
            available: false,
            reason: Some(error.to_string()),
        },
    }))
}

fn validate_registration_ingress(
    is_cloud: bool,
    ingress: IngressMode,
    public_address: Option<&str>,
) -> Result<(), AppError> {
    if is_cloud && ingress != IngressMode::PublicEdge {
        return Err(AppError::ValidationError(
            "Cloud remote servers must use public_edge ingress".into(),
        ));
    }
    if ingress == IngressMode::Central {
        return Err(AppError::ValidationError(
            "Remote servers must use public_edge, private_relay, or private_only ingress".into(),
        ));
    }
    if ingress == IngressMode::PublicEdge
        && public_address.is_none_or(|value| value.trim().is_empty())
    {
        return Err(AppError::ValidationError(
            "Public-edge servers require their public IP address or hostname".into(),
        ));
    }
    if ingress != IngressMode::PublicEdge && public_address.is_some() {
        return Err(AppError::ValidationError(
            "Only public-edge servers accept a public address".into(),
        ));
    }
    Ok(())
}

pub async fn create_handler(
    State(state): State<SharedState>,
    Extension(_user): Extension<AuthUser>,
    RequireOrg(org, _): RequireOrg<perm::ServerManage>,
    Json(request): Json<CreateServerRequest>,
) -> Result<impl IntoResponse, AppError> {
    let OrgScope {
        organization_id,
        user,
        ..
    } = org;
    if crate::billing::entitlement::requires_server_entitlement(state.is_cloud, user.is_sysadmin) {
        crate::billing::entitlement::require_active_subscription_for_server(
            &state,
            &organization_id,
        )
        .await?;
    }
    if request.name.trim().is_empty() || request.name.trim().len() > 100 {
        return Err(AppError::ValidationError(
            "Server name must be between 1 and 100 characters".into(),
        ));
    }
    let target = SshTarget::parse(&request.host, request.port.unwrap_or(22), &request.user)?;
    validate_registration_ingress(
        state.is_cloud,
        request.ingress_mode,
        request.public_address.as_deref(),
    )?;
    if matches!(request.podman_mode, PodmanMode::Rootless) && target.user == "root" {
        return Err(AppError::ValidationError("Rootless Podman needs a non-root SSH user. Use an unprivileged user, or choose rootful.".into()));
    }
    let (private_key_encrypted, generated_public_key) = match request.credential {
        crate::dtos::server::ServerCredentialInput::Generate => {
            let key = SshExecutor::generate_key().await?;
            (key.private_key_encrypted, Some(key.public_key))
        }
        crate::dtos::server::ServerCredentialInput::Import { private_key } => {
            validate_imported_private_key(&private_key)?;
            (
                encrypt_data(&private_key, &get_encryption_key())
                    .map_err(AppError::InternalError)?,
                None,
            )
        }
    };
    let record = repository::insert_pending(
        &state,
        &Uuid::now_v7().to_string(),
        &organization_id,
        request.name.trim(),
        &target.host,
        target.port,
        &target.user,
        &private_key_encrypted,
        request.ingress_mode,
        request.podman_mode,
        request.public_address.as_deref(),
    )
    .await?;
    crate::git::repositories::audit_log::record(
        &state,
        &organization_id,
        &user,
        "server.created",
        Some("server"),
        Some(&record.id),
        Some(&record.name),
        None,
        None,
        None,
        None,
    )
    .await
    .ok();
    Ok(created(ServerRegistrationDto {
        server: dto_with_count(&state, record, 0),
        generated_public_key,
    }))
}

pub async fn preflight_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerManage>,
    // The extractor covers `organization_id`.
    Path((_, server_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let organization_id = org.organization_id;
    let server = repository::get_for_server_route(&state, &server_id, &organization_id)
        .await?
        .ok_or(AppError::ResourceNotFound)?;
    if server.kind == "local" {
        return Err(AppError::ValidationError(
            "The local server does not need SSH preflight".into(),
        ));
    }
    if server.host_key_fingerprint.is_none() {
        let target = SshTarget::parse(
            server.ssh_host.as_deref().unwrap_or_default(),
            server.ssh_port.unwrap_or_default(),
            server.ssh_user.as_deref().unwrap_or_default(),
        )?;
        let observed = SshExecutor::discover_host_key(&target).await?;
        let server = dto(&state, server).await?;
        return Ok(ok(ServerPreflightDto {
            server,
            observed_host_key_fingerprint: Some(observed.value),
        }));
    }
    let server = repository::ensure_host_key_known_hosts(&state, server).await?;
    match setup::preflight(&server).await {
        Ok((facts, next)) => {
            let facts_json = serde_json::to_string(&facts)
                .map_err(|error| AppError::InternalError(error.to_string()))?;
            let updated =
                repository::set_facts_and_state(&state, &server.id, &facts_json, next).await?;
            Ok(ok(ServerPreflightDto {
                server: dto(&state, updated).await?,
                observed_host_key_fingerprint: None,
            }))
        }
        Err(error) => {
            let message = error.to_string();
            let _ = repository::set_state_error(
                &state,
                &server.id,
                ServerState::Offline,
                "preflight_failed",
                &message,
            )
            .await;
            Err(error)
        }
    }
}

pub async fn confirm_host_key_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerManage>,
    // The extractor covers `organization_id`.
    Path((_, server_id)): Path<(String, String)>,
    Json(request): Json<ConfirmHostKeyRequest>,
) -> Result<impl IntoResponse, AppError> {
    let organization_id = org.organization_id;
    let server = repository::get_for_server_route(&state, &server_id, &organization_id)
        .await?
        .ok_or(AppError::ResourceNotFound)?;
    if server.kind != "ssh" || server.state != ServerState::NeedsVerification {
        return Err(AppError::ValidationError(
            "This server is not waiting for host-key verification".into(),
        ));
    }
    let target = SshTarget::parse(
        server.ssh_host.as_deref().unwrap_or_default(),
        server.ssh_port.unwrap_or_default(),
        server.ssh_user.as_deref().unwrap_or_default(),
    )?;
    let observed = SshExecutor::discover_host_key(&target).await?;
    if request.fingerprint.trim() != observed.value {
        return Err(AppError::ValidationError("The submitted fingerprint does not match the SSH host currently reachable at this address".into()));
    }
    let confirmed = repository::confirm_host_key(
        &state,
        &server.id,
        &observed.value,
        observed.known_hosts_line(),
        ServerState::NeedsSetup,
    )
    .await?;
    Ok(ok(dto(&state, confirmed).await?))
}

pub async fn healthcheck_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerManage>,
    // The extractor covers `organization_id`.
    Path((_, server_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let organization_id = org.organization_id;
    let server = repository::get_for_server_route(&state, &server_id, &organization_id)
        .await?
        .ok_or(AppError::ResourceNotFound)?;
    if server.kind != "ssh" {
        return Err(AppError::ValidationError(
            "The local server is checked by litepod's normal API health endpoint".into(),
        ));
    }
    if server.host_key_fingerprint.is_none() {
        return Err(AppError::ValidationError(
            "Confirm the SSH host fingerprint before running a health check".into(),
        ));
    }
    let server = repository::ensure_host_key_known_hosts(&state, server).await?;
    match setup::health_check(&server).await {
        Ok((facts, next)) => {
            let facts_json = serde_json::to_string(&facts)
                .map_err(|error| AppError::InternalError(error.to_string()))?;
            let updated =
                repository::set_facts_and_state(&state, &server.id, &facts_json, next).await?;
            Ok(ok(dto(&state, updated).await?))
        }
        Err(error) => {
            let message = crate::servers::ssh::redact(&error.to_string());
            let _ = repository::set_state_error(
                &state,
                &server.id,
                ServerState::Degraded,
                "health_check_failed",
                &message,
            )
            .await;
            Err(error)
        }
    }
}

async fn maintenance_server(
    state: &SharedState,
    organization_id: &str,
    server_id: &str,
) -> Result<ServerRecord, AppError> {
    let server = repository::get_for_server_route(state, server_id, organization_id)
        .await?
        .ok_or(AppError::ResourceNotFound)?;
    if server.kind != "ssh" {
        return Err(AppError::ValidationError(
            "Maintenance actions are available for remote servers only".into(),
        ));
    }
    if server.host_key_fingerprint.is_none() {
        return Err(AppError::ValidationError(
            "Confirm the SSH host fingerprint before running maintenance".into(),
        ));
    }
    repository::ensure_host_key_known_hosts(state, server).await
}

fn maintenance_dto(result: setup::MaintenanceOutcome) -> ServerMaintenanceResultDto {
    ServerMaintenanceResultDto {
        action: result.action,
        success: result.success,
        stdout: result.stdout,
        stderr: result.stderr,
        duration_ms: result.duration_ms,
    }
}

pub async fn maintenance_update_packages_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerManage>,
    // The extractor covers `organization_id`.
    Path((_, server_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let organization_id = org.organization_id;
    let server = maintenance_server(&state, &organization_id, &server_id).await?;
    Ok(ok(maintenance_dto(setup::update_packages(&server).await?)))
}

pub async fn maintenance_restart_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerManage>,
    // The extractor covers `organization_id`.
    Path((_, server_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let organization_id = org.organization_id;
    let server = maintenance_server(&state, &organization_id, &server_id).await?;
    let result = setup::restart_server(&server).await?;
    if result.success {
        repository::set_state(&state, &server.id, ServerState::Offline).await?;
    }
    Ok(ok(maintenance_dto(result)))
}

pub async fn maintenance_prune_images_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerManage>,
    // The extractor covers `organization_id`.
    Path((_, server_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let organization_id = org.organization_id;
    let server = maintenance_server(&state, &organization_id, &server_id).await?;
    Ok(ok(maintenance_dto(setup::prune_images(&server).await?)))
}

pub async fn maintenance_restart_podman_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerManage>,
    // The extractor covers `organization_id`.
    Path((_, server_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let organization_id = org.organization_id;
    let server = maintenance_server(&state, &organization_id, &server_id).await?;
    Ok(ok(maintenance_dto(setup::restart_podman(&server).await?)))
}

pub async fn maintenance_check_disk_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerManage>,
    // The extractor covers `organization_id`.
    Path((_, server_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let organization_id = org.organization_id;
    let server = maintenance_server(&state, &organization_id, &server_id).await?;
    Ok(ok(maintenance_dto(setup::check_disk(&server).await?)))
}

pub async fn maintenance_create_swap_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerManage>,
    // The extractor covers `organization_id`.
    Path((_, server_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let organization_id = org.organization_id;
    let server = maintenance_server(&state, &organization_id, &server_id).await?;
    Ok(ok(maintenance_dto(setup::create_swap(&server).await?)))
}

/// Proves the server can actually pull an image and run a container, which a health check only
/// implies. The server's state is deliberately left alone: a registry the host cannot reach is
/// not a reason to stop deploying to it.
pub async fn test_deploy_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerManage>,
    // The extractor covers `organization_id`.
    Path((_, server_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let organization_id = org.organization_id;
    let server = repository::get_for_server_route(&state, &server_id, &organization_id)
        .await?
        .ok_or(AppError::ResourceNotFound)?;
    if server.kind != "ssh" {
        return Err(AppError::ValidationError(
            "Test deploys run on remote servers; the local server already runs litepod itself"
                .into(),
        ));
    }
    if server.host_key_fingerprint.is_none() {
        return Err(AppError::ValidationError(
            "Confirm the SSH host fingerprint before running a test deploy".into(),
        ));
    }
    let server = repository::ensure_host_key_known_hosts(&state, server).await?;
    let outcome = setup::test_deploy(&server).await?;
    Ok(ok(ServerTestDeployDto {
        success: outcome.exit_code == 0,
        image: setup::TEST_IMAGE.to_string(),
        exit_code: outcome.exit_code,
        stdout: outcome.stdout,
        stderr: outcome.stderr,
        duration_ms: outcome.duration_ms,
    }))
}

async fn execute_setup_job(state: SharedState, server_id: String, job_id: String) {
    let _ = repository::start_job(&state, &job_id, "starting").await;
    let _ = repository::set_state(&state, &server_id, ServerState::SettingUp).await;
    let result = async {
        // Background worker: ownership was already checked when the job was
        // queued, and there is no caller here to scope the lookup against.
        let server = repository::get(&state, &server_id)
            .await?
            .ok_or(AppError::ResourceNotFound)?;
        let server = repository::ensure_host_key_known_hosts(&state, server).await?;
        let facts = setup::run_setup(&server, |phase, message| {
            let state = state.clone();
            let job_id = job_id.clone();
            async move { repository::append_job_log(&state, &job_id, phase, &message).await }
        })
        .await?;
        let facts_json = serde_json::to_string(&facts)
            .map_err(|error| AppError::InternalError(error.to_string()))?;
        repository::set_facts_and_state(&state, &server_id, &facts_json, ServerState::Ready)
            .await?;
        Ok::<(), AppError>(())
    }
    .await;
    match result {
        Ok(()) => {
            let _ = repository::finish_job(&state, &job_id, true, "complete", None).await;
        }
        Err(error) => {
            let message = crate::servers::ssh::redact(&error.to_string());
            let _ = repository::append_job_log(&state, &job_id, "failed", &message).await;
            let _ = repository::set_state_error(
                &state,
                &server_id,
                ServerState::Degraded,
                "setup_failed",
                &message,
            )
            .await;
            let _ = repository::finish_job(
                &state,
                &job_id,
                false,
                "failed",
                Some(("setup_failed", &message)),
            )
            .await;
        }
    }
}

async fn queue_setup(
    state: SharedState,
    user: AuthUser,
    organization_id: String,
    server_id: String,
    kind: &'static str,
) -> Result<impl IntoResponse, AppError> {
    let server = repository::get_for_server_route(&state, &server_id, &organization_id)
        .await?
        .ok_or(AppError::ResourceNotFound)?;
    if server.kind != "ssh" {
        return Err(AppError::ValidationError(
            "The local server cannot be set up over SSH".into(),
        ));
    }
    if server.host_key_fingerprint.is_none() {
        return Err(AppError::ValidationError(
            "Confirm the SSH host fingerprint before Setup or Repair".into(),
        ));
    }
    if repository::has_active_setup_job(&state, &server_id).await? {
        return Err(AppError::ResourceInUse(
            "A Setup or Repair job is already running for this server".into(),
        ));
    }
    let job = repository::create_job(&state, &server_id, &user.user_id, kind).await?;
    tokio::spawn(execute_setup_job(state, server_id, job.id.clone()));
    Ok(accepted(job_dto(job)))
}

pub async fn setup_handler(
    State(state): State<SharedState>,
    Extension(_user): Extension<AuthUser>,
    RequireOrg(org, _): RequireOrg<perm::ServerManage>,
    // The extractor covers `organization_id`.
    Path((_, server_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let OrgScope {
        organization_id,
        user,
        ..
    } = org;
    queue_setup(state, user, organization_id, server_id, "setup").await
}

pub async fn repair_handler(
    State(state): State<SharedState>,
    Extension(_user): Extension<AuthUser>,
    RequireOrg(org, _): RequireOrg<perm::ServerManage>,
    // The extractor covers `organization_id`.
    Path((_, server_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let OrgScope {
        organization_id,
        user,
        ..
    } = org;
    queue_setup(state, user, organization_id, server_id, "repair").await
}

pub async fn list_jobs_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerRead>,
    // The extractor covers `organization_id`.
    Path((_, server_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let organization_id = org.organization_id;
    repository::get_for_server_route(&state, &server_id, &organization_id)
        .await?
        .ok_or(AppError::ResourceNotFound)?;
    Ok(ok(repository::list_jobs(&state, &server_id)
        .await?
        .into_iter()
        .map(job_dto)
        .collect::<Vec<_>>()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloud_registration_requires_public_edge_and_address() {
        assert!(
            validate_registration_ingress(true, IngressMode::PrivateRelay, Some("198.51.100.10"))
                .is_err()
        );
        assert!(validate_registration_ingress(true, IngressMode::PublicEdge, None).is_err());
        assert!(
            validate_registration_ingress(true, IngressMode::PublicEdge, Some("198.51.100.10"))
                .is_ok()
        );
    }

    #[test]
    fn self_hosted_registration_keeps_existing_remote_ingress_choices() {
        assert!(validate_registration_ingress(false, IngressMode::PrivateRelay, None).is_ok());
        assert!(validate_registration_ingress(false, IngressMode::PrivateOnly, None).is_ok());
    }

    #[test]
    fn reads_host_memory_from_meminfo() {
        assert_eq!(
            memory_bytes_from_meminfo("MemTotal:       16384 kB\n"),
            Some(16_777_216)
        );
    }

    #[test]
    fn cloud_geo_dto_anchors_the_fleet_on_the_control_plane() {
        let geo = ServerGeo {
            latitude: 50.4777,
            longitude: 12.3649,
            city: Some("Falkenstein".into()),
            country: Some("Germany".into()),
            timezone: Some("Europe/Berlin".into()),
            asn: Some("AS24940 HETZNER-AS Hetzner Online GmbH".into()),
        };
        let dto = cloud_geo_dto(geo);
        assert_eq!(dto.server_id, "cloud");
        assert_eq!(dto.kind, "cloud");
        assert!(dto.is_main, "the cloud hub must be the globe's anchor pin");
        assert_eq!(dto.state, ServerState::Ready);
        assert_eq!(dto.latitude, Some(50.4777));
        assert_eq!(dto.longitude, Some(12.3649));
        assert_eq!(dto.city.as_deref(), Some("Falkenstein"));
        assert_eq!(dto.unlocated_reason, None);
    }
}
