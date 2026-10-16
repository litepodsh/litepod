use axum::{
    Json,
    extract::{Multipart, Path, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};

use crate::cqrs::commands::project as project_cmd;
use crate::cqrs::queries::project as project_q;
use crate::dtos::project::{
    CreateApplicationRequest, CreateEnvironmentRequest, CreateProjectRequest,
    UpdateApplicationRequest, UpdateEnvironmentRequest, UpdateProjectRequest,
};
use crate::dtos::response::{AppError, created, ok};
use crate::icons;
use crate::security::authz::Permission;
use crate::security::authz::extract::{RequireApp, RequireEnv, RequireOrg, RequireProject, perm};
use crate::state::state::SharedState;

#[utoipa::path(
    get,
    path = "/api/v1/organizations/{organization_id}/projects",
    params(("organization_id" = String, Path, description = "Organization UUID")),
    responses((status = 200, description = "Projects for the organization")),
    tag = "projects"
)]
pub async fn list_projects_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgRead>,
) -> Result<impl IntoResponse, AppError> {
    let visible = org.permissions.visible_projects(Permission::ProjectRead);
    let projects = project_q::list_projects(&state, &org.organization_id).await?;
    // ponytail: filters in Rust after fetching the org's projects; move into SQL if an org ever has thousands.
    Ok(ok(projects
        .into_iter()
        .filter(|p| visible.contains(&p.id))
        .collect::<Vec<_>>()))
}

#[utoipa::path(
    post,
    path = "/api/v1/organizations/{organization_id}/projects",
    params(("organization_id" = String, Path, description = "Organization UUID")),
    request_body = CreateProjectRequest,
    responses((status = 201, description = "Project created")),
    tag = "projects"
)]
pub async fn create_project_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ProjectCreate>,
    Json(req): Json<CreateProjectRequest>,
) -> Result<impl IntoResponse, AppError> {
    let organization_id = org.organization_id;
    let project = project_cmd::create_project(&state, &organization_id, &req).await?;
    crate::handler::dashboard::invalidate_organization_dashboard(&state, &organization_id).await;
    crate::git::repositories::audit_log::record(
        &state,
        &organization_id,
        &org.user,
        "project.created",
        Some("project"),
        Some(&project.id),
        Some(&project.name),
        None,
        None,
        None,
        None,
    )
    .await
    .ok();
    Ok(created(project))
}

#[utoipa::path(
    get,
    path = "/api/v1/organizations/{organization_id}/projects/{project_id}",
    params(
        ("organization_id" = String, Path, description = "Organization UUID"),
        ("project_id" = String, Path, description = "Project UUID")
    ),
    responses((status = 200, description = "Project hierarchy")),
    tag = "projects"
)]
pub async fn get_project_handler(
    State(state): State<SharedState>,
    RequireProject(scope, _): RequireProject<perm::ProjectRead>,
) -> Result<impl IntoResponse, AppError> {
    let visible = scope
        .permissions
        .visible_environments(Permission::AppRead, &scope.project_id);
    let mut detail =
        project_q::get_project(&state, &scope.organization_id, &scope.project_id).await?;
    detail.environments.retain(|e| visible.contains(&e.id));
    Ok(ok(detail))
}

#[utoipa::path(
    patch,
    path = "/api/v1/organizations/{organization_id}/projects/{project_id}",
    params(
        ("organization_id" = String, Path, description = "Organization UUID"),
        ("project_id" = String, Path, description = "Project UUID")
    ),
    request_body = UpdateProjectRequest,
    responses((status = 200, description = "Project updated")),
    tag = "projects"
)]
pub async fn update_project_handler(
    State(state): State<SharedState>,
    RequireProject(org, _): RequireProject<perm::ProjectUpdate>,
    Json(req): Json<UpdateProjectRequest>,
) -> Result<impl IntoResponse, AppError> {
    let organization_id = org.organization_id;
    let project_id = org.project_id;
    let project = project_cmd::update_project(&state, &organization_id, &project_id, &req).await?;
    crate::handler::dashboard::invalidate_organization_dashboard(&state, &organization_id).await;
    crate::git::repositories::audit_log::record(
        &state,
        &organization_id,
        &org.user,
        "project.updated",
        Some("project"),
        Some(&project_id),
        Some(&project.name),
        None,
        None,
        None,
        None,
    )
    .await
    .ok();
    Ok(ok(project))
}

#[utoipa::path(
    delete,
    path = "/api/v1/organizations/{organization_id}/projects/{project_id}",
    params(
        ("organization_id" = String, Path, description = "Organization UUID"),
        ("project_id" = String, Path, description = "Project UUID")
    ),
    responses((status = 204, description = "Project deleted")),
    tag = "projects"
)]
pub async fn delete_project_handler(
    State(state): State<SharedState>,
    RequireProject(org, _): RequireProject<perm::ProjectDelete>,
) -> Result<impl IntoResponse, AppError> {
    let organization_id = org.organization_id;
    let project_id = org.project_id;
    // Fetched before delete — the name would otherwise be gone.
    let name = project_q::get_project(&state, &organization_id, &project_id)
        .await
        .ok()
        .map(|p| p.project.name);
    project_cmd::delete_project(&state, &organization_id, &project_id).await?;
    crate::handler::dashboard::invalidate_organization_dashboard(&state, &organization_id).await;
    crate::git::repositories::audit_log::record(
        &state,
        &organization_id,
        &org.user,
        "project.deleted",
        Some("project"),
        Some(&project_id),
        name.as_deref(),
        None,
        None,
        None,
        None,
    )
    .await
    .ok();
    Ok(StatusCode::NO_CONTENT)
}

// ── Icon ─────────────────────────────────────────────────────────────────────

#[utoipa::path(
    post,
    path = "/api/v1/organizations/{organization_id}/projects/{project_id}/icon",
    params(
        ("organization_id" = String, Path, description = "Organization UUID"),
        ("project_id" = String, Path, description = "Project UUID")
    ),
    responses((status = 200, description = "Project icon uploaded")),
    tag = "projects"
)]
pub async fn upload_project_icon_handler(
    State(state): State<SharedState>,
    RequireProject(org, _): RequireProject<perm::ProjectUpdate>,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, AppError> {
    let organization_id = org.organization_id;
    let project_id = org.project_id;
    let field = multipart
        .next_field()
        .await
        .map_err(|e| AppError::ValidationError(format!("Invalid multipart upload: {e}")))?
        .ok_or_else(|| AppError::ValidationError("Choose one icon to upload".into()))?;
    if field.name() != Some("icon") {
        return Err(AppError::ValidationError(
            "Upload field must be named icon".into(),
        ));
    }
    let bytes = field
        .bytes()
        .await
        .map_err(|e| AppError::ValidationError(format!("Could not read upload: {e}")))?;
    if bytes.len() > icons::MAX_ICON_BYTES {
        return Err(AppError::ValidationError(format!(
            "Icon exceeds the {}MiB upload limit",
            icons::MAX_ICON_BYTES / (1024 * 1024)
        )));
    }
    if multipart
        .next_field()
        .await
        .map_err(|e| AppError::ValidationError(format!("Invalid multipart upload: {e}")))?
        .is_some()
    {
        return Err(AppError::ValidationError("Upload exactly one icon".into()));
    }

    let filename = icons::store(&state, "projects", &project_id, &bytes).await?;
    let icon_url =
        format!("/api/v1/organizations/{organization_id}/projects/{project_id}/icon/{filename}");
    let project = project_cmd::update_project(
        &state,
        &organization_id,
        &project_id,
        &UpdateProjectRequest {
            name: None,
            description: None,
            icon_url: Some(Some(icon_url)),
        },
    )
    .await?;
    Ok(ok(project))
}

#[utoipa::path(
    get,
    path = "/api/v1/organizations/{organization_id}/projects/{project_id}/icon/{filename}",
    params(
        ("organization_id" = String, Path, description = "Organization UUID"),
        ("project_id" = String, Path, description = "Project UUID"),
        ("filename" = String, Path, description = "Stored icon filename")
    ),
    responses((status = 200, description = "Project icon bytes")),
    tag = "projects"
)]
pub async fn serve_project_icon_handler(
    State(state): State<SharedState>,
    RequireProject(scope, _): RequireProject<perm::ProjectRead>,
    // The extractor covers `organization_id` and `project_id`.
    Path((_organization_id, _project_id, filename)): Path<(String, String, String)>,
) -> Result<Response, AppError> {
    let project_id = scope.project_id;
    let bytes = icons::read(&state, "projects", &project_id, &filename).await?;
    let content_type = icons::content_type_for(&filename);
    let mut response = Response::new(axum::body::Body::from(bytes));
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static(content_type),
    );
    headers.insert(
        header::HeaderName::from_static("x-content-type-options"),
        axum::http::HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("private, max-age=31536000, immutable"),
    );
    if content_type == "image/svg+xml" {
        headers.insert(
            header::HeaderName::from_static("content-security-policy"),
            axum::http::HeaderValue::from_static("script-src 'none'"),
        );
    }
    Ok(response)
}

#[utoipa::path(
    post,
    path = "/api/v1/organizations/{organization_id}/projects/{project_id}/environments",
    params(
        ("organization_id" = String, Path, description = "Organization UUID"),
        ("project_id" = String, Path, description = "Project UUID")
    ),
    request_body = CreateEnvironmentRequest,
    responses((status = 201, description = "Environment created")),
    tag = "projects"
)]
pub async fn create_environment_handler(
    State(state): State<SharedState>,
    RequireProject(org, _): RequireProject<perm::EnvCreate>,
    Json(req): Json<CreateEnvironmentRequest>,
) -> Result<impl IntoResponse, AppError> {
    let organization_id = org.organization_id;
    let project_id = org.project_id;
    let environment =
        project_cmd::create_environment(&state, &organization_id, &project_id, &req).await?;
    crate::handler::dashboard::invalidate_organization_dashboard(&state, &organization_id).await;
    crate::git::repositories::audit_log::record(
        &state,
        &organization_id,
        &org.user,
        "environment.created",
        Some("environment"),
        Some(&environment.id),
        Some(&environment.name),
        None,
        None,
        None,
        None,
    )
    .await
    .ok();
    Ok(created(environment))
}

#[utoipa::path(
    patch,
    path = "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}",
    params(
        ("organization_id" = String, Path, description = "Organization UUID"),
        ("project_id" = String, Path, description = "Project UUID"),
        ("environment_id" = String, Path, description = "Environment UUID")
    ),
    request_body = UpdateEnvironmentRequest,
    responses((status = 200, description = "Environment updated")),
    tag = "projects"
)]
pub async fn update_environment_handler(
    State(state): State<SharedState>,
    RequireEnv(org, _): RequireEnv<perm::EnvUpdate>,
    Json(req): Json<UpdateEnvironmentRequest>,
) -> Result<impl IntoResponse, AppError> {
    let organization_id = org.organization_id;
    let project_id = org.project_id;
    let environment_id = org.environment_id;
    let environment = project_cmd::update_environment(
        &state,
        &organization_id,
        &project_id,
        &environment_id,
        &req,
    )
    .await?;
    crate::handler::dashboard::invalidate_organization_dashboard(&state, &organization_id).await;
    crate::git::repositories::audit_log::record(
        &state,
        &organization_id,
        &org.user,
        "environment.updated",
        Some("environment"),
        Some(&environment_id),
        Some(&environment.name),
        None,
        None,
        None,
        None,
    )
    .await
    .ok();
    Ok(ok(environment))
}

#[utoipa::path(
    delete,
    path = "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}",
    params(
        ("organization_id" = String, Path, description = "Organization UUID"),
        ("project_id" = String, Path, description = "Project UUID"),
        ("environment_id" = String, Path, description = "Environment UUID")
    ),
    responses((status = 204, description = "Environment deleted")),
    tag = "projects"
)]
pub async fn delete_environment_handler(
    State(state): State<SharedState>,
    RequireEnv(org, _): RequireEnv<perm::EnvDelete>,
) -> Result<impl IntoResponse, AppError> {
    let organization_id = org.organization_id;
    let project_id = org.project_id;
    let environment_id = org.environment_id;
    let name = crate::git::repositories::project::get_environment(
        &state,
        &organization_id,
        &project_id,
        &environment_id,
    )
    .await
    .ok()
    .flatten()
    .map(|e| e.name);
    project_cmd::delete_environment(&state, &organization_id, &project_id, &environment_id).await?;
    crate::handler::dashboard::invalidate_organization_dashboard(&state, &organization_id).await;
    crate::git::repositories::audit_log::record(
        &state,
        &organization_id,
        &org.user,
        "environment.deleted",
        Some("environment"),
        Some(&environment_id),
        name.as_deref(),
        None,
        None,
        None,
        None,
    )
    .await
    .ok();
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications",
    params(
        ("organization_id" = String, Path, description = "Organization UUID"),
        ("project_id" = String, Path, description = "Project UUID"),
        ("environment_id" = String, Path, description = "Environment UUID")
    ),
    request_body = CreateApplicationRequest,
    responses((status = 201, description = "Application created")),
    tag = "projects"
)]
pub async fn create_application_handler(
    State(state): State<SharedState>,
    RequireEnv(org, _): RequireEnv<perm::AppCreate>,
    Json(req): Json<CreateApplicationRequest>,
) -> Result<impl IntoResponse, AppError> {
    let organization_id = org.organization_id;
    let project_id = org.project_id;
    let environment_id = org.environment_id;
    let application = project_cmd::create_application(
        &state,
        &organization_id,
        &project_id,
        &environment_id,
        &req,
    )
    .await?;
    crate::handler::dashboard::invalidate_organization_dashboard(&state, &organization_id).await;
    crate::git::repositories::audit_log::record(
        &state,
        &organization_id,
        &org.user,
        "application.created",
        Some("application"),
        Some(&application.id),
        Some(&application.name),
        None,
        None,
        None,
        None,
    )
    .await
    .ok();
    Ok(created(application))
}

#[utoipa::path(
    get,
    path = "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}",
    params(
        ("organization_id" = String, Path, description = "Organization UUID"),
        ("project_id" = String, Path, description = "Project UUID"),
        ("environment_id" = String, Path, description = "Environment UUID"),
        ("application_id" = String, Path, description = "Application UUID")
    ),
    responses((status = 200, description = "Application details")),
    tag = "projects"
)]
pub async fn get_application_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppRead>,
) -> Result<impl IntoResponse, AppError> {
    let organization_id = app.organization_id;
    let (project_id, environment_id, application_id) =
        (app.project_id, app.environment_id, app.application_id);
    Ok(ok(project_q::get_application(
        &state,
        &organization_id,
        &project_id,
        &environment_id,
        &application_id,
    )
    .await?))
}

pub async fn sample_resource_baseline_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppRead>,
) -> Result<impl IntoResponse, AppError> {
    Ok(ok(project_q::sample_resource_baseline(
        &state,
        &app.organization_id,
        &app.project_id,
        &app.environment_id,
        &app.application_id,
    )
    .await?))
}

#[utoipa::path(
    patch,
    path = "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}",
    params(
        ("organization_id" = String, Path, description = "Organization UUID"),
        ("project_id" = String, Path, description = "Project UUID"),
        ("environment_id" = String, Path, description = "Environment UUID"),
        ("application_id" = String, Path, description = "Application UUID")
    ),
    request_body = UpdateApplicationRequest,
    responses((status = 200, description = "Application updated")),
    tag = "projects"
)]
pub async fn update_application_handler(
    State(state): State<SharedState>,
    RequireApp(org, _): RequireApp<perm::AppUpdate>,
    Json(req): Json<UpdateApplicationRequest>,
) -> Result<impl IntoResponse, AppError> {
    let organization_id = org.organization_id;
    let (project_id, environment_id, application_id) =
        (org.project_id, org.environment_id, org.application_id);
    let application = project_cmd::update_application(
        &state,
        &organization_id,
        &project_id,
        &environment_id,
        &application_id,
        &req,
    )
    .await?;
    crate::handler::dashboard::invalidate_organization_dashboard(&state, &organization_id).await;
    crate::git::repositories::audit_log::record(
        &state,
        &organization_id,
        &org.user,
        "application.updated",
        Some("application"),
        Some(&application_id),
        Some(&application.name),
        None,
        None,
        None,
        None,
    )
    .await
    .ok();
    Ok(ok(application))
}

#[utoipa::path(
    delete,
    path = "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}",
    params(
        ("organization_id" = String, Path, description = "Organization UUID"),
        ("project_id" = String, Path, description = "Project UUID"),
        ("environment_id" = String, Path, description = "Environment UUID"),
        ("application_id" = String, Path, description = "Application UUID")
    ),
    responses((status = 204, description = "Application deleted")),
    tag = "projects"
)]
pub async fn delete_application_handler(
    State(state): State<SharedState>,
    RequireApp(org, _): RequireApp<perm::AppDelete>,
) -> Result<impl IntoResponse, AppError> {
    let organization_id = org.organization_id;
    let (project_id, environment_id, application_id) =
        (org.project_id, org.environment_id, org.application_id);
    let name = project_q::get_application(
        &state,
        &organization_id,
        &project_id,
        &environment_id,
        &application_id,
    )
    .await
    .ok()
    .map(|a| a.name);
    project_cmd::delete_application(
        &state,
        &organization_id,
        &project_id,
        &environment_id,
        &application_id,
    )
    .await?;
    crate::handler::dashboard::invalidate_organization_dashboard(&state, &organization_id).await;
    crate::git::repositories::audit_log::record(
        &state,
        &organization_id,
        &org.user,
        "application.deleted",
        Some("application"),
        Some(&application_id),
        name.as_deref(),
        None,
        None,
        None,
        None,
    )
    .await
    .ok();
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use axum::{body::to_bytes, response::IntoResponse};

    use super::*;
    use crate::dtos::project::CreateProjectRequest;
    // The two-organization fixture lives in `crate::testing` so the org_scope
    // tests can share it instead of keeping a second copy in sync.
    use crate::testing::{auth_user, cleanup_database, granted, granted_env, test_state};
    use std::marker::PhantomData;

    // Role and membership assertions used to live here, asserted by calling the
    // handler directly. They moved to `middleware::org_scope`'s tests, which drive
    // a real router so the extractor actually runs — proving it once for every
    // organization-scoped route instead of once per handler. What is left here is
    // handler logic: scoping a child id, and refusing to delete production.

    #[tokio::test]
    async fn project_handlers_enforce_membership_roles_scope_and_production_protection() {
        let (state, path) = test_state().await;
        let create_request = CreateProjectRequest {
            name: "litepod".to_owned(),
            description: None,
        };
        let created = create_project_handler(
            State(state.clone()),
            RequireOrg(granted("organization-a", "editor", 2), PhantomData),
            Json(create_request),
        )
        .await
        .unwrap()
        .into_response();
        assert_eq!(created.status(), StatusCode::CREATED);

        let project = project_cmd::create_project(
            &state,
            "organization-a",
            &CreateProjectRequest {
                name: "Scoped project".to_owned(),
                description: None,
            },
        )
        .await
        .unwrap();
        // A project id from another organization is refused by `RequireProject`
        // itself (404 before any permission check) — see
        // `security::authz::extract` tests; the handler never sees it.

        let detail = project_q::get_project(&state, "organization-a", &project.id)
            .await
            .unwrap();
        let production = &detail.environments[0];
        let protected_delete = match delete_environment_handler(
            State(state.clone()),
            RequireEnv(
                granted_env("organization-a", &project.id, &production.id, "editor", 2),
                PhantomData,
            ),
        )
        .await
        {
            Err(error) => error.into_response(),
            Ok(_) => panic!("Production environment deletion should be rejected"),
        };
        assert_eq!(protected_delete.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body = to_bytes(protected_delete.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(
            std::str::from_utf8(&body)
                .unwrap()
                .contains("Production environment cannot be deleted")
        );

        drop(state);
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn a_member_lists_only_projects_it_was_granted() {
        let (state, path) = test_state().await;
        let (granted_project, _, _) =
            crate::testing::seed_application(&state, "organization-a").await;
        sqlx::query(
            "INSERT INTO projects (id, organization_id, name, created_at, updated_at) VALUES ('hidden', 'organization-a', 'hidden', 0, 0)",
        )
        .execute(&state.db)
        .await
        .unwrap();
        crate::testing::set_base_role(&state, "organization-a", "viewer", 4).await;
        crate::testing::assign(
            &state,
            "organization-a",
            "viewer",
            3,
            "project",
            &granted_project,
        )
        .await;

        let router = axum::Router::new()
            .route(
                "/api/v1/organizations/{organization_id}/projects",
                axum::routing::get(list_projects_handler),
            )
            .layer(axum::middleware::from_fn(
                |mut req: axum::extract::Request, next: axum::middleware::Next| async move {
                    req.extensions_mut().insert(auth_user("viewer"));
                    next.run(req).await
                },
            ))
            .with_state(state);
        let response = tower::ServiceExt::oneshot(
            router,
            axum::http::Request::builder()
                .uri("/api/v1/organizations/organization-a/projects")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = String::from_utf8(
            to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        assert!(body.contains(&granted_project), "{body}");
        assert!(!body.contains("\"hidden\""), "{body}");
        cleanup_database(&path);
    }
}
