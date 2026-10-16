//! Per-application HTTP/TCP health checking, on top of (not instead of) the
//! Podman-level `restart_policy: unless-stopped` that already recovers a
//! crashed process (`deployment::operations::resources::managed_container_builder`).
//! This catches the other failure: a process that is still running but no
//! longer answering.

mod checker;

#[cfg(test)]
mod tests;

use std::time::Duration;

use sqlx::Row;

use crate::{
    dtos::health_check::HealthCheckDto, dtos::response::AppError, state::state::SharedState,
};

const TICK_SECONDS: u64 = 15;
const COLUMNS: &str = "application_id, enabled, protocol, path, port, interval_seconds, timeout_seconds, unhealthy_threshold, restart_on_unhealthy, consecutive_failures, last_status, last_checked_at, last_error, last_restarted_at";

pub(crate) struct HealthCheckRow {
    pub application_id: String,
    pub protocol: String,
    pub path: String,
    pub port: Option<i64>,
    pub timeout_seconds: i64,
    pub unhealthy_threshold: i64,
    pub restart_on_unhealthy: bool,
    pub consecutive_failures: i64,
}

fn read_row(row: &sqlx::sqlite::SqliteRow) -> Result<HealthCheckDto, AppError> {
    let err = |e: sqlx::Error| AppError::DatabaseError(e.to_string());
    Ok(HealthCheckDto {
        application_id: row.try_get(0).map_err(err)?,
        enabled: row.try_get::<i64, _>(1).map_err(err)? != 0,
        protocol: row.try_get(2).map_err(err)?,
        path: row.try_get(3).map_err(err)?,
        port: row.try_get(4).map_err(err)?,
        interval_seconds: row.try_get(5).map_err(err)?,
        timeout_seconds: row.try_get(6).map_err(err)?,
        unhealthy_threshold: row.try_get(7).map_err(err)?,
        restart_on_unhealthy: row.try_get::<i64, _>(8).map_err(err)? != 0,
        consecutive_failures: row.try_get(9).map_err(err)?,
        last_status: row.try_get(10).map_err(err)?,
        last_checked_at: row.try_get(11).map_err(err)?,
        last_error: row.try_get(12).map_err(err)?,
        last_restarted_at: row.try_get(13).map_err(err)?,
    })
}

fn read_check_row(row: &sqlx::sqlite::SqliteRow) -> Result<HealthCheckRow, AppError> {
    let err = |e: sqlx::Error| AppError::DatabaseError(e.to_string());
    Ok(HealthCheckRow {
        application_id: row.try_get(0).map_err(err)?,
        protocol: row.try_get(2).map_err(err)?,
        path: row.try_get(3).map_err(err)?,
        port: row.try_get(4).map_err(err)?,
        timeout_seconds: row.try_get(6).map_err(err)?,
        unhealthy_threshold: row.try_get(7).map_err(err)?,
        restart_on_unhealthy: row.try_get::<i64, _>(8).map_err(err)? != 0,
        consecutive_failures: row.try_get(9).map_err(err)?,
    })
}

/// The row for an application, defaulted (disabled, unconfigured) when none
/// exists yet - every application has an implicit health check row, it is
/// just off until someone turns it on.
pub async fn get(state: &SharedState, application_id: &str) -> Result<HealthCheckDto, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(&format!(
        "SELECT {COLUMNS} FROM application_health_checks WHERE application_id = ?1"
    ))
    .bind(application_id)
    .fetch_optional(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    match row {
        Some(row) => read_row(&row),
        None => Ok(HealthCheckDto {
            application_id: application_id.to_string(),
            enabled: false,
            protocol: "http".into(),
            path: "/".into(),
            port: None,
            interval_seconds: 30,
            timeout_seconds: 5,
            unhealthy_threshold: 3,
            restart_on_unhealthy: false,
            consecutive_failures: 0,
            last_status: "unknown".into(),
            last_checked_at: None,
            last_error: None,
            last_restarted_at: None,
        }),
    }
}

pub async fn upsert(
    state: &SharedState,
    application_id: &str,
    req: &crate::dtos::health_check::UpdateHealthCheckRequest,
) -> Result<HealthCheckDto, AppError> {
    if !matches!(req.protocol.as_str(), "http" | "tcp") {
        return Err(AppError::ValidationError(
            "Protocol must be http or tcp".into(),
        ));
    }
    if !(5..=3600).contains(&req.interval_seconds) {
        return Err(AppError::ValidationError(
            "Interval must be between 5 seconds and 1 hour".into(),
        ));
    }
    if !(1..=60).contains(&req.timeout_seconds) {
        return Err(AppError::ValidationError(
            "Timeout must be between 1 and 60 seconds".into(),
        ));
    }
    if !(1..=20).contains(&req.unhealthy_threshold) {
        return Err(AppError::ValidationError(
            "Unhealthy threshold must be between 1 and 20".into(),
        ));
    }
    if let Some(port) = req.port {
        if !(1..=65535).contains(&port) {
            return Err(AppError::ValidationError("Port must be 1-65535".into()));
        }
    }
    let path = if req.path.trim().is_empty() {
        "/".to_string()
    } else {
        req.path.trim().to_string()
    };
    let now = chrono::Utc::now().timestamp_millis();
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "INSERT INTO application_health_checks (application_id, enabled, protocol, path, port, interval_seconds, timeout_seconds, unhealthy_threshold, restart_on_unhealthy, consecutive_failures, last_status, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, 'unknown', ?10, ?10) \
         ON CONFLICT (application_id) DO UPDATE SET \
         enabled = excluded.enabled, protocol = excluded.protocol, path = excluded.path, port = excluded.port, \
         interval_seconds = excluded.interval_seconds, timeout_seconds = excluded.timeout_seconds, \
         unhealthy_threshold = excluded.unhealthy_threshold, restart_on_unhealthy = excluded.restart_on_unhealthy, \
         updated_at = excluded.updated_at",
    )
    .bind(application_id)
    .bind(i64::from(req.enabled))
    .bind(&req.protocol)
    .bind(&path)
    .bind(req.port)
    .bind(req.interval_seconds)
    .bind(req.timeout_seconds)
    .bind(req.unhealthy_threshold)
    .bind(i64::from(req.restart_on_unhealthy))
    .bind(now)
    .execute(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    get(state, application_id).await
}

pub fn spawn(state: SharedState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(TICK_SECONDS));
        loop {
            interval.tick().await;
            if let Err(error) = tick(&state).await {
                tracing::warn!(%error, "health check tick failed");
            }
        }
    });
}

async fn tick(state: &SharedState) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let now = chrono::Utc::now().timestamp_millis();
    let rows = sqlx::query(&format!(
        "SELECT {COLUMNS} FROM application_health_checks \
         WHERE enabled = 1 AND (last_checked_at IS NULL OR last_checked_at + (interval_seconds * 1000) <= ?1)"
    ))
    .bind(now)
    .fetch_all(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let mut due = Vec::with_capacity(rows.len());
    for row in &rows {
        due.push(read_check_row(row)?);
    }
    for check in due {
        run_one(state, &conn, check).await;
    }
    Ok(())
}

async fn run_one(state: &SharedState, conn: &sqlx::SqlitePool, check: HealthCheckRow) {
    let now = chrono::Utc::now().timestamp_millis();
    let (healthy, error) = match checker::probe(state, &check).await {
        Ok(result) => result,
        Err(error) => (false, Some(error.to_string())),
    };
    let failures = if healthy {
        0
    } else {
        check.consecutive_failures + 1
    };
    let status = if healthy { "healthy" } else { "unhealthy" };
    let _ = sqlx::query(
        "UPDATE application_health_checks SET last_status = ?1, consecutive_failures = ?2, last_checked_at = ?3, last_error = ?4 WHERE application_id = ?5",
    )
    .bind(status)
    .bind(failures)
    .bind(now)
    .bind(&error)
    .bind(&check.application_id)
    .execute(conn)
    .await;

    if !healthy && check.restart_on_unhealthy && failures >= check.unhealthy_threshold {
        tracing::warn!(
            application_id = %check.application_id,
            failures,
            error = error.as_deref().unwrap_or(""),
            "health check threshold reached, restarting application"
        );
        let restart = crate::deployment::start(state, &check.application_id, None).await;
        if let Err(error) = restart {
            tracing::warn!(application_id = %check.application_id, %error, "health check restart failed");
        } else {
            let _ = sqlx::query(
                "UPDATE application_health_checks SET consecutive_failures = 0, last_restarted_at = ?1 WHERE application_id = ?2",
            )
            .bind(now)
            .bind(&check.application_id)
            .execute(conn)
            .await;
        }
    }
}
