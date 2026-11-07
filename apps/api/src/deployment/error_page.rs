//! The page visitors see when an application's domain is in Caddy but the
//! application does not answer: Caddy's own 502/503/504 from `reverse_proxy`.
//!
//! Every application route (`litepod-route-<domain_id>`) gets a twin in
//! `srv0.errors` that matches the same host (and path) plus those status codes.
//! The twins are rebuilt from the live routes on every config edit (`sync`), so
//! they follow domains as they come and go without each caller knowing about
//! them. Errors the application answers itself (its own 502 page) pass through
//! untouched: Caddy only runs error routes for errors raised inside Caddy.
//!
//! Each application picks the default page, its own HTML, or none; the instance
//! setting turns the default page on or off for everyone who did not pick.
//!
//! Private-relay domains are routes of the central Caddy too, so they are covered
//! here. Public-edge servers run their own Caddy: `edge_token` carries the page
//! in their manifest (`servers::ingress::sync_server_routes`).

use std::{collections::HashMap, sync::LazyLock};

use serde::{Deserialize, Serialize};
use sqlx::Row;
use utoipa::ToSchema;

use crate::{dtos::response::AppError, state::state::SharedState};

pub const PAGE: &str = include_str!("error_page.html");
const APP_ROUTE_PREFIX: &str = "litepod-route-";
const ROUTE_SUFFIX: &str = "-error-page";
const STATUS_MATCH: &str = "{http.error.status_code} in [502, 503, 504]";

/// Caddy expands placeholders in a `static_response` body: escape the page's own
/// braces, then put in the two placeholders the page shows.
static BODY: LazyLock<String> = LazyLock::new(|| {
    super::maintenance::escape_placeholders(PAGE)
        .replace("__LITEPOD_STATUS__", "{http.error.status_code}")
        .replace("__LITEPOD_REQUEST_ID__", "{http.request.uuid}")
});

/// What each domain shows, read once per Caddy edit.
#[derive(Debug, Default)]
pub struct Pages {
    pub default_enabled: bool,
    /// Domains whose application chose: `None` for no page, `Some(body)` for its own.
    pub chosen: HashMap<String, Option<String>>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorPageDto {
    pub enabled: bool,
    /// The application's own page; `None` means the default page.
    pub page_html: Option<String>,
    /// The default page, for the preview.
    pub default_html: String,
    /// Whether the instance serves the default page at all.
    pub default_enabled: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateErrorPageRequest {
    pub enabled: Option<bool>,
    /// Blank goes back to the default page.
    pub page_html: Option<String>,
}

fn db(error: sqlx::Error) -> AppError {
    AppError::DatabaseError(error.to_string())
}

pub async fn load_enabled(state: &SharedState) -> Result<bool, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let enabled: Option<i64> = sqlx::query_scalar("SELECT enabled FROM error_page_settings WHERE id = 1")
        .fetch_optional(&conn)
        .await
        .map_err(db)?;
    Ok(enabled.unwrap_or(1) != 0)
}

/// Saves the instance setting and applies it to the central Caddy right away.
pub async fn set_enabled(state: &SharedState, enabled: bool) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("UPDATE error_page_settings SET enabled = ?1 WHERE id = 1")
        .bind(enabled)
        .execute(&conn)
        .await
        .map_err(db)?;
    // An empty edit is enough: `sync` runs on every edit.
    super::operations::edit_caddy_config(state, |_| Ok(())).await?;
    // Public-edge servers carry the page in their own manifest.
    let (_, failed) = crate::servers::ingress::reconcile_running(state).await?;
    if failed > 0 {
        tracing::warn!(failed, "some remote servers did not take the error page setting yet");
    }
    Ok(())
}

pub async fn load(state: &SharedState) -> Result<Pages, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(
        "SELECT d.id, p.enabled, p.page_html FROM application_error_page p \
         JOIN application_domains d ON d.application_id = p.application_id",
    )
    .fetch_all(&conn)
    .await
    .map_err(db)?;
    let mut chosen = HashMap::new();
    for row in rows {
        let domain_id: String = row.get(0);
        let enabled: bool = row.get(1);
        let page: Option<String> = row.get(2);
        match (enabled, page) {
            (false, _) => chosen.insert(domain_id, None),
            (true, Some(page)) => chosen.insert(domain_id, Some(super::maintenance::escape_placeholders(&page))),
            (true, None) => None,
        };
    }
    Ok(Pages { default_enabled: load_enabled(state).await?, chosen })
}

pub async fn get(state: &SharedState, application_id: &str) -> Result<ErrorPageDto, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query("SELECT enabled, page_html FROM application_error_page WHERE application_id = ?1")
        .bind(application_id)
        .fetch_optional(&conn)
        .await
        .map_err(db)?;
    let (enabled, page_html) = row.map_or((true, None), |row| (row.get(0), row.get(1)));
    Ok(ErrorPageDto {
        enabled,
        page_html,
        default_html: PAGE.to_owned(),
        default_enabled: load_enabled(state).await?,
    })
}

/// Saves the application's choice and applies it to the central Caddy and, for an
/// application on a public-edge server, to that server.
pub async fn update(
    state: &SharedState,
    application_id: &str,
    request: UpdateErrorPageRequest,
) -> Result<ErrorPageDto, AppError> {
    let before = get(state, application_id).await?;
    let page_html = match request.page_html.as_deref() {
        Some(page) if page.trim().is_empty() => None,
        Some(page) => {
            if page.len() > super::maintenance::MAX_PAGE_BYTES {
                return Err(AppError::ValidationError(format!(
                    "The error page must be at most {} KiB",
                    super::maintenance::MAX_PAGE_BYTES / 1024
                )));
            }
            if page.contains('\0') {
                return Err(AppError::ValidationError("The error page must be text".into()));
            }
            Some(page.to_owned())
        }
        None => before.page_html.clone(),
    };
    let enabled = request.enabled.unwrap_or(before.enabled);
    if enabled == before.enabled && page_html == before.page_html {
        return Ok(before);
    }
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "INSERT INTO application_error_page (application_id, enabled, page_html, updated_at) VALUES (?1, ?2, ?3, ?4) \
         ON CONFLICT(application_id) DO UPDATE SET enabled = excluded.enabled, page_html = excluded.page_html, updated_at = excluded.updated_at",
    )
    .bind(application_id)
    .bind(enabled)
    .bind(page_html.as_deref())
    .bind(chrono::Utc::now().timestamp())
    .execute(&conn)
    .await
    .map_err(db)?;
    super::operations::edit_caddy_config(state, |_| Ok(())).await?;
    if !super::domains::application_uses_local_caddy(state, application_id).await? {
        crate::servers::ingress::sync_application(state, application_id).await?;
    }
    get(state, application_id).await
}

/// Rebuilds the error twins of every application route in `srv0`. They go last,
/// after more specific error routes such as a maintenance companion's fallback.
pub fn sync(config: &mut serde_json::Value, pages: &Pages) {
    let Some(server) = config
        .pointer_mut("/apps/http/servers/srv0")
        .and_then(serde_json::Value::as_object_mut)
    else {
        return;
    };
    let wanted: Vec<serde_json::Value> = server
        .get("routes")
        .and_then(serde_json::Value::as_array)
        .map(|routes| routes.iter().filter_map(|route| error_route(route, pages)).collect())
        .unwrap_or_default();
    if wanted.is_empty() && !server.contains_key("errors") {
        return;
    }
    let Some(routes) = server
        .entry("errors")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .map(|errors| errors.entry("routes").or_insert_with(|| serde_json::json!([])))
        .and_then(serde_json::Value::as_array_mut)
    else {
        return;
    };
    routes.retain(|route| {
        !route["@id"]
            .as_str()
            .is_some_and(|id| id.starts_with(APP_ROUTE_PREFIX) && id.ends_with(ROUTE_SUFFIX))
    });
    routes.extend(wanted);
}

/// The page body (escaped, with Caddy placeholders) one domain shows, if any.
fn body_for<'a>(pages: &'a Pages, domain_id: &str) -> Option<&'a str> {
    match pages.chosen.get(domain_id) {
        Some(choice) => choice.as_deref(),
        None if pages.default_enabled => Some(BODY.as_str()),
        None => None,
    }
}

/// Base64 of the page for a public-edge manifest line, which the helper places in
/// the domain's `handle_errors` block. The same placeholders work there.
pub(crate) fn edge_token(pages: &Pages, domain_id: &str) -> Option<String> {
    use base64::Engine;
    body_for(pages, domain_id).map(|body| base64::engine::general_purpose::STANDARD.encode(body))
}

fn error_route(route: &serde_json::Value, pages: &Pages) -> Option<serde_json::Value> {
    let id = route["@id"].as_str()?;
    let domain_id = id.strip_prefix(APP_ROUTE_PREFIX)?;
    let body = body_for(pages, domain_id)?;
    let matchers: Vec<serde_json::Value> = route["match"]
        .as_array()?
        .iter()
        .map(|matcher| {
            let mut matcher = matcher.clone();
            matcher["expression"] = STATUS_MATCH.into();
            matcher
        })
        .collect();
    if matchers.is_empty() {
        return None;
    }
    Some(serde_json::json!({
        "@id": format!("{id}{ROUTE_SUFFIX}"),
        "match": matchers,
        "handle": [{
            "handler": "static_response",
            "status_code": "{http.error.status_code}",
            "headers": {
                "Content-Type": ["text/html; charset=utf-8"],
                "Cache-Control": ["no-store"],
                "Retry-After": ["10"],
                "X-Robots-Tag": ["noindex"],
            },
            "body": body,
        }],
        "terminal": true,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn config() -> serde_json::Value {
        json!({ "apps": { "http": { "servers": { "srv0": {
            "routes": [
                { "@id": "litepod-route-a", "match": [{ "host": ["a.example.com"] }], "handle": [] },
                { "@id": "litepod-route-b", "match": [{ "host": ["b.example.com"], "path": ["/api", "/api/*"] }], "handle": [] },
                { "@id": "litepod-server-domain", "match": [{ "host": ["panel.example.com"] }], "handle": [] },
                { "handle": [{ "handler": "reverse_proxy" }] },
            ],
            "errors": { "routes": [
                { "@id": "litepod-route-a-maint-errors", "match": [{ "host": ["a.example.com"] }] },
            ] },
        } } } } })
    }

    fn on() -> Pages {
        Pages { default_enabled: true, ..Default::default() }
    }

    fn error_routes(config: &serde_json::Value) -> Vec<serde_json::Value> {
        config["apps"]["http"]["servers"]["srv0"]["errors"]["routes"].as_array().unwrap().clone()
    }

    #[test]
    fn covers_only_application_routes_and_keeps_other_error_routes_first() {
        let mut config = config();
        sync(&mut config, &on());
        let routes = error_routes(&config);
        let ids: Vec<&str> = routes.iter().map(|r| r["@id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["litepod-route-a-maint-errors", "litepod-route-a-error-page", "litepod-route-b-error-page"]);
        assert_eq!(routes[2]["match"][0]["host"][0], "b.example.com");
        assert_eq!(routes[2]["match"][0]["path"][1], "/api/*");
        assert_eq!(routes[2]["match"][0]["expression"], STATUS_MATCH);
        assert_eq!(routes[1]["handle"][0]["status_code"], "{http.error.status_code}");
    }

    #[test]
    fn is_idempotent_and_follows_removed_routes() {
        let mut config = config();
        sync(&mut config, &on());
        let once = config.clone();
        sync(&mut config, &on());
        assert_eq!(config, once);

        config["apps"]["http"]["servers"]["srv0"]["routes"].as_array_mut().unwrap().remove(0);
        sync(&mut config, &on());
        let ids: Vec<String> = error_routes(&config).iter().map(|r| r["@id"].as_str().unwrap().to_owned()).collect();
        assert_eq!(ids, ["litepod-route-a-maint-errors", "litepod-route-b-error-page"]);
    }

    #[test]
    fn disabling_removes_only_the_error_pages() {
        let mut config = config();
        sync(&mut config, &on());
        sync(&mut config, &Pages::default());
        let ids: Vec<String> = error_routes(&config).iter().map(|r| r["@id"].as_str().unwrap().to_owned()).collect();
        assert_eq!(ids, ["litepod-route-a-maint-errors"]);

        let mut empty = json!({ "apps": { "http": { "servers": { "srv0": { "routes": [] } } } } });
        sync(&mut empty, &Pages::default());
        assert!(empty["apps"]["http"]["servers"]["srv0"].get("errors").is_none());
    }

    #[test]
    fn an_application_can_use_its_own_page_or_none() {
        let mut pages = on();
        pages.chosen.insert("a".into(), Some("<h1>own</h1>".into()));
        pages.chosen.insert("b".into(), None);
        let mut config = config();
        sync(&mut config, &pages);
        let routes = error_routes(&config);
        let ids: Vec<&str> = routes.iter().map(|r| r["@id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["litepod-route-a-maint-errors", "litepod-route-a-error-page"]);
        assert_eq!(routes[1]["handle"][0]["body"], "<h1>own</h1>");

        // With the default page off, an application's own page still shows.
        pages.default_enabled = false;
        pages.chosen.remove("b");
        let mut config = self::config();
        sync(&mut config, &pages);
        assert_eq!(error_routes(&config).len(), 2);
    }

    #[tokio::test]
    async fn reads_each_applications_choice_per_domain() {
        let data_dir = std::env::temp_dir().join(format!("litepod-error-page-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&data_dir).unwrap();
        let (state, _path) = crate::testing::test_state_with_network("", data_dir.clone()).await;
        let (_, _, application_id) = crate::testing::seed_application(&state, "organization-a").await;
        sqlx::query("INSERT INTO application_domains (id, application_id, host, port, https, is_primary, created_at, updated_at) VALUES ('domain-1', ?1, 'app.example.com', 80, 0, 1, 1, 1)")
            .bind(&application_id)
            .execute(&state.db)
            .await
            .unwrap();

        // Nothing chosen: the default page, on for the instance.
        let page = get(&state, &application_id).await.unwrap();
        assert!(page.enabled && page.page_html.is_none() && page.default_enabled);
        let pages = load(&state).await.unwrap();
        assert!(pages.default_enabled && pages.chosen.is_empty());

        sqlx::query("INSERT INTO application_error_page (application_id, enabled, page_html, updated_at) VALUES (?1, 1, '<p>{x}</p>', 1)")
            .bind(&application_id)
            .execute(&state.db)
            .await
            .unwrap();
        let pages = load(&state).await.unwrap();
        assert_eq!(pages.chosen["domain-1"].as_deref(), Some(r"<p>\{x\}</p>"));

        sqlx::query("UPDATE application_error_page SET enabled = 0")
            .execute(&state.db)
            .await
            .unwrap();
        assert_eq!(load(&state).await.unwrap().chosen["domain-1"], None);

        let too_big = UpdateErrorPageRequest {
            enabled: None,
            page_html: Some("x".repeat(super::super::maintenance::MAX_PAGE_BYTES + 1)),
        };
        assert!(matches!(update(&state, &application_id, too_big).await, Err(AppError::ValidationError(_))));
        std::fs::remove_dir_all(data_dir).ok();
    }

    #[test]
    fn pages_avoid_caddyfile_shorthands() {
        // A public-edge server's Caddyfile expands these even when escaped, so a
        // script's `${host}` would turn into the site's name.
        let shorthands = ["host", "hostport", "port", "method", "path", "query", "remote", "remote_host", "remote_port", "uri", "scheme", "file", "dir", "client_ip", "upstream_hostport"];
        for (name, page) in [("error page", PAGE), ("maintenance page", super::super::maintenance::DEFAULT_PAGE)] {
            for shorthand in shorthands {
                assert!(!page.contains(&format!("{{{shorthand}}}")), "{name} contains {{{shorthand}}}");
            }
        }
    }

    #[test]
    fn body_keeps_page_braces_literal_and_fills_in_caddy_placeholders() {
        assert!(PAGE.len() < super::super::maintenance::MAX_PAGE_BYTES);
        assert!(PAGE.contains(r#"name="litepod-error""#));
        assert!(!BODY.contains("__LITEPOD_"));
        assert!(BODY.contains("'{http.error.status_code}'"));
        assert!(BODY.contains("'{http.request.uuid}'"));
        // Every other brace is escaped, so Caddy leaves the CSS and script alone.
        let unescaped = BODY.replace("{http.error.status_code}", "").replace("{http.request.uuid}", "");
        assert!(!unescaped.replace("\\{", "").replace("\\}", "").contains(['{', '}']));
    }
}
