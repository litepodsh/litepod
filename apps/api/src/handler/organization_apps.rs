use axum::{
    extract::State,
    response::{IntoResponse, Response},
};

use crate::{
    cqrs::queries::organization_apps::{list_organization_apps, list_organization_apps_stats},
    dtos::response::{AppError, ok},
    security::authz::Permission,
    security::authz::extract::{RequireOrg, perm},
    state::state::SharedState,
};

/// The organization-wide application list behind the home "Apps" table and the
/// apps/domains/analytics pages. One request replaces the per-application
/// deployment and log-target calls those pages used to fan out.
#[utoipa::path(
    get,
    path = "/api/v1/organizations/{organization_id}/apps/summary",
    params(("organization_id" = String, Path, description = "Organization UUID")),
    responses((status = 200, description = "Applications with status, domains, last deploy and container counts")),
    tag = "projects"
)]
pub async fn list_organization_apps_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgRead>,
) -> Result<Response, AppError> {
    let visible_projects = org.permissions.visible_projects(Permission::ProjectRead);
    let apps = list_organization_apps(&state, &org.organization_id).await?;
    // Same scoping the project hierarchy applies: a project the caller cannot
    // read hides all of its applications, and so does an unreadable environment.
    let apps = apps
        .into_iter()
        .filter(|app| {
            visible_projects.contains(&app.project.id)
                && org
                    .permissions
                    .visible_environments(Permission::AppRead, &app.project.id)
                    .contains(&app.environment.id)
        })
        .collect::<Vec<_>>();
    Ok(ok(apps).into_response())
}

/// Live stats for every application in the organization in one request. The
/// home overview used to fan out one `/stats` request per application; this
/// returns the same shape keyed by application id so a single poll updates the
/// whole board.
#[utoipa::path(
    get,
    path = "/api/v1/organizations/{organization_id}/applications-stats",
    params(("organization_id" = String, Path, description = "Organization UUID")),
    responses((status = 200, description = "Live stats for the organization's applications")),
    tag = "projects"
)]
pub async fn list_organization_apps_stats_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgRead>,
) -> Result<Response, AppError> {
    let visible_projects = org.permissions.visible_projects(Permission::ProjectRead);
    let stats = list_organization_apps_stats(&state, &org.organization_id).await?;
    // Same scoping the apps summary applies: hide applications under a project
    // or environment the caller cannot read.
    let stats = stats
        .into_iter()
        .filter(|row| {
            visible_projects.contains(&row.project_id)
                && org
                    .permissions
                    .visible_environments(Permission::AppRead, &row.project_id)
                    .contains(&row.environment_id)
        })
        .map(|row| row.stats)
        .collect::<Vec<_>>();
    Ok(ok(stats).into_response())
}
