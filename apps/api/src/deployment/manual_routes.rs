//! Standalone Caddy routes managed from the console, not tied to any
//! application. Each route is either a reverse proxy (`hostname -> host:port`)
//! or a redirect (`hostname -> URL`). They live only on the central proxy for
//! now (`server_id = "local"`); remote public-edge servers own their own Caddy.
//!
//! The route is persisted first and pushed to Caddy second, exactly like an
//! application domain: if the push fails the row stays and the console shows it
//! as `missing` until Reconcile runs.

use std::collections::HashSet;

use chrono::Utc;
use sqlx::Row;
use uuid::Uuid;

use crate::{
    dtos::{deployment::ManualRouteDto, response::AppError},
    state::state::SharedState,
};

/// The one server manual routes are allowed on for now: the embedded central
/// Caddy, whose row is always seeded with id `local`.
pub const CENTRAL_SERVER_ID: &str = "local";

const SELECT_COLUMNS: &str =
    "id, server_id, host, kind, target, https, redirect_status, created_at, updated_at";

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

pub fn route_id(id: &str) -> String {
    format!("litepod-manual-{id}")
}

/// User-supplied fields for a create or update.
#[derive(Debug, Clone)]
pub struct NewManualRoute {
    pub host: String,
    pub kind: String,
    pub target: String,
    pub https: bool,
    pub redirect_status: i64,
}

fn row_to_dto(row: &sqlx::sqlite::SqliteRow) -> Result<ManualRouteDto, AppError> {
    let text = |i: usize| -> Result<String, AppError> {
        row.try_get(i)
            .map_err(|e| AppError::DatabaseError(e.to_string()))
    };
    let int = |i: usize| -> Result<i64, AppError> {
        row.try_get(i)
            .map_err(|e| AppError::DatabaseError(e.to_string()))
    };
    Ok(ManualRouteDto {
        id: text(0)?,
        server_id: text(1)?,
        host: text(2)?,
        kind: text(3)?,
        target: text(4)?,
        https: int(5)? != 0,
        redirect_status: int(6)?,
        created_at: int(7)?,
        updated_at: int(8)?,
    })
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// A hostname is about to become a Caddy `match.host` value and, on public-edge
/// servers, a fragment of a root-owned Caddyfile. Same character rules as
/// `servers::ingress::route_fragment`.
fn valid_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && !host.contains(|c: char| {
            c.is_whitespace() || matches!(c, '{' | '}' | '`' | '"' | '\\' | '\0' | '/')
        })
        && host
            .split('.')
            .all(|label| !label.is_empty() && label.len() <= 63)
}

fn valid_upstream(target: &str) -> bool {
    let Some((host, port)) = target.rsplit_once(':') else {
        return false;
    };
    let port_ok = port.parse::<u16>().map(|p| p > 0).unwrap_or(false);
    let host_ok = !host.is_empty()
        && !host.contains(|c: char| c.is_whitespace() || matches!(c, '/' | '{' | '}' | '\\'));
    port_ok && host_ok
}

fn valid_redirect_target(target: &str) -> bool {
    (target.starts_with("http://") || target.starts_with("https://"))
        && target.len() > "https://".len()
        && !target.contains(char::is_whitespace)
}

pub fn validate(new: &NewManualRoute) -> Result<(), AppError> {
    if !valid_host(&new.host) {
        return Err(AppError::ValidationError("Invalid hostname".into()));
    }
    match new.kind.as_str() {
        "reverse_proxy" => {
            if !valid_upstream(&new.target) {
                return Err(AppError::ValidationError(
                    "Upstream must be host:port (no scheme, no path)".into(),
                ));
            }
        }
        "redirect" => {
            if !valid_redirect_target(&new.target) {
                return Err(AppError::ValidationError(
                    "Redirect target must be an absolute http(s):// URL".into(),
                ));
            }
            if ![301, 302, 307, 308].contains(&new.redirect_status) {
                return Err(AppError::ValidationError(
                    "Redirect status must be 301, 302, 307 or 308".into(),
                ));
            }
        }
        _ => {
            return Err(AppError::ValidationError(
                "kind must be reverse_proxy or redirect".into(),
            ));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Persistence
// ---------------------------------------------------------------------------

pub async fn list(state: &SharedState, server_id: &str) -> Result<Vec<ManualRouteDto>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(&format!(
        "SELECT {SELECT_COLUMNS} FROM caddy_manual_routes WHERE server_id = ?1 ORDER BY host"
    ))
    .bind(server_id)
    .fetch_all(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    rows.iter().map(row_to_dto).collect()
}

pub async fn list_all(state: &SharedState) -> Result<Vec<ManualRouteDto>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(&format!(
        "SELECT {SELECT_COLUMNS} FROM caddy_manual_routes ORDER BY host"
    ))
    .fetch_all(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    rows.iter().map(row_to_dto).collect()
}

pub async fn get(state: &SharedState, id: &str) -> Result<Option<ManualRouteDto>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(&format!(
        "SELECT {SELECT_COLUMNS} FROM caddy_manual_routes WHERE id = ?1"
    ))
    .bind(id)
    .fetch_optional(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    row.as_ref().map(row_to_dto).transpose()
}

async fn host_taken(
    state: &SharedState,
    host: &str,
    except_id: Option<&str>,
) -> Result<bool, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query("SELECT id FROM caddy_manual_routes WHERE host = ?1 AND id != ?2")
        .bind(host)
        .bind(except_id.unwrap_or(""))
        .fetch_optional(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok(row.is_some())
}

pub async fn create(
    state: &SharedState,
    server_id: &str,
    new: NewManualRoute,
) -> Result<ManualRouteDto, AppError> {
    if server_id != CENTRAL_SERVER_ID {
        return Err(AppError::ValidationError(
            "Manual routes are only supported on the central proxy".into(),
        ));
    }
    validate(&new)?;
    if host_taken(state, &new.host, None).await? {
        return Err(AppError::ValidationError(format!(
            "A route for {} already exists",
            new.host
        )));
    }
    let id = Uuid::now_v7().to_string();
    let now = now_ms();
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "INSERT INTO caddy_manual_routes (id, server_id, host, kind, target, https, redirect_status, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
    )
    .bind(&id)
    .bind(server_id)
    .bind(&new.host)
    .bind(&new.kind)
    .bind(&new.target)
    .bind(new.https as i64)
    .bind(new.redirect_status)
    .bind(now)
    .execute(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    drop(conn);
    let route = get(state, &id).await?.ok_or(AppError::ResourceNotFound)?;
    sync_manual_route(state, &route).await?;
    Ok(route)
}

pub async fn update(
    state: &SharedState,
    id: &str,
    new: NewManualRoute,
) -> Result<ManualRouteDto, AppError> {
    let current = get(state, id).await?.ok_or(AppError::ResourceNotFound)?;
    validate(&new)?;
    if host_taken(state, &new.host, Some(id)).await? {
        return Err(AppError::ValidationError(format!(
            "A route for {} already exists",
            new.host
        )));
    }
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE caddy_manual_routes SET host = ?2, kind = ?3, target = ?4, https = ?5, redirect_status = ?6, updated_at = ?7 WHERE id = ?1",
    )
    .bind(id)
    .bind(&new.host)
    .bind(&new.kind)
    .bind(&new.target)
    .bind(new.https as i64)
    .bind(new.redirect_status)
    .bind(now_ms())
    .execute(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    drop(conn);
    // Host may have changed - drop the old route from Caddy before pushing the new one.
    if current.host != new.host {
        let _ = crate::deployment::operations::edit_caddy_config(state, |config| {
            remove_manual_route_from_config(config, id, &current.host)
        })
        .await;
    }
    let route = get(state, id).await?.ok_or(AppError::ResourceNotFound)?;
    sync_manual_route(state, &route).await?;
    Ok(route)
}

pub async fn delete(state: &SharedState, id: &str) -> Result<ManualRouteDto, AppError> {
    let route = get(state, id).await?.ok_or(AppError::ResourceNotFound)?;
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("DELETE FROM caddy_manual_routes WHERE id = ?1")
        .bind(id)
        .execute(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    drop(conn);
    remove_manual_route(state, id, &route.host).await;
    Ok(route)
}

// ---------------------------------------------------------------------------
// Caddy admin API
// ---------------------------------------------------------------------------

pub async fn sync_manual_route(
    state: &SharedState,
    route: &ManualRouteDto,
) -> Result<(), AppError> {
    let route = route.clone();
    crate::deployment::operations::edit_caddy_config(state, move |config| {
        apply_manual_route_to_config(config, &route)
    })
    .await
}

pub async fn remove_manual_route(state: &SharedState, id: &str, host: &str) {
    let _ = crate::deployment::operations::edit_caddy_config(state, |config| {
        remove_manual_route_from_config(config, id, host)
    })
    .await;
}

/// Re-pushes every persisted manual route. Caddy keeps its config in memory, so
/// a restart drops these just like application routes; called from the boot
/// reconcile alongside `domains::reconcile_all_routes`.
pub async fn reconcile_all(state: &SharedState) -> Result<(usize, usize), AppError> {
    let mut pushed = 0;
    let mut failed = 0;
    for route in list_all(state).await? {
        match sync_manual_route(state, &route).await {
            Ok(()) => pushed += 1,
            Err(error) => {
                failed += 1;
                tracing::warn!(host = %route.host, %error, "could not reconcile manual Caddy route");
            }
        }
    }
    Ok((pushed, failed))
}

fn manual_handle(route: &ManualRouteDto) -> serde_json::Value {
    match route.kind.as_str() {
        "redirect" => serde_json::json!({
            "handler": "static_response",
            "status_code": route.redirect_status,
            "headers": { "Location": [format!("{}{{http.request.uri}}", route.target)] }
        }),
        // reverse_proxy (and any unknown kind falls back to it defensively)
        _ => serde_json::json!({
            "handler": "reverse_proxy",
            "upstreams": [{ "dial": route.target }]
        }),
    }
}

/// Inserts (or replaces) the route for one manual route. Mirrors
/// `deployment::operations::apply_app_domain_to_config` — kept as a sibling
/// rather than a shared helper because the two navigate the same borrow-heavy
/// JSON tree and factoring the TLS block out fights the borrow checker for no
/// real gain.
/// `ponytail:` ~40 lines duplicated with the app-domain path; unify only if a
/// third caller appears.
pub(crate) fn apply_manual_route_to_config(
    config: &mut serde_json::Value,
    route: &ManualRouteDto,
) -> Result<(), AppError> {
    let id = route_id(&route.id);
    let hostname = route.host.as_str();
    let https = route.https;
    let tls_policy_id = format!("{id}-tls");

    let apps = config
        .as_object_mut()
        .and_then(|root| root.get_mut("apps"))
        .and_then(serde_json::Value::as_object_mut)
        .ok_or_else(|| AppError::InternalError("Caddy configuration has no apps section".into()))?;
    let http = apps
        .entry("http")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or_else(|| AppError::InternalError("Caddy HTTP app is invalid".into()))?;
    let servers = http
        .entry("servers")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or_else(|| AppError::InternalError("Caddy HTTP servers are invalid".into()))?;
    let server = servers
        .entry("srv0")
        .or_insert_with(|| serde_json::json!({ "listen": [":80"], "routes": [] }))
        .as_object_mut()
        .ok_or_else(|| AppError::InternalError("Caddy srv0 is invalid".into()))?;

    let routes = server
        .entry("routes")
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()
        .ok_or_else(|| AppError::InternalError("Caddy routes are invalid".into()))?;
    routes.retain(|r| r.get("@id").and_then(serde_json::Value::as_str) != Some(id.as_str()));
    routes.insert(
        0,
        serde_json::json!({
            "@id": id,
            "match": [{ "host": [hostname] }],
            "handle": [manual_handle(route)],
            "terminal": true
        }),
    );

    let listen = server
        .entry("listen")
        .or_insert_with(|| serde_json::json!([":80"]))
        .as_array_mut()
        .ok_or_else(|| AppError::InternalError("Caddy listen addresses are invalid".into()))?;
    let inherited = super::caddy_admin::inherits_sockets(listen);
    if https && !inherited && !listen.iter().any(|addr| addr == ":443") {
        listen.push(serde_json::Value::String(":443".into()));
    }
    super::caddy_admin::adapt_inherited_sockets(server, https);

    let conn_policies = server
        .entry("tls_connection_policies")
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()
        .ok_or_else(|| {
            AppError::InternalError("Caddy TLS connection policies are invalid".into())
        })?;
    conn_policies.retain(|policy| {
        policy
            .get("match")
            .and_then(|m| m.get("sni"))
            .and_then(serde_json::Value::as_array)
            .is_none_or(|values| !values.iter().any(|value| value.as_str() == Some(hostname)))
    });
    if https {
        conn_policies.push(serde_json::json!({ "match": { "sni": [hostname] } }));
    }

    let automatic_https = server
        .entry("automatic_https")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or_else(|| AppError::InternalError("Caddy automatic HTTPS is invalid".into()))?;
    automatic_https.remove("disable");
    let skipped_hosts = automatic_https
        .entry("skip")
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()
        .ok_or_else(|| AppError::InternalError("Caddy HTTPS skip list is invalid".into()))?;
    skipped_hosts.retain(|host| host.as_str() != Some(hostname));
    if !https {
        skipped_hosts.push(serde_json::Value::String(hostname.into()));
    }
    let remove_automatic_https = automatic_https.is_empty()
        || (automatic_https.len() == 1
            && automatic_https
                .get("skip")
                .is_some_and(|value| value.as_array().is_some_and(Vec::is_empty)));
    if remove_automatic_https {
        server.remove("automatic_https");
    }

    let tls = apps
        .entry("tls")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or_else(|| AppError::InternalError("Caddy TLS app is invalid".into()))?;
    let automation = tls
        .entry("automation")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or_else(|| AppError::InternalError("Caddy TLS automation is invalid".into()))?;
    let policies = automation
        .entry("policies")
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()
        .ok_or_else(|| AppError::InternalError("Caddy TLS policies are invalid".into()))?;
    policies.retain(|policy| {
        let same_id =
            policy.get("@id").and_then(serde_json::Value::as_str) == Some(tls_policy_id.as_str());
        let claims_host = policy
            .get("subjects")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|subjects| subjects.iter().any(|s| s.as_str() == Some(hostname)));
        !same_id && !claims_host
    });
    if https {
        policies.insert(
            0,
            serde_json::json!({ "@id": tls_policy_id, "subjects": [hostname] }),
        );
    }
    Ok(())
}

pub(crate) fn remove_manual_route_from_config(
    config: &mut serde_json::Value,
    id: &str,
    hostname: &str,
) -> Result<(), AppError> {
    let route_id = route_id(id);
    let tls_policy_id = format!("{route_id}-tls");
    let Some(server) = config
        .pointer_mut("/apps/http/servers/srv0")
        .and_then(serde_json::Value::as_object_mut)
    else {
        return Ok(());
    };
    if let Some(routes) = server
        .get_mut("routes")
        .and_then(serde_json::Value::as_array_mut)
    {
        routes.retain(|r| {
            r.get("@id").and_then(serde_json::Value::as_str) != Some(route_id.as_str())
        });
    }
    if let Some(conn) = server
        .get_mut("tls_connection_policies")
        .and_then(serde_json::Value::as_array_mut)
    {
        conn.retain(|policy| {
            policy
                .get("match")
                .and_then(|m| m.get("sni"))
                .and_then(serde_json::Value::as_array)
                .is_none_or(|values| !values.iter().any(|value| value.as_str() == Some(hostname)))
        });
    }
    if let Some(skip) = server
        .get_mut("automatic_https")
        .and_then(|value| value.get_mut("skip"))
        .and_then(serde_json::Value::as_array_mut)
    {
        skip.retain(|host| host.as_str() != Some(hostname));
    }
    if let Some(policies) = config
        .pointer_mut("/apps/tls/automation/policies")
        .and_then(serde_json::Value::as_array_mut)
    {
        policies.retain(|policy| {
            policy.get("@id").and_then(serde_json::Value::as_str) != Some(tls_policy_id.as_str())
        });
    }
    Ok(())
}

/// Route ids the console must keep when pruning orphans: every persisted manual
/// route, as its Caddy `@id`.
pub async fn keep_route_ids(state: &SharedState) -> Result<HashSet<String>, AppError> {
    Ok(list_all(state)
        .await?
        .into_iter()
        .map(|route| route_id(&route.id))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dto(kind: &str, https: bool) -> ManualRouteDto {
        ManualRouteDto {
            id: "abc".into(),
            server_id: "local".into(),
            host: "demo.example.com".into(),
            kind: kind.into(),
            target: if kind == "redirect" {
                "https://example.org".into()
            } else {
                "127.0.0.1:8080".into()
            },
            https,
            redirect_status: 308,
            created_at: 0,
            updated_at: 0,
        }
    }

    fn base_config() -> serde_json::Value {
        serde_json::json!({
            "apps": { "http": { "servers": { "srv0": {
                "listen": [":80"],
                "routes": [{ "@id": "catch-all", "handle": [{ "handler": "static_response", "body": "hi" }] }]
            } } } }
        })
    }

    #[test]
    fn reverse_proxy_route_is_prepended_and_dials_target() {
        let mut cfg = base_config();
        apply_manual_route_to_config(&mut cfg, &dto("reverse_proxy", false)).unwrap();
        let routes = cfg["apps"]["http"]["servers"]["srv0"]["routes"]
            .as_array()
            .unwrap();
        assert_eq!(routes[0]["@id"], "litepod-manual-abc");
        assert_eq!(routes[0]["handle"][0]["handler"], "reverse_proxy");
        assert_eq!(
            routes[0]["handle"][0]["upstreams"][0]["dial"],
            "127.0.0.1:8080"
        );
        assert_eq!(routes[1]["@id"], "catch-all", "bootstrap route stays last");
        // http-only: host is added to the automatic-https skip list.
        assert_eq!(
            cfg["apps"]["http"]["servers"]["srv0"]["automatic_https"]["skip"][0],
            "demo.example.com"
        );
    }

    #[test]
    fn redirect_route_emits_static_response_with_location() {
        let mut cfg = base_config();
        apply_manual_route_to_config(&mut cfg, &dto("redirect", false)).unwrap();
        let handle = &cfg["apps"]["http"]["servers"]["srv0"]["routes"][0]["handle"][0];
        assert_eq!(handle["handler"], "static_response");
        assert_eq!(handle["status_code"], 308);
        assert_eq!(
            handle["headers"]["Location"][0],
            "https://example.org{http.request.uri}"
        );
    }

    #[test]
    fn https_route_opens_443_and_registers_one_tls_policy_per_host() {
        let mut cfg = base_config();
        // A stale foreign policy already claims this host.
        cfg["apps"]["tls"] = serde_json::json!({
            "automation": { "policies": [{ "@id": "litepod-route-old-tls", "subjects": ["demo.example.com"] }] }
        });
        apply_manual_route_to_config(&mut cfg, &dto("reverse_proxy", true)).unwrap();
        let listen = cfg["apps"]["http"]["servers"]["srv0"]["listen"]
            .as_array()
            .unwrap();
        assert!(listen.iter().any(|a| a == ":443"));
        let policies = cfg["apps"]["tls"]["automation"]["policies"]
            .as_array()
            .unwrap();
        assert_eq!(
            policies.len(),
            1,
            "the stale policy for this host was replaced"
        );
        assert_eq!(policies[0]["@id"], "litepod-manual-abc-tls");
    }

    #[test]
    fn remove_is_the_inverse_of_apply() {
        let mut cfg = base_config();
        let route = dto("reverse_proxy", true);
        apply_manual_route_to_config(&mut cfg, &route).unwrap();
        remove_manual_route_from_config(&mut cfg, &route.id, &route.host).unwrap();
        let routes = cfg["apps"]["http"]["servers"]["srv0"]["routes"]
            .as_array()
            .unwrap();
        assert!(routes.iter().all(|r| r["@id"] != "litepod-manual-abc"));
        assert_eq!(routes.len(), 1);
        let policies = cfg["apps"]["tls"]["automation"]["policies"]
            .as_array()
            .unwrap();
        assert!(
            policies
                .iter()
                .all(|p| p["@id"] != "litepod-manual-abc-tls")
        );
    }

    #[test]
    fn validation_rejects_bad_input() {
        assert!(
            validate(&NewManualRoute {
                host: "bad host".into(),
                kind: "reverse_proxy".into(),
                target: "127.0.0.1:80".into(),
                https: true,
                redirect_status: 308,
            })
            .is_err()
        );
        assert!(
            validate(&NewManualRoute {
                host: "ok.example.com".into(),
                kind: "reverse_proxy".into(),
                target: "http://127.0.0.1:80".into(), // scheme not allowed
                https: true,
                redirect_status: 308,
            })
            .is_err()
        );
        assert!(
            validate(&NewManualRoute {
                host: "ok.example.com".into(),
                kind: "redirect".into(),
                target: "example.org".into(), // not absolute
                https: true,
                redirect_status: 308,
            })
            .is_err()
        );
        assert!(
            validate(&NewManualRoute {
                host: "ok.example.com".into(),
                kind: "reverse_proxy".into(),
                target: "127.0.0.1:8080".into(),
                https: true,
                redirect_status: 308,
            })
            .is_ok()
        );
    }
}
