use std::net::IpAddr;

use chrono::Utc;
use sqlx::Row;
use uuid::Uuid;

use crate::{
    dtos::{deployment::*, response::AppError},
    servers::model::{IngressMode, ServerState},
    state::state::SharedState,
};

use super::caddy_sync::*;
use super::validate::*;

static DOMAIN_WRITE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub(super) fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

/// Validates a requested domain pin: when set, it must be one of the
/// application's target servers (its single server, or a member of its group).
/// A group-targeted application must always pin; a single-server application
/// may leave the pin `None` (the server is derived from the application).
async fn resolve_domain_server(
    state: &SharedState,
    application_id: &str,
    requested: Option<String>,
) -> Result<Option<String>, AppError> {
    match requested {
        Some(requested) => {
            let targets =
                crate::servers::groups::resolve_target_servers(state, application_id).await?;
            if !targets.iter().any(|server| server.id == requested) {
                return Err(AppError::ValidationError(
                    "Pin the domain to a server the application deploys to".into(),
                ));
            }
            Ok(Some(requested))
        }
        None => {
            let conn = crate::git::repositories::db::get_conn(state).await;
            let row = sqlx::query("SELECT server_group_id FROM applications WHERE id = ?1")
                .bind(application_id)
                .fetch_optional(&conn)
                .await
                .map_err(|error| AppError::DatabaseError(error.to_string()))?
                .ok_or(AppError::ApplicationNotFound)?;
            let group_id: Option<String> = row
                .try_get(0)
                .map_err(|error| AppError::DatabaseError(error.to_string()))?;
            if group_id.is_some() {
                return Err(AppError::ValidationError(
                    "This application targets a server group; pin the domain to a server first"
                        .into(),
                ));
            }
            Ok(None)
        }
    }
}

/// The concrete server a domain routes to: its pin when set, otherwise the
/// application's single server. The pin is enforced in `resolve_domain_server`,
/// so the fallback here only serves single-server applications.
async fn effective_domain_server(
    state: &SharedState,
    application_id: &str,
    server_id: Option<String>,
) -> Result<String, AppError> {
    if let Some(server_id) = server_id {
        return Ok(server_id);
    }
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query("SELECT server_id FROM applications WHERE id = ?1")
        .bind(application_id)
        .fetch_optional(&conn)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?
        .ok_or(AppError::ApplicationNotFound)?;
    row.try_get::<Option<String>, _>(0)
        .map_err(|error| AppError::DatabaseError(error.to_string()))?
        .ok_or_else(|| {
            AppError::ValidationError(
                "This domain's application targets a server group; pin the domain to a server first"
                    .into(),
            )
        })
}

pub(super) const SELECT_COLUMNS: &str =
    "id, application_id, host, port, https, is_primary, created_at, updated_at, server_id";

pub(super) fn row_to_dto(row: &sqlx::sqlite::SqliteRow) -> Result<DomainDto, AppError> {
    let read = |index: usize| -> Result<String, AppError> {
        row.try_get(index)
            .map_err(|e| AppError::DatabaseError(e.to_string()))
    };
    let read_int = |index: usize| -> Result<i64, AppError> {
        row.try_get(index)
            .map_err(|e| AppError::DatabaseError(e.to_string()))
    };
    let host = read(2)?;
    let port = read_int(3)?;
    let https = read_int(4)? != 0;
    Ok(DomainDto {
        certificate_id: None,
        certificate_name: None,
        certificate_provider: "automatic".into(),
        has_custom_certificate: false,
        id: read(0)?,
        application_id: read(1)?,
        url: domain_url(&host, port, https),
        is_local: is_local(&host),
        required_dns_target: None,
        ingress_mode: "central".into(),
        route_health: "pending".into(),
        route_health_reason: None,
        host,
        port,
        https,
        is_primary: read_int(5)? != 0,
        server_id: row
            .try_get::<Option<String>, _>(8)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        created_at: read_int(6)?,
        updated_at: read_int(7)?,
    })
}

pub(super) async fn with_ingress_details(
    state: &SharedState,
    mut domain: DomainDto,
) -> Result<DomainDto, AppError> {
    let certificate = crate::deployment::certificates::load(state, &route_id(&domain.id)).await?;
    domain.certificate_id = certificate.certificate_id;
    domain.certificate_name = certificate.certificate_name;
    domain.certificate_provider = certificate.certificate_provider;
    domain.has_custom_certificate = certificate.has_custom_certificate;
    let conn = crate::git::repositories::db::get_conn(state).await;
    let server_id = match domain.server_id.clone() {
        Some(server_id) => server_id,
        None => {
            let row = sqlx::query("SELECT server_id FROM applications WHERE id = ?1")
                .bind(&domain.application_id)
                .fetch_optional(&conn)
                .await
                .map_err(|error| AppError::DatabaseError(error.to_string()))?
                .ok_or(AppError::ApplicationNotFound)?;
            row.try_get::<Option<String>, _>(0)
                .map_err(|error| AppError::DatabaseError(error.to_string()))?
                .ok_or_else(|| {
                    AppError::ValidationError(
                        "This domain's application targets a server group; pin the domain to a server first"
                            .into(),
                    )
                })?
        }
    };
    let row =
        sqlx::query("SELECT kind, ingress_mode, public_address, state FROM servers WHERE id = ?1")
            .bind(&server_id)
            .fetch_optional(&conn)
            .await
            .map_err(|error| AppError::DatabaseError(error.to_string()))?
            .ok_or(AppError::ApplicationNotFound)?;
    let kind: String = row
        .try_get(0)
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    let mode: IngressMode = row
        .try_get::<String, _>(1)
        .map_err(|error| AppError::DatabaseError(error.to_string()))?
        .parse()?;
    let public_address: Option<String> = row
        .try_get(2)
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    let server_state: ServerState = row
        .try_get::<String, _>(3)
        .map_err(|error| AppError::DatabaseError(error.to_string()))?
        .parse()?;

    if domain.is_local {
        domain.ingress_mode = if kind == "local" {
            "central"
        } else {
            mode.as_str()
        }
        .into();
        domain.route_health = "ready".into();
        return Ok(domain);
    }
    domain.ingress_mode = mode.as_str().into();
    match mode {
        IngressMode::Central => {
            domain.required_dns_target = Some(state.public_ip.clone());
            domain.route_health = "ready".into();
        }
        IngressMode::PublicEdge => {
            domain.required_dns_target =
                public_address.filter(|address| !address.trim().is_empty());
            if server_state != ServerState::Ready {
                domain.route_health = "unavailable".into();
                domain.route_health_reason =
                    Some("Selected public-edge server is unavailable".into());
            } else if crate::servers::ingress::domain_endpoint(state, &domain)
                .await?
                .is_some()
            {
                domain.route_health = "ready".into();
            } else {
                domain.route_health = "pending".into();
                domain.route_health_reason =
                    Some("Deploy the application to create its loopback route".into());
            }
        }
        IngressMode::PrivateRelay => {
            domain.required_dns_target = Some(state.public_ip.clone());
            if server_state != ServerState::Ready {
                domain.route_health = "unavailable".into();
                domain.route_health_reason =
                    Some("Selected private-relay server is unavailable".into());
            } else if crate::servers::ingress::domain_endpoint(state, &domain)
                .await?
                .is_some()
            {
                domain.route_health = "ready".into();
            } else {
                domain.route_health = "pending".into();
                domain.route_health_reason =
                    Some("Deploy the application to start its SSH relay".into());
            }
        }
        IngressMode::PrivateOnly => {
            domain.route_health = "private_only".into();
            domain.route_health_reason = Some("This server does not permit public domains".into());
        }
    }
    Ok(domain)
}

/// Maps a unique-index violation to a conflict, matching how `update_port` read
/// libsql errors before domains existed.
pub(super) fn map_write_error(error: sqlx::Error) -> AppError {
    let message = error.to_string();
    if message.to_ascii_lowercase().contains("unique") {
        AppError::AlreadyExists
    } else {
        AppError::DatabaseError(message)
    }
}

/// For a local host the `(host, port)` unique index can only trip on the port,
/// so naming it makes the failure actionable in the domain dialog. A proxied
/// host collides on the hostname instead, and stays a plain conflict.
pub(super) fn map_domain_write_error(error: sqlx::Error, host: &str, port: i64) -> AppError {
    if !is_local(host) {
        return map_write_error(error);
    }
    let message = error.to_string();
    if message.to_ascii_lowercase().contains("unique") {
        AppError::ValidationError(format!("Port {port} is already used by another domain"))
    } else {
        AppError::DatabaseError(message)
    }
}

pub async fn list(state: &SharedState, application_id: &str) -> Result<Vec<DomainDto>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(&format!("SELECT {SELECT_COLUMNS} FROM application_domains WHERE application_id = ?1 ORDER BY created_at, id"))
        .bind(application_id)
        .fetch_all(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let mut domains = Vec::new();
    for row in &rows {
        domains.push(with_ingress_details(state, row_to_dto(row)?).await?);
    }
    Ok(domains)
}

pub async fn get(
    state: &SharedState,
    application_id: &str,
    domain_id: &str,
) -> Result<DomainDto, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(&format!(
        "SELECT {SELECT_COLUMNS} FROM application_domains WHERE id = ?1 AND application_id = ?2"
    ))
    .bind(domain_id)
    .bind(application_id)
    .fetch_optional(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?
    .ok_or(AppError::ResourceNotFound)?;
    with_ingress_details(state, row_to_dto(&row)?).await
}

pub(super) fn validation_target(domain: &DomainDto) -> Result<&str, AppError> {
    if domain.is_local || domain.host.ends_with(".sslip.io") {
        return Err(AppError::ValidationError(
            "This domain does not require DNS validation".into(),
        ));
    }
    domain.required_dns_target.as_deref().ok_or_else(|| {
        AppError::ValidationError(
            "The selected server needs a public address before DNS can be validated".into(),
        )
    })
}

/// Resolves one persisted application domain and compares it to the address
/// its server expects (`required_dns_target`). This is deliberately read-only.
pub async fn validate_dns(
    state: &SharedState,
    application_id: &str,
    domain_id: &str,
) -> Result<DomainDnsValidationDto, AppError> {
    let domain = get(state, application_id, domain_id).await?;
    let target = validation_target(&domain)?.to_string();
    Ok(crate::deployment::dns_validation::validate(&domain.host, &[target]).await)
}

/// The address a public host routed through `server` must point to. Mirrors
/// `with_ingress_details`.
fn server_dns_target(
    state: &SharedState,
    server: &crate::servers::repository::ServerRecord,
) -> Option<String> {
    match server.ingress_mode {
        IngressMode::Central | IngressMode::PrivateRelay => Some(state.public_ip.clone()),
        IngressMode::PublicEdge => server
            .public_address
            .clone()
            .filter(|address| !address.trim().is_empty()),
        IngressMode::PrivateOnly => None,
    }
}

/// Same check as [`validate_dns`] for a hostname that is not saved yet. A
/// group app without a pin expects any of its members' addresses.
pub async fn validate_host_dns(
    state: &SharedState,
    application_id: &str,
    host: &str,
    server_id: Option<&str>,
) -> Result<DomainDnsValidationDto, AppError> {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    check_public_hostname(&host)?;
    let mut servers = crate::servers::groups::resolve_target_servers(state, application_id).await?;
    if let Some(server_id) = server_id {
        servers.retain(|server| server.id == server_id);
        if servers.is_empty() {
            return Err(AppError::ValidationError(
                "Pin the domain to a server the application deploys to".into(),
            ));
        }
    }
    let mut targets = servers
        .iter()
        .filter_map(|server| server_dns_target(state, server))
        .collect::<Vec<_>>();
    targets.sort_unstable();
    targets.dedup();
    if targets.is_empty() {
        return Err(AppError::ValidationError(
            "The selected server needs a public address before DNS can be validated".into(),
        ));
    }
    Ok(crate::deployment::dns_validation::validate(&host, &targets).await)
}

pub async fn create(
    state: &SharedState,
    application_id: &str,
    request: CreateDomainRequest,
) -> Result<DomainDto, AppError> {
    create_with_origin(state, application_id, request, false).await
}

pub(super) async fn create_with_origin(
    state: &SharedState,
    application_id: &str,
    request: CreateDomainRequest,
    is_generated: bool,
) -> Result<DomainDto, AppError> {
    let _guard = DOMAIN_WRITE_LOCK.lock().await;
    let _certificates = crate::deployment::certificates::INVENTORY_LOCK.lock().await;
    create_with_origin_locked(state, application_id, request, is_generated).await
}

/// [`create_with_origin`] without acquiring [`DOMAIN_WRITE_LOCK`]. Callers that
/// must keep a port allocation and its insert atomic (see [`generate_with`])
/// hold the lock across both and call this instead, so re-locking does not
/// deadlock.
async fn create_with_origin_locked(
    state: &SharedState,
    application_id: &str,
    request: CreateDomainRequest,
    is_generated: bool,
) -> Result<DomainDto, AppError> {
    let host = request.host.trim().to_ascii_lowercase();
    let https = request.https.unwrap_or(false);
    validate(state, &host, request.port, https)?;
    let server_id = resolve_domain_server(state, application_id, request.server_id.clone()).await?;
    let effective_server_id =
        effective_domain_server(state, application_id, server_id.clone()).await?;
    reject_reserved_host(state, &host).await?;
    validate_domain_for_application(state, &effective_server_id, &host).await?;
    reject_conflicting_public_host(state, &effective_server_id, &host, request.port, None).await?;

    let conn = crate::git::repositories::db::get_conn(state).await;
    let existing = list(state, application_id).await?;
    let is_primary = request.is_primary.unwrap_or(false) || existing.is_empty();
    let certificate = crate::deployment::certificates::prepare_for_state(
        state,
        &Default::default(),
        &request.certificate,
        &host,
        https,
    )
    .await?;
    let mut tx = conn.begin().await.map_err(map_write_error)?;
    if is_primary {
        sqlx::query("UPDATE application_domains SET is_primary=0 WHERE application_id=?")
            .bind(application_id)
            .execute(&mut *tx)
            .await
            .map_err(map_write_error)?;
    }
    let id = Uuid::now_v7().to_string();
    let timestamp = now_ms();
    sqlx::query(
        "INSERT INTO application_domains (id, application_id, host, port, https, is_primary, created_at, updated_at, is_generated, server_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
    )
    .bind(id.clone())
    .bind(application_id)
    .bind(host.clone())
    .bind(request.port)
    .bind(https as i64)
    .bind(is_primary as i64)
    .bind(timestamp)
    .bind(timestamp)
    .bind(is_generated as i64)
    .bind(server_id)
    .execute(&mut *tx)
    .await
    .map_err(|error| map_domain_write_error(error, &host, request.port))?;
    crate::deployment::certificates::save_on(&mut tx, &route_id(&id), &certificate).await?;
    tx.commit().await.map_err(map_write_error)?;
    let created = get(state, application_id, &id).await?;
    if let Err(error) = reconcile_after_write(state, application_id, &created, None).await {
        let mut tx = conn.begin().await.map_err(map_write_error)?;
        sqlx::query("DELETE FROM application_domains WHERE id=?")
            .bind(&id)
            .execute(&mut *tx)
            .await
            .map_err(map_write_error)?;
        sqlx::query("DELETE FROM domain_certificates WHERE route_id=?")
            .bind(route_id(&id))
            .execute(&mut *tx)
            .await
            .map_err(map_write_error)?;
        if let Some(primary) = existing.iter().find(|domain| domain.is_primary) {
            sqlx::query("UPDATE application_domains SET is_primary=1 WHERE id=?")
                .bind(&primary.id)
                .execute(&mut *tx)
                .await
                .map_err(map_write_error)?;
        }
        tx.commit().await.map_err(map_write_error)?;
        return Err(error);
    }
    Ok(created)
}

pub async fn update(
    state: &SharedState,
    application_id: &str,
    domain_id: &str,
    request: UpdateDomainRequest,
) -> Result<DomainDto, AppError> {
    let _guard = DOMAIN_WRITE_LOCK.lock().await;
    let _certificates = crate::deployment::certificates::INVENTORY_LOCK.lock().await;
    let existing = list(state, application_id).await?;
    let current = get(state, application_id, domain_id).await?;
    let previous_certificate =
        crate::deployment::certificates::load(state, &route_id(domain_id)).await?;
    let host = request
        .host
        .map(|value| value.trim().to_ascii_lowercase())
        .unwrap_or(current.host.clone());
    let port = request.port.unwrap_or(current.port);
    let https = request.https.unwrap_or(current.https);
    validate(state, &host, port, https)?;
    // A pin omitted on update keeps the existing pin; an explicit one is validated.
    let server_id = match request.server_id.clone() {
        Some(_) => resolve_domain_server(state, application_id, request.server_id).await?,
        None => current.server_id.clone(),
    };
    let effective_server_id =
        effective_domain_server(state, application_id, server_id.clone()).await?;
    reject_reserved_host(state, &host).await?;
    validate_domain_for_application(state, &effective_server_id, &host).await?;
    reject_conflicting_public_host(state, &effective_server_id, &host, port, Some(domain_id))
        .await?;

    let conn = crate::git::repositories::db::get_conn(state).await;
    let certificate = crate::deployment::certificates::prepare_for_state(
        state,
        &previous_certificate,
        &request.certificate,
        &host,
        https,
    )
    .await?;
    let is_primary = request.is_primary.unwrap_or(current.is_primary);
    let mut tx = conn.begin().await.map_err(map_write_error)?;
    if is_primary && !current.is_primary {
        sqlx::query("UPDATE application_domains SET is_primary=0 WHERE application_id=?")
            .bind(application_id)
            .execute(&mut *tx)
            .await
            .map_err(map_write_error)?;
    }

    sqlx::query(
        "UPDATE application_domains SET host = ?1, port = ?2, https = ?3, is_primary = ?4, server_id = ?5, updated_at = ?6 WHERE id = ?7 AND application_id = ?8",
    )
    .bind(host.clone())
    .bind(port)
    .bind(https as i64)
    .bind(is_primary as i64)
    .bind(server_id)
    .bind(now_ms())
    .bind(domain_id)
    .bind(application_id)
    .execute(&mut *tx)
    .await
    .map_err(|error| map_domain_write_error(error, &host, port))?;
    crate::deployment::certificates::save_on(&mut tx, &route_id(domain_id), &certificate).await?;
    tx.commit().await.map_err(map_write_error)?;
    let updated = get(state, application_id, domain_id).await?;
    if let Err(error) = reconcile_after_write(state, application_id, &updated, Some(&current)).await
    {
        let mut tx = conn.begin().await.map_err(map_write_error)?;
        crate::deployment::certificates::save_on(
            &mut tx,
            &route_id(domain_id),
            &previous_certificate,
        )
        .await?;
        sqlx::query("UPDATE application_domains SET is_primary=0 WHERE application_id=?")
            .bind(application_id)
            .execute(&mut *tx)
            .await
            .map_err(map_write_error)?;
        sqlx::query("UPDATE application_domains SET host=?, port=?, https=?, is_primary=?, updated_at=? WHERE id=?")
            .bind(&current.host).bind(current.port).bind(current.https as i64).bind(current.is_primary as i64).bind(current.updated_at).bind(domain_id).execute(&mut *tx).await.map_err(map_write_error)?;
        if let Some(primary) = existing.iter().find(|domain| domain.is_primary) {
            sqlx::query("UPDATE application_domains SET is_primary=1 WHERE id=?")
                .bind(&primary.id)
                .execute(&mut *tx)
                .await
                .map_err(map_write_error)?;
        }
        tx.commit().await.map_err(map_write_error)?;
        let _ = reconcile_after_write(state, application_id, &current, Some(&updated)).await;
        return Err(error);
    }
    if updated.host != current.host
        || updated.https != current.https
        || updated.is_primary != current.is_primary
    {
        mark_stale_if_marker_route(state, application_id, domain_id).await?;
    }
    Ok(updated)
}

/// A domain behind a route that an env marker reads (`{{url:8080}}`) changes
/// that variable's value on the next deploy, so the running app is now out of
/// date. Call before the domain's proxy route row is deleted.
async fn marker_route_of(
    state: &SharedState,
    application_id: &str,
    domain_id: &str,
) -> Result<bool, AppError> {
    let Some(route) = crate::deployment::compose::proxy_routes::list(state, application_id)
        .await?
        .into_iter()
        .find(|route| route.domain_id == domain_id)
    else {
        return Ok(false);
    };
    crate::deployment::compose::marker_routes::references_route(
        state,
        application_id,
        &route.service,
        route.container_port,
    )
    .await
}

async fn mark_stale_if_marker_route(
    state: &SharedState,
    application_id: &str,
    domain_id: &str,
) -> Result<(), AppError> {
    if marker_route_of(state, application_id, domain_id).await? {
        mark_stale(state, application_id, true).await?;
    }
    Ok(())
}

pub async fn delete(
    state: &SharedState,
    application_id: &str,
    domain_id: &str,
) -> Result<(), AppError> {
    let _guard = DOMAIN_WRITE_LOCK.lock().await;
    let _certificates = crate::deployment::certificates::INVENTORY_LOCK.lock().await;
    let removed = get(state, application_id, domain_id).await?;
    // Read before the delete: the proxy route row goes away with the domain.
    let read_by_marker = marker_route_of(state, application_id, domain_id).await?;
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("DELETE FROM application_domains WHERE id = ?1 AND application_id = ?2")
        .bind(domain_id)
        .bind(application_id)
        .execute(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;

    sqlx::query("DELETE FROM domain_certificates WHERE route_id=?")
        .bind(route_id(domain_id))
        .execute(&conn)
        .await
        .map_err(map_write_error)?;

    // Promote the oldest survivor so an application never loses its public URL
    // while it still has domains.
    if removed.is_primary {
        if let Some(next) = list(state, application_id).await?.into_iter().next() {
            sqlx::query(
                "UPDATE application_domains SET is_primary = 1, updated_at = ?1 WHERE id = ?2",
            )
            .bind(now_ms())
            .bind(next.id)
            .execute(&conn)
            .await
            .map_err(map_write_error)?;
        }
    }
    remove_caddy_route(state, application_id, domain_id, &removed.host).await;
    if removed.is_local || read_by_marker {
        mark_stale(state, application_id, true).await?;
    }
    Ok(())
}

/// Lowest port in the configured range not already claimed by a local domain.
pub(super) async fn allocate_local_port(state: &SharedState) -> Result<i64, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query("SELECT port FROM application_domains")
        .fetch_all(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let mut taken = std::collections::HashSet::new();
    for row in &rows {
        taken.insert(
            row.try_get::<i64, _>(0)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        );
    }
    (state.port_range.0..=state.port_range.1)
        .find(|port| !taken.contains(port) && !state.reserved_ports.contains(port))
        .ok_or_else(|| {
            AppError::ValidationError(format!(
                "No free host port in range {}-{}",
                state.port_range.0, state.port_range.1
            ))
        })
}

/// Ports the domain dialog must refuse. Reserved ports come from configuration;
/// taken ports are every port already published by a local domain.
pub async fn port_constraints(state: &SharedState) -> Result<PortConstraintsDto, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query("SELECT host, port FROM application_domains")
        .fetch_all(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let mut taken_ports = Vec::new();
    for row in &rows {
        let host: String = row
            .try_get(0)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        let port: i64 = row
            .try_get(1)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        if is_local(&host) {
            taken_ports.push(port);
        }
    }
    taken_ports.sort_unstable();
    taken_ports.dedup();

    let mut reserved_ports: Vec<i64> = state.reserved_ports.iter().copied().collect();
    reserved_ports.sort_unstable();

    Ok(PortConstraintsDto {
        reserved_ports,
        taken_ports,
    })
}

/// Seeds a domain when the deploy target has none yet. A group-targeted
/// application fans out across servers, so there is no single server to
/// auto-generate an address for: it is left to the user to pin a domain per
/// server explicitly.
pub async fn seed_if_missing(state: &SharedState, application_id: &str) -> Result<(), AppError> {
    if needs_default_domain(state, application_id).await? {
        generate(state, application_id).await?;
    }
    Ok(())
}

/// Whether this application should get an auto-generated default domain: it
/// deploys to a single server and has no domains yet. Callers that want to log
/// around the generation can gate on this instead of [`seed_if_missing`].
pub async fn needs_default_domain(
    state: &SharedState,
    application_id: &str,
) -> Result<bool, AppError> {
    if targets_server_group(state, application_id).await? {
        return Ok(false);
    }
    Ok(list(state, application_id).await?.is_empty())
}

/// Whether the application deploys to a server group instead of a single
/// server. Group apps have no `applications.server_id` and are the only ones
/// that require an explicit per-server domain pin.
async fn targets_server_group(state: &SharedState, application_id: &str) -> Result<bool, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query("SELECT server_group_id FROM applications WHERE id = ?1")
        .bind(application_id)
        .fetch_optional(&conn)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?
        .ok_or(AppError::ApplicationNotFound)?;
    let group_id: Option<String> = row
        .try_get(0)
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    Ok(group_id.is_some())
}

/// Seeds the domain a fresh application is reachable on: a published localhost
/// port in development, an sslip.io hostname otherwise.
pub async fn generate(state: &SharedState, application_id: &str) -> Result<DomainDto, AppError> {
    generate_with(state, application_id, true).await
}

/// Same as [`generate`]; `is_primary = false` adds a secondary domain (one per
/// extra Compose route a `{{url:PORT}}` env marker asks for) without taking the
/// primary flag away from the app's main address.
pub async fn generate_with(
    state: &SharedState,
    application_id: &str,
    is_primary: bool,
) -> Result<DomainDto, AppError> {
    // Hold the write lock across choosing the local port and inserting its row.
    // Two concurrent generations would otherwise both pick the same free port
    // (the loser then fails the `(host, port)` unique index), which the group
    // fan-out and multiple Compose markers can trigger. The lock makes the
    // allocation see every previously committed port.
    let _guard = DOMAIN_WRITE_LOCK.lock().await;
    let _certificates = crate::deployment::certificates::INVENTORY_LOCK.lock().await;
    let (host, port) = if state.dev_mode {
        ("localhost".to_string(), allocate_local_port(state).await?)
    } else {
        let address = sslip_address(state, application_id).await?;
        (
            format!(
                "litepod-{}.{}.sslip.io",
                &Uuid::now_v7().simple().to_string()[..12],
                address
            ),
            super::super::app_container_port(state, application_id).await?,
        )
    };
    create_with_origin_locked(
        state,
        application_id,
        CreateDomainRequest {
            certificate: Default::default(),
            host,
            port,
            https: Some(false),
            is_primary: Some(is_primary),
            // A group replica pins its seeded domain to the server it is deploying.
            server_id: crate::servers::replica::current(),
        },
        true,
    )
    .await
}

/// A hostname suggestion only; callers still choose whether to save it.
pub async fn random_sslip_host(
    state: &SharedState,
    application_id: &str,
) -> Result<String, AppError> {
    let address = sslip_address(state, application_id).await?;
    Ok(format!(
        "litepod-{}.{}.sslip.io",
        &Uuid::now_v7().simple().to_string()[..12],
        address
    ))
}

pub(super) async fn sslip_address(
    state: &SharedState,
    application_id: &str,
) -> Result<String, AppError> {
    // The replica being deployed when inside a group fan-out, else the app's server.
    let server_id = crate::servers::replica::target_server_id(state, application_id)
        .await?
        .ok_or_else(|| {
            AppError::ValidationError(
                "This application targets a server group; pin the domain to a server first".into(),
            )
        })?;

    let server = crate::servers::repository::get(state, &server_id)
        .await?
        .ok_or(AppError::ApplicationNotFound)?;
    if !matches!(server.ingress_mode, IngressMode::PublicEdge) {
        return Ok(state.public_ip.clone());
    }
    server
        .public_address
        .as_deref()
        .map(str::trim)
        .filter(|address| !address.is_empty())
        .ok_or_else(|| {
            AppError::ValidationError(
                "Public-edge servers need a public IP or hostname before generating sslip.io domains"
                    .into(),
            )
        })
        .and_then(|address| {
            address.parse::<IpAddr>().map_err(|_| {
                AppError::ValidationError(format!(
                    "sslip.io domains require the public-edge server's public address to be an IP address, not '{address}'"
                ))
            })
        })
        .map(|address| address.to_string())
}

/// Fetch one domain by id alone. The caller already knows which server it
/// belongs to (it came from [`edge_route_targets`] or a Caddy fragment).
pub async fn get_by_id(state: &SharedState, domain_id: &str) -> Result<DomainDto, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(&format!(
        "SELECT {SELECT_COLUMNS} FROM application_domains WHERE id = ?1"
    ))
    .bind(domain_id)
    .fetch_optional(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?
    .ok_or(AppError::ResourceNotFound)?;
    with_ingress_details(state, row_to_dto(&row)?).await
}
