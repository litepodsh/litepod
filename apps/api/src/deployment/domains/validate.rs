use sqlx::Row;

use crate::{dtos::response::AppError, servers::model::IngressMode, state::state::SharedState};

/// Hosts served by a published container port rather than a Caddy route.
pub fn is_local(host: &str) -> bool {
    matches!(
        host.trim().to_ascii_lowercase().as_str(),
        "localhost" | "127.0.0.1" | "::1"
    )
}

pub fn domain_url(host: &str, port: i64, https: bool) -> String {
    let scheme = if https { "https" } else { "http" };
    // Proxied application domains are always entered through Caddy's shared
    // public listeners (:80 or :443). Their stored port is the internal
    // container target, never a public URL port.
    if !is_local(host) {
        return format!("{scheme}://{host}");
    }
    let default_port = if https { 443 } else { 80 };
    // Bracket IPv6 literals so the port separator stays unambiguous.
    let host_part = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    if port == default_port {
        format!("{scheme}://{host_part}")
    } else {
        format!("{scheme}://{host_part}:{port}")
    }
}

pub(super) fn check_host(host: &str) -> Result<(), AppError> {
    let host = host.trim();
    if host.is_empty() {
        return Err(AppError::ValidationError("Host is required".into()));
    }
    if host.len() > 253 {
        return Err(AppError::ValidationError("Host is too long".into()));
    }
    // Colons are allowed so IPv6 literals pass; a scheme is still rejected because `//` contains `/`.
    if !host
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'))
    {
        return Err(AppError::ValidationError("Host must contain only letters, digits, dots, hyphens, or colons — no scheme, path, or spaces".into()));
    }
    Ok(())
}

/// Suffixes that never resolve on the public internet.
const INTERNAL_SUFFIXES: &[&str] = &[
    "localhost",
    "local",
    "internal",
    "lan",
    "home",
    "corp",
    "intranet",
    "test",
    "invalid",
    "arpa",
];

/// A public DNS name worth looking up: dotted, valid labels, not an IP
/// literal and not an internal-only suffix.
pub(super) fn check_public_hostname(host: &str) -> Result<(), AppError> {
    let invalid =
        || AppError::ValidationError("Enter a public hostname such as app.example.com".into());
    if host.len() > 253 || host.parse::<std::net::IpAddr>().is_ok() {
        return Err(invalid());
    }
    let labels = host.split('.').collect::<Vec<_>>();
    let label_ok = |label: &&str| {
        (1..=63).contains(&label.len())
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    };
    let tld = labels.last().copied().unwrap_or_default();
    if labels.len() < 2
        || !labels.iter().all(label_ok)
        || tld.chars().all(|c| c.is_ascii_digit())
        || INTERNAL_SUFFIXES.contains(&tld)
    {
        return Err(invalid());
    }
    Ok(())
}

pub(super) fn check_https(host: &str, https: bool) -> Result<(), AppError> {
    if https && is_local(host) {
        return Err(AppError::ValidationError("HTTPS is not available for local hosts — Caddy cannot issue a certificate for localhost".into()));
    }
    Ok(())
}

pub fn validate(state: &SharedState, host: &str, port: i64, https: bool) -> Result<(), AppError> {
    check_host(host)?;
    check_https(host, https)?;
    if !(1..=65_535).contains(&port) {
        return Err(AppError::ValidationError(
            "Port must be between 1 and 65535".into(),
        ));
    }
    // The configured range governs auto-generation only. A user picking a port
    // by hand may use any free one that is not claimed by infrastructure.
    if is_local(host) && state.reserved_ports.contains(&port) {
        return Err(AppError::ValidationError(format!(
            "Port {port} is reserved for infrastructure"
        )));
    }
    Ok(())
}

/// Public hostnames are an ingress capability of the selected server, not a property an organization
/// can opt into while creating a domain. This check runs before the row is written so a
/// private-only workload can never leave a stale central-Caddy route behind.
pub fn validate_domain_for_ingress(ingress_mode: IngressMode, host: &str) -> Result<(), AppError> {
    if !is_local(host) && !ingress_mode.allows_public_domains() {
        return Err(AppError::ValidationError(
            "This application is assigned to a private-only server and cannot use a public domain"
                .into(),
        ));
    }
    Ok(())
}

/// Caddy - central and edge alike - keys a site block by hostname, so two
/// domains that resolve to the same server with the same host adapt to two site
/// blocks for one address. Caddy then rejects the whole config ("ambiguous site
/// definition") and every later reload on that server fails. The `(host, port)`
/// unique index does not catch this: a Compose app fronting two container ports
/// of one service can legitimately hold two rows with the same host and
/// different ports. Reject that before the row is written.
///
/// An exact `(host, port)` duplicate is left to the unique index, which reports
/// it as a 409 conflict; this guard only fires for a *different* port.
pub(super) async fn reject_conflicting_public_host(
    state: &SharedState,
    server_id: &str,
    host: &str,
    port: i64,
    exclude_domain_id: Option<&str>,
) -> Result<(), AppError> {
    if is_local(host) {
        return Ok(());
    }
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(
        "SELECT d.id FROM application_domains d \
         JOIN applications a ON a.id = d.application_id \
         WHERE d.host = ?1 AND d.port <> ?3 \
           AND COALESCE(d.server_id, a.server_id) = ?2",
    )
    .bind(host)
    .bind(server_id)
    .bind(port)
    .fetch_all(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    let conflict = rows.iter().any(|row| match row.try_get::<String, _>(0) {
        Ok(id) => exclude_domain_id != Some(id.as_str()),
        Err(_) => true,
    });
    if conflict {
        return Err(AppError::ValidationError(format!(
            "Another domain on this server already uses the host {host}; each public domain needs its own hostname"
        )));
    }
    Ok(())
}

pub(super) async fn validate_domain_for_application(
    state: &SharedState,
    server_id: &str,
    host: &str,
) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query("SELECT ingress_mode FROM servers WHERE id = ?1")
        .bind(server_id)
        .fetch_optional(&conn)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?
        .ok_or(AppError::ApplicationNotFound)?;
    let ingress_mode: String = row
        .try_get(0)
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    let ingress_mode = ingress_mode.parse::<IngressMode>()?;
    validate_domain_for_ingress(ingress_mode, host)
}
