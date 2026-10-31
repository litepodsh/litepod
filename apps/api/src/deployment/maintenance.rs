//! Per-application maintenance mode.
//!
//! One row in `application_maintenance` per application that has touched the
//! feature. While it is `enabled`, the Caddy route of every domain of the
//! application answers with a 503 page instead of proxying to the containers,
//! which keep running. Turning it off puts the normal route back.
//!
//! The page is one of three things, in this order of precedence:
//!
//! 1. an uploaded **site** (a ZIP with its own CSS and images, unpacked under the
//!    application's data directory) - Caddy rewrites the request to a public
//!    route of this API, which serves the files and answers the HTML document with
//!    a 503 and everything else with a 200;
//! 2. the application's own **inline HTML** (`page_html`), which Caddy answers by
//!    itself;
//! 3. the built-in **default** template.
//!
//! The inline page is also what a site falls back to: a site whose files are gone
//! is never routed, so a damaged upload cannot turn the page into a 404.
//!
//! Routes are always rebuilt from the database - on a deploy, on route recovery,
//! on a basic-auth change - so the mode survives all of them without any of them
//! knowing about it: each one asks [`load_payload_for_domain`] when it builds a
//! route.
//!
//! Applications whose domains are routed by a public-edge server's own Caddy
//! are not covered yet: that route is built by the edge helper script, which
//! needs its own change. Turning the mode on for them is refused rather than
//! left to do nothing.

use std::path::{Path, PathBuf};

use chrono::Utc;
use sqlx::Row;
use uuid::Uuid;

use crate::{
    dtos::{
        deployment::{MaintenanceDto, MaintenanceSiteDto, UpdateMaintenanceRequest},
        response::AppError,
    },
    state::state::SharedState,
};

use super::{domains, source_archive};

/// Upper bound for a custom page. It is stored in the database and written into
/// Caddy's config, which is reloaded on every route change, so it stays small.
pub const MAX_PAGE_BYTES: usize = 512 * 1024;

/// Largest ZIP accepted for a maintenance site.
pub const MAX_SITE_ARCHIVE_BYTES: usize = 10 * 1024 * 1024;

/// What a maintenance site may unpack to: a page, not a project.
const SITE_LIMITS: source_archive::Limits = source_archive::Limits {
    max_entries: 500,
    max_unpacked_bytes: 20 * 1024 * 1024,
};

/// Shown when the application has no page of its own.
pub const DEFAULT_PAGE: &str = include_str!("maintenance_default.html");

/// The public route of this API that serves an application's maintenance site.
/// Caddy rewrites a visitor's request onto it; the application id follows.
pub const SITE_PATH_PREFIX: &str = "/__litepod/maintenance";

/// What a route needs to answer with the maintenance page.
#[derive(Debug, Clone)]
pub(crate) struct MaintenancePayload {
    pub html: String,
    /// Present when an uploaded site should be served instead of `html`.
    pub site: Option<SiteRoute>,
    /// `container:port` of the Railpack-built page while it is live. Wins over `site`.
    pub companion: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct SiteRoute {
    pub application_id: String,
    /// Where Caddy reaches this API (`host:port`).
    pub upstream: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
struct MaintenanceRow {
    enabled: bool,
    page: Option<String>,
    site_id: Option<String>,
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

fn db(error: sqlx::Error) -> AppError {
    AppError::DatabaseError(error.to_string())
}

fn internal(error: impl std::fmt::Display) -> AppError {
    AppError::InternalError(error.to_string())
}

/// `None` means "use the default page". Blank input is the way back to it.
pub(crate) fn normalize_page(page: Option<&str>) -> Result<Option<String>, AppError> {
    let Some(page) = page else {
        return Ok(None);
    };
    if page.trim().is_empty() {
        return Ok(None);
    }
    if page.len() > MAX_PAGE_BYTES {
        return Err(AppError::ValidationError(format!(
            "The maintenance page must be at most {} KiB",
            MAX_PAGE_BYTES / 1024
        )));
    }
    if page.contains('\0') {
        return Err(AppError::ValidationError(
            "The maintenance page must be text".into(),
        ));
    }
    Ok(Some(page.to_owned()))
}

async fn fetch(state: &SharedState, application_id: &str) -> Result<Option<MaintenanceRow>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(
        "SELECT enabled, page_html, site_id FROM application_maintenance WHERE application_id = ?1",
    )
    .bind(application_id)
    .fetch_optional(&conn)
    .await
    .map_err(db)?;
    row.map(|row| {
        let enabled: i64 = row.try_get(0).map_err(db)?;
        Ok(MaintenanceRow {
            enabled: enabled != 0,
            page: row.try_get(1).map_err(db)?,
            site_id: row.try_get(2).map_err(db)?,
        })
    })
    .transpose()
}

/// Where an application's maintenance site is unpacked.
pub fn site_dir(state: &SharedState, application_id: &str) -> PathBuf {
    state
        .data_dir
        .join("applications")
        .join(application_id)
        .join("maintenance-site")
}

/// File count and total size of an unpacked site, when it is usable (it has an
/// `index.html`).
fn site_stats(dir: &Path) -> Option<(usize, u64)> {
    if !dir.join("index.html").is_file() {
        return None;
    }
    let mut files = 0usize;
    let mut bytes = 0u64;
    let mut pending = vec![dir.to_path_buf()];
    while let Some(current) = pending.pop() {
        for entry in std::fs::read_dir(&current).ok()?.flatten() {
            let metadata = entry.metadata().ok()?;
            if metadata.is_dir() {
                pending.push(entry.path());
            } else if metadata.is_file() {
                files += 1;
                bytes += metadata.len();
            }
        }
    }
    Some((files, bytes))
}

async fn dto(
    state: &SharedState,
    application_id: &str,
    row: Option<MaintenanceRow>,
) -> Result<MaintenanceDto, AppError> {
    let row = row.unwrap_or_default();
    let site = match row.site_id {
        Some(id) => {
            let dir = site_dir(state, application_id);
            tokio::task::spawn_blocking(move || site_stats(&dir))
                .await
                .map_err(internal)?
                .map(|(files, bytes)| MaintenanceSiteDto { id, files, bytes })
        }
        None => None,
    };
    let (companion_status, companion_error) =
        super::maintenance_companion::status(state, application_id).await?;
    Ok(MaintenanceDto {
        enabled: row.enabled,
        custom_html: row.page,
        default_html: DEFAULT_PAGE.to_owned(),
        site,
        companion_status,
        companion_error,
    })
}

pub async fn get(state: &SharedState, application_id: &str) -> Result<MaintenanceDto, AppError> {
    let row = fetch(state, application_id).await?;
    dto(state, application_id, row).await
}

async fn write(
    state: &SharedState,
    application_id: &str,
    enabled: bool,
    page: Option<&str>,
) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    // `site_id` is not in the update: an inline page change never touches the site.
    sqlx::query(
        "INSERT INTO application_maintenance (application_id, enabled, page_html, updated_at) \
         VALUES (?1, ?2, ?3, ?4) \
         ON CONFLICT(application_id) DO UPDATE SET \
             enabled = excluded.enabled, page_html = excluded.page_html, updated_at = excluded.updated_at",
    )
    .bind(application_id)
    .bind(i64::from(enabled))
    .bind(page)
    .bind(now_ms())
    .execute(&conn)
    .await
    .map_err(db)?;
    Ok(())
}

async fn write_site_id(
    state: &SharedState,
    application_id: &str,
    site_id: Option<&str>,
) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "INSERT INTO application_maintenance (application_id, enabled, page_html, site_id, updated_at) \
         VALUES (?1, 0, NULL, ?2, ?3) \
         ON CONFLICT(application_id) DO UPDATE SET site_id = excluded.site_id, updated_at = excluded.updated_at",
    )
    .bind(application_id)
    .bind(site_id)
    .bind(now_ms())
    .execute(&conn)
    .await
    .map_err(db)?;
    Ok(())
}

/// Public-edge domains are routed by the server's own Caddy, which this does
/// not drive yet.
async fn ensure_supported(
    state: &SharedState,
    domains_of_app: &[crate::dtos::deployment::DomainDto],
) -> Result<(), AppError> {
    for domain in domains_of_app.iter().filter(|domain| !domain.is_local) {
        if crate::servers::ingress::domain_is_public_edge(state, domain).await? {
            return Err(AppError::ValidationError(
                "Maintenance mode is not available yet for applications on a public-edge server"
                    .into(),
            ));
        }
    }
    Ok(())
}

/// Rebuilds every route of the application from the database. Used when the page
/// behind a route changes without the mode being switched.
pub(crate) async fn refresh_routes(state: &SharedState, application_id: &str) -> Result<(), AppError> {
    let domains_of_app = domains::list(state, application_id).await?;
    reconcile_domains(state, &domains_of_app).await
}

/// Rebuilds the route of every domain of the application, stopping at the first
/// that cannot be rebuilt.
async fn reconcile_domains(
    state: &SharedState,
    domains_of_app: &[crate::dtos::deployment::DomainDto],
) -> Result<(), AppError> {
    for domain in domains_of_app.iter().filter(|domain| !domain.is_local) {
        super::reconcile_domain_route(state, &domain.id).await?;
    }
    Ok(())
}

/// Applies the change and rebuilds every domain's route. If a route cannot be
/// rebuilt the previous state is put back and the error returned: "saved" must
/// not leave visitors on a page the user thinks they replaced.
pub async fn update(
    state: &SharedState,
    application_id: &str,
    request: UpdateMaintenanceRequest,
) -> Result<MaintenanceDto, AppError> {
    let previous = fetch(state, application_id).await?;
    let before = previous.clone().unwrap_or_default();

    let page = match request.page_html.as_deref() {
        Some(page) => normalize_page(Some(page))?,
        None => before.page.clone(),
    };
    let enabled = request.enabled.unwrap_or(before.enabled);

    let domains_of_app = domains::list(state, application_id).await?;
    if enabled {
        ensure_supported(state, &domains_of_app).await?;
    }
    if previous.is_some() && enabled == before.enabled && page == before.page {
        return dto(state, application_id, previous).await;
    }

    let turning_on = enabled && !before.enabled;
    let turning_off = !enabled && before.enabled;

    // Turning on: the compiled page is started and proven healthy BEFORE any route
    // moves. If it cannot be, the static copy of the same ZIP serves instead, so
    // visitors never land on a container that is not answering.
    if turning_on && before.site_id.is_some() {
        if let Err(error) = super::maintenance_companion::activate(state, application_id).await {
            tracing::info!(application_id, %error, "serving the maintenance site from its static copy");
        }
    }

    write(state, application_id, enabled, page.as_deref()).await?;

    // A page edited while the mode is off changes nothing a visitor can see.
    if enabled || before.enabled {
        if let Err(error) = reconcile_domains(state, &domains_of_app).await {
            tracing::warn!(
                application_id,
                %error,
                "maintenance mode could not be applied; restoring the previous state"
            );
            write(state, application_id, before.enabled, before.page.as_deref()).await?;
            // Best effort: routes already switched go back to what they were.
            let _ = reconcile_domains(state, &domains_of_app).await;
            if turning_on {
                super::maintenance_companion::deactivate(state, application_id).await;
            }
            return Err(error);
        }
    }

    // Turning off: the route is back on the application first, only then is the
    // page's container stopped.
    if turning_off {
        super::maintenance_companion::deactivate(state, application_id).await;
    }
    get(state, application_id).await
}

/// Keeps a ZIP as the application's maintenance site, replacing any earlier one.
///
/// The archive is unpacked into a scratch directory first and swapped in only
/// once it checks out (safe paths, small, an `index.html` at its root), so a bad
/// upload never touches the site that is serving.
pub async fn upload_site(
    state: &SharedState,
    application_id: &str,
    bytes: &[u8],
) -> Result<MaintenanceDto, AppError> {
    if bytes.is_empty() {
        return Err(AppError::ValidationError("The archive is empty".into()));
    }
    if bytes.len() > MAX_SITE_ARCHIVE_BYTES {
        return Err(AppError::ValidationError(format!(
            "The ZIP is larger than {} MB, the limit for a maintenance page",
            MAX_SITE_ARCHIVE_BYTES / (1024 * 1024)
        )));
    }
    let current = fetch(state, application_id).await?.unwrap_or_default();
    let domains_of_app = domains::list(state, application_id).await?;
    if current.enabled {
        ensure_supported(state, &domains_of_app).await?;
    }

    let dir = site_dir(state, application_id);
    let parent = dir.parent().expect("site dir has a parent").to_path_buf();
    tokio::fs::create_dir_all(&parent).await.map_err(internal)?;
    let staging = parent.join(format!("maintenance-site.new-{}", Uuid::now_v7()));
    let old = parent.join(format!("maintenance-site.old-{}", Uuid::now_v7()));

    let archive = bytes.to_vec();
    let unpack_into = staging.clone();
    let site_id = tokio::task::spawn_blocking(move || -> Result<String, String> {
        let result = source_archive::unpack_bytes(&archive, &unpack_into, &SITE_LIMITS).and_then(
            |id| {
                if unpack_into.join("index.html").is_file() {
                    Ok(id)
                } else {
                    Err("The ZIP needs an index.html at its root (or inside its single top folder)"
                        .into())
                }
            },
        );
        if result.is_err() {
            let _ = std::fs::remove_dir_all(&unpack_into);
        }
        result
    })
    .await
    .map_err(internal)?
    .map_err(AppError::ValidationError)?;

    if dir.exists() {
        tokio::fs::rename(&dir, &old).await.map_err(internal)?;
    }
    tokio::fs::rename(&staging, &dir).await.map_err(internal)?;
    let _ = tokio::fs::remove_dir_all(&old).await;

    write_site_id(state, application_id, Some(&site_id)).await?;
    if current.enabled {
        reconcile_domains(state, &domains_of_app).await?;
    }
    // The static copy above already serves the page. Railpack builds the same ZIP
    // in the background; if that fails the copy keeps serving, so the failure is
    // recorded for the user instead of failing the upload.
    if let Err(error) = super::maintenance_companion::build(state, application_id, bytes).await {
        tracing::warn!(application_id, %error, "maintenance companion could not be built");
        super::maintenance_companion::set_status(
            state,
            application_id,
            "failed",
            Some(&error.to_string()),
        )
        .await?;
    }
    get(state, application_id).await
}

/// Drops the uploaded site; the page goes back to the inline one (or the default).
pub async fn remove_site(
    state: &SharedState,
    application_id: &str,
) -> Result<MaintenanceDto, AppError> {
    let current = fetch(state, application_id).await?.unwrap_or_default();
    match tokio::fs::remove_dir_all(site_dir(state, application_id)).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(internal(error)),
    }
    // Visitors move back to the static page or the inline one BEFORE the
    // container behind them is removed.
    super::maintenance_companion::set_live(state, application_id, false).await?;
    if current.enabled {
        let domains_of_app = domains::list(state, application_id).await?;
        reconcile_domains(state, &domains_of_app).await?;
    }
    super::maintenance_companion::teardown(state, application_id).await;
    if current.site_id.is_some() {
        write_site_id(state, application_id, None).await?;
        if current.enabled {
            let domains_of_app = domains::list(state, application_id).await?;
            reconcile_domains(state, &domains_of_app).await?;
        }
    }
    get(state, application_id).await
}

/// The page a route must answer with, or `None` when the application is not in
/// maintenance. Read every time a route is built.
pub(crate) async fn load_payload_for_domain(
    state: &SharedState,
    domain_id: &str,
) -> Result<Option<MaintenancePayload>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(
        "SELECT m.application_id, m.page_html, m.site_id FROM application_maintenance m \
         JOIN application_domains d ON d.application_id = m.application_id \
         WHERE d.id = ?1 AND m.enabled = 1",
    )
    .bind(domain_id)
    .fetch_optional(&conn)
    .await
    .map_err(db)?;
    let Some(row) = row else {
        return Ok(None);
    };
    let application_id: String = row.try_get(0).map_err(db)?;
    let page: Option<String> = row.try_get(1).map_err(db)?;
    let site_id: Option<String> = row.try_get(2).map_err(db)?;

    // A site is routed only while its files are really there: a damaged or
    // half-removed upload falls back to the inline page instead of a 404.
    let site_application_id = application_id.clone();
    let site = (site_id.is_some()
        && !state.caddy_server_upstream.is_empty()
        && site_dir(state, &application_id).join("index.html").is_file())
    .then(|| SiteRoute {
        application_id,
        upstream: state.caddy_server_upstream.clone(),
    });
    let companion = super::maintenance_companion::live_upstream(state, &site_application_id).await?;
    Ok(Some(MaintenancePayload {
        html: page.unwrap_or_else(|| DEFAULT_PAGE.to_owned()),
        site,
        companion,
    }))
}

/// Caddy expands `{...}` placeholders in a `static_response` body, which would
/// eat CSS rules and script blocks of a custom page. A backslash makes them literal.
fn escape_placeholders(html: &str) -> String {
    let mut escaped = String::with_capacity(html.len());
    for ch in html.chars() {
        if matches!(ch, '{' | '}' | '\\') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

/// The handler that answers with the inline page, in place of `reverse_proxy`.
/// `no-store` and `noindex` keep CDNs and search engines from holding on to a
/// temporary page.
pub(crate) fn response_handle(payload: &MaintenancePayload) -> serde_json::Value {
    serde_json::json!({
        "handler": "static_response",
        "status_code": 503,
        "headers": {
            "Content-Type": ["text/html; charset=utf-8"],
            "Cache-Control": ["no-store"],
            "Retry-After": ["300"],
            "X-Robots-Tag": ["noindex"],
        },
        "body": escape_placeholders(&payload.html),
    })
}

/// Everything a route answers with while in maintenance. For a site: rewrite the
/// request onto this API's public site route and proxy it there, leaving the
/// `Host` header as the visitor sent it (the route checks it). For an inline
/// page: one `static_response`.
pub(crate) fn response_handles(payload: &MaintenancePayload) -> Vec<serde_json::Value> {
    if let Some(upstream) = &payload.companion {
        // The headers go BEFORE the proxy: inside `handle_response` Caddy drops them.
        // Only the HTML document is forced to 503; its CSS, images and fonts stay 200.
        return vec![
            serde_json::json!({
                "handler": "headers",
                "response": { "set": {
                    "Cache-Control": ["no-store"],
                    "Retry-After": ["300"],
                    "X-Robots-Tag": ["noindex"],
                }},
            }),
            serde_json::json!({
                "handler": "reverse_proxy",
                "upstreams": [{ "dial": upstream }],
                "handle_response": [{
                    "match": { "headers": { "Content-Type": ["text/html*"] } },
                    "status_code": "503",
                    "routes": [{ "handle": [{ "handler": "copy_response" }] }],
                }],
            }),
        ];
    }
    match &payload.site {
        Some(site) => vec![
            serde_json::json!({
                "handler": "rewrite",
                "uri": format!(
                    "{SITE_PATH_PREFIX}/{}{{http.request.uri.path}}",
                    site.application_id
                ),
            }),
            serde_json::json!({
                "handler": "reverse_proxy",
                "upstreams": [{ "dial": site.upstream }],
            }),
        ],
        None => vec![response_handle(payload)],
    }
}

/// Id of the per-host `errors` route that answers with the inline page when the
/// companion stops answering while maintenance is on.
pub(crate) fn errors_route_id(route_id: &str) -> String {
    format!("{route_id}-maint-errors")
}

/// The live fallback for a companion-backed route: if the proxy cannot reach the
/// container (it died after the route moved), visitors of this host get the
/// inline page with a 503 instead of Caddy's 502. Matches only `hostname`.
pub(crate) fn errors_route(
    route_id: &str,
    hostname: &str,
    payload: &MaintenancePayload,
) -> serde_json::Value {
    serde_json::json!({
        "@id": errors_route_id(route_id),
        "match": [{ "host": [hostname] }],
        "handle": [response_handle(payload)],
        "terminal": true,
    })
}

/// A file of the maintenance site, ready to send.
pub struct SiteFile {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

/// The relative path inside the site a request asks for, or `None` when it tries
/// to leave it. `path` is already percent-decoded.
fn clean_request_path(path: &str) -> Option<PathBuf> {
    let mut clean = PathBuf::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => return None,
            segment if segment.contains(['\\', '\0']) => return None,
            segment => clean.push(segment),
        }
    }
    Some(clean)
}

fn content_type_for_path(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("html" | "htm") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("json" | "map") => "application/json",
        Some("webmanifest") => "application/manifest+json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("avif") => "image/avif",
        Some("ico") => "image/x-icon",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("ttf") => "font/ttf",
        Some("otf") => "font/otf",
        Some("txt") => "text/plain; charset=utf-8",
        Some("xml") => "application/xml",
        Some("mp4") => "video/mp4",
        Some("webm") => "video/webm",
        _ => "application/octet-stream",
    }
}

/// Reads one file of an unpacked site. The HTML document is a 503 (the application
/// is down, whatever the page looks like); everything it loads is a 200, because a
/// browser does not apply a stylesheet or draw an image that came back as an error.
fn read_site_file(root: &Path, relative: &Path) -> Option<SiteFile> {
    let root = root.canonicalize().ok()?;
    let mut candidate = root.join(relative);
    if candidate.is_dir() {
        candidate = candidate.join("index.html");
    }
    // Symlinks cannot be unpacked, but the check costs nothing and the guarantee
    // ("nothing outside the site is ever served") is worth stating in code.
    let found = candidate
        .canonicalize()
        .ok()
        .filter(|path| path.starts_with(&root) && path.is_file());
    let file = match found {
        Some(file) => file,
        None => {
            // A page navigation falls back to the site's index (single-page sites);
            // a missing asset is a plain 404.
            let is_page = relative
                .extension()
                .and_then(|extension| extension.to_str())
                .is_none_or(|extension| matches!(extension, "html" | "htm"));
            if !is_page {
                return None;
            }
            root.join("index.html")
        }
    };
    let body = std::fs::read(&file).ok()?;
    let content_type = content_type_for_path(&file);
    let status = if content_type.starts_with("text/html") {
        503
    } else {
        200
    };
    Some(SiteFile {
        status,
        content_type,
        body,
    })
}

/// Serves one request of an application's maintenance site, or `None` (a 404).
///
/// Nothing is served unless the application is in maintenance with a site, and
/// only to a request whose `Host` is one of the application's own domains. That
/// check is what keeps an uploaded page - somebody's own HTML and scripts - from
/// ever being served from Litepod's origin, where it could act on a signed-in
/// user's session.
pub async fn serve_site(
    state: &SharedState,
    application_id: &str,
    host: &str,
    path: &str,
) -> Option<SiteFile> {
    let row = fetch(state, application_id).await.ok()??;
    if !row.enabled || row.site_id.is_none() {
        return None;
    }
    let host = host.split(':').next().unwrap_or_default().trim();
    let owned = domains::list(state, application_id).await.ok()?;
    if host.is_empty() || !owned.iter().any(|domain| domain.host.eq_ignore_ascii_case(host)) {
        return None;
    }
    let relative = clean_request_path(path)?;
    let root = site_dir(state, application_id);
    tokio::task::spawn_blocking(move || read_site_file(&root, &relative))
        .await
        .ok()?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inline(html: &str) -> MaintenancePayload {
        MaintenancePayload {
            html: html.into(),
            site: None,
            companion: None,
        }
    }

    #[test]
    fn blank_page_means_the_default_and_an_oversized_one_is_refused() {
        assert_eq!(normalize_page(None).unwrap(), None);
        assert_eq!(normalize_page(Some("  \n")).unwrap(), None);
        assert_eq!(
            normalize_page(Some("<p>back soon</p>")).unwrap().as_deref(),
            Some("<p>back soon</p>")
        );
        assert!(matches!(
            normalize_page(Some(&"a".repeat(MAX_PAGE_BYTES + 1))),
            Err(AppError::ValidationError(_))
        ));
        assert!(matches!(
            normalize_page(Some("a\0b")),
            Err(AppError::ValidationError(_))
        ));
    }

    #[test]
    fn braces_and_backslashes_are_escaped_so_caddy_keeps_them_literal() {
        assert_eq!(
            escape_placeholders("body { color: red } a\\b {http.request.host}"),
            "body \\{ color: red \\} a\\\\b \\{http.request.host\\}"
        );
    }

    #[test]
    fn the_response_is_a_cacheless_noindex_503() {
        let handle = response_handle(&inline("<p>hi</p>"));
        assert_eq!(handle["handler"], "static_response");
        assert_eq!(handle["status_code"], 503);
        assert_eq!(handle["headers"]["Cache-Control"][0], "no-store");
        assert_eq!(handle["headers"]["X-Robots-Tag"][0], "noindex");
        assert_eq!(handle["body"], "<p>hi</p>");
    }

    #[test]
    fn the_default_page_is_a_complete_document() {
        assert!(DEFAULT_PAGE.contains("<html"));
        assert!(DEFAULT_PAGE.len() < MAX_PAGE_BYTES);
    }

    #[test]
    fn a_site_is_a_rewrite_onto_the_public_route_and_a_proxy_to_this_api() {
        let handles = response_handles(&MaintenancePayload {
            html: "<p>fallback</p>".into(),
            site: Some(SiteRoute {
                application_id: "app-1".into(),
                upstream: "litepod-api:6001".into(),
            }),
            companion: None,
        });
        assert_eq!(handles.len(), 2);
        assert_eq!(handles[0]["handler"], "rewrite");
        assert_eq!(
            handles[0]["uri"],
            "/__litepod/maintenance/app-1{http.request.uri.path}"
        );
        assert_eq!(handles[1]["handler"], "reverse_proxy");
        assert_eq!(handles[1]["upstreams"][0]["dial"], "litepod-api:6001");
        // The visitor's Host must reach the API unchanged: it is what the route checks.
        assert!(handles[1].get("headers").is_none());
    }

    #[test]
    fn without_a_site_the_route_is_the_inline_page() {
        let handles = response_handles(&inline("<p>hi</p>"));
        assert_eq!(handles.len(), 1);
        assert_eq!(handles[0]["handler"], "static_response");
    }

    #[test]
    fn a_request_cannot_climb_out_of_the_site() {
        assert_eq!(clean_request_path("/").unwrap(), PathBuf::new());
        assert_eq!(
            clean_request_path("/css/./site.css").unwrap(),
            PathBuf::from("css/site.css")
        );
        assert!(clean_request_path("/../secret").is_none());
        assert!(clean_request_path("/a/../../secret").is_none());
        assert!(clean_request_path("/a\\b").is_none());
        assert!(clean_request_path("/a\0b").is_none());
    }

    #[test]
    fn the_document_is_a_503_and_what_it_loads_is_a_200() {
        let dir = std::env::temp_dir().join(format!("litepod-site-{}", Uuid::now_v7()));
        std::fs::create_dir_all(dir.join("css")).unwrap();
        std::fs::write(dir.join("index.html"), "<h1>soon</h1>").unwrap();
        std::fs::write(dir.join("css/site.css"), "h1{color:red}").unwrap();

        let page = read_site_file(&dir, Path::new("")).unwrap();
        assert_eq!(page.status, 503);
        assert!(page.content_type.starts_with("text/html"));

        let css = read_site_file(&dir, Path::new("css/site.css")).unwrap();
        assert_eq!(css.status, 200);
        assert_eq!(css.content_type, "text/css; charset=utf-8");
        assert_eq!(css.body, b"h1{color:red}");

        // A page navigation to a path that is not a file gets the index; a missing
        // asset is a 404.
        assert_eq!(
            read_site_file(&dir, Path::new("pricing")).unwrap().status,
            503
        );
        assert!(read_site_file(&dir, Path::new("css/missing.css")).is_none());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn zip_of(files: &[(&str, &str)]) -> Vec<u8> {
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        for (name, content) in files {
            zip.start_file(*name, options).unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    /// An application of `organization-a` with one domain, and a scratch data dir.
    async fn app_with_domain() -> (SharedState, PathBuf, PathBuf, String) {
        let data_dir = std::env::temp_dir().join(format!("litepod-maint-data-{}", Uuid::now_v7()));
        std::fs::create_dir_all(&data_dir).unwrap();
        let (state, path) = crate::testing::test_state_with_network("", data_dir.clone()).await;
        let (_, _, application_id) = crate::testing::seed_application(&state, "organization-a").await;
        // The seeded ssh server wants its host key confirmed before any route work;
        // the local server (seeded by the migrations) routes through the central Caddy.
        sqlx::query("UPDATE applications SET server_id = 'local' WHERE id = ?1")
            .bind(&application_id)
            .execute(&state.db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO application_domains (id, application_id, host, port, https, is_primary, created_at, updated_at) VALUES ('domain-1', ?1, 'app.example.com', 80, 0, 1, 1, 1)")
            .bind(&application_id)
            .execute(&state.db)
            .await
            .unwrap();
        (state, path, data_dir, application_id)
    }

    fn turn_on() -> UpdateMaintenanceRequest {
        UpdateMaintenanceRequest {
            enabled: Some(true),
            page_html: None,
        }
    }

    #[tokio::test]
    async fn an_uploaded_site_is_served_only_in_maintenance_and_only_to_its_own_host() {
        let (state, path, data_dir, app) = app_with_domain().await;
        let site = zip_of(&[
            ("index.html", "<h1>soon</h1>"),
            ("css/site.css", "h1{color:red}"),
        ]);

        let uploaded = upload_site(&state, &app, &site).await.unwrap();
        let info = uploaded.site.expect("the site is listed after the upload");
        assert_eq!(info.files, 2);
        assert!(!uploaded.enabled, "uploading must not turn the mode on");

        // Off: nothing is served, even to the right host.
        assert!(serve_site(&state, &app, "app.example.com", "/").await.is_none());

        update(&state, &app, turn_on()).await.unwrap();

        let page = serve_site(&state, &app, "app.example.com", "/").await.unwrap();
        assert_eq!(page.status, 503);
        assert_eq!(page.body, b"<h1>soon</h1>");
        let css = serve_site(&state, &app, "APP.example.com:443", "/css/site.css")
            .await
            .unwrap();
        assert_eq!(css.status, 200, "what the page loads is not an error");

        // Any other host - Litepod's own origin included - gets nothing.
        for host in ["litepod.example.com", "", "evil.test"] {
            assert!(
                serve_site(&state, &app, host, "/").await.is_none(),
                "host {host:?} must not be served"
            );
        }
        assert!(
            serve_site(&state, &app, "app.example.com", "/../index.html")
                .await
                .is_none()
        );

        std::fs::remove_dir_all(&data_dir).unwrap();
        crate::testing::cleanup_database(&path);
    }

    #[tokio::test]
    async fn a_bad_upload_is_refused_and_leaves_the_working_site_alone() {
        let (state, path, data_dir, app) = app_with_domain().await;
        upload_site(&state, &app, &zip_of(&[("index.html", "<h1>v1</h1>")]))
            .await
            .unwrap();
        update(&state, &app, turn_on()).await.unwrap();

        let no_index = zip_of(&[("about.html", "<p>no index</p>")]);
        for (name, bytes) in [
            ("no index.html", no_index),
            ("not a zip", b"just text".to_vec()),
            ("empty", Vec::new()),
            ("too large", vec![0u8; MAX_SITE_ARCHIVE_BYTES + 1]),
        ] {
            let refused = upload_site(&state, &app, &bytes).await.unwrap_err();
            assert!(
                matches!(refused, AppError::ValidationError(_)),
                "{name}: {refused:?}"
            );
        }
        let still = serve_site(&state, &app, "app.example.com", "/").await.unwrap();
        assert_eq!(still.body, b"<h1>v1</h1>", "a refused upload must not replace the site");

        // A good one does replace it, and leaves no scratch directories behind.
        upload_site(&state, &app, &zip_of(&[("index.html", "<h1>v2</h1>")]))
            .await
            .unwrap();
        let now = serve_site(&state, &app, "app.example.com", "/").await.unwrap();
        assert_eq!(now.body, b"<h1>v2</h1>");
        let leftovers = std::fs::read_dir(data_dir.join("applications").join(&app))
            .unwrap()
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().contains(".new-")
                || entry.file_name().to_string_lossy().contains(".old-"))
            .count();
        assert_eq!(leftovers, 0);

        std::fs::remove_dir_all(&data_dir).unwrap();
        crate::testing::cleanup_database(&path);
    }

    #[tokio::test]
    async fn removing_the_site_goes_back_to_the_inline_page() {
        let (state, path, data_dir, app) = app_with_domain().await;
        upload_site(&state, &app, &zip_of(&[("index.html", "<h1>site</h1>")]))
            .await
            .unwrap();
        update(
            &state,
            &app,
            UpdateMaintenanceRequest {
                enabled: Some(true),
                page_html: Some("<p>inline</p>".into()),
            },
        )
        .await
        .unwrap();

        let removed = remove_site(&state, &app).await.unwrap();
        assert!(removed.site.is_none());
        assert!(removed.enabled, "removing the site keeps the mode on");
        assert_eq!(removed.custom_html.as_deref(), Some("<p>inline</p>"));
        assert!(serve_site(&state, &app, "app.example.com", "/").await.is_none());
        assert!(!site_dir(&state, &app).exists());

        std::fs::remove_dir_all(&data_dir).unwrap();
        crate::testing::cleanup_database(&path);
    }

    #[tokio::test]
    async fn a_site_whose_files_are_gone_is_not_routed() {
        let (state, path, data_dir, app) = app_with_domain().await;
        upload_site(&state, &app, &zip_of(&[("index.html", "<h1>site</h1>")]))
            .await
            .unwrap();
        update(&state, &app, turn_on()).await.unwrap();
        std::fs::remove_dir_all(site_dir(&state, &app)).unwrap();

        // The test state has no Caddy upstream, so the inline page is the route
        // either way; what matters is that nothing points at the missing files.
        let payload = load_payload_for_domain(&state, "domain-1")
            .await
            .unwrap()
            .unwrap();
        assert!(payload.site.is_none());
        assert_eq!(payload.html, DEFAULT_PAGE);

        std::fs::remove_dir_all(&data_dir).unwrap();
        crate::testing::cleanup_database(&path);
    }

    #[test]
    fn nothing_outside_the_site_directory_is_ever_served() {
        let base = std::env::temp_dir().join(format!("litepod-site-{}", Uuid::now_v7()));
        let dir = base.join("site");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("index.html"), "<h1>soon</h1>").unwrap();
        std::fs::write(base.join("secret.txt"), "do not serve").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(base.join("secret.txt"), dir.join("link.txt")).unwrap();

        assert!(read_site_file(&dir, Path::new("../secret.txt")).is_none());
        #[cfg(unix)]
        assert!(
            read_site_file(&dir, Path::new("link.txt")).is_none(),
            "a symlink out of the site must not be followed"
        );

        std::fs::remove_dir_all(&base).unwrap();
    }

    async fn companion_rows(state: &SharedState, parent: &str) -> Vec<(String, String, i64, Option<String>)> {
        sqlx::query_as(
            "SELECT id, build_method, container_port, server_id FROM applications WHERE maintenance_for = ?1",
        )
        .bind(parent)
        .fetch_all(&state.db)
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn uploading_a_site_creates_one_hidden_companion_that_builds_with_railpack() {
        let (state, path, data_dir, app) = app_with_domain().await;
        let site = zip_of(&[("index.html", "<h1>soon</h1>")]);

        let first = upload_site(&state, &app, &site).await.unwrap();
        let rows = companion_rows(&state, &app).await;
        assert_eq!(rows.len(), 1, "exactly one companion per parent");
        let (companion, method, port, server) = rows[0].clone();
        assert_eq!(method, "railpack");
        assert_eq!(port, 80);
        assert_eq!(server.as_deref(), Some("local"), "same server as its parent");
        assert_eq!(first.companion_status, "building", "{:?}", first.companion_error);

        // It has no domain and is never listed.
        let domains: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM application_domains WHERE application_id = ?1")
            .bind(&companion)
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert_eq!(domains, 0);
        let (organization, project, environment): (String, String, String) = sqlx::query_as(
            "SELECT p.organization_id, p.id, e.id FROM applications a JOIN environments e ON e.id = a.environment_id JOIN projects p ON p.id = e.project_id WHERE a.id = ?1",
        )
        .bind(&app)
        .fetch_one(&state.db)
        .await
        .unwrap();
        let listed = crate::git::repositories::project::list_applications(&state, &organization, &project, &environment)
            .await
            .unwrap();
        assert!(listed.iter().all(|row| row.id != companion), "a companion is never listed");
        assert!(listed.iter().any(|row| row.id == app));

        // A second upload reuses it instead of piling up hidden apps.
        upload_site(&state, &app, &zip_of(&[("index.html", "<h1>later</h1>")])).await.unwrap();
        assert_eq!(companion_rows(&state, &app).await.len(), 1);

        std::fs::remove_dir_all(&data_dir).unwrap();
        crate::testing::cleanup_database(&path);
    }

    #[tokio::test]
    async fn the_other_page_sources_never_create_a_companion() {
        let (state, path, data_dir, app) = app_with_domain().await;
        update(
            &state,
            &app,
            UpdateMaintenanceRequest { enabled: Some(true), page_html: Some("<h1>x</h1>".into()) },
        )
        .await
        .unwrap();
        update(&state, &app, UpdateMaintenanceRequest { enabled: Some(false), page_html: Some(String::new()) })
            .await
            .unwrap();
        assert!(companion_rows(&state, &app).await.is_empty());

        std::fs::remove_dir_all(&data_dir).unwrap();
        crate::testing::cleanup_database(&path);
    }

    #[tokio::test]
    async fn removing_the_site_or_the_parent_removes_the_companion() {
        let (state, path, data_dir, app) = app_with_domain().await;
        let site = zip_of(&[("index.html", "<h1>soon</h1>")]);

        upload_site(&state, &app, &site).await.unwrap();
        assert_eq!(companion_rows(&state, &app).await.len(), 1);
        let removed = remove_site(&state, &app).await.unwrap();
        assert!(companion_rows(&state, &app).await.is_empty());
        assert_eq!(removed.companion_status, "none");

        upload_site(&state, &app, &site).await.unwrap();
        let companion = companion_rows(&state, &app).await[0].0.clone();
        let (organization, project, environment): (String, String, String) = sqlx::query_as(
            "SELECT p.organization_id, p.id, e.id FROM applications a JOIN environments e ON e.id = a.environment_id JOIN projects p ON p.id = e.project_id WHERE a.id = ?1",
        )
        .bind(&app)
        .fetch_one(&state.db)
        .await
        .unwrap();
        crate::cqrs::commands::project::delete_application(&state, &organization, &project, &environment, &app)
            .await
            .unwrap();
        let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM applications WHERE id = ?1")
            .bind(&companion)
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert_eq!(left, 0, "deleting the parent leaves no hidden application behind");

        std::fs::remove_dir_all(&data_dir).unwrap();
        crate::testing::cleanup_database(&path);
    }

    #[tokio::test]
    async fn a_failed_build_is_recorded_and_changes_nothing_else() {
        let (state, path, data_dir, app) = app_with_domain().await;
        upload_site(&state, &app, &zip_of(&[("index.html", "<h1>soon</h1>")])).await.unwrap();
        update(&state, &app, turn_on()).await.unwrap();
        let companion = companion_rows(&state, &app).await[0].0.clone();

        super::super::maintenance_companion::on_build_finished(&state, &companion, &Err("boom".into())).await;

        let after = get(&state, &app).await.unwrap();
        assert_eq!(after.companion_status, "failed");
        assert_eq!(after.companion_error.as_deref(), Some("boom"));
        assert!(after.enabled, "a failed build must not turn maintenance off");
        assert!(after.site.is_some(), "the static copy keeps serving");
        // An ordinary application's build end is ignored.
        super::super::maintenance_companion::on_build_finished(&state, &app, &Err("x".into())).await;
        assert_eq!(get(&state, &app).await.unwrap().companion_status, "failed");

        std::fs::remove_dir_all(&data_dir).unwrap();
        crate::testing::cleanup_database(&path);
    }

    #[tokio::test]
    async fn a_companion_is_routed_only_while_live_ready_and_running() {
        let (state, path, data_dir, app) = app_with_domain().await;
        upload_site(&state, &app, &zip_of(&[("index.html", "<h1>soon</h1>")])).await.unwrap();
        let companion = companion_rows(&state, &app).await[0].0.clone();
        let upstream = || super::super::maintenance_companion::live_upstream(&state, &app);

        // Built and parked, but not live: nothing routes to it.
        sqlx::query("UPDATE application_maintenance SET companion_status = 'ready' WHERE application_id = ?1")
            .bind(&app).execute(&state.db).await.unwrap();
        sqlx::query("UPDATE application_deployments SET container_name = 'maintenance-page', status = 'stopped' WHERE application_id = ?1")
            .bind(&companion).execute(&state.db).await.unwrap();
        assert_eq!(upstream().await.unwrap(), None);

        // Live but its container is stopped: still nothing.
        sqlx::query("UPDATE application_maintenance SET companion_live = 1 WHERE application_id = ?1")
            .bind(&app).execute(&state.db).await.unwrap();
        assert_eq!(upstream().await.unwrap(), None, "a stopped container is never routed");

        sqlx::query("UPDATE application_deployments SET status = 'running' WHERE application_id = ?1")
            .bind(&companion).execute(&state.db).await.unwrap();
        assert_eq!(upstream().await.unwrap().as_deref(), Some("maintenance-page:80"));

        // A failed rebuild takes it out of service even though it is flagged live.
        sqlx::query("UPDATE application_maintenance SET companion_status = 'failed' WHERE application_id = ?1")
            .bind(&app).execute(&state.db).await.unwrap();
        assert_eq!(upstream().await.unwrap(), None);

        std::fs::remove_dir_all(&data_dir).unwrap();
        crate::testing::cleanup_database(&path);
    }

    #[tokio::test]
    async fn turning_on_with_an_unstartable_companion_falls_back_to_the_static_copy() {
        let (state, path, data_dir, app) = app_with_domain().await;
        upload_site(&state, &app, &zip_of(&[("index.html", "<h1>soon</h1>")])).await.unwrap();
        // Marked built, but there is no container to start (no Podman in tests).
        sqlx::query("UPDATE application_maintenance SET companion_status = 'ready' WHERE application_id = ?1")
            .bind(&app).execute(&state.db).await.unwrap();

        let on = update(&state, &app, turn_on()).await.unwrap();
        assert!(on.enabled, "a companion that cannot start must not block maintenance");
        let payload = load_payload_for_domain(&state, "domain-1").await.unwrap().unwrap();
        assert!(payload.companion.is_none(), "nothing is routed to a container that did not start");
        let page = serve_site(&state, &app, "app.example.com", "/").await.unwrap();
        assert_eq!(page.status, 503, "the static copy serves instead");

        let live: i64 = sqlx::query_scalar("SELECT companion_live FROM application_maintenance WHERE application_id = ?1")
            .bind(&app).fetch_one(&state.db).await.unwrap();
        assert_eq!(live, 0);

        std::fs::remove_dir_all(&data_dir).unwrap();
        crate::testing::cleanup_database(&path);
    }

    #[tokio::test]
    async fn removing_the_site_while_live_clears_the_flag_before_the_companion_goes() {
        let (state, path, data_dir, app) = app_with_domain().await;
        upload_site(&state, &app, &zip_of(&[("index.html", "<h1>soon</h1>")])).await.unwrap();
        sqlx::query("UPDATE application_maintenance SET companion_live = 1, companion_status = 'ready' WHERE application_id = ?1")
            .bind(&app).execute(&state.db).await.unwrap();

        remove_site(&state, &app).await.unwrap();

        assert!(companion_rows(&state, &app).await.is_empty());
        let live: i64 = sqlx::query_scalar("SELECT companion_live FROM application_maintenance WHERE application_id = ?1")
            .bind(&app).fetch_one(&state.db).await.unwrap();
        assert_eq!(live, 0);

        std::fs::remove_dir_all(&data_dir).unwrap();
        crate::testing::cleanup_database(&path);
    }
}
