use axum::{
    extract::State,
    response::{IntoResponse, Response},
};

use crate::{
    cqrs::queries::dashboard::organization_dashboard,
    dtos::response::{AppError, ok},
    security::authz::extract::{RequireOrg, perm},
    security::authz::{Permission, Visible},
    state::state::SharedState,
};

#[utoipa::path(
    get,
    path = "/api/v1/organizations/{organization_id}/dashboard",
    params(("organization_id" = String, Path, description = "Organization UUID")),
    responses((status = 200, description = "Home dashboard summary")),
    tag = "projects"
)]
pub async fn organization_dashboard_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgRead>,
) -> Result<Response, AppError> {
    let organization_id = &org.organization_id;
    let visible = org.permissions.visible_projects(Permission::ProjectRead);
    let key = format!("litepod:cache:dashboard:{organization_id}");
    // The cache holds the whole organization's dashboard; filtering happens on
    // the way out so a scoped user's view is never what gets cached.
    if let Ok(Some(cached)) = state.ephemeral.get_json::<serde_json::Value>(&key).await {
        return Ok(ok(visible_only(cached, &visible)).into_response());
    }
    let dashboard = organization_dashboard(&state, organization_id).await?;
    let value =
        serde_json::to_value(&dashboard).map_err(|e| AppError::InternalError(e.to_string()))?;
    if let Err(error) = state.ephemeral.set_json(&key, &value, 15).await {
        tracing::debug!(%error, "Dragonfly dashboard cache unavailable");
    }
    Ok(ok(visible_only(value, &visible)).into_response())
}

/// Drops `projects[]` entries whose `project.id` the caller can't read, and
/// recounts `total_projects` to match.
fn visible_only(mut dashboard: serde_json::Value, visible: &Visible) -> serde_json::Value {
    if matches!(visible, Visible::All) {
        return dashboard;
    }
    if let Some(projects) = dashboard.get_mut("projects").and_then(|p| p.as_array_mut()) {
        projects.retain(|entry| {
            entry
                .pointer("/project/id")
                .and_then(|id| id.as_str())
                .is_some_and(|id| visible.contains(id))
        });
        let total = projects.len();
        dashboard["total_projects"] = total.into();
    }
    dashboard
}

#[cfg(test)]
mod visibility_tests {
    use super::*;

    #[test]
    fn a_scoped_view_keeps_only_visible_projects_and_recounts() {
        let dashboard = serde_json::json!({
            "total_projects": 2,
            "projects": [
                { "project": { "id": "p1" }, "environment": null },
                { "project": { "id": "p2" }, "environment": null }
            ]
        });
        let visible = Visible::Only(["p2".to_string()].into_iter().collect());
        let filtered = visible_only(dashboard, &visible);
        assert_eq!(filtered["total_projects"], 1);
        assert_eq!(filtered["projects"][0]["project"]["id"], "p2");
    }
}

pub async fn invalidate_organization_dashboard(state: &SharedState, organization_id: &str) {
    let key = format!("litepod:cache:dashboard:{organization_id}");
    if let Err(error) = state.ephemeral.delete(&key).await {
        tracing::debug!(%error, "Dragonfly dashboard invalidation unavailable");
    }
}
