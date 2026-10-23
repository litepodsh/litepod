use axum::{Extension, Json, extract::State, response::IntoResponse};
use serde::{Deserialize, Serialize};
use sqlx::Row;

use super::service::{env_disabled, instance_id};
use crate::dtos::response::{AppError, ok};
use crate::security::middleware::auth::AuthUser;
use crate::state::state::SharedState;

fn db_err(error: sqlx::Error) -> AppError {
    AppError::DatabaseError(error.to_string())
}

#[derive(Debug, Serialize)]
pub struct TelemetryDto {
    pub enabled: bool,
    /// `LITEPOD_TELEMETRY=off` is set: the stored toggle has no effect.
    pub env_locked: bool,
    pub notice_acknowledged: bool,
    pub instance_id: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateTelemetryRequest {
    pub enabled: bool,
}

async fn load(state: &SharedState) -> Result<TelemetryDto, AppError> {
    let id = instance_id(&state.db).await.map_err(db_err)?;
    let row =
        sqlx::query("SELECT enabled, notice_acknowledged FROM instance_telemetry WHERE id = 1")
            .fetch_one(&state.db)
            .await
            .map_err(db_err)?;
    Ok(TelemetryDto {
        enabled: row.get::<i64, _>(0) != 0,
        env_locked: env_disabled(),
        // Cloud never pings, so there is nothing to announce.
        notice_acknowledged: state.is_cloud || row.get::<i64, _>(1) != 0,
        instance_id: id,
    })
}

pub async fn get_handler(
    State(state): State<SharedState>,
    Extension(user): Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    if !user.is_sysadmin {
        return Err(AppError::InsufficientPermissions);
    }
    Ok(ok(load(&state).await?))
}

/// Choosing a value also counts as having seen the first-login notice.
pub async fn update_handler(
    State(state): State<SharedState>,
    Extension(user): Extension<AuthUser>,
    Json(request): Json<UpdateTelemetryRequest>,
) -> Result<impl IntoResponse, AppError> {
    if !user.is_sysadmin {
        return Err(AppError::InsufficientPermissions);
    }
    sqlx::query(
        "UPDATE instance_telemetry SET enabled = ?1, notice_acknowledged = 1, updated_at = datetime('now') WHERE id = 1",
    )
    .bind(request.enabled as i64)
    .execute(&state.db)
    .await
    .map_err(db_err)?;
    let _ = crate::security::security_events::record(
        &state,
        None,
        &user,
        "settings.telemetry_updated",
        Some(if request.enabled {
            "enabled"
        } else {
            "disabled"
        }),
    )
    .await;
    Ok(ok(load(&state).await?))
}

pub async fn acknowledge_handler(
    State(state): State<SharedState>,
    Extension(user): Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    if !user.is_sysadmin {
        return Err(AppError::InsufficientPermissions);
    }
    sqlx::query(
        "UPDATE instance_telemetry SET notice_acknowledged = 1, updated_at = datetime('now') WHERE id = 1",
    )
    .execute(&state.db)
    .await
    .map_err(db_err)?;
    Ok(ok(load(&state).await?))
}
