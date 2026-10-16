use std::time::Duration;

use crate::{scheduled_tasks, state::state::SharedState};

pub fn spawn(state: SharedState) {
    let purge_state = state.clone();
    tokio::spawn(async move {
        if let Err(error) = scheduled_tasks::runner::reconcile_on_startup(&state).await {
            tracing::warn!(%error, "scheduled tasks startup reconcile failed");
        }
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        loop {
            interval.tick().await;
            if let Err(error) = scheduled_tasks::runner::reconcile_running(&state).await {
                tracing::warn!(%error, "scheduled tasks reconcile failed");
            }
            if let Err(error) = tick(&state).await {
                tracing::warn!(%error, "scheduled tasks scheduler tick failed");
            }
        }
    });
    // run history is retained for the configured number of days; purge hourly so the log tables cannot grow unbounded
    tokio::spawn(async move {
        if let Err(error) = scheduled_tasks::purge_expired_history(&purge_state).await {
            tracing::warn!(%error, "scheduled tasks history purge failed");
        }
        let mut hourly = tokio::time::interval(Duration::from_secs(60 * 60));
        loop {
            hourly.tick().await;
            if let Err(error) = scheduled_tasks::purge_expired_history(&purge_state).await {
                tracing::warn!(%error, "scheduled tasks history purge failed");
            }
        }
    });
}

async fn tick(state: &SharedState) -> Result<(), crate::dtos::response::AppError> {
    use sqlx::Row;
    let now = chrono::Utc::now().timestamp_millis();
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query("SELECT id, organization_id, cron_expression FROM scheduled_tasks WHERE enabled = 1 AND next_run_at <= ?1")
        .bind(now)
        .fetch_all(&conn)
        .await
        .map_err(|e| crate::dtos::response::AppError::DatabaseError(e.to_string()))?;
    let mut due = Vec::new();
    for row in &rows {
        due.push((
            row.try_get::<String, _>(0).unwrap(),
            row.try_get::<String, _>(1).unwrap(),
            row.try_get::<String, _>(2).unwrap(),
        ));
    }
    for (task_id, organization_id, cron) in due {
        let next = scheduled_tasks::next_run_at(&cron)?;
        let changed = sqlx::query("UPDATE scheduled_tasks SET next_run_at = ?1 WHERE id = ?2 AND organization_id = ?3 AND next_run_at <= ?4")
            .bind(next)
            .bind(task_id.clone())
            .bind(organization_id.clone())
            .bind(now)
            .execute(&conn)
            .await
            .map_err(|e| crate::dtos::response::AppError::DatabaseError(e.to_string()))?
            .rows_affected();
        if changed > 0 {
            let execution =
                scheduled_tasks::queue_run(state, &organization_id, &task_id, "scheduled").await?;
            let worker = state.clone();
            tokio::spawn(async move {
                let _ = scheduled_tasks::runner::execute(worker, execution.id).await;
            });
        }
    }
    Ok(())
}
