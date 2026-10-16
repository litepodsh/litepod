use axum::{Extension, Json, extract::State};
use podman_api::Podman;
use serde::{Deserialize, Serialize};

use crate::{
    dtos::response::{AppError, ok},
    security::middleware::auth::AuthUser,
    state::state::SharedState,
};

#[derive(Debug, Deserialize)]
pub struct PruneRequest {
    pub containers: Option<bool>,
    pub images: Option<bool>,
    pub volumes: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PruneResult {
    pub containers_freed: i64,
    pub containers_space: i64,
    pub images_freed: i64,
    pub images_space: i64,
    pub volumes_freed: i64,
    pub volumes_space: i64,
}

pub async fn delete_deployment_history_handler(
    State(state): State<SharedState>,
    Extension(user): Extension<AuthUser>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    if !user.is_sysadmin {
        return Err(AppError::InsufficientPermissions);
    }
    let deleted = crate::deployment::history::delete_all_completed(&state).await?;
    let _ = crate::security::security_events::record(
        &state,
        None,
        &user,
        "maintenance.history_deleted",
        Some(&format!("{deleted:?}")),
    )
    .await;
    Ok(ok(deleted))
}

pub async fn prune_handler(
    State(state): State<SharedState>,
    Extension(user): Extension<AuthUser>,
    Json(request): Json<PruneRequest>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    if !user.is_sysadmin {
        return Err(AppError::InsufficientPermissions);
    }

    let podman = Podman::new(&state.podman_socket)
        .map_err(|e| AppError::InternalError(format!("Podman unavailable: {e}")))?;

    let mut result = PruneResult {
        containers_freed: 0,
        containers_space: 0,
        images_freed: 0,
        images_space: 0,
        volumes_freed: 0,
        volumes_space: 0,
    };

    if request.containers.unwrap_or(false) {
        let report = podman
            .containers()
            .prune(&Default::default())
            .await
            .map_err(|e| AppError::InternalError(format!("Container prune failed: {e}")))?;
        result.containers_freed = report.len() as i64;
        result.containers_space = report.iter().map(|r| r.size.unwrap_or(0) as i64).sum();
    }

    if request.images.unwrap_or(false) {
        if let Some(report) = podman
            .images()
            .prune(&Default::default())
            .await
            .map_err(|e| AppError::InternalError(format!("Image prune failed: {e}")))?
        {
            result.images_freed = report.len() as i64;
            result.images_space = report.iter().map(|r| r.size.unwrap_or(0) as i64).sum();
        }
    }

    if request.volumes.unwrap_or(false) {
        let report = podman
            .volumes()
            .prune(&Default::default())
            .await
            .map_err(|e| AppError::InternalError(format!("Volume prune failed: {e}")))?;
        result.volumes_freed = report.len() as i64;
        result.volumes_space = report.iter().map(|r| r.size.unwrap_or(0) as i64).sum();
    }

    crate::notifications::notify_cleanup(
        &state,
        crate::notifications::event::CleanupSummary {
            containers_freed: result.containers_freed,
            containers_space: result.containers_space,
            images_freed: result.images_freed,
            images_space: result.images_space,
            volumes_freed: result.volumes_freed,
            volumes_space: result.volumes_space,
        },
        Some(crate::handler::common::actor_label(&user).as_str()),
    )
    .await;

    let _ = crate::security::security_events::record(
        &state,
        None,
        &user,
        "maintenance.pruned",
        Some(&format!(
            "containers:{}, images:{}, volumes:{}",
            request.containers.unwrap_or(false),
            request.images.unwrap_or(false),
            request.volumes.unwrap_or(false)
        )),
    )
    .await;

    Ok(ok(result))
}
