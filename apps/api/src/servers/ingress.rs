//! Server-specific ingress reconciliation.
//!
//! Public-edge servers own their Caddy configuration: a domain resolves
//! directly to that server's public address and its native Caddy proxies only
//! to the application's persisted loopback endpoint. Central Caddy is never
//! asked to dial a remote container name.

use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    sync::LazyLock,
};

use chrono::Utc;
use sqlx::Row;

use crate::{
    deployment::{domains, runtime_endpoints},
    dtos::{deployment::DomainDto, response::AppError},
    servers::{
        model::{IngressMode, ServerState, SshTarget},
        repository,
        runtime::ServerRuntime,
        setup,
        ssh::{SecretFile, SshExecutor, SshRelay},
    },
    state::state::SharedState,
};

const RELAY_PORT_MIN: i64 = 43_000;
const RELAY_PORT_MAX: i64 = 43_999;

struct RelayEndpoint {
    server_id: String,
    relay_port: u16,
}

// Keyed by relay_port (already globally unique, `UNIQUE(relay_port)`) rather
// than application_id, so an application with more than one remote endpoint
// (a Compose app with several publicly routed services) can hold more than
// one live SSH local-forward at once.
struct LiveRelay {
    target: SshTarget,
    remote_port: u16,
    credentials: (String, String, Option<String>),
    relay: SshRelay,
}

// ponytail: serialize SSH starts; use per-port locks if many simultaneous
// relay creations make the two-second SSH startup wait a bottleneck.
static RELAYS: LazyLock<tokio::sync::Mutex<HashMap<u16, LiveRelay>>> =
    LazyLock::new(|| tokio::sync::Mutex::new(HashMap::new()));

// Deployment promotion holds this while replacing endpoints and routes, so
// a background pass cannot put an old endpoint back after the switch.
static RECONCILIATION: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub(crate) async fn lock_reconciliation() -> tokio::sync::MutexGuard<'static, ()> {
    RECONCILIATION.lock().await
}

async fn relay_endpoint(
    state: &SharedState,
    application_id: &str,
    server_id: &str,
    container_port: u16,
) -> Result<RelayEndpoint, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let existing = sqlx::query(
        "SELECT server_id, relay_port FROM application_relay_endpoints WHERE application_id = ?1 AND container_port = ?2",
    )
    .bind(application_id)
    .bind(container_port as i64)
    .fetch_optional(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    if let Some(row) = existing {
        let existing_server: String = row
            .try_get(0)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?;
        if existing_server != server_id {
            return Err(AppError::DatabaseError(
                "Persisted relay endpoint does not match application server".into(),
            ));
        }
        let port: i64 = row
            .try_get(1)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?;
        return Ok(RelayEndpoint {
            server_id: existing_server,
            relay_port: u16::try_from(port)
                .map_err(|_| AppError::DatabaseError("Invalid persisted relay port".into()))?,
        });
    }

    for _ in RELAY_PORT_MIN..=RELAY_PORT_MAX {
        let rows = sqlx::query("SELECT relay_port FROM application_relay_endpoints")
            .fetch_all(&conn)
            .await
            .map_err(|error| AppError::DatabaseError(error.to_string()))?;
        let mut used = HashSet::new();
        for row in &rows {
            used.insert(
                row.try_get::<i64, _>(0)
                    .map_err(|error| AppError::DatabaseError(error.to_string()))?,
            );
        }
        let Some(port) = (RELAY_PORT_MIN..=RELAY_PORT_MAX).find(|port| !used.contains(port)) else {
            return Err(AppError::ValidationError(
                "No central SSH relay port is available; retry after removing an unused application"
                    .into(),
            ));
        };
        let now = Utc::now().timestamp_millis();
        match sqlx::query(
            "INSERT INTO application_relay_endpoints (application_id, server_id, relay_port, container_port, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
        )
        .bind(application_id)
        .bind(server_id)
        .bind(port)
        .bind(container_port as i64)
        .bind(now)
        .execute(&conn)
        .await
        {
            Ok(_) => {
                return Ok(RelayEndpoint {
                    server_id: server_id.to_string(),
                    relay_port: port as u16,
                });
            }
            Err(error) if error.to_string().to_ascii_lowercase().contains("unique") => continue,
            Err(error) => return Err(AppError::DatabaseError(error.to_string())),
        }
    }
    Err(AppError::ValidationError(
        "Could not allocate a central SSH relay port".into(),
    ))
}

async fn ensure_relay(
    server: &repository::ServerRecord,
    relay_port: u16,
    remote_port: u16,
) -> Result<(), AppError> {
    let target = SshTarget::parse(
        server.ssh_host.as_deref().ok_or_else(|| {
            AppError::InternalError("Private-relay server has no SSH host".into())
        })?,
        server.ssh_port.ok_or_else(|| {
            AppError::InternalError("Private-relay server has no SSH port".into())
        })?,
        server.ssh_user.as_deref().ok_or_else(|| {
            AppError::InternalError("Private-relay server has no SSH user".into())
        })?,
    )?;
    let credentials = (
        server.private_key_encrypted.clone().ok_or_else(|| {
            AppError::InternalError("Private-relay SSH credential is unavailable".into())
        })?,
        server.host_key_fingerprint.clone().ok_or_else(|| {
            AppError::ValidationError("Confirm the private-relay SSH host fingerprint first".into())
        })?,
        server.host_key_known_hosts.clone(),
    );
    let mut relays = RELAYS.lock().await;
    if let Some(live) = relays.get_mut(&relay_port)
        && live.target == target
        && live.remote_port == remote_port
        && live.credentials == credentials
        && live.relay.is_running()?
    {
        return Ok(());
    }
    if let Some(stale) = relays.remove(&relay_port) {
        stale.relay.stop().await;
    }
    let relay = SshExecutor::start_local_forward(
        &target,
        &credentials.0,
        &credentials.1,
        credentials.2.as_deref(),
        relay_port,
        remote_port,
    )
    .await?;
    relays.insert(
        relay_port,
        LiveRelay {
            target,
            remote_port,
            credentials,
            relay,
        },
    );
    Ok(())
}

async fn remove_relay(relay_port: u16) {
    let mut relays = RELAYS.lock().await;
    if let Some(live) = relays.remove(&relay_port) {
        live.relay.stop().await;
    }
}

fn is_internal_relay_host(host: &str) -> bool {
    !host.is_empty()
        && !matches!(
            host,
            "localhost" | "127.0.0.1" | "0.0.0.0" | "::1" | "host.containers.internal"
        )
}

fn central_relay_host(state: &SharedState) -> Result<String, AppError> {
    state
        .caddy_server_upstream
        .rsplit_once(':')
        .map(|(host, _)| host.to_string())
        .filter(|host| is_internal_relay_host(host))
        .ok_or_else(|| {
            AppError::InternalError("Caddy API upstream must include a host and port".into())
        })
}

fn route_fragment(host: &str, endpoint_port: u16) -> Result<String, AppError> {
    // Domain persistence already validates hosts, but this is a separate
    // privilege boundary: the string becomes a root-owned Caddyfile fragment.
    if host.is_empty()
        || host.contains(|character: char| {
            character.is_whitespace() || matches!(character, '{' | '}' | '`' | '"' | '\\' | '\0')
        })
    {
        return Err(AppError::ValidationError(
            "Refusing an unsafe hostname for the server Caddy route".into(),
        ));
    }
    // `.test` is reserved for local testing and cannot obtain a public ACME
    // certificate. Caddy's internal issuer keeps a public-edge route usable
    // on a LAN without exposing the service or requiring external DNS.
    let tls = if host.to_ascii_lowercase().ends_with(".test") {
        "\ttls internal\n"
    } else {
        ""
    };
    Ok(format!(
        "{host} {{\n{tls}\treverse_proxy 127.0.0.1:{endpoint_port}\n}}\n"
    ))
}

async fn application_server(
    state: &SharedState,
    application_id: &str,
) -> Result<repository::ServerRecord, AppError> {
    // During a group fan-out the ambient replica is the server to sync; for a
    // single-server app this resolves to its own `server_id`. A group app
    // outside a fan-out has no single server (its domains are pinned), so it
    // cannot be synced as a whole.
    let server_id = crate::servers::replica::target_server_id(state, application_id)
        .await?
        .ok_or_else(|| {
            AppError::ValidationError(
                "This application targets a server group; sync through a replica".into(),
            )
        })?;
    repository::get(state, &server_id)
        .await?
        .ok_or_else(|| AppError::DatabaseError("Application references a missing server".into()))
}

/// The server a domain routes to: its pinned `server_id` when set (group apps
/// must pin), otherwise the application's single server.
async fn domain_server(
    state: &SharedState,
    domain: &DomainDto,
) -> Result<repository::ServerRecord, AppError> {
    if let Some(server_id) = domain.server_id.as_deref() {
        return repository::get(state, server_id)
            .await?
            .ok_or_else(|| AppError::DatabaseError("Domain pins a missing server".into()));
    }
    application_server(state, &domain.application_id).await
}

/// The remote endpoint a domain's own Compose proxy route targets, for an
/// application with more than one publicly routed service. `None` for a
/// domain with no proxy route - a non-Compose remote deploy (git/static),
/// which only ever has one endpoint for the whole application - so the
/// caller resolves the endpoint using the application's current container port.
pub(crate) async fn domain_endpoint(
    state: &SharedState,
    domain: &DomainDto,
) -> Result<Option<runtime_endpoints::RuntimeEndpoint>, AppError> {
    let Some(server_id) =
        crate::servers::replica::target_server_id(state, &domain.application_id).await?
    else {
        return Ok(None);
    };
    let route = crate::deployment::compose::proxy_routes::list(state, &domain.application_id)
        .await?
        .into_iter()
        .find(|route| route.domain_id == domain.id);
    match route {
        Some(route) => {
            runtime_endpoints::get_for_service(
                state,
                &domain.application_id,
                &server_id,
                route.container_port,
            )
            .await
        }
        None => {
            let container_port =
                crate::deployment::operations::app_container_port(state, &domain.application_id)
                    .await?;
            runtime_endpoints::get_for_service(
                state,
                &domain.application_id,
                &server_id,
                u16::try_from(container_port).map_err(|_| {
                    AppError::DatabaseError("Invalid persisted application container port".into())
                })?,
            )
            .await
        }
    }
}

/// Push one already-persisted public domain to a public-edge server. A domain
/// can be configured before its first deployment, so the absence of a runtime
/// endpoint is intentionally a no-op; the deploy calls this again after its
/// loopback listener exists.
pub async fn sync_domain(state: &SharedState, domain: &DomainDto) -> Result<(), AppError> {
    if domain.is_local {
        return Ok(());
    }
    let server =
        repository::ensure_host_key_known_hosts(state, domain_server(state, domain).await?).await?;
    match server.ingress_mode {
        IngressMode::Central => Ok(()),
        IngressMode::PrivateOnly => Err(AppError::ValidationError(
            "Private-only servers cannot publish public domains".into(),
        )),
        IngressMode::PrivateRelay => {
            if state.dev_mode {
                return Err(AppError::ValidationError(
                    "Private-relay ingress requires litepod to run in its Podman network; it is disabled in local development so the relay can never bind a host-facing port"
                        .into(),
                ));
            }
            if server.state != ServerState::Ready {
                return Err(AppError::ResourceInUse(format!(
                    "Private-relay server '{}' is unavailable",
                    server.name
                )));
            }
            let Some(endpoint) = domain_endpoint(state, domain).await? else {
                return Ok(());
            };
            if endpoint.server_id != server.id {
                return Err(AppError::DatabaseError(
                    "Remote endpoint does not match application server".into(),
                ));
            }
            let relay = relay_endpoint(
                state,
                &domain.application_id,
                &server.id,
                endpoint.container_port,
            )
            .await?;
            if relay.server_id != server.id {
                return Err(AppError::DatabaseError(
                    "Relay endpoint does not match application server".into(),
                ));
            }
            ensure_relay(&server, relay.relay_port, endpoint.host_port).await?;
            let relay_host = central_relay_host(state)?;
            crate::deployment::caddy_route(
                state,
                &domains::route_id(&domain.id),
                &domain.host,
                &relay_host,
                relay.relay_port as i64,
                domain.https,
            )
            .await
        }
        IngressMode::PublicEdge => {
            if server
                .public_address
                .as_deref()
                .is_none_or(|address| address.trim().is_empty())
            {
                return Err(AppError::ValidationError(
                    "Public-edge servers need a public DNS/IP address before publishing domains"
                        .into(),
                ));
            }
            // A single-domain write reconciles the whole server: one SSH call,
            // and the routes.d directory can never drift from the database.
            sync_server_routes(state, &server).await.map(|_| ())
        }
    }
}

fn manifest_line(domain_id: &str, host: &str, loopback_port: u16) -> String {
    format!("{domain_id} {host} {loopback_port}\n")
}

/// A server provisioned before the declarative helper shipped has no
/// `sync-routes-check` arm and exits 64.
async fn helper_supports_sync_routes(server: &repository::ServerRecord) -> bool {
    setup::caddy_helper(server, &[OsString::from("sync-routes-check")])
        .await
        .is_ok()
}

/// Rebuild one public-edge server's entire `routes.d/` from the database in a
/// single helper call. Every routable non-local domain on the server becomes
/// one manifest line; anything else on disk is removed. Returns the number of
/// routes written. A stale helper falls back to the per-domain `install-route`
/// loop.
pub async fn sync_server_routes(
    state: &SharedState,
    server: &repository::ServerRecord,
) -> Result<usize, AppError> {
    if !matches!(server.ingress_mode, IngressMode::PublicEdge) {
        return Ok(0);
    }
    if server.state != ServerState::Ready {
        return Err(AppError::ResourceInUse(format!(
            "Public-edge server '{}' is unavailable",
            server.name
        )));
    }

    let targets = domains::edge_route_targets(state, &server.id).await?;

    let conn = crate::git::repositories::db::get_conn(state).await;
    let app_row = sqlx::query("SELECT id FROM applications WHERE server_id = ?1 LIMIT 1")
        .bind(&server.id)
        .fetch_optional(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    drop(conn);
    let Some(app_row) = app_row else {
        // No applications on this server: nothing LitePod manages here.
        return Ok(0);
    };
    let app_for_workspace: String = app_row
        .try_get(0)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;

    let certificates_supported =
        setup::caddy_helper(server, &[OsString::from("certificates-check")])
            .await
            .is_ok();
    let basic_auth_supported = setup::caddy_helper(server, &[OsString::from("basic-auth-check")])
        .await
        .is_ok();
    let mut lines = Vec::new();
    for target in &targets {
        let domain = domains::get_by_id(state, &target.domain_id).await?;
        let Some(endpoint) = domain_endpoint(state, &domain).await? else {
            continue; // not deployed yet; excluded from the manifest
        };
        if endpoint.server_id != server.id {
            return Err(AppError::DatabaseError(
                "Remote endpoint does not match application server".into(),
            ));
        }
        let certificate =
            crate::deployment::certificates::load(state, &domains::route_id(&domain.id)).await?;
        let auth = crate::deployment::basic_auth::load_payload_for_domain(state, &domain.id)
            .await?
            .map(|payload| crate::deployment::basic_auth::edge_token(&payload));
        if auth.is_some() && !(certificates_supported && basic_auth_supported) {
            return Err(AppError::ValidationError(
                "Repair this server to enable basic authentication".into(),
            ));
        }
        if !certificates_supported {
            if certificate.certificate_provider != "automatic" || !domain.https {
                return Err(AppError::ValidationError(
                    "Repair this server to enable HTTPS certificate settings".into(),
                ));
            }
            lines.push(manifest_line(&domain.id, &domain.host, endpoint.host_port));
        } else {
            let token = if certificate.certificate_provider == "custom" {
                crate::deployment::certificates::install_remote(
                    state,
                    server,
                    &domain.application_id,
                    &certificate,
                )
                .await?
            } else {
                "-".into()
            };
            // The auth token is appended only when set, so a helper that predates
            // basic auth still parses every unprotected line.
            let auth_field = auth.map(|token| format!(" {token}")).unwrap_or_default();
            lines.push(format!(
                "{} {} {} {} {} {}{}\n",
                domain.id,
                domain.host,
                endpoint.host_port,
                domain.https,
                certificate.certificate_provider,
                token,
                auth_field
            ));
        }
    }

    if !helper_supports_sync_routes(server).await {
        tracing::warn!(
            server = %server.name,
            "edge route helper predates sync-routes; using the per-domain fallback. Repair this server to enable declarative sync."
        );
        return sync_server_routes_legacy(state, server).await;
    }

    let manifest = SecretFile::write(&lines.concat())?;
    let runtime = ServerRuntime::for_application(state, &app_for_workspace).await?;
    let uploaded = runtime
        .upload_workspace_file(&app_for_workspace, manifest.path(), "caddy-routes.manifest")
        .await?
        .ok_or_else(|| AppError::InternalError("Edge manifest upload used Local runtime".into()))?;
    setup::caddy_helper(
        server,
        &[OsString::from("sync-routes"), OsString::from(uploaded)],
    )
    .await?;
    Ok(lines.len())
}

/// Pre-declarative fallback: push one `install-route` per domain. Used only
/// when the server's helper has no `sync-routes` arm. Kept until every server
/// is repaired.
async fn sync_server_routes_legacy(
    state: &SharedState,
    server: &repository::ServerRecord,
) -> Result<usize, AppError> {
    let targets = domains::edge_route_targets(state, &server.id).await?;
    let mut pushed = 0;
    for target in &targets {
        let domain = domains::get_by_id(state, &target.domain_id).await?;
        let Some(endpoint) = domain_endpoint(state, &domain).await? else {
            continue;
        };
        if endpoint.server_id != server.id {
            return Err(AppError::DatabaseError(
                "Remote endpoint does not match application server".into(),
            ));
        }
        let fragment = SecretFile::write(&route_fragment(&domain.host, endpoint.host_port)?)?;
        let runtime = ServerRuntime::for_application(state, &domain.application_id).await?;
        let filename = format!("caddy-{}.caddy", domain.id);
        let source = runtime
            .upload_workspace_file(&domain.application_id, fragment.path(), &filename)
            .await?
            .ok_or_else(|| {
                AppError::InternalError("Public-edge route upload used Local runtime".into())
            })?;
        setup::caddy_helper(
            server,
            &[
                OsString::from("install-route"),
                OsString::from(source),
                OsString::from(&domain.id),
            ],
        )
        .await?;
        pushed += 1;
    }
    Ok(pushed)
}

/// Sync every non-local domain after a remote runtime has obtained its
/// endpoint. The caller uses this after a successful container start, so a
/// Caddy route is never written for a listener that has not been created.
pub async fn sync_application(state: &SharedState, application_id: &str) -> Result<(), AppError> {
    let server = repository::ensure_host_key_known_hosts(
        state,
        application_server(state, application_id).await?,
    )
    .await?;
    if matches!(server.ingress_mode, IngressMode::PublicEdge) {
        return sync_server_routes(state, &server).await.map(|_| ());
    }
    // Central and private-relay apps still reconcile one domain at a time.
    for domain in domains::list(state, application_id)
        .await?
        .into_iter()
        .filter(|domain| !domain.is_local)
    {
        sync_domain(state, &domain).await?;
    }
    Ok(())
}

/// Recreates remote Caddy fragments and private SSH forwarding processes after
/// litepod restarts. Central Caddy configuration can survive longer than the API
/// container that owned a relay, so this deliberately runs at boot instead of
/// waiting for the next deploy.
pub async fn reconcile_running(state: &SharedState) -> Result<(usize, usize), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(
        "SELECT DISTINCT s.id FROM application_deployments d \
         JOIN applications a ON a.id = d.application_id \
         JOIN servers s ON s.id = a.server_id \
         WHERE d.status = 'running' AND s.kind = 'ssh'",
    )
    .fetch_all(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    let mut server_ids = Vec::new();
    for row in &rows {
        server_ids.push(
            row.try_get::<String, _>(0)
                .map_err(|error| AppError::DatabaseError(error.to_string()))?,
        );
    }
    let mut reconciled = 0;
    let mut failed = 0;
    for server_id in server_ids {
        let outcome = async {
            let server = repository::get(state, &server_id).await?.ok_or_else(|| {
                AppError::DatabaseError("Running app references a missing server".into())
            })?;
            let server = repository::ensure_host_key_known_hosts(state, server).await?;
            match server.ingress_mode {
                IngressMode::PrivateRelay => {
                    let (ok, failed) = reconcile_private_relays(state, Some(&server.id)).await?;
                    return Ok((ok, failed));
                }
                IngressMode::PublicEdge => {
                    let _guard = lock_reconciliation().await;
                    sync_server_routes(state, &server).await?;
                }
                _ => return Ok((0, 0)),
            }
            Ok::<_, AppError>((1, 0))
        }
        .await;
        match outcome {
            Ok((ok, errors)) => {
                reconciled += ok;
                failed += errors;
            }
            Err(error) => {
                // Server state belongs to `servers::monitor`, which probes the host
                // directly. A failed route sync is evidence about one server's
                // ingress, not proof the host is unhealthy, and demoting on it left
                // a briefly unreachable VM unusable until someone clicked Health check.
                failed += 1;
                tracing::warn!(server_id, error = %error, "could not reconcile remote ingress");
            }
        }
    }
    Ok((reconciled, failed))
}

async fn running_relay_domains(
    state: &SharedState,
    server_id: Option<&str>,
) -> Result<Vec<String>, AppError> {
    sqlx::query_scalar(
        "SELECT domains.id FROM application_domains domains \
         JOIN applications a ON a.id = domains.application_id \
         JOIN application_deployments d ON d.application_id = a.id \
         JOIN servers s ON s.id = a.server_id \
         WHERE d.status = 'running' AND s.kind = 'ssh' \
         AND s.ingress_mode = 'private_relay' AND lower(trim(domains.host)) NOT IN ('localhost', '127.0.0.1', '::1') \
         AND (?1 IS NULL OR s.id = ?1)",
    )
    .bind(server_id)
    .fetch_all(&state.db)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))
}

async fn reconcile_private_relays(
    state: &SharedState,
    server_id: Option<&str>,
) -> Result<(usize, usize), AppError> {
    let domain_ids = running_relay_domains(state, server_id).await?;
    let mut reconciled = 0;
    let mut failed = 0;
    for domain_id in domain_ids {
        let _guard = lock_reconciliation().await;
        let outcome = async {
            // Recheck after waiting for a deployment promotion, stop, or delete.
            let running: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM application_domains domains \
                 JOIN application_deployments d ON d.application_id = domains.application_id \
                 WHERE domains.id = ?1 AND d.status = 'running')",
            )
            .bind(&domain_id)
            .fetch_one(&state.db)
            .await
            .map_err(|error| AppError::DatabaseError(error.to_string()))?;
            if !running {
                return Ok(false);
            }
            let domain = domains::get_by_id(state, &domain_id).await?;
            sync_domain(state, &domain).await?;
            Ok::<_, AppError>(true)
        }
        .await;
        match outcome {
            Ok(true) => reconciled += 1,
            Ok(false) => (),
            Err(error) => {
                failed += 1;
                tracing::warn!(domain_id, error = %error, "could not restore private relay");
            }
        }
    }
    Ok((reconciled, failed))
}

pub fn spawn_relay_supervisor(state: SharedState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        interval.tick().await; // boot reconciliation already runs immediately
        loop {
            interval.tick().await;
            if let Err(error) = reconcile_private_relays(&state, None).await {
                tracing::warn!(error = %error, "could not supervise private relays");
            }
        }
    });
}

/// Remove a route from a public-edge server. This is best-effort at the domain
/// delete boundary so a transient remote outage cannot leave the database row
/// undeleted; the next server repair/reconcile can remove a stale fragment.
pub async fn remove_domain(
    state: &SharedState,
    application_id: &str,
    domain_id: &str,
    hostname: &str,
) -> Result<(), AppError> {
    let server = repository::ensure_host_key_known_hosts(
        state,
        application_server(state, application_id).await?,
    )
    .await?;
    if matches!(server.ingress_mode, IngressMode::PrivateRelay) {
        crate::deployment::remove_caddy_route_config(
            state,
            &domains::route_id(domain_id),
            hostname,
        )
        .await?;
        if domains::list(state, application_id).await?.is_empty() {
            // An application can hold more than one relay now (one per
            // publicly routed service); once it has no domains left, stop
            // every one of them, same as the single relay this replaced.
            let conn = crate::git::repositories::db::get_conn(state).await;
            let ports = sqlx::query(
                "SELECT relay_port FROM application_relay_endpoints WHERE application_id = ?1",
            )
            .bind(application_id)
            .fetch_all(&conn)
            .await
            .map_err(|error| AppError::DatabaseError(error.to_string()))?;
            for row in &ports {
                let relay_port: i64 = row
                    .try_get(0)
                    .map_err(|error| AppError::DatabaseError(error.to_string()))?;
                remove_relay(relay_port as u16).await;
            }
        }
        return Ok(());
    }
    if !matches!(server.ingress_mode, IngressMode::PublicEdge) {
        return Ok(());
    }
    // The row is already deleted by the time this runs, so a whole-server
    // resync simply drops the fragment: no separate remove-route to fail
    // silently and orphan the file.
    sync_server_routes(state, &server).await.map(|_| ())
}

pub async fn public_edge_healthy(server: &repository::ServerRecord) -> Result<(), AppError> {
    if !matches!(server.ingress_mode, IngressMode::PublicEdge) {
        return Ok(());
    }
    setup::caddy_helper(server, &[OsString::from("health")])
        .await
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::{is_internal_relay_host, manifest_line, route_fragment};

    #[tokio::test]
    async fn relay_recovery_only_visits_running_public_domains_and_isolates_failures() {
        let (state, path) = crate::testing::test_state().await;
        let (_, _, application_id) =
            crate::testing::seed_application(&state, "organization-a").await;
        sqlx::query("UPDATE servers SET ingress_mode = 'private_relay', host_key_known_hosts = 'fixture' WHERE id = 'organization-a-server'")
            .execute(&state.db).await.unwrap();
        sqlx::query("INSERT INTO application_deployments (application_id, status, container_name, hostname, updated_at) VALUES (?1, 'running', 'app', 'app.example.com', 0)")
            .bind(&application_id).execute(&state.db).await.unwrap();
        for (id, local) in [
            ("domain-a", false),
            ("domain-b", false),
            ("domain-local", true),
        ] {
            sqlx::query("INSERT INTO application_domains (id, application_id, host, port, created_at, updated_at) VALUES (?1, ?2, ?3, 443, 0, 0)")
                .bind(id).bind(&application_id).bind(if local { "localhost".to_string() } else { format!("{id}.example.com") })
                .execute(&state.db).await.unwrap();
        }
        assert_eq!(
            super::running_relay_domains(&state, None)
                .await
                .unwrap()
                .len(),
            2
        );
        // No runtime endpoint yet: both domains are harmless no-ops, not one
        // server-level no-op that silently skips private-relay reconciliation.
        assert_eq!(super::reconcile_running(&state).await.unwrap(), (2, 0));
        sqlx::query("UPDATE servers SET state = 'offline' WHERE id = 'organization-a-server'")
            .execute(&state.db)
            .await
            .unwrap();
        assert_eq!(
            super::reconcile_private_relays(&state, None).await.unwrap(),
            (0, 2)
        );
        for status in ["stopped", "deploying", "not_deployed", "failed"] {
            sqlx::query("UPDATE application_deployments SET status = ?1")
                .bind(status)
                .execute(&state.db)
                .await
                .unwrap();
            assert!(
                super::running_relay_domains(&state, None)
                    .await
                    .unwrap()
                    .is_empty()
            );
            assert_eq!(
                super::reconcile_private_relays(&state, None).await.unwrap(),
                (0, 0)
            );
        }
        state.db.close().await;
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn manifest_line_is_id_host_port_with_no_extra_whitespace() {
        let line = manifest_line("01a0-1111", "kong8001.example.com", 42019);
        assert_eq!(line, "01a0-1111 kong8001.example.com 42019\n");
        let fields: Vec<&str> = line.trim_end().split(' ').collect();
        assert_eq!(fields.len(), 3, "sync-routes parses exactly three fields");
    }

    #[test]
    fn route_fragment_is_loopback_only() {
        let fragment = route_fragment("gitea.example.com", 42_123).unwrap();
        assert!(fragment.contains("127.0.0.1:42123"));
        assert!(!fragment.contains("0.0.0.0"));
        assert!(!fragment.contains("tls internal"));
    }

    #[test]
    fn route_fragment_uses_internal_tls_for_test_domains() {
        let fragment = route_fragment("uptime.test", 42_000).unwrap();
        assert!(fragment.contains("tls internal"));
        assert!(fragment.contains("reverse_proxy 127.0.0.1:42000"));
    }

    #[test]
    fn route_fragment_rejects_caddy_syntax() {
        assert!(route_fragment("example.com { respond", 42_123).is_err());
    }

    #[test]
    fn relay_targets_are_never_host_loopback() {
        assert!(is_internal_relay_host("litepod-api"));
        assert!(!is_internal_relay_host("localhost"));
        assert!(!is_internal_relay_host("127.0.0.1"));
        assert!(!is_internal_relay_host("host.containers.internal"));
    }
}
