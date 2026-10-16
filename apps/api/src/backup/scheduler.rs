use std::time::Duration;

use chrono::{NaiveTime, Utc};

use crate::backup::config;
use crate::state::state::SharedState;

/// Public entry point: spawn the background backup scheduler. Idempotent:
/// safe to call once at boot, same shape as `updater::spawn`.
pub fn spawn(state: SharedState) {
    tokio::spawn(async move {
        run_loop(state).await;
    });
}

async fn run_loop(state: SharedState) {
    let mut ticker = tokio::time::interval(Duration::from_secs(60));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        ticker.tick().await;
        if let Err(error) = tick(&state).await {
            tracing::warn!(error = %error, "backup scheduler tick failed");
        }
    }
}

async fn tick(state: &SharedState) -> Result<(), String> {
    let Some(s3_config) = config::load(state)
        .await
        .map_err(|error| error.to_string())?
    else {
        return Ok(());
    };
    if !s3_config.schedule_enabled {
        return Ok(());
    }
    let Some(interval) = s3_config.schedule_interval.as_deref() else {
        return Ok(());
    };
    let Some(schedule_time) = s3_config.schedule_time.as_deref().and_then(parse_time) else {
        return Ok(());
    };

    let now = Utc::now();
    if now.time() < schedule_time {
        return Ok(());
    }

    let last_scheduled_at = last_scheduled_run(state)
        .await
        .map_err(|error| error.to_string())?;
    let due = match (interval, last_scheduled_at) {
        (_, None) => true,
        ("daily", Some(last)) => last.date_naive() < now.date_naive(),
        ("weekly", Some(last)) => now.signed_duration_since(last) >= chrono::Duration::days(7),
        _ => false,
    };
    if !due {
        return Ok(());
    }

    // `try_lock_owned` inside `run::start` is the actual guard against a
    // concurrent manual backup — a busy lock here just means this tick skips
    // and the next one (or the next scheduled window) tries again.
    match crate::backup::run::start(state, "scheduled", None).await {
        Ok(record) => tracing::info!(backup_id = %record.id, "scheduled backup started"),
        Err(error) => tracing::warn!(error = %error, "scheduled backup did not start"),
    }
    Ok(())
}

fn parse_time(value: &str) -> Option<NaiveTime> {
    let (hour, minute) = value.split_once(':')?;
    NaiveTime::from_hms_opt(hour.trim().parse().ok()?, minute.trim().parse().ok()?, 0)
}

async fn last_scheduled_run(
    state: &SharedState,
) -> Result<Option<chrono::DateTime<Utc>>, crate::dtos::response::AppError> {
    use crate::dtos::response::AppError;
    use sqlx::Row;
    let conn = crate::git::repositories::db::get_conn(state).await;
    let Some(row) = sqlx::query(
        "SELECT started_at FROM backup_history WHERE triggered_by = 'scheduled' ORDER BY started_at DESC LIMIT 1",
    )
    .fetch_optional(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?
    else {
        return Ok(None);
    };
    let started_at: i64 = row
        .try_get(0)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok(chrono::DateTime::from_timestamp_millis(started_at))
}
