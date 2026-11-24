//! Background scheduler for enabled volume backup policies, same 60-second tick
//! as `database_backup::scheduler`. A due policy starts a run and rolls its
//! `next_run_at` forward so a crashed tick restarts cleanly.

use std::time::Duration;

use chrono::Utc;
use sqlx::Row;

use crate::state::state::SharedState;

/// Spawn the background loop. Call once at boot.
pub fn spawn(state: SharedState) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(60));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            if let Err(error) = tick(&state).await {
                tracing::warn!(error = %error, "volume backup scheduler tick failed");
            }
        }
    });
}

async fn tick(state: &SharedState) -> Result<(), String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let now = Utc::now().timestamp_millis();
    let rows = sqlx::query(
        "SELECT application_id, volume_name, cron_expression FROM volume_backup_policies \
         WHERE enabled = 1 AND next_run_at IS NOT NULL AND next_run_at <= ?1",
    )
    .bind(now)
    .fetch_all(&conn)
    .await
    .map_err(|e| e.to_string())?;
    // Due volumes of one application run together, as one entry in its history.
    let mut due: Vec<(String, Vec<(String, String)>)> = Vec::new();
    for row in &rows {
        let application_id: String = row.try_get(0).map_err(|e| e.to_string())?;
        let volume_name: String = row.try_get(1).map_err(|e| e.to_string())?;
        let cron_expression: String = row.try_get(2).map_err(|e| e.to_string())?;
        match due.iter_mut().find(|(app, _)| *app == application_id) {
            Some((_, volumes)) => volumes.push((volume_name, cron_expression)),
            None => due.push((application_id, vec![(volume_name, cron_expression)])),
        }
    }
    for (application_id, volumes) in &due {
        let names: Vec<String> = volumes.iter().map(|(name, _)| name.clone()).collect();
        match crate::volume_backup::run::request_scheduled_run(state, application_id, &names).await
        {
            Ok(group_id) => {
                tracing::info!(%application_id, %group_id, "scheduled volume backup started")
            }
            Err(error) => {
                tracing::warn!(%application_id, error = %error, "scheduled volume backup did not start")
            }
        }
        for (volume_name, cron_expression) in volumes {
            let next =
                crate::volume_backup::run::next_run_at(cron_expression).map_err(|e| e.to_string())?;
            // Guard on `<= now` so two racing ticks cannot both advance it.
            sqlx::query(
                "UPDATE volume_backup_policies SET next_run_at = ?1 \
                 WHERE application_id = ?2 AND volume_name = ?3 AND next_run_at <= ?4",
            )
            .bind(next)
            .bind(application_id)
            .bind(volume_name)
            .bind(now)
            .execute(&conn)
            .await
            .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
