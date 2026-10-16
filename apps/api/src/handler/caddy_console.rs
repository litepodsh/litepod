//! Per-server Caddy console. Mirrors [`podman_console`]: every route resolves a
//! server the caller's organization owns (or the shared `local` server) and
//! then acts on that server's proxy. Management is implemented for the central
//! embedded Caddy only (`server_id = "local"`); a remote server can be selected
//! but only its ingress health is reported. Replaces the sysadmin-only
//! `/debug/caddy` surface.

use std::{collections::HashSet, ffi::OsString};

use axum::{
    Json,
    extract::{Path, State},
    response::IntoResponse,
};
use serde::Deserialize;
use serde_json::json;

use crate::{
    deployment::{
        domains::{self, EdgeRouteTarget},
        manual_routes,
    },
    dtos::{
        deployment::{
            EdgeRouteRow, EdgeRoutesDto, RouteDiffDto, RouteDiffEntryDto, RouteReconcileResultDto,
        },
        response::{AppError, ok},
    },
    git::repositories::audit_log,
    handler::debug::{caddy_live_config, caddy_live_routes},
    security::authz::extract::{RequireOrg, perm},
    security::middleware::org_scope::OrgScope,
    servers::{self, model::IngressMode, repository},
    state::state::SharedState,
};

/// The org extractor covers `{organization_id}`; handlers take `{server_id}`.
type ServerPath = (String, String);

#[derive(Deserialize)]
pub struct ManualRouteRequest {
    pub host: String,
    pub kind: String,
    pub target: String,
    #[serde(default)]
    pub https: bool,
    #[serde(default)]
    pub redirect_status: Option<i64>,
}

impl From<ManualRouteRequest> for manual_routes::NewManualRoute {
    fn from(request: ManualRouteRequest) -> Self {
        Self {
            host: request.host.trim().to_string(),
            kind: request.kind,
            target: request.target.trim().to_string(),
            https: request.https,
            redirect_status: request.redirect_status.unwrap_or(308),
        }
    }
}

#[derive(Deserialize)]
pub struct ConfirmRequest {
    pub confirmation: String,
}

fn audit(state: &SharedState, org: &OrgScope, action: &str, target: &str) {
    let state = state.clone();
    let organization_id = org.organization_id.clone();
    let user = org.user.clone();
    let action = action.to_string();
    let target = target.to_string();
    tokio::spawn(async move {
        audit_log::record(
            &state,
            &organization_id,
            &user,
            &action,
            Some("caddy_route"),
            Some(&target),
            Some(&target),
            None,
            None,
            None,
            None,
        )
        .await
        .ok();
    });
}

/// Route diff, reconcile and prune work on the central proxy and on a
/// public-edge SSH server. Manual routes, the Caddyfile and the live config
/// stay central-only (there is no edge equivalent).
async fn require_managed(
    state: &SharedState,
    organization_id: &str,
    server_id: &str,
) -> Result<repository::ServerRecord, AppError> {
    let server = repository::get_for_server_route(state, server_id, organization_id)
        .await?
        .ok_or(AppError::ResourceNotFound)?;
    let ok = server.kind == "local"
        || (server.kind == "ssh" && matches!(server.ingress_mode, IngressMode::PublicEdge));
    if !ok {
        return Err(AppError::ValidationError(
            "The Caddy console manages the central proxy and public-edge servers only".into(),
        ));
    }
    Ok(server)
}

/// Resolves a server the org owns and rejects anything but the central proxy.
/// Manual routes, the Caddyfile and the live config are central-only.
async fn require_central(
    state: &SharedState,
    organization_id: &str,
    server_id: &str,
) -> Result<repository::ServerRecord, AppError> {
    let server = require_managed(state, organization_id, server_id).await?;
    if server.kind != "local" {
        return Err(AppError::ValidationError(
            "This server runs its own edge proxy; the Caddy console manages the central proxy only"
                .into(),
        ));
    }
    Ok(server)
}

// ---------------------------------------------------------------------------
// Route diff / reconcile / prune
// ---------------------------------------------------------------------------

pub async fn routes_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerManage>,
    Path((_, server_id)): Path<ServerPath>,
) -> Result<impl IntoResponse, AppError> {
    require_central(&state, &org.organization_id, &server_id).await?;
    Ok(ok(build_route_diff(&state).await?))
}

async fn build_route_diff(state: &SharedState) -> Result<RouteDiffDto, AppError> {
    let expected = domains::expected_routes(state).await?;
    let manual = manual_routes::list_all(state).await?;
    let live = caddy_live_routes(state).await?;
    let mut entries = Vec::new();

    for route in &expected {
        let live_route = live
            .iter()
            .find(|candidate| candidate.route_id.as_deref() == Some(route.route_id.as_str()))
            .or_else(|| {
                live.iter()
                    .find(|candidate| candidate.hosts.iter().any(|host| host == &route.host))
            });
        let live_dial = live_route.and_then(|found| found.dial.clone());
        let status = match (route.dial.as_deref(), live_dial.as_deref()) {
            (None, _) => "unroutable",
            (Some(_), None) => "missing",
            (Some(expected_dial), Some(actual)) if expected_dial == actual => "ok",
            _ => "mismatched",
        };
        entries.push(RouteDiffEntryDto {
            host: route.host.clone(),
            route_id: route.route_id.clone(),
            application_id: Some(route.application_id.clone()),
            application_name: Some(route.application_name.clone()),
            source: "app".into(),
            status: status.to_string(),
            expected_dial: route.dial.clone(),
            live_dial,
        });
    }

    for route in &manual {
        let route_id = manual_routes::route_id(&route.id);
        let live_route = live
            .iter()
            .find(|candidate| candidate.route_id.as_deref() == Some(route_id.as_str()));
        let (expected_dial, status) = if route.kind == "reverse_proxy" {
            let status = match live_route.and_then(|found| found.dial.as_deref()) {
                None => "missing",
                Some(actual) if actual == route.target => "ok",
                Some(_) => "mismatched",
            };
            (Some(route.target.clone()), status)
        } else {
            let status = if live_route.is_some() {
                "ok"
            } else {
                "missing"
            };
            (
                Some(format!(
                    "redirect {} \u{2192} {}",
                    route.redirect_status, route.target
                )),
                status,
            )
        };
        entries.push(RouteDiffEntryDto {
            host: route.host.clone(),
            route_id,
            application_id: None,
            application_name: None,
            source: "manual".into(),
            status: status.to_string(),
            expected_dial,
            live_dial: live_route.and_then(|found| found.dial.clone()),
        });
    }

    // Routes Caddy still serves that no database row explains.
    for route in &live {
        let Some(route_id) = route.route_id.as_deref() else {
            continue;
        };
        let is_app = route_id.starts_with("litepod-route-");
        let is_manual = route_id.starts_with("litepod-manual-");
        if !is_app && !is_manual {
            continue;
        }
        if entries.iter().any(|entry| entry.route_id == route_id) {
            continue;
        }
        entries.push(RouteDiffEntryDto {
            host: route.hosts.first().cloned().unwrap_or_default(),
            route_id: route_id.to_string(),
            application_id: None,
            application_name: None,
            source: if is_manual { "manual" } else { "app" }.into(),
            status: "orphaned".to_string(),
            expected_dial: None,
            live_dial: route.dial.clone(),
        });
    }

    let needs_reconcile = entries
        .iter()
        .filter(|entry| entry.status == "missing" || entry.status == "mismatched")
        .count();
    Ok(RouteDiffDto {
        entries,
        expected,
        live,
        caddy_admin_url: state.caddy_admin_url.clone(),
        needs_reconcile,
    })
}

pub async fn reconcile_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerManage>,
    Path((_, server_id)): Path<ServerPath>,
) -> Result<impl IntoResponse, AppError> {
    let server = require_managed(&state, &org.organization_id, &server_id).await?;
    if server.kind != "local" {
        let server = repository::ensure_host_key_known_hosts(&state, server).await?;
        let pushed = crate::servers::ingress::sync_server_routes(&state, &server).await?;
        audit(&state, &org, "caddy.routes_reconciled", &server.id);
        return Ok(ok(RouteReconcileResultDto { pushed, failed: 0 }));
    }
    let (app_pushed, app_failed) = domains::reconcile_all_routes(&state).await?;
    let (manual_pushed, manual_failed) = manual_routes::reconcile_all(&state).await?;
    audit(&state, &org, "caddy.routes_reconciled", "central");
    Ok(ok(RouteReconcileResultDto {
        pushed: app_pushed + manual_pushed,
        failed: app_failed + manual_failed,
    }))
}

pub async fn prune_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerManage>,
    Path((_, server_id)): Path<ServerPath>,
) -> Result<impl IntoResponse, AppError> {
    let server = require_managed(&state, &org.organization_id, &server_id).await?;
    if server.kind != "local" {
        let server = repository::ensure_host_key_known_hosts(&state, server).await?;
        // Count fragments on disk that no current domain explains, then let
        // the declarative sync remove them (it prunes as a side effect).
        let (disk_ids, _stale) = edge_disk_route_ids(&server).await;
        let keep: HashSet<String> = domains::edge_route_targets(&state, &server.id)
            .await?
            .into_iter()
            .map(|target| target.domain_id)
            .collect();
        let dropped = disk_ids.iter().filter(|id| !keep.contains(*id)).count();
        crate::servers::ingress::sync_server_routes(&state, &server).await?;
        audit(&state, &org, "caddy.orphans_pruned", &server.id);
        return Ok(ok(json!({ "dropped": dropped })));
    }

    let expected = domains::expected_routes(&state).await?;
    let mut keep_ids: HashSet<String> = expected
        .iter()
        .map(|route| route.route_id.clone())
        .collect();
    keep_ids.extend(manual_routes::keep_route_ids(&state).await?);
    keep_ids.insert("litepod-server-domain".into());

    let mut keep_hosts: HashSet<String> = expected.iter().map(|route| route.host.clone()).collect();
    keep_hosts.extend(
        manual_routes::list_all(&state)
            .await?
            .into_iter()
            .map(|route| route.host),
    );
    if let Some(domain) = server_domain_host(&state).await {
        keep_hosts.insert(domain);
    }

    let dropped =
        crate::deployment::operations::prune_orphaned_caddy_config(&state, &keep_ids, &keep_hosts)
            .await?;
    audit(&state, &org, "caddy.orphans_pruned", "central");
    Ok(ok(json!({ "dropped": dropped })))
}

async fn server_domain_host(state: &SharedState) -> Option<String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query_scalar::<_, String>("SELECT domain FROM server_domain_settings WHERE id = 1")
        .fetch_optional(&conn)
        .await
        .ok()
        .flatten()
}

// ---------------------------------------------------------------------------
// Manual routes CRUD
// ---------------------------------------------------------------------------

pub async fn list_manual_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerManage>,
    Path((_, server_id)): Path<ServerPath>,
) -> Result<impl IntoResponse, AppError> {
    require_central(&state, &org.organization_id, &server_id).await?;
    Ok(ok(manual_routes::list(&state, &server_id).await?))
}

pub async fn create_manual_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerManage>,
    Path((_, server_id)): Path<ServerPath>,
    Json(request): Json<ManualRouteRequest>,
) -> Result<impl IntoResponse, AppError> {
    require_central(&state, &org.organization_id, &server_id).await?;
    let route = manual_routes::create(&state, &server_id, request.into()).await?;
    audit(&state, &org, "caddy.route_created", &route.host);
    Ok(ok(route))
}

pub async fn update_manual_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerManage>,
    Path((_, server_id, id)): Path<(String, String, String)>,
    Json(request): Json<ManualRouteRequest>,
) -> Result<impl IntoResponse, AppError> {
    require_central(&state, &org.organization_id, &server_id).await?;
    let route = manual_routes::update(&state, &id, request.into()).await?;
    audit(&state, &org, "caddy.route_updated", &route.host);
    Ok(ok(route))
}

pub async fn delete_manual_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerManage>,
    Path((_, server_id, id)): Path<(String, String, String)>,
    Json(request): Json<ConfirmRequest>,
) -> Result<impl IntoResponse, AppError> {
    require_central(&state, &org.organization_id, &server_id).await?;
    let existing = manual_routes::get(&state, &id)
        .await?
        .ok_or(AppError::ResourceNotFound)?;
    if request.confirmation != existing.host {
        return Err(AppError::ValidationError(format!(
            "Type \"{}\" exactly to confirm deleting this route",
            existing.host
        )));
    }
    let route = manual_routes::delete(&state, &id).await?;
    audit(&state, &org, "caddy.route_deleted", &route.host);
    Ok(ok(json!({ "deleted": route.id })))
}

// ---------------------------------------------------------------------------
// Caddyfile / live config / health
// ---------------------------------------------------------------------------

pub async fn config_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerManage>,
    Path((_, server_id)): Path<ServerPath>,
) -> Result<impl IntoResponse, AppError> {
    require_central(&state, &org.organization_id, &server_id).await?;
    let content = tokio::fs::read_to_string(&state.caddyfile_path)
        .await
        .map_err(|error| AppError::InternalError(format!("Could not read Caddyfile: {error}")))?;
    Ok(ok(
        json!({ "path": state.caddyfile_path, "content": content }),
    ))
}

pub async fn status_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerManage>,
    Path((_, server_id)): Path<ServerPath>,
) -> Result<impl IntoResponse, AppError> {
    require_central(&state, &org.organization_id, &server_id).await?;
    Ok(ok(json!({
        "admin_url": state.caddy_admin_url,
        "active_config": caddy_live_config(&state).await?,
    })))
}

/// Works for any selected server: the central proxy reports whether its admin
/// API answers; a remote public-edge server reports its edge Caddy health.
pub async fn health_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerManage>,
    Path((_, server_id)): Path<ServerPath>,
) -> Result<impl IntoResponse, AppError> {
    let server = repository::get_for_server_route(&state, &server_id, &org.organization_id)
        .await?
        .ok_or(AppError::ResourceNotFound)?;
    if server.kind == "local" {
        let healthy = caddy_live_config(&state).await.is_ok();
        return Ok(ok(json!({
            "server_id": server.id,
            "kind": "central",
            "healthy": healthy,
            "detail": if healthy { "Central Caddy admin API is reachable" } else { "Central Caddy admin API is unreachable" },
        })));
    }
    let result = servers::ingress::public_edge_healthy(&server).await;
    Ok(ok(json!({
        "server_id": server.id,
        "kind": "edge",
        "healthy": result.is_ok(),
        "detail": match result {
            Ok(()) => "Edge Caddy responded".to_string(),
            Err(error) => error.to_string(),
        },
        "managed": false,
    })))
}

/// Route ids of the fragments a public-edge server has on disk right now, plus
/// whether its helper is too old to answer (`list-routes` missing).
async fn edge_disk_route_ids(server: &repository::ServerRecord) -> (Vec<String>, bool) {
    match crate::servers::setup::caddy_helper(server, &[OsString::from("list-routes")]).await {
        Ok(out) => (
            out.lines()
                .filter_map(|line| line.split_whitespace().next().map(str::to_string))
                .collect(),
            false,
        ),
        Err(_) => (Vec::new(), true),
    }
}

/// Classify every DB domain against what is on disk. `disk_ids` is the set of
/// fragment route ids; `stale` is true when the helper could not be asked.
fn classify_edge_rows(
    targets: &[EdgeRouteTarget],
    disk_ids: &HashSet<String>,
    stale: bool,
) -> Vec<EdgeRouteRow> {
    let target_ids: HashSet<&str> = targets.iter().map(|t| t.domain_id.as_str()).collect();
    let mut rows: Vec<EdgeRouteRow> = targets
        .iter()
        .map(|t| EdgeRouteRow {
            domain_id: t.domain_id.clone(),
            host: t.host.clone(),
            application_name: t.application_name.clone(),
            status: if !t.routable {
                "pending".to_string()
            } else if stale || disk_ids.contains(&t.domain_id) {
                "ok".to_string()
            } else {
                "missing".to_string()
            },
        })
        .collect();
    if !stale {
        for id in disk_ids {
            if !target_ids.contains(id.as_str()) {
                rows.push(EdgeRouteRow {
                    domain_id: id.clone(),
                    host: String::new(),
                    application_name: String::new(),
                    status: "orphaned".to_string(),
                });
            }
        }
    }
    rows
}

/// Route diff for a server's own edge Caddy: every non-local domain that
/// points here, matched against the fragments actually on disk.
pub async fn edge_routes_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerManage>,
    Path((_, server_id)): Path<ServerPath>,
) -> Result<impl IntoResponse, AppError> {
    let server = repository::get_for_server_route(&state, &server_id, &org.organization_id)
        .await?
        .ok_or(AppError::ResourceNotFound)?;
    let targets = domains::edge_route_targets(&state, &server.id).await?;

    // Only a public-edge server owns a per-server route helper. A private-relay
    // or private-only server routes through the central proxy, so there is no
    // helper to probe and nothing on this page to repair.
    if !matches!(server.ingress_mode, IngressMode::PublicEdge) {
        return Ok(ok(EdgeRoutesDto {
            rows: classify_edge_rows(&targets, &HashSet::new(), true),
            manages_routes: false,
            stale_helper: false,
        }));
    }

    let (disk_ids, stale_helper) = edge_disk_route_ids(&server).await;
    let disk_ids: HashSet<String> = disk_ids.into_iter().collect();
    Ok(ok(EdgeRoutesDto {
        rows: classify_edge_rows(&targets, &disk_ids, stale_helper),
        manages_routes: true,
        stale_helper,
    }))
}

#[cfg(test)]
mod tests {
    use super::{EdgeRouteTarget, classify_edge_rows};
    use std::collections::HashSet;

    fn target(id: &str, host: &str, routable: bool) -> EdgeRouteTarget {
        EdgeRouteTarget {
            domain_id: id.into(),
            host: host.into(),
            application_name: "app".into(),
            routable,
        }
    }

    #[test]
    fn classify_edge_rows_covers_ok_missing_pending_orphaned() {
        let targets = vec![
            target("d-ok", "ok.example.com", true),
            target("d-missing", "m.example.com", true),
            target("d-pending", "p.example.com", false),
        ];
        let disk: HashSet<String> = ["d-ok".to_string(), "d-orphan".to_string()]
            .into_iter()
            .collect();

        let rows = classify_edge_rows(&targets, &disk, false);
        let status = |id: &str| {
            rows.iter()
                .find(|r| r.domain_id == id)
                .unwrap_or_else(|| panic!("no row for {id}"))
                .status
                .clone()
        };
        assert_eq!(status("d-ok"), "ok");
        assert_eq!(status("d-missing"), "missing");
        assert_eq!(status("d-pending"), "pending");
        assert_eq!(status("d-orphan"), "orphaned");
    }

    #[test]
    fn classify_edge_rows_hides_orphans_and_trusts_disk_when_helper_is_stale() {
        let targets = vec![target("d-ok", "ok.example.com", true)];
        let disk: HashSet<String> = ["d-stale-orphan".to_string()].into_iter().collect();

        let rows = classify_edge_rows(&targets, &disk, true);
        assert_eq!(
            rows.len(),
            1,
            "no orphan rows when the helper cannot be asked"
        );
        assert_eq!(rows[0].status, "ok", "a routable domain is assumed present");
    }
}
