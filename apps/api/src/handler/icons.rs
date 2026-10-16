//! Application icon upload/serve — the application-scoped sibling of
//! `handler::organizations::{upload_icon_handler, serve_icon_handler}`. Kept
//! in its own file rather than folded into `handler::projects` since both
//! entity kinds share the same shape and the same underlying `icons` module.

use axum::{
    extract::{Multipart, Path, State},
    http::header,
    response::{IntoResponse, Response},
};

use crate::cqrs::commands::project as project_cmd;
use crate::dtos::project::UpdateApplicationRequest;
use crate::dtos::response::{AppError, ok};
use crate::icons;
use crate::security::authz::extract::{RequireApp, perm};
use crate::state::state::SharedState;

// Leading four segments (organization_id, project_id, environment_id,
// application_id) plus one trailing — same shape as `DomainPath`/`WorkspacePath`
// in `handler::deployments`.
type IconPath = (String, String, String, String, String);

#[utoipa::path(
    post, path = "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/icon",
    responses((status = 200, description = "Application icon uploaded")),
    tag = "projects"
)]
pub async fn upload_application_icon_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppUpdate>,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, AppError> {
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

    let filename = icons::store(&state, "applications", &app.application_id, &bytes).await?;
    let icon_url = format!(
        "/api/v1/organizations/{}/projects/{}/environments/{}/applications/{}/icon/{filename}",
        app.organization_id, app.project_id, app.environment_id, app.application_id
    );
    let application = project_cmd::update_application(
        &state,
        &app.organization_id,
        &app.project_id,
        &app.environment_id,
        &app.application_id,
        &UpdateApplicationRequest {
            name: None,
            description: None,
            icon_url: Some(Some(icon_url)),
            server_id: None,
            server_group_id: None,
        },
    )
    .await?;
    Ok(ok(application))
}

// No #[utoipa::path] here — utoipa's macro can't resolve a destructured tuple
// `Path` pattern (same reason `update_domain_handler` skips it).
pub async fn serve_application_icon_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppRead>,
    // The extractor covers the leading segments.
    Path((_, _, _, _, filename)): Path<IconPath>,
) -> Result<Response, AppError> {
    let bytes = icons::read(&state, "applications", &app.application_id, &filename).await?;
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
