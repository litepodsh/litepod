use axum::{
    Json,
    extract::{Path, State},
    response::IntoResponse,
};

use crate::database_backup::run;
use crate::dtos::database_backup::{RunDatabaseBackupRequest, UpdateDatabaseBackupPolicyRequest};
use crate::dtos::response::{AppError, ok};
use crate::handler::common::actor_label;
use crate::security::authz::extract::{RequireApp, perm};
use crate::state::state::SharedState;

/// The backup policy for one managed database application.
pub async fn get_backup_policy_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::BackupRead>,
) -> Result<impl IntoResponse, AppError> {
    Ok(ok(run::get_policy(&state, &app.application_id).await?))
}

pub async fn update_backup_policy_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::BackupManage>,
    Json(req): Json<UpdateDatabaseBackupPolicyRequest>,
) -> Result<impl IntoResponse, AppError> {
    let actor = actor_label(&app.user);
    Ok(ok(run::set_policy(
        &state,
        &app.application_id,
        &req,
        Some(&actor),
    )
    .await?))
}

/// Starts a manual backup; the run proceeds in the background and the returned
/// row has `status = "running"`.
pub async fn run_backup_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::BackupManage>,
    Json(_req): Json<RunDatabaseBackupRequest>,
) -> Result<impl IntoResponse, AppError> {
    Ok(ok(
        run::request_run(&state, &app.application_id, "manual").await?
    ))
}

/// History of backup runs for the application, newest first.
pub async fn list_backups_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::BackupRead>,
) -> Result<impl IntoResponse, AppError> {
    Ok(ok(run::list_backups(&state, &app.application_id).await?))
}

/// Restores a succeeded backup into the live container.
pub async fn restore_backup_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::BackupRestore>,
    Path(backup_id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let actor = actor_label(&app.user);
    run::restore_backup(&state, &app.application_id, &backup_id, &actor).await?;
    Ok(ok(true))
}

/// Deletes one backup record and its S3 object.
pub async fn delete_backup_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::BackupManage>,
    Path(backup_id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    run::remove_backup(&state, &app.application_id, &backup_id).await?;
    Ok(ok(true))
}
