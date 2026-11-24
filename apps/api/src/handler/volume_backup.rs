use axum::{
    Json,
    extract::{Multipart, Path, Query, State},
    response::IntoResponse,
};

use crate::dtos::response::{AppError, ok};
use crate::dtos::volume_backup::{
    NewVolumeFromTarRequest, RestoreFromS3Request, RestoreVolumeBackupRequest,
    RunVolumeBackupRequest, S3TarsQuery, UpdateVolumeBackupPolicyRequest,
};
use crate::handler::common::actor_label;
use crate::security::authz::extract::{RequireApp, perm};
use crate::state::state::SharedState;
use crate::volume_backup::run;

/// `.../applications/{application_id}/volume-backups/{volume_name}`; the name
/// is checked against the app's own volumes in `run`.
type VolumePath = (String, String, String, String, String);
/// `.../volume-backups/{volume_name}/{backup_id}`
type BackupPath = (String, String, String, String, String, String);

pub async fn get_policy_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::BackupRead>,
    Path((_, _, _, _, volume_name)): Path<VolumePath>,
) -> Result<impl IntoResponse, AppError> {
    Ok(ok(run::get_policy(
        &state,
        &app.application_id,
        &volume_name,
    )
    .await?))
}

pub async fn update_policy_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::BackupManage>,
    Path((_, _, _, _, volume_name)): Path<VolumePath>,
    Json(req): Json<UpdateVolumeBackupPolicyRequest>,
) -> Result<impl IntoResponse, AppError> {
    let actor = actor_label(&app.user);
    Ok(ok(run::set_policy(
        &state,
        &app.application_id,
        &volume_name,
        &req,
        Some(&actor),
    )
    .await?))
}

/// Restores into a new detached volume (returned) or over the original.
pub async fn restore_backup_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::BackupRestore>,
    Path((_, _, _, _, volume_name, backup_id)): Path<BackupPath>,
    Json(req): Json<RestoreVolumeBackupRequest>,
) -> Result<impl IntoResponse, AppError> {
    let actor = actor_label(&app.user);
    Ok(ok(run::restore_backup(
        &state,
        &app.application_id,
        &volume_name,
        &backup_id,
        &req,
        &actor,
    )
    .await?))
}

pub async fn delete_backup_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::BackupManage>,
    Path((_, _, _, _, volume_name, backup_id)): Path<BackupPath>,
) -> Result<impl IntoResponse, AppError> {
    run::remove_backup(&state, &app.application_id, &volume_name, &backup_id).await?;
    Ok(ok(true))
}

/// `.tar` files in one of the organization's S3 destinations.
pub async fn list_s3_tars_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::BackupRestore>,
    Query(query): Query<S3TarsQuery>,
) -> Result<impl IntoResponse, AppError> {
    let target = query.s3_target_id.as_deref().filter(|id| !id.is_empty());
    Ok(ok(
        run::list_s3_tars(&state, &app.application_id, target).await?
    ))
}

/// Creates a detached volume from a `.tar` in S3.
pub async fn restore_from_s3_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::BackupRestore>,
    Json(mut req): Json<RestoreFromS3Request>,
) -> Result<impl IntoResponse, AppError> {
    req.s3_target_id = req.s3_target_id.filter(|id| !id.trim().is_empty());
    Ok(ok(
        run::restore_from_s3(&state, &app.application_id, &req).await?
    ))
}

/// Creates a detached volume from an uploaded `.tar`; name and placement ride
/// in the query so the file can be the only multipart field.
pub async fn restore_from_upload_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::BackupRestore>,
    Query(req): Query<NewVolumeFromTarRequest>,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, AppError> {
    let field = multipart
        .next_field()
        .await
        .map_err(|error| AppError::ValidationError(error.to_string()))?
        .ok_or_else(|| AppError::ValidationError("Choose a .tar file to upload".into()))?;
    let bytes = field
        .bytes()
        .await
        .map_err(|error| AppError::ValidationError(format!("Upload failed: {error}")))?;
    Ok(ok(run::restore_from_upload(
        &state,
        &app.application_id,
        &req,
        bytes.to_vec(),
    )
    .await?))
}

/// `.../applications/{application_id}/volume-backup-groups/{group_id}`
type GroupPath = (String, String, String, String, String);

/// Every backup of the application; rows of one run share `group_id`.
pub async fn list_app_runs_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::BackupRead>,
) -> Result<impl IntoResponse, AppError> {
    Ok(ok(run::list_app_runs(&state, &app.application_id).await?))
}

/// Starts a run of the picked volumes (all when none) in the background;
/// returns its group id.
pub async fn run_app_backup_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::BackupManage>,
    Json(req): Json<RunVolumeBackupRequest>,
) -> Result<impl IntoResponse, AppError> {
    let actor = actor_label(&app.user);
    let group_id = run::request_app_run(
        &state,
        &app.application_id,
        req.s3_target_id.as_deref(),
        req.stop_app,
        &req.volumes,
        &actor,
    )
    .await?;
    Ok(ok(serde_json::json!({ "group_id": group_id })))
}

/// Restores every volume of a whole-application run over the originals.
pub async fn restore_app_run_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::BackupRestore>,
    Path((_, _, _, _, group_id)): Path<GroupPath>,
) -> Result<impl IntoResponse, AppError> {
    let actor = actor_label(&app.user);
    run::restore_app_run(&state, &app.application_id, &group_id, &actor).await?;
    Ok(ok(true))
}
