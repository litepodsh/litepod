use axum::{
    extract::State,
    response::{IntoResponse, Response},
};

use crate::{
    cqrs::queries::organization_apps::list_organization_apps,
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
