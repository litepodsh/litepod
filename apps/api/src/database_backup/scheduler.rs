//! Background scheduler for enabled per-database backup policies, in the same
//! 60-second tick shape as `backup::scheduler`. A due policy starts a backup
//! run and rolls its `next_run_at` forward so a crashed tick restarts cleanly.

use std::time::Duration;

use chrono::Utc;
use sqlx::Row;

use crate::state::state::SharedState;

/// Public entry point: spawn the background loop. Idempotent, call once at
/// boot like `updater::spawn`.
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
            tracing::warn!(error = %error, "database backup scheduler tick failed");
        }
    }
}

struct DuePolicy {
    application_id: String,
    cron_expression: String,
}

async fn tick(state: &SharedState) -> Result<(), String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let now = Utc::now().timestamp_millis();
    let rows = sqlx::query(
        "SELECT application_id, cron_expression FROM database_backup_policies \
         WHERE enabled = 1 AND next_run_at IS NOT NULL AND next_run_at <= ?1",
    )
    .bind(now)
    .fetch_all(&conn)
    .await
    .map_err(|e| e.to_string())?;
    let mut due: Vec<DuePolicy> = Vec::new();
    for row in &rows {
        due.push(DuePolicy {
            application_id: row.try_get(0).map_err(|e| e.to_string())?,
            cron_expression: row.try_get(1).map_err(|e| e.to_string())?,
        });
    }
    for policy in due {
        match crate::database_backup::run::request_run(state, &policy.application_id, "scheduled")
            .await
        {
            Ok(record) => tracing::info!(
                application_id = %policy.application_id,
                backup_id = %record.id,
                "scheduled database backup started"
            ),
            Err(error) => tracing::warn!(
                application_id = %policy.application_id,
                error = %error,
                "scheduled database backup did not start"
            ),
        }
        let next = crate::scheduled_tasks::next_run_at(&policy.cron_expression)
            .map_err(|e| e.to_string())?;
        // Guard on `<= now` so two racing ticks cannot both advance it.
        sqlx::query(
            "UPDATE database_backup_policies SET next_run_at = ?1 \
             WHERE application_id = ?2 AND next_run_at <= ?3",
        )
        .bind(next)
        .bind(&policy.application_id)
        .bind(now)
        .execute(&conn)
        .await
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}
