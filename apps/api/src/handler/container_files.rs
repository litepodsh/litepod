//! File explorer over a container's own filesystem. Every route needs `app.exec`: browsing and
//! editing a container's files is as powerful as a shell in it, so it gets the terminal's floor.

use axum::{
    Json,
    extract::{Multipart, Path, Query, State},
    http::header,
    response::{IntoResponse, Response},
};

use crate::{
    dtos::{
        deployment::{CreateWorkspaceEntryRequest, UpdateWorkspaceTextRequest},
        response::{AppError, ok},
    },
    git::repositories::audit_log,
    handler::deployments::container_names,
    podman::containers::container_files,
    security::authz::extract::{RequireApp, perm},
    servers::runtime::ServerRuntime,
    state::state::SharedState,
};

type ContainerPath = (String, String, String, String, String);

#[derive(serde::Deserialize)]
pub struct FilePathQuery {
    pub path: String,
}

#[derive(serde::Deserialize, Default)]
pub struct FileListQuery {
    pub path: Option<String>,
}

/// The runtime for `container_id`, once it is proven to belong to the application. The
/// extractor only proves the application is the caller's; a guessed container name must not
/// reach another tenant's container.
async fn target(
    state: &SharedState,
    application_id: &str,
    container_id: &str,
) -> Result<ServerRuntime, AppError> {
    if !container_names(state, application_id)
        .await?
        .iter()
        .any(|name| name == container_id)
    {
        return Err(AppError::ResourceNotFound);
    }
    ServerRuntime::for_application(state, application_id).await
}

async fn audit(
    state: &SharedState,
    app: &crate::security::middleware::org_scope::AppScope,
    action: &str,
    container: &str,
    path: &str,
) {
    audit_log::record(
        state,
        &app.organization_id,
        &app.user,
        action,
        Some("container"),
        Some(container),
        Some(path),
        None,
        None,
        None,
        None,
    )
    .await
    .ok();
}

pub async fn list_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppExec>,
    Path((_, _, _, _, container_id)): Path<ContainerPath>,
    Query(query): Query<FileListQuery>,
) -> Result<impl IntoResponse, AppError> {
    let runtime = target(&state, &app.application_id, &container_id).await?;
    Ok(ok(
        container_files::list(&runtime, &container_id, query.path.as_deref()).await?,
    ))
}

pub async fn read_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppExec>,
    Path((_, _, _, _, container_id)): Path<ContainerPath>,
    Query(query): Query<FilePathQuery>,
) -> Result<impl IntoResponse, AppError> {
    let runtime = target(&state, &app.application_id, &container_id).await?;
    Ok(ok(
        container_files::read_file(&runtime, &container_id, &query.path).await?,
    ))
}

pub async fn save_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppExec>,
    Path((_, _, _, _, container_id)): Path<ContainerPath>,
    Query(query): Query<FilePathQuery>,
    Json(request): Json<UpdateWorkspaceTextRequest>,
) -> Result<impl IntoResponse, AppError> {
    let runtime = target(&state, &app.application_id, &container_id).await?;
    let result = container_files::write_text_file(
        &runtime,
        &app.application_id,
        &container_id,
        &query.path,
        request.content,
    )
    .await?;
    audit(&state, &app, "container.file_updated", &container_id, &query.path).await;
    Ok(ok(result))
}

pub async fn create_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppExec>,
    Path((_, _, _, _, container_id)): Path<ContainerPath>,
    Json(request): Json<CreateWorkspaceEntryRequest>,
) -> Result<impl IntoResponse, AppError> {
    let runtime = target(&state, &app.application_id, &container_id).await?;
    let result = container_files::create_entry(
        &runtime,
        &app.application_id,
        &container_id,
        &request.path,
        &request.kind,
        request.content.unwrap_or_default().into_bytes(),
    )
    .await?;
    audit(&state, &app, "container.file_created", &container_id, &request.path).await;
    Ok(ok(result))
}

pub async fn upload_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppExec>,
    Path((_, _, _, _, container_id)): Path<ContainerPath>,
    Query(query): Query<FilePathQuery>,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, AppError> {
    let runtime = target(&state, &app.application_id, &container_id).await?;
    let field = multipart
        .next_field()
        .await
        .map_err(|error| AppError::ValidationError(error.to_string()))?
        .ok_or_else(|| AppError::ValidationError("Choose one file to upload".into()))?;
    let bytes = field
        .bytes()
        .await
        .map_err(|error| AppError::InternalError(error.to_string()))?;
    let result = container_files::create_entry(
        &runtime,
        &app.application_id,
        &container_id,
        &query.path,
        "file",
        bytes.to_vec(),
    )
    .await?;
    audit(&state, &app, "container.file_uploaded", &container_id, &query.path).await;
    Ok(ok(result))
}

pub async fn download_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppExec>,
    Path((_, _, _, _, container_id)): Path<ContainerPath>,
    Query(query): Query<FilePathQuery>,
) -> Result<Response, AppError> {
    let runtime = target(&state, &app.application_id, &container_id).await?;
    let (bytes, mime) =
        container_files::read_file_bytes(&runtime, &container_id, &query.path).await?;
    audit(&state, &app, "container.file_downloaded", &container_id, &query.path).await;
    let filename = std::path::Path::new(&query.path)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .replace(['"', '\\', '\r', '\n'], "_");
    let mut response = Response::new(axum::body::Body::from(bytes));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        axum::http::HeaderValue::from_str(&mime.unwrap_or_else(|| "application/octet-stream".into()))
            .map_err(|error| AppError::InternalError(error.to_string()))?,
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        axum::http::HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
            .map_err(|error| AppError::InternalError(error.to_string()))?,
    );
    Ok(response)
}
