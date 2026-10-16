use axum::{
    Extension, Json,
    extract::{Path, State},
    response::IntoResponse,
};

use crate::backup::{config, download, restore, run, s3 as s3_client};
use crate::dtos::response::{AppError, ok};
use crate::security::authz::extract::{RequireOrg, perm};
use crate::security::middleware::auth::AuthUser;
use crate::security::middleware::org_scope::OrgScope;
use crate::state::state::SharedState;

pub async fn get_s3_config_handler(
    State(state): State<SharedState>,
    Extension(user): Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    if !user.is_sysadmin {
        return Err(AppError::InsufficientPermissions);
    }
    Ok(ok(config::load_dto(&state).await?))
}

pub async fn update_s3_config_handler(
    State(state): State<SharedState>,
    Extension(user): Extension<AuthUser>,
    Json(req): Json<config::UpdateS3ConfigRequest>,
) -> Result<impl IntoResponse, AppError> {
    if !user.is_sysadmin {
        return Err(AppError::InsufficientPermissions);
    }
    if req.endpoint_url.trim().is_empty()
        || req.bucket.trim().is_empty()
        || req.access_key_id.trim().is_empty()
        || req.region.trim().is_empty()
    {
        return Err(AppError::ValidationError(
            "Endpoint, bucket, region, and access key id are required".into(),
        ));
    }

    let secret_access_key = match req.secret_access_key.as_deref().filter(|k| !k.is_empty()) {
        Some(provided) => provided.to_string(),
        None => {
            let existing = config::load(&state).await?;
            existing.map(|c| c.secret_access_key).ok_or_else(|| {
                AppError::ValidationError("A secret access key is required".into())
            })?
        }
    };

    let candidate = config::S3Config {
        endpoint_url: req.endpoint_url.clone(),
        bucket: req.bucket.clone(),
        region: req.region.clone(),
        access_key_id: req.access_key_id.clone(),
        secret_access_key: secret_access_key.clone(),
        schedule_enabled: req.schedule_enabled,
        schedule_interval: req.schedule_interval.clone(),
        schedule_time: req.schedule_time.clone(),
    };
    // Apply/verify externally first. A failed connectivity check must never
    // leave the UI claiming a destination is reachable when it is not (same
    // sequencing as the server-domain settings' Caddy sync).
    s3_client::test_connection(&candidate).await?;

    let _ = crate::security::security_events::record(
        &state,
        None,
        &user,
        "backup.s3_config_updated",
        None,
    )
    .await;
    Ok(ok(config::save(&state, &req, &secret_access_key).await?))
}

pub async fn list_backup_history_handler(
    State(state): State<SharedState>,
    Extension(user): Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    if !user.is_sysadmin {
        return Err(AppError::InsufficientPermissions);
    }
    Ok(ok(run::list(&state).await?))
}

pub async fn trigger_backup_handler(
    State(state): State<SharedState>,
    Extension(user): Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    if !user.is_sysadmin {
        return Err(AppError::InsufficientPermissions);
    }
    let result = run::start(&state, "manual", Some(user.user_id.clone())).await?;
    let _ = crate::security::security_events::record(&state, None, &user, "backup.triggered", None)
        .await;
    Ok(ok(result))
}

pub async fn trigger_restore_handler(
    State(state): State<SharedState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    if !user.is_sysadmin {
        return Err(AppError::InsufficientPermissions);
    }
    let result = restore::start(&state, &id, Some(user.user_id.clone())).await?;
    let _ =
        crate::security::security_events::record(&state, None, &user, "backup.restored", Some(&id))
            .await;
    Ok(ok(result))
}

pub async fn list_restore_history_handler(
    State(state): State<SharedState>,
    Extension(user): Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    if !user.is_sysadmin {
        return Err(AppError::InsufficientPermissions);
    }
    Ok(ok(restore::list(&state).await?))
}

pub async fn download_backup_handler(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
    Extension(user): Extension<AuthUser>,
) -> Result<axum::response::Response, AppError> {
    if !user.is_sysadmin {
        return Err(AppError::InsufficientPermissions);
    }

    let format_str = params.get("format").map(|s| s.as_str()).unwrap_or("tar");
    let format = download::DownloadFormat::from_str(format_str).ok_or_else(|| {
        AppError::ValidationError(format!(
            "Unsupported format: {format_str}. Use tar, zip, or 7z"
        ))
    })?;

    let (bytes, filename, mime) = download::download_backup(&state, &id, format).await?;

    let mut response = axum::response::Response::new(axum::body::Body::from(bytes));
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_str(mime)
            .map_err(|error| AppError::InternalError(error.to_string()))?,
    );
    response.headers_mut().insert(
        axum::http::header::CONTENT_DISPOSITION,
        axum::http::HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
            .map_err(|error| AppError::InternalError(error.to_string()))?,
    );
    Ok(response)
}

// ---------------------------------------------------------------------------
// Named S3 targets for database backups, one set per organization
// ---------------------------------------------------------------------------

/// Records a target change in the organization's audit log.
async fn audit_target(state: &SharedState, org: &OrgScope, action: &str, id: &str, name: &str) {
    crate::git::repositories::audit_log::record(
        state,
        &org.organization_id,
        &org.user,
        action,
        Some("backup_s3_target"),
        Some(id),
        Some(name),
        None,
        None,
        None,
        None,
    )
    .await
    .ok();
}

/// The organization's targets with their non-secret settings (endpoint, bucket,
/// access key id). Managers only: a viewer picks a target by name through
/// `list_s3_target_meta_handler` and never needs the connection details.
pub async fn list_s3_targets_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::BackupTargetsManage>,
) -> Result<impl IntoResponse, AppError> {
    Ok(ok(crate::backup::target::list(
        &state,
        &org.organization_id,
    )
    .await?))
}

/// The organization's targets as id/name pairs, for the policy selector.
pub async fn list_s3_target_meta_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::BackupRead>,
) -> Result<impl IntoResponse, AppError> {
    let targets = crate::backup::target::list_meta(&state, &org.organization_id).await?;
    Ok(ok(targets
        .into_iter()
        .map(|(id, name)| serde_json::json!({ "id": id, "name": name }))
        .collect::<Vec<_>>()))
}

/// Creates a target after a successful connectivity check. The endpoint is
/// vetted before that check dials it: on cloud the dialing server is the
/// control plane.
pub async fn create_s3_target_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::BackupTargetsManage>,
    Json(req): Json<crate::backup::target::CreateS3TargetRequest>,
) -> Result<impl IntoResponse, AppError> {
    crate::backup::target::check_endpoint(state.is_cloud, req.endpoint_url.trim()).await?;
    let candidate = crate::backup::config::S3Config {
        endpoint_url: req.endpoint_url.clone(),
        bucket: req.bucket.clone(),
        region: req.region.clone(),
        access_key_id: req.access_key_id.clone(),
        secret_access_key: req.secret_access_key.clone(),
        schedule_enabled: false,
        schedule_interval: None,
        schedule_time: None,
    };
    s3_client::test_connection(&candidate).await?;
    let created =
        crate::backup::target::create(&state, &org.organization_id, &req, Some(&org.user.email))
            .await?;
    audit_target(
        &state,
        &org,
        "backup.s3_target_created",
        &created.id,
        &created.name,
    )
    .await;
    Ok(ok(created))
}

/// Updates one of the organization's targets; `secret_access_key: null` keeps
/// the stored secret.
pub async fn update_s3_target_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::BackupTargetsManage>,
    Path((_organization_id, id)): Path<(String, String)>,
    Json(req): Json<crate::backup::target::UpdateS3TargetRequest>,
) -> Result<impl IntoResponse, AppError> {
    let Some(existing) = crate::backup::target::get(&state, &org.organization_id, &id).await?
    else {
        return Err(AppError::ResourceNotFound);
    };
    crate::backup::target::check_endpoint(state.is_cloud, req.endpoint_url.trim()).await?;
    let secret_access_key = match req.secret_access_key.as_deref().filter(|k| !k.is_empty()) {
        Some(provided) => provided.to_string(),
        None => existing.secret_access_key.clone(),
    };
    let candidate = crate::backup::config::S3Config {
        endpoint_url: req.endpoint_url.clone(),
        bucket: req.bucket.clone(),
        region: req.region.clone(),
        access_key_id: req.access_key_id.clone(),
        secret_access_key,
        schedule_enabled: false,
        schedule_interval: None,
        schedule_time: None,
    };
    s3_client::test_connection(&candidate).await?;
    let updated = crate::backup::target::update(&state, &org.organization_id, &id, &req).await?;
    audit_target(&state, &org, "backup.s3_target_updated", &id, &updated.name).await;
    Ok(ok(updated))
}

/// Deletes one of the organization's targets.
pub async fn delete_s3_target_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::BackupTargetsManage>,
    Path((_organization_id, id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let Some(existing) = crate::backup::target::get(&state, &org.organization_id, &id).await?
    else {
        return Err(AppError::ResourceNotFound);
    };
    crate::backup::target::delete(&state, &org.organization_id, &id).await?;
    audit_target(
        &state,
        &org,
        "backup.s3_target_deleted",
        &id,
        &existing.name,
    )
    .await;
    Ok(ok(true))
}
