use axum::{
    Json,
    extract::{Path, State},
};
use url::Url;
use uuid::Uuid;

use crate::security::authz::extract::{RequireOrg, perm};
use crate::{
    deployment::{self, compose::store::ComposeFileStore},
    dtos::{
        response::{AppError, created, ok},
        template::{
            DragonflyDbPasswordRotationRequest, DragonflyDbTemplateConfigUpdate,
            DragonflyDbTemplateInput, KeyDbPasswordRotationRequest, KeyDbTemplateConfigUpdate,
            KeyDbTemplateInput, MariaDbPasswordRotationRequest, MariaDbTemplateConfigUpdate,
            MariaDbTemplateInput, MongoDbPasswordRotationRequest, MongoDbTemplateConfigUpdate,
            MongoDbTemplateInput, PostgresPasswordRotationRequest, PostgresTemplateConfigUpdate,
            PostgresTemplateInput, SqlServerPasswordRotationRequest, SqlServerTemplateConfigUpdate,
            SqlServerTemplateInput,
        },
    },
    git::repositories::project,
    state::state::SharedState,
    templates::internal_templates::{dragonflydb, keydb, mariadb, mongodb, postgres, sqlserver},
};

type TemplatePath = (String, String, String);
type TemplateResourcePath = (String, String, String, String);

async fn context(
    state: &SharedState,
    organization_id: &str,
    project_id: &str,
    environment_id: &str,
) -> Result<(String, String), AppError> {
    let project = project::get_project(state, organization_id, project_id)
        .await
        .map_err(AppError::DatabaseError)?
        .ok_or(AppError::ProjectNotFound)?;
    let environment = project::get_environment(state, organization_id, project_id, environment_id)
        .await
        .map_err(AppError::DatabaseError)?
        .ok_or(AppError::EnvironmentNotFound)?;
    Ok((project.name, environment.name))
}

pub(crate) fn public_host(state: &SharedState) -> String {
    let configured_host = Url::parse(&state.public_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .filter(|host| host != "localhost");
    configured_host.unwrap_or_else(|| state.public_ip.clone())
}

pub async fn preview_postgres_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::AppCreate>,
    // The extractor covers `organization_id`.
    Path((_, project_id, environment_id)): Path<TemplatePath>,
    Json(mut input): Json<PostgresTemplateInput>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let organization_id = org.organization_id;
    let (project_name, environment_name) =
        context(&state, &organization_id, &project_id, &environment_id).await?;
    let instance_id = input
        .template_instance_id
        .clone()
        .unwrap_or_else(|| Uuid::now_v7().to_string());
    let runtime_name = deployment::deployment_identity(
        &project_name,
        &environment_name,
        &input.application_name,
        &instance_id,
        &state.public_ip,
    )
    .0;
    input.template_instance_id = Some(instance_id);
    let used_ports = ComposeFileStore::list_host_ports(&state)
        .await
        .map_err(AppError::InternalError)?;
    let reserved_ports = state
        .reserved_ports
        .iter()
        .filter_map(|port| u16::try_from(*port).ok())
        .collect();
    Ok(ok(postgres::preview(
        input,
        &format!("{runtime_name}-postgres"),
        &public_host(&state),
        &reserved_ports,
        &used_ports,
    )?))
}

pub async fn create_postgres_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::AppCreate>,
    // The extractor covers `organization_id`.
    Path((_, project_id, environment_id)): Path<TemplatePath>,
    Json(input): Json<PostgresTemplateInput>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let organization_id = org.organization_id;
    let (project_name, environment_name) =
        context(&state, &organization_id, &project_id, &environment_id).await?;
    let dto = postgres::create(
        &state,
        &organization_id,
        &project_id,
        &environment_id,
        &project_name,
        &environment_name,
        &public_host(&state),
        input,
    )
    .await?;
    let _ = crate::git::repositories::audit_log::record(
        &state,
        &organization_id,
        &org.user,
        "postgres.created",
        Some("application"),
        Some(&dto.application.id),
        Some(&dto.application.name),
        None,
        None,
        None,
        None,
    )
    .await;
    Ok(created(dto))
}

pub async fn get_postgres_resource_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::AppRead>,
    // The extractor covers `organization_id`.
    Path((_, project_id, environment_id, application_id)): Path<TemplateResourcePath>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let organization_id = org.organization_id;
    Ok(ok(postgres::resource_detail(
        &state,
        &organization_id,
        &project_id,
        &environment_id,
        &application_id,
        &public_host(&state),
    )
    .await?))
}

pub async fn update_postgres_resource_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::AppUpdate>,
    // The extractor covers `organization_id`.
    Path((_, project_id, environment_id, application_id)): Path<TemplateResourcePath>,
    Json(input): Json<PostgresTemplateConfigUpdate>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let organization_id = org.organization_id;
    Ok(ok(postgres::update_resource(
        &state,
        &organization_id,
        &project_id,
        &environment_id,
        &application_id,
        &public_host(&state),
        input,
    )
    .await?))
}

pub async fn rotate_postgres_password_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::AppEnvUpdate>,
    // The extractor covers `organization_id`.
    Path((_, project_id, environment_id, application_id)): Path<TemplateResourcePath>,
    Json(input): Json<PostgresPasswordRotationRequest>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let organization_id = org.organization_id;
    let result = postgres::rotate_password(
        &state,
        &organization_id,
        &project_id,
        &environment_id,
        &application_id,
        &public_host(&state),
        input.password,
    )
    .await?;
    let _ = crate::git::repositories::audit_log::record(
        &state,
        &organization_id,
        &org.user,
        "postgres.password_rotated",
        Some("application"),
        Some(&application_id),
        None,
        None,
        None,
        None,
        None,
    )
    .await;
    Ok(ok(result))
}

pub async fn preview_mongodb_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::AppCreate>,
    // The extractor covers `organization_id`.
    Path((_, project_id, environment_id)): Path<TemplatePath>,
    Json(mut input): Json<MongoDbTemplateInput>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let organization_id = org.organization_id;
    let (project_name, environment_name) =
        context(&state, &organization_id, &project_id, &environment_id).await?;
    let instance_id = input
        .template_instance_id
        .clone()
        .unwrap_or_else(|| Uuid::now_v7().to_string());
    let runtime_name = deployment::deployment_identity(
        &project_name,
        &environment_name,
        &input.application_name,
        &instance_id,
        &state.public_ip,
    )
    .0;
    input.template_instance_id = Some(instance_id);
    let used_ports = ComposeFileStore::list_host_ports(&state)
        .await
        .map_err(AppError::InternalError)?;
    let reserved_ports = state
        .reserved_ports
        .iter()
        .filter_map(|port| u16::try_from(*port).ok())
        .collect();
    Ok(ok(mongodb::preview(
        input,
        &format!("{runtime_name}-mongo"),
        &public_host(&state),
        &reserved_ports,
        &used_ports,
    )?))
}

pub async fn create_mongodb_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::AppCreate>,
    // The extractor covers `organization_id`.
    Path((_, project_id, environment_id)): Path<TemplatePath>,
    Json(input): Json<MongoDbTemplateInput>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let organization_id = org.organization_id;
    let (project_name, environment_name) =
        context(&state, &organization_id, &project_id, &environment_id).await?;
    let dto = mongodb::create(
        &state,
        &organization_id,
        &project_id,
        &environment_id,
        &project_name,
        &environment_name,
        &public_host(&state),
        input,
    )
    .await?;
    let _ = crate::git::repositories::audit_log::record(
        &state,
        &organization_id,
        &org.user,
        "mongodb.created",
        Some("application"),
        Some(&dto.application.id),
        Some(&dto.application.name),
        None,
        None,
        None,
        None,
    )
    .await;
    Ok(created(dto))
}

pub async fn get_mongodb_resource_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::AppRead>,
    // The extractor covers `organization_id`.
    Path((_, project_id, environment_id, application_id)): Path<TemplateResourcePath>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let organization_id = org.organization_id;
    Ok(ok(mongodb::resource_detail(
        &state,
        &organization_id,
        &project_id,
        &environment_id,
        &application_id,
        &public_host(&state),
    )
    .await?))
}

pub async fn update_mongodb_resource_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::AppUpdate>,
    // The extractor covers `organization_id`.
    Path((_, project_id, environment_id, application_id)): Path<TemplateResourcePath>,
    Json(input): Json<MongoDbTemplateConfigUpdate>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let organization_id = org.organization_id;
    Ok(ok(mongodb::update_resource(
        &state,
        &organization_id,
        &project_id,
        &environment_id,
        &application_id,
        &public_host(&state),
        input,
    )
    .await?))
}

pub async fn rotate_mongodb_password_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::AppEnvUpdate>,
    // The extractor covers `organization_id`.
    Path((_, project_id, environment_id, application_id)): Path<TemplateResourcePath>,
    Json(input): Json<MongoDbPasswordRotationRequest>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let organization_id = org.organization_id;
    let result = mongodb::rotate_password(
        &state,
        &organization_id,
        &project_id,
        &environment_id,
        &application_id,
        &public_host(&state),
        input.password,
    )
    .await?;
    let _ = crate::git::repositories::audit_log::record(
        &state,
        &organization_id,
        &org.user,
        "mongodb.password_rotated",
        Some("application"),
        Some(&application_id),
        None,
        None,
        None,
        None,
        None,
    )
    .await;
    Ok(ok(result))
}

pub async fn preview_mariadb_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::AppCreate>,
    // The extractor covers `organization_id`.
    Path((_, project_id, environment_id)): Path<TemplatePath>,
    Json(mut input): Json<MariaDbTemplateInput>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let organization_id = org.organization_id;
    let (project_name, environment_name) =
        context(&state, &organization_id, &project_id, &environment_id).await?;
    let instance_id = input
        .template_instance_id
        .clone()
        .unwrap_or_else(|| Uuid::now_v7().to_string());
    let runtime_name = deployment::deployment_identity(
        &project_name,
        &environment_name,
        &input.application_name,
        &instance_id,
        &state.public_ip,
    )
    .0;
    input.template_instance_id = Some(instance_id);
    let used_ports = ComposeFileStore::list_host_ports(&state)
        .await
        .map_err(AppError::InternalError)?;
    let reserved_ports = state
        .reserved_ports
        .iter()
        .filter_map(|port| u16::try_from(*port).ok())
        .collect();
    Ok(ok(mariadb::preview(
        input,
        &format!("{runtime_name}-mariadb"),
        &public_host(&state),
        &reserved_ports,
        &used_ports,
    )?))
}

pub async fn create_mariadb_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::AppCreate>,
    // The extractor covers `organization_id`.
    Path((_, project_id, environment_id)): Path<TemplatePath>,
    Json(input): Json<MariaDbTemplateInput>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let organization_id = org.organization_id;
    let (project_name, environment_name) =
        context(&state, &organization_id, &project_id, &environment_id).await?;
    let dto = mariadb::create(
        &state,
        &organization_id,
        &project_id,
        &environment_id,
        &project_name,
        &environment_name,
        &public_host(&state),
        input,
    )
    .await?;
    let _ = crate::git::repositories::audit_log::record(
        &state,
        &organization_id,
        &org.user,
        "mariadb.created",
        Some("application"),
        Some(&dto.application.id),
        Some(&dto.application.name),
        None,
        None,
        None,
        None,
    )
    .await;
    Ok(created(dto))
}

pub async fn get_mariadb_resource_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::AppRead>,
    // The extractor covers `organization_id`.
    Path((_, project_id, environment_id, application_id)): Path<TemplateResourcePath>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let organization_id = org.organization_id;
    Ok(ok(mariadb::resource_detail(
        &state,
        &organization_id,
        &project_id,
        &environment_id,
        &application_id,
        &public_host(&state),
    )
    .await?))
}

pub async fn update_mariadb_resource_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::AppUpdate>,
    // The extractor covers `organization_id`.
    Path((_, project_id, environment_id, application_id)): Path<TemplateResourcePath>,
    Json(input): Json<MariaDbTemplateConfigUpdate>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let organization_id = org.organization_id;
    Ok(ok(mariadb::update_resource(
        &state,
        &organization_id,
        &project_id,
        &environment_id,
        &application_id,
        &public_host(&state),
        input,
    )
    .await?))
}

pub async fn rotate_mariadb_password_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::AppEnvUpdate>,
    // The extractor covers `organization_id`.
    Path((_, project_id, environment_id, application_id)): Path<TemplateResourcePath>,
    Json(input): Json<MariaDbPasswordRotationRequest>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let organization_id = org.organization_id;
    Ok(ok(mariadb::rotate_password(
        &state,
        &organization_id,
        &project_id,
        &environment_id,
        &application_id,
        &public_host(&state),
        input,
    )
    .await?))
}

pub async fn preview_dragonflydb_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::AppCreate>,
    // The extractor covers `organization_id`.
    Path((_, project_id, environment_id)): Path<TemplatePath>,
    Json(mut input): Json<DragonflyDbTemplateInput>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let organization_id = org.organization_id;
    let (project_name, environment_name) =
        context(&state, &organization_id, &project_id, &environment_id).await?;
    let instance_id = input
        .template_instance_id
        .clone()
        .unwrap_or_else(|| Uuid::now_v7().to_string());
    let runtime_name = deployment::deployment_identity(
        &project_name,
        &environment_name,
        &input.application_name,
        &instance_id,
        &state.public_ip,
    )
    .0;
    input.template_instance_id = Some(instance_id);
    let used_ports = ComposeFileStore::list_host_ports(&state)
        .await
        .map_err(AppError::InternalError)?;
    let reserved_ports = state
        .reserved_ports
        .iter()
        .filter_map(|port| u16::try_from(*port).ok())
        .collect();
    Ok(ok(dragonflydb::preview(
        input,
        &format!("{runtime_name}-dragonflydb"),
        &public_host(&state),
        &reserved_ports,
        &used_ports,
    )?))
}

pub async fn create_dragonflydb_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::AppCreate>,
    // The extractor covers `organization_id`.
    Path((_, project_id, environment_id)): Path<TemplatePath>,
    Json(input): Json<DragonflyDbTemplateInput>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let organization_id = org.organization_id;
    let (project_name, environment_name) =
        context(&state, &organization_id, &project_id, &environment_id).await?;
    let dto = dragonflydb::create(
        &state,
        &organization_id,
        &project_id,
        &environment_id,
        &project_name,
        &environment_name,
        &public_host(&state),
        input,
    )
    .await?;
    let _ = crate::git::repositories::audit_log::record(
        &state,
        &organization_id,
        &org.user,
        "dragonflydb.created",
        Some("application"),
        Some(&dto.application.id),
        Some(&dto.application.name),
        None,
        None,
        None,
        None,
    )
    .await;
    Ok(created(dto))
}

pub async fn get_dragonflydb_resource_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::AppRead>,
    // The extractor covers `organization_id`.
    Path((_, project_id, environment_id, application_id)): Path<TemplateResourcePath>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let organization_id = org.organization_id;
    Ok(ok(dragonflydb::resource_detail(
        &state,
        &organization_id,
        &project_id,
        &environment_id,
        &application_id,
        &public_host(&state),
    )
    .await?))
}

pub async fn update_dragonflydb_resource_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::AppUpdate>,
    // The extractor covers `organization_id`.
    Path((_, project_id, environment_id, application_id)): Path<TemplateResourcePath>,
    Json(input): Json<DragonflyDbTemplateConfigUpdate>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let organization_id = org.organization_id;
    Ok(ok(dragonflydb::update_resource(
        &state,
        &organization_id,
        &project_id,
        &environment_id,
        &application_id,
        &public_host(&state),
        input,
    )
    .await?))
}

pub async fn rotate_dragonflydb_password_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::AppEnvUpdate>,
    // The extractor covers `organization_id`.
    Path((_, project_id, environment_id, application_id)): Path<TemplateResourcePath>,
    Json(input): Json<DragonflyDbPasswordRotationRequest>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let organization_id = org.organization_id;
    Ok(ok(dragonflydb::rotate_password(
        &state,
        &organization_id,
        &project_id,
        &environment_id,
        &application_id,
        &public_host(&state),
        input,
    )
    .await?))
}

pub async fn preview_keydb_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::AppCreate>,
    // The extractor covers `organization_id`.
    Path((_, project_id, environment_id)): Path<TemplatePath>,
    Json(mut input): Json<KeyDbTemplateInput>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let organization_id = org.organization_id;
    let (project_name, environment_name) =
        context(&state, &organization_id, &project_id, &environment_id).await?;
    let instance_id = input
        .template_instance_id
        .clone()
        .unwrap_or_else(|| Uuid::now_v7().to_string());
    let runtime_name = deployment::deployment_identity(
        &project_name,
        &environment_name,
        &input.application_name,
        &instance_id,
        &state.public_ip,
    )
    .0;
    input.template_instance_id = Some(instance_id);
    let used_ports = ComposeFileStore::list_host_ports(&state)
        .await
        .map_err(AppError::InternalError)?;
    let reserved_ports = state
        .reserved_ports
        .iter()
        .filter_map(|port| u16::try_from(*port).ok())
        .collect();
    Ok(ok(keydb::preview(
        input,
        &format!("{runtime_name}-keydb"),
        &public_host(&state),
        &reserved_ports,
        &used_ports,
    )?))
}

pub async fn create_keydb_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::AppCreate>,
    // The extractor covers `organization_id`.
    Path((_, project_id, environment_id)): Path<TemplatePath>,
    Json(input): Json<KeyDbTemplateInput>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let organization_id = org.organization_id;
    let (project_name, environment_name) =
        context(&state, &organization_id, &project_id, &environment_id).await?;
    let dto = keydb::create(
        &state,
        &organization_id,
        &project_id,
        &environment_id,
        &project_name,
        &environment_name,
        &public_host(&state),
        input,
    )
    .await?;
    let _ = crate::git::repositories::audit_log::record(
        &state,
        &organization_id,
        &org.user,
        "keydb.created",
        Some("application"),
        Some(&dto.application.id),
        Some(&dto.application.name),
        None,
        None,
        None,
        None,
    )
    .await;
    Ok(created(dto))
}

pub async fn get_keydb_resource_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::AppRead>,
    // The extractor covers `organization_id`.
    Path((_, project_id, environment_id, application_id)): Path<TemplateResourcePath>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let organization_id = org.organization_id;
    Ok(ok(keydb::resource_detail(
        &state,
        &organization_id,
        &project_id,
        &environment_id,
        &application_id,
        &public_host(&state),
    )
    .await?))
}

pub async fn update_keydb_resource_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::AppUpdate>,
    // The extractor covers `organization_id`.
    Path((_, project_id, environment_id, application_id)): Path<TemplateResourcePath>,
    Json(input): Json<KeyDbTemplateConfigUpdate>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let organization_id = org.organization_id;
    Ok(ok(keydb::update_resource(
        &state,
        &organization_id,
        &project_id,
        &environment_id,
        &application_id,
        &public_host(&state),
        input,
    )
    .await?))
}

pub async fn rotate_keydb_password_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::AppEnvUpdate>,
    // The extractor covers `organization_id`.
    Path((_, project_id, environment_id, application_id)): Path<TemplateResourcePath>,
    Json(input): Json<KeyDbPasswordRotationRequest>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let organization_id = org.organization_id;
    Ok(ok(keydb::rotate_password(
        &state,
        &organization_id,
        &project_id,
        &environment_id,
        &application_id,
        &public_host(&state),
        input,
    )
    .await?))
}

pub async fn preview_sqlserver_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::AppCreate>,
    // The extractor covers `organization_id`.
    Path((_, project_id, environment_id)): Path<TemplatePath>,
    Json(mut input): Json<SqlServerTemplateInput>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let organization_id = org.organization_id;
    let (project_name, environment_name) =
        context(&state, &organization_id, &project_id, &environment_id).await?;
    let instance_id = input
        .template_instance_id
        .clone()
        .unwrap_or_else(|| Uuid::now_v7().to_string());
    let runtime_name = deployment::deployment_identity(
        &project_name,
        &environment_name,
        &input.application_name,
        &instance_id,
        &state.public_ip,
    )
    .0;
    input.template_instance_id = Some(instance_id);
    let used_ports = ComposeFileStore::list_host_ports(&state)
        .await
        .map_err(AppError::InternalError)?;
    let reserved_ports = state
        .reserved_ports
        .iter()
        .filter_map(|port| u16::try_from(*port).ok())
        .collect();
    Ok(ok(sqlserver::preview(
        input,
        &format!("{runtime_name}-sqlserver"),
        &public_host(&state),
        &reserved_ports,
        &used_ports,
    )?))
}

pub async fn create_sqlserver_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::AppCreate>,
    // The extractor covers `organization_id`.
    Path((_, project_id, environment_id)): Path<TemplatePath>,
    Json(input): Json<SqlServerTemplateInput>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let organization_id = org.organization_id;
    let (project_name, environment_name) =
        context(&state, &organization_id, &project_id, &environment_id).await?;
    let dto = sqlserver::create(
        &state,
        &organization_id,
        &project_id,
        &environment_id,
        &project_name,
        &environment_name,
        &public_host(&state),
        input,
    )
    .await?;
    let _ = crate::git::repositories::audit_log::record(
        &state,
        &organization_id,
        &org.user,
        "sqlserver.created",
        Some("application"),
        Some(&dto.application.id),
        Some(&dto.application.name),
        None,
        None,
        None,
        None,
    )
    .await;
    Ok(created(dto))
}

pub async fn get_sqlserver_resource_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::AppRead>,
    // The extractor covers `organization_id`.
    Path((_, project_id, environment_id, application_id)): Path<TemplateResourcePath>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let organization_id = org.organization_id;
    Ok(ok(sqlserver::resource_detail(
        &state,
        &organization_id,
        &project_id,
        &environment_id,
        &application_id,
        &public_host(&state),
    )
    .await?))
}

pub async fn update_sqlserver_resource_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::AppUpdate>,
    // The extractor covers `organization_id`.
    Path((_, project_id, environment_id, application_id)): Path<TemplateResourcePath>,
    Json(input): Json<SqlServerTemplateConfigUpdate>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let organization_id = org.organization_id;
    Ok(ok(sqlserver::update_resource(
        &state,
        &organization_id,
        &project_id,
        &environment_id,
        &application_id,
        &public_host(&state),
        input,
    )
    .await?))
}

pub async fn rotate_sqlserver_password_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::AppEnvUpdate>,
    // The extractor covers `organization_id`.
    Path((_, project_id, environment_id, application_id)): Path<TemplateResourcePath>,
    Json(input): Json<SqlServerPasswordRotationRequest>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let organization_id = org.organization_id;
    Ok(ok(sqlserver::rotate_password(
        &state,
        &organization_id,
        &project_id,
        &environment_id,
        &application_id,
        &public_host(&state),
        input,
    )
    .await?))
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{HashMap, HashSet},
        sync::{Arc, Mutex},
    };

    use axum::extract::{Path, State};
    use uuid::Uuid;

    use super::*;
    use crate::{
        cqrs::commands::project as project_cmd,
        dtos::project::{CreateApplicationRequest, CreateEnvironmentRequest, CreateProjectRequest},
        migrations,
        state::state::AppState,
    };

    // This module keeps its own database fixture; `granted` is borrowed from the
    // shared one because it only builds a value, no state.
    use crate::testing::granted;

    async fn test_state() -> (SharedState, std::path::PathBuf) {
        let path =
            std::env::temp_dir().join(format!("litepod-template-handlers-{}.db", Uuid::now_v7()));
        let db = crate::testing::sqlite_pool_at(&path).await;
        let data_dir = std::path::PathBuf::from(format!("{}-data", path.display()));
        std::fs::create_dir_all(&data_dir).unwrap();
        migrations::run(&db).await;
        let state = Arc::new(AppState {
            started_at: chrono::Local::now(),
            db,
            caddy_url: String::new(),
            caddy_admin_url: String::new(),
            caddyfile_path: Default::default(),
            caddy_server_upstream: String::new(),
            podman_socket: String::new(),
            pending_execs: Mutex::new(HashMap::new()),
            data_dir: data_dir.clone(),
            data_volume: None,
            host_data_dir: Default::default(),
            podman_network: String::new(),
            public_url: String::new(),
            oauth_states: Mutex::new(HashMap::new()),
            public_ip: String::new(),
            is_cloud: false,
            polar: None,
            dev_mode: false,
            port_range: (0, 0),
            reserved_ports: HashSet::new(),
            volume_access: Default::default(),
            podman_host: Default::default(),
            queue_runtime: Default::default(),
            version: String::new(),
            git_sha: String::new(),
            image_repo: String::new(),
            image_tag: String::new(),
            update_channel: Default::default(),
            version_manifest_url: String::new(),
            update_interval_secs: 0,
            container_name: String::new(),
            caddy_container: String::new(),
            stack_env_file: None,
            update_state: Default::default(),
            backup_lock: Default::default(),
            rate_limiter: tokio::sync::RwLock::new(
                crate::security::rate_limit::RateLimiterState::from_config(Default::default()),
            ),
            ephemeral: Default::default(),
        });
        let conn = state.db.clone();
        for user in ["editor", "viewer"] {
            sqlx::query(
                "INSERT INTO users (id, email, hashed_password, name) VALUES (?1, ?2, 'hash', ?1)",
            )
            .bind(user)
            .bind(format!("{user}@example.com"))
            .execute(&conn)
            .await
            .unwrap();
        }
        for organization in ["organization-a", "organization-b"] {
            sqlx::query(
                "INSERT INTO organizations (id, name, slug, created_by) VALUES (?1, ?1, ?1, 'editor')",
            )
            .bind(organization)
            .execute(&conn)
            .await
            .unwrap();
        }
        for (organization, user, role) in [
            ("organization-a", "editor", 2_i64),
            ("organization-b", "editor", 2_i64),
            ("organization-a", "viewer", 3_i64),
        ] {
            sqlx::query(
                "INSERT INTO organization_members (organization_id, user_id, role_id) VALUES (?1, ?2, ?3)",
            )
            .bind(organization)
            .bind(user)
            .bind(role)
            .execute(&conn)
            .await
            .unwrap();
        }
        (state, path)
    }

    fn cleanup(path: &std::path::Path) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{}", path.display(), suffix));
        }
        let _ = std::fs::remove_dir_all(format!("{}-data", path.display()));
    }

    /// The role half of this test (a Viewer refused a write, a non-member refused
    /// everything) moved to `middleware::org_scope`, where the extractor that now
    /// enforces it actually runs. What remains is the handler's own job: an
    /// application id that does not resolve inside the caller's organization must
    /// be a 404, not a leak.
    #[tokio::test]
    async fn mariadb_resource_scopes_the_application_to_the_organization() {
        let (state, path) = test_state().await;
        let missing = get_mariadb_resource_handler(
            State(state.clone()),
            RequireOrg(
                granted("organization-a", "viewer", 3),
                std::marker::PhantomData,
            ),
            Path((
                "organization-a".into(),
                "project".into(),
                "environment".into(),
                "missing".into(),
            )),
        )
        .await;
        assert!(matches!(missing, Err(AppError::ApplicationNotFound)));
        cleanup(&path);
    }

    #[tokio::test]
    async fn mariadb_resource_rejects_a_dragonflydb_application() {
        let (state, path) = test_state().await;
        let project = project_cmd::create_project(
            &state,
            "organization-a",
            &CreateProjectRequest {
                name: "Project".into(),
                description: None,
            },
        )
        .await
        .unwrap();
        let environment = project_cmd::create_environment(
            &state,
            "organization-a",
            &project.id,
            &CreateEnvironmentRequest {
                name: "Staging".into(),
                description: None,
            },
        )
        .await
        .unwrap();
        let application = project_cmd::create_application_with_id(
            &state,
            "organization-a",
            &project.id,
            &environment.id,
            &Uuid::now_v7().to_string(),
            Some("dragonflydb"),
            &CreateApplicationRequest {
                name: "Cache".into(),
                description: None,
                server_id: Some("local".into()),
                server_group_id: None,
            },
        )
        .await
        .unwrap();
        let result = get_mariadb_resource_handler(
            State(state.clone()),
            RequireOrg(
                granted("organization-a", "viewer", 3),
                std::marker::PhantomData,
            ),
            Path((
                "organization-a".into(),
                project.id,
                environment.id,
                application.id,
            )),
        )
        .await;
        assert!(matches!(result, Err(AppError::ResourceNotFound)));
        cleanup(&path);
    }

    /// The role half of this test (a Viewer refused a write, a non-member refused
    /// everything) moved to `middleware::org_scope`, where the extractor that now
    /// enforces it actually runs. What remains is the handler's own job: an
    /// application id that does not resolve inside the caller's organization must
    /// be a 404, not a leak.
    #[tokio::test]
    async fn sqlserver_resource_scopes_the_application_to_the_organization() {
        let (state, path) = test_state().await;
        let missing = get_sqlserver_resource_handler(
            State(state.clone()),
            RequireOrg(
                granted("organization-a", "viewer", 3),
                std::marker::PhantomData,
            ),
            Path((
                "organization-a".into(),
                "project".into(),
                "environment".into(),
                "missing".into(),
            )),
        )
        .await;
        assert!(matches!(missing, Err(AppError::ApplicationNotFound)));
        cleanup(&path);
    }

    #[tokio::test]
    async fn sqlserver_resource_rejects_a_postgres_application() {
        let (state, path) = test_state().await;
        let project = project_cmd::create_project(
            &state,
            "organization-a",
            &CreateProjectRequest {
                name: "Project".into(),
                description: None,
            },
        )
        .await
        .unwrap();
        let environment = project_cmd::create_environment(
            &state,
            "organization-a",
            &project.id,
            &CreateEnvironmentRequest {
                name: "Staging".into(),
                description: None,
            },
        )
        .await
        .unwrap();
        let application = project_cmd::create_application_with_id(
            &state,
            "organization-a",
            &project.id,
            &environment.id,
            &Uuid::now_v7().to_string(),
            Some("postgres"),
            &CreateApplicationRequest {
                name: "Database".into(),
                description: None,
                server_id: Some("local".into()),
                server_group_id: None,
            },
        )
        .await
        .unwrap();
        let result = get_sqlserver_resource_handler(
            State(state.clone()),
            RequireOrg(
                granted("organization-a", "viewer", 3),
                std::marker::PhantomData,
            ),
            Path((
                "organization-a".into(),
                project.id,
                environment.id,
                application.id,
            )),
        )
        .await;
        assert!(matches!(result, Err(AppError::ResourceNotFound)));
        cleanup(&path);
    }
}
