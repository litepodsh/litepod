use axum::{Extension, Json, extract::State, http::StatusCode, response::IntoResponse};
use serde::{Deserialize, Serialize};

use crate::dtos::response::ApiResponse;
use crate::security::middleware::auth::AuthUser;
use crate::state::SharedState;
use crate::updater;

#[derive(Serialize, utoipa::ToSchema)]
pub struct SystemVersionResponse {
    pub version: String,
    pub git_sha: String,
    pub image: String,
    pub image_repo: String,
    pub image_tag: String,
    pub update_interval_secs: u64,
    pub container_name: String,
    pub is_cloud: bool,
    pub state: updater::UpdateState,
    pub uptime_secs: ServiceUptimes,
}

/// Seconds each core container has been running, read live from Podman on every
/// request (the updater snapshot is hourly). `None` when not running, unknown,
/// or the caller is not a sysadmin.
#[derive(Serialize, Default, utoipa::ToSchema)]
pub struct ServiceUptimes {
    pub api: Option<i64>,
    pub caddy: Option<i64>,
    pub dragonfly: Option<i64>,
}

async fn service_uptimes(state: &SharedState) -> ServiceUptimes {
    let dragonfly =
        std::env::var("DRAGONFLY_CONTAINER").unwrap_or_else(|_| "litepod-dragonfly".into());
    let (caddy, dragonfly) = tokio::join!(
        updater::apply::container_started_at(&state.caddy_container, &state.podman_socket),
        updater::apply::container_started_at(&dragonfly, &state.podman_socket),
    );
    let now = chrono::Utc::now();
    let secs = |started: chrono::DateTime<chrono::Utc>| (now - started).num_seconds().max(0);
    ServiceUptimes {
        api: Some(secs(state.started_at.to_utc())),
        caddy: caddy.map(secs),
        dragonfly: dragonfly.map(secs),
    }
}

/// GET /api/v1/system/version
/// Build-time identity + live update state of this running instance. Its latest
/// version is always the one published for the active update channel.
#[utoipa::path(
    get,
    path = "/api/v1/system/version",
    responses(
        (status = 200, description = "litepod version + updater state; latest version belongs to the active channel", body = SystemVersionResponse),
    ),
    tag = "system"
)]
pub async fn get_version_handler(
    State(state): State<SharedState>,
    Extension(user): Extension<AuthUser>,
) -> impl IntoResponse {
    let snapshot = state.update_state.read().await.clone();
    // Build identity (git_sha, image_repo, image_tag, container_name) narrows
    // down the exact running artifact for an attacker probing known CVEs; only
    // sysadmins get it. Everyone authenticated still needs `version`/`is_cloud`
    // for the UI's update banner and plan gating.
    let body = if user.is_sysadmin {
        SystemVersionResponse {
            version: state.version.clone(),
            git_sha: state.git_sha.clone(),
            image: format!("{}:{}", state.image_repo, state.image_tag),
            image_repo: state.image_repo.clone(),
            image_tag: state.image_tag.clone(),
            update_interval_secs: state.update_interval_secs,
            container_name: state.container_name.clone(),
            is_cloud: state.is_cloud,
            state: snapshot,
            uptime_secs: service_uptimes(&state).await,
        }
    } else {
        SystemVersionResponse {
            version: state.version.clone(),
            git_sha: String::new(),
            image: String::new(),
            image_repo: String::new(),
            image_tag: String::new(),
            update_interval_secs: state.update_interval_secs,
            container_name: String::new(),
            is_cloud: state.is_cloud,
            state: snapshot,
            uptime_secs: ServiceUptimes::default(),
        }
    };
    (
        StatusCode::OK,
        Json(ApiResponse {
            error: None,
            data: Some(body),
        }),
    )
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct UpdateCheckResponse {
    pub triggered: bool,
    pub note: String,
}

/// POST /api/v1/system/update/check
/// Force an immediate update check (does not wait for the periodic timer).
/// Returns once the check is scheduled; the actual pull+recreate happens
/// in the background.
#[utoipa::path(
    post,
    path = "/api/v1/system/update/check",
    responses(
        (status = 202, description = "Check scheduled", body = UpdateCheckResponse),
        (status = 503, description = "Updater disabled"),
    ),
    tag = "system"
)]
pub async fn post_check_update_handler(
    State(state): State<SharedState>,
    Extension(user): Extension<AuthUser>,
) -> impl IntoResponse {
    if !user.is_sysadmin {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse::<UpdateCheckResponse> {
                error: Some("sysadmin required".into()),
                data: None,
            }),
        )
            .into_response();
    }
    if state.update_interval_secs == 0 {
        let note = "updater disabled (LITEPOD_UPDATE_INTERVAL=0)".to_string();
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ApiResponse {
                error: Some(note.clone()),
                data: Some(UpdateCheckResponse {
                    triggered: false,
                    note,
                }),
            }),
        )
            .into_response();
    }
    let state_for_task = state.clone();
    tokio::spawn(async move {
        updater::detect_once(&state_for_task).await;
    });
    (
        StatusCode::ACCEPTED,
        Json(ApiResponse {
            error: None,
            data: Some(UpdateCheckResponse {
                triggered: true,
                note: "check scheduled".into(),
            }),
        }),
    )
        .into_response()
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct UpdateApplyResponse {
    pub triggered: bool,
    pub note: String,
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct UpdateApplyRequest {
    pub target_version: String,
    pub channel: updater::UpdateChannel,
    pub core_services: updater::check::CoreServiceTargets,
}

#[derive(Debug, PartialEq, Eq)]
enum ApplyValidationError {
    NotAvailable,
    SnapshotChanged,
}

fn validate_apply_request(
    snapshot: &updater::UpdateState,
    request: &UpdateApplyRequest,
) -> Result<(), ApplyValidationError> {
    if !snapshot.update_available
        || !snapshot.last_check_succeeded
        || !snapshot.core_services.supported
    {
        return Err(ApplyValidationError::NotAvailable);
    }
    if snapshot.channel != request.channel
        || snapshot.latest_version != request.target_version
        || snapshot.core_services.caddy.target_tag != request.core_services.caddy
        || snapshot.core_services.dragonfly.target_tag != request.core_services.dragonfly
    {
        return Err(ApplyValidationError::SnapshotChanged);
    }
    Ok(())
}

/// POST /api/v1/system/update/apply
/// Manually install a previously-detected update: pulls the tracked image
/// and recreates this container. Requires `update_available` to already be
/// true (run a check first). The actual pull+recreate happens in the
/// background.
#[utoipa::path(
    post,
    path = "/api/v1/system/update/apply",
    request_body = UpdateApplyRequest,
    responses(
        (status = 202, description = "Install scheduled", body = UpdateApplyResponse),
        (status = 409, description = "No update pending", body = UpdateApplyResponse),
        (status = 503, description = "Updater disabled", body = UpdateApplyResponse),
    ),
    tag = "system"
)]
pub async fn post_apply_update_handler(
    State(state): State<SharedState>,
    Extension(user): Extension<AuthUser>,
    Json(request): Json<UpdateApplyRequest>,
) -> impl IntoResponse {
    if !user.is_sysadmin {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse::<UpdateApplyResponse> {
                error: Some("sysadmin required".into()),
                data: None,
            }),
        )
            .into_response();
    }
    if state.update_interval_secs == 0 {
        let note = "updater disabled (LITEPOD_UPDATE_INTERVAL=0)".to_string();
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ApiResponse {
                error: Some(note.clone()),
                data: Some(UpdateApplyResponse {
                    triggered: false,
                    note,
                }),
            }),
        )
            .into_response();
    }

    // Take availability, channel and target from one lock snapshot, then carry
    // the exact UI-confirmed values into the background task. A check that
    // finishes after this point cannot redirect the apply to another release.
    let snapshot = state.update_state.read().await.clone();
    if let Err(error) = validate_apply_request(&snapshot, &request) {
        let note = match error {
            ApplyValidationError::NotAvailable => "no update pending — run a check first",
            ApplyValidationError::SnapshotChanged => {
                "update status changed — refresh and confirm the newly shown target"
            }
        }
        .to_string();
        return (
            StatusCode::CONFLICT,
            Json(ApiResponse {
                error: Some(note.clone()),
                data: Some(UpdateApplyResponse {
                    triggered: false,
                    note,
                }),
            }),
        )
            .into_response();
    }

    let state_for_task = state.clone();
    let target = request.target_version;
    let channel = request.channel;
    let core_targets = request.core_services;
    let _ = crate::security::security_events::record(
        &state,
        None,
        &user,
        "system.update_applied",
        Some(&format!("{target} ({})", channel.as_str())),
    )
    .await;
    tokio::spawn(async move {
        if let Err(e) =
            updater::apply_release(&state_for_task, target.clone(), channel, core_targets).await
        {
            tracing::error!(error = %e, "updater: manual install failed");
            let mut s = state_for_task.update_state.write().await;
            if s.channel == channel && s.latest_version == target {
                s.last_check_note = format!("install failed: {e}");
            }
        }
    });
    (
        StatusCode::ACCEPTED,
        Json(ApiResponse {
            error: None,
            data: Some(UpdateApplyResponse {
                triggered: true,
                note: "install started".into(),
            }),
        }),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn available_snapshot() -> updater::UpdateState {
        updater::UpdateState {
            channel: updater::UpdateChannel::Alpha,
            current_version: "v1.0.0-alpha.1".into(),
            latest_version: "v1.0.0-alpha.2".into(),
            update_available: true,
            last_check_at: None,
            last_check_succeeded: true,
            last_check_note: String::new(),
            updater_running: true,
            core_services: updater::CoreUpdateState {
                supported: true,
                caddy: updater::CoreServiceUpdateState {
                    target_tag: "2.11.4".into(),
                    ..Default::default()
                },
                dragonfly: updater::CoreServiceUpdateState {
                    target_tag: "v1.39.0".into(),
                    ..Default::default()
                },
                ..Default::default()
            },
        }
    }

    #[test]
    fn apply_validation_accepts_the_exact_shown_snapshot() {
        let snapshot = available_snapshot();
        let request = UpdateApplyRequest {
            target_version: snapshot.latest_version.clone(),
            channel: snapshot.channel,
            core_services: updater::check::CoreServiceTargets {
                caddy: "2.11.4".into(),
                dragonfly: "v1.39.0".into(),
            },
        };
        assert_eq!(validate_apply_request(&snapshot, &request), Ok(()));
    }

    #[test]
    fn apply_validation_rejects_changed_target_or_channel() {
        let snapshot = available_snapshot();
        let changed_target = UpdateApplyRequest {
            target_version: "v1.0.0-alpha.3".into(),
            channel: snapshot.channel,
            core_services: updater::check::CoreServiceTargets {
                caddy: "2.11.4".into(),
                dragonfly: "v1.39.0".into(),
            },
        };
        assert_eq!(
            validate_apply_request(&snapshot, &changed_target),
            Err(ApplyValidationError::SnapshotChanged)
        );

        let changed_channel = UpdateApplyRequest {
            target_version: snapshot.latest_version.clone(),
            channel: updater::UpdateChannel::Stable,
            core_services: updater::check::CoreServiceTargets {
                caddy: "2.11.4".into(),
                dragonfly: "v1.39.0".into(),
            },
        };
        assert_eq!(
            validate_apply_request(&snapshot, &changed_channel),
            Err(ApplyValidationError::SnapshotChanged)
        );
    }

    #[test]
    fn apply_validation_rejects_a_no_longer_available_update() {
        let mut snapshot = available_snapshot();
        snapshot.update_available = false;
        let request = UpdateApplyRequest {
            target_version: snapshot.latest_version.clone(),
            channel: snapshot.channel,
            core_services: updater::check::CoreServiceTargets {
                caddy: "2.11.4".into(),
                dragonfly: "v1.39.0".into(),
            },
        };
        assert_eq!(
            validate_apply_request(&snapshot, &request),
            Err(ApplyValidationError::NotAvailable)
        );
    }
}
