use chrono::Utc;
use sqlx::Row;
use uuid::Uuid;

use crate::dtos::deployment::{
    DeploymentRunActorDto, DeploymentRunDetailDto, DeploymentRunDto, DeploymentRunLogDto,
    DeploymentRunReplicaDto, DeploymentRunStepDto,
};
use crate::{dtos::response::AppError, state::state::SharedState};

const DEFAULT_LOG_TAIL: usize = 500;
const MAX_LOG_TAIL: usize = 1_000;

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

pub async fn start(
    state: &SharedState,
    application_id: &str,
    source_type: &str,
    build_method: &str,
    triggered_by: &str,
    branch: Option<&str>,
    created_by: Option<&str>,
) -> Result<String, AppError> {
    let id = Uuid::now_v7().to_string();
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("INSERT INTO application_deployment_runs (id, application_id, source_type, build_method, status, started_at, triggered_by, branch, created_by) VALUES (?1, ?2, ?3, ?4, 'building', ?5, ?6, ?7, ?8)")
        .bind(id.clone())
        .bind(application_id)
        .bind(source_type)
        .bind(build_method)
        .bind(now_ms())
        .bind(triggered_by)
        .bind(branch)
        .bind(created_by)
        .execute(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    sqlx::query("DELETE FROM application_deployment_runs WHERE id IN (SELECT id FROM application_deployment_runs WHERE application_id = ?1 ORDER BY started_at DESC, id DESC LIMIT -1 OFFSET 10)")
        .bind(application_id)
        .execute(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    append(state, &id, "info", "Deployment started").await?;
    Ok(id)
}

/// Written once the shallow clone succeeds - not known at `start()` time since
/// that runs before the repo is cloned.
pub async fn set_commit(
    state: &SharedState,
    run_id: &str,
    sha: &str,
    message: &str,
    author_name: Option<&str>,
    author_email: Option<&str>,
) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE application_deployment_runs SET commit_sha = ?1, commit_message = ?2, commit_author_name = ?3, commit_author_email = ?4 WHERE id = ?5",
    )
    .bind(sha)
    .bind(message)
    .bind(author_name)
    .bind(author_email)
    .bind(run_id)
    .execute(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok(())
}

/// Records which artifact a static run deployed. Set once, right after
/// `start()`, from whatever `application_deployments.artifact_id` was
/// current at that moment - the same thing `set_commit` does for git/compose,
/// just for the one piece of source identity static deploys have.
pub async fn set_artifact(
    state: &SharedState,
    run_id: &str,
    artifact_id: &str,
) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("UPDATE application_deployment_runs SET artifact_id = ?1 WHERE id = ?2")
        .bind(artifact_id)
        .bind(run_id)
        .execute(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok(())
}

/// The `commit_sha` currently on a run, if any - `git::pipeline` reads this
/// at the top of every run to tell a normal deploy (nothing set yet) apart
/// from a rollback (`pin_commit` already seeded the target commit).
pub async fn commit_sha(state: &SharedState, run_id: &str) -> Result<Option<String>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let Some(row) = sqlx::query("SELECT commit_sha FROM application_deployment_runs WHERE id = ?1")
        .bind(run_id)
        .fetch_optional(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?
    else {
        return Ok(None);
    };
    let sha: Option<String> = row
        .try_get(0)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok(sha)
}

/// Records the exact Compose YAML a `source_type='compose'` run deployed.
/// `ComposeFileStore` is one mutable file with no history of its own, so
/// this is the only place that text survives past the next edit+redeploy.
pub async fn set_compose_content(
    state: &SharedState,
    run_id: &str,
    content: &str,
) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("UPDATE application_deployment_runs SET compose_content = ?1 WHERE id = ?2")
        .bind(content)
        .bind(run_id)
        .execute(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok(())
}

/// Seeds a run's `commit_sha` before the pipeline clones anything - a
/// rollback's way of telling `git::pipeline` "pin the checkout to this exact
/// commit instead of the branch tip". Message/author are left blank; the
/// pipeline overwrites all four via the normal `set_commit` call once it has
/// actually read the pinned commit object.
pub async fn pin_commit(state: &SharedState, run_id: &str, sha: &str) -> Result<(), AppError> {
    set_commit(state, run_id, sha, "", None, None).await
}

/// What a rollback needs from the run it is redeploying: whether it is safe
/// to redeploy at all, and the one piece of source identity relevant to its
/// `source_type` (`commit_sha` for git/compose, `artifact_id` for static).
pub struct RollbackSource {
    pub source_type: String,
    pub status: String,
    pub commit_sha: Option<String>,
    pub artifact_id: Option<String>,
    pub compose_content: Option<String>,
}

pub async fn rollback_source(
    state: &SharedState,
    application_id: &str,
    run_id: &str,
) -> Result<RollbackSource, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(
        "SELECT source_type, status, commit_sha, artifact_id, compose_content FROM application_deployment_runs WHERE id = ?1 AND application_id = ?2",
    )
    .bind(run_id)
    .bind(application_id)
    .fetch_optional(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?
    .ok_or(AppError::ResourceNotFound)?;
    Ok(RollbackSource {
        source_type: row
            .try_get(0)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        status: row
            .try_get(1)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        commit_sha: row
            .try_get(2)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        artifact_id: row
            .try_get(3)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        compose_content: row
            .try_get(4)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
    })
}

/// Moves a run between lifecycle states while it is still in flight. A run's
/// states are its own (`building`, `deploying`, `succeeded`, `failed`,
/// `canceled`) and never the container's (`running`).
pub async fn set_status(state: &SharedState, run_id: &str, status: &str) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("UPDATE application_deployment_runs SET status = ?1 WHERE id = ?2")
        .bind(status)
        .bind(run_id)
        .execute(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok(())
}

pub async fn append(
    state: &SharedState,
    run_id: &str,
    level: &str,
    message: &str,
) -> Result<(), AppError> {
    let sanitized = crate::deployment::log_filter::strip_ansi_sequences(message);
    // Replicas of one run deploy in parallel; each keeps its own filter state so
    // their interleaved output does not confuse each other's de-duplication.
    let server_id = crate::servers::replica::current();
    let message = {
        let mut filters = state.queue_runtime.log_filters.lock().await;
        let filter = filters
            .entry(filter_key(run_id, server_id.as_deref()))
            .or_default();
        let Some(message) = filter.filter(&sanitized) else {
            return Ok(());
        };
        crate::deployment::log_filter::truncate_line(message)
    };
    let Some((sequence, job_id)) = state
        .queue_runtime
        .log_buffers
        .enqueue(
            state,
            run_id,
            level,
            &message,
            now_ms(),
            server_id.as_deref(),
        )
        .await?
    else {
        return Ok(());
    };
    if let Some(job_id) = job_id {
        let _ = state
            .queue_runtime
            .events
            .send(crate::deployment::queue::QueueEvent {
                job_id,
                kind: "log".into(),
                sequence: Some(sequence),
                level: Some(level.into()),
                payload: message,
                server_id,
            });
    }
    Ok(())
}

fn filter_key(run_id: &str, server_id: Option<&str>) -> String {
    match server_id {
        Some(server_id) => format!("{run_id}/{server_id}"),
        None => run_id.to_string(),
    }
}

/// Records that a replica of a group deploy entered `status` inside `run_id`.
/// `deploying` stamps `started_at`; terminal states stamp `finished_at`.
pub async fn set_replica(
    state: &SharedState,
    run_id: &str,
    server_id: &str,
    server_name: &str,
    status: &str,
    error: Option<&str>,
) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let now = now_ms();
    sqlx::query(
        "INSERT INTO application_deployment_run_replicas (run_id, server_id, server_name, status, started_at, finished_at, last_error) \
         VALUES (?1, ?2, ?3, ?4, CASE WHEN ?4 = 'deploying' THEN ?5 END, CASE WHEN ?4 IN ('succeeded', 'failed', 'canceled') THEN ?5 END, ?6) \
         ON CONFLICT (run_id, server_id) DO UPDATE SET status = excluded.status, \
           started_at = COALESCE(excluded.started_at, started_at), \
           finished_at = excluded.finished_at, last_error = excluded.last_error",
    )
    .bind(run_id)
    .bind(server_id)
    .bind(server_name)
    .bind(status)
    .bind(now)
    .bind(error)
    .execute(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok(())
}

tokio::task_local! {
    static CURRENT_RUN: String;
}

/// Runs a queue pipeline with `run_id` as the ambient run, so code deep in the
/// rollout (`step`) can record progress without threading the id through.
pub async fn for_run<T>(run_id: &str, future: impl std::future::Future<Output = T>) -> T {
    CURRENT_RUN.scope(run_id.to_string(), future).await
}

/// The steps a queued run of this shape goes through, in order. A step the
/// pipeline never reaches ends up `skipped`.
pub fn planned_steps(source_type: &str, build_method: &str) -> &'static [&'static str] {
    match (source_type, build_method) {
        ("git", "compose") => &["clone", "start", "health_check", "route"],
        ("git", _) => &["clone", "build", "start", "health_check", "route"],
        ("image", _) => &["pull", "start", "health_check", "route"],
        ("static", _) => &["prepare", "start", "route"],
        _ => &["prepare", "pull", "start", "health_check", "route"],
    }
}

/// Seeds the run's (or the current replica's) steps as `pending` and starts
/// the first one.
pub async fn plan_steps(state: &SharedState, run_id: &str, names: &[&str]) -> Result<(), AppError> {
    let server_id = crate::servers::replica::current().unwrap_or_default();
    let conn = crate::git::repositories::db::get_conn(state).await;
    for (position, name) in names.iter().enumerate() {
        sqlx::query("INSERT INTO application_deployment_run_steps (run_id, server_id, position, name, status) VALUES (?1, ?2, ?3, ?4, 'pending') ON CONFLICT DO NOTHING")
            .bind(run_id)
            .bind(&server_id)
            .bind(position as i64)
            .bind(name)
            .execute(&conn)
            .await
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    }
    match names.first() {
        Some(first) => begin_step(state, run_id, &server_id, first).await,
        None => Ok(()),
    }
}

/// Marks `name` as the running step of the ambient run: the previous running
/// step succeeds and any pending step before it is skipped. A no-op outside a
/// queue pipeline (lifecycle actions record no steps).
pub async fn step(state: &SharedState, name: &str) {
    let Ok(run_id) = CURRENT_RUN.try_with(Clone::clone) else {
        return;
    };
    let server_id = crate::servers::replica::current().unwrap_or_default();
    if let Err(error) = begin_step(state, &run_id, &server_id, name).await {
        tracing::warn!(run_id, %error, "could not record deployment step");
    }
}

/// Appends a log line to the ambient run. A no-op outside a queue pipeline.
pub async fn note(state: &SharedState, level: &str, message: &str) {
    let Ok(run_id) = CURRENT_RUN.try_with(Clone::clone) else {
        return;
    };
    let _ = append(state, &run_id, level, message).await;
}

async fn begin_step(
    state: &SharedState,
    run_id: &str,
    server_id: &str,
    name: &str,
) -> Result<(), AppError> {
    let now = now_ms();
    let conn = crate::git::repositories::db::get_conn(state).await;
    let db = |e: sqlx::Error| AppError::DatabaseError(e.to_string());
    sqlx::query("UPDATE application_deployment_run_steps SET status = 'succeeded', finished_at = ?1 WHERE run_id = ?2 AND server_id = ?3 AND status = 'running' AND name <> ?4")
        .bind(now).bind(run_id).bind(server_id).bind(name)
        .execute(&conn).await.map_err(db)?;
    // An unplanned step is appended after the planned ones.
    sqlx::query("INSERT INTO application_deployment_run_steps (run_id, server_id, position, name, status) SELECT ?1, ?2, COALESCE(MAX(position) + 1, 0), ?3, 'pending' FROM application_deployment_run_steps WHERE run_id = ?1 AND server_id = ?2 ON CONFLICT DO NOTHING")
        .bind(run_id).bind(server_id).bind(name)
        .execute(&conn).await.map_err(db)?;
    sqlx::query("UPDATE application_deployment_run_steps SET status = 'skipped' WHERE run_id = ?1 AND server_id = ?2 AND status = 'pending' AND position < (SELECT position FROM application_deployment_run_steps WHERE run_id = ?1 AND server_id = ?2 AND name = ?3)")
        .bind(run_id).bind(server_id).bind(name)
        .execute(&conn).await.map_err(db)?;
    sqlx::query("UPDATE application_deployment_run_steps SET status = 'running', started_at = ?1 WHERE run_id = ?2 AND server_id = ?3 AND name = ?4 AND status = 'pending'")
        .bind(now).bind(run_id).bind(server_id).bind(name)
        .execute(&conn).await.map_err(db)?;
    drop(conn);
    emit_steps(state, run_id).await;
    Ok(())
}

/// Closes the open steps of a finished run (or of one replica): the running
/// step takes the outcome, steps never reached are skipped.
async fn close_steps(state: &SharedState, run_id: &str, server_id: Option<&str>, status: &str) {
    let outcome = if status == "succeeded" {
        "succeeded"
    } else {
        "failed"
    };
    let conn = crate::git::repositories::db::get_conn(state).await;
    let result = sqlx::query("UPDATE application_deployment_run_steps SET status = CASE status WHEN 'running' THEN ?1 ELSE 'skipped' END, finished_at = CASE status WHEN 'running' THEN ?2 END WHERE run_id = ?3 AND (?4 IS NULL OR server_id = ?4) AND status IN ('running', 'pending')")
        .bind(outcome)
        .bind(now_ms())
        .bind(run_id)
        .bind(server_id)
        .execute(&conn)
        .await;
    drop(conn);
    if matches!(result, Ok(done) if done.rows_affected() > 0) {
        emit_steps(state, run_id).await;
    }
}

/// Pushes the run's whole step list to live viewers as a `steps` event.
async fn emit_steps(state: &SharedState, run_id: &str) {
    let Ok(Some(job_id)) =
        sqlx::query_scalar::<_, String>("SELECT id FROM deployment_jobs WHERE run_id = ?1")
            .bind(run_id)
            .fetch_optional(&state.db)
            .await
    else {
        return;
    };
    let Ok(steps) = steps_for(state, run_id).await else {
        return;
    };
    let _ = state
        .queue_runtime
        .events
        .send(crate::deployment::queue::QueueEvent {
            job_id,
            kind: "steps".into(),
            sequence: None,
            level: None,
            payload: serde_json::to_string(&steps).unwrap_or_default(),
            server_id: None,
        });
}

pub async fn steps_for(
    state: &SharedState,
    run_id: &str,
) -> Result<Vec<DeploymentRunStepDto>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query("SELECT name, status, started_at, finished_at, server_id FROM application_deployment_run_steps WHERE run_id = ?1 ORDER BY server_id, position")
        .bind(run_id)
        .fetch_all(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let db = |e: sqlx::Error| AppError::DatabaseError(e.to_string());
    rows.iter()
        .map(|row| {
            let server_id: String = row.try_get(4).map_err(db)?;
            Ok(DeploymentRunStepDto {
                name: row.try_get(0).map_err(db)?,
                status: row.try_get(1).map_err(db)?,
                started_at: row.try_get(2).map_err(db)?,
                finished_at: row.try_get(3).map_err(db)?,
                server_id: (!server_id.is_empty()).then_some(server_id),
            })
        })
        .collect()
}

/// Replica rows for the given runs, keyed by run id.
async fn replicas_for(
    state: &SharedState,
    run_ids: &[String],
) -> Result<std::collections::HashMap<String, Vec<DeploymentRunReplicaDto>>, AppError> {
    let mut by_run: std::collections::HashMap<String, Vec<DeploymentRunReplicaDto>> =
        std::collections::HashMap::new();
    if run_ids.is_empty() {
        return Ok(by_run);
    }
    let placeholders = (1..=run_ids.len())
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT rr.run_id, rr.server_id, rr.server_name, rr.status, rr.started_at, rr.finished_at, rr.last_error, \
         s.kind, s.facts_json, s.public_address, s.ssh_host \
         FROM application_deployment_run_replicas rr LEFT JOIN servers s ON s.id = rr.server_id \
         WHERE rr.run_id IN ({placeholders}) ORDER BY rr.server_name"
    );
    let mut query = sqlx::query(&sql);
    for run_id in run_ids {
        query = query.bind(run_id);
    }
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = query
        .fetch_all(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let db = |e: sqlx::Error| AppError::DatabaseError(e.to_string());
    for row in &rows {
        let kind: Option<String> = row.try_get(7).map_err(db)?;
        let facts: Option<String> = row.try_get(8).map_err(db)?;
        let public_address: Option<String> = row.try_get(9).map_err(db)?;
        let ssh_host: Option<String> = row.try_get(10).map_err(db)?;
        let facts = facts
            .and_then(|facts| {
                serde_json::from_str::<crate::servers::setup::ServerFacts>(&facts).ok()
            })
            .unwrap_or_default();
        let (os, distribution, _) =
            crate::dtos::server::os_marks(kind.as_deref().unwrap_or("ssh"), &facts);
        let run_id: String = row.try_get(0).map_err(db)?;
        by_run
            .entry(run_id)
            .or_default()
            .push(DeploymentRunReplicaDto {
                server_id: row.try_get(1).map_err(db)?,
                server_name: row.try_get(2).map_err(db)?,
                status: row.try_get(3).map_err(db)?,
                started_at: row.try_get(4).map_err(db)?,
                finished_at: row.try_get(5).map_err(db)?,
                last_error: row.try_get(6).map_err(db)?,
                os,
                distribution,
                address: public_address
                    .or(ssh_host)
                    .filter(|address| !address.is_empty()),
            });
    }
    Ok(by_run)
}

pub async fn finish(
    state: &SharedState,
    run_id: &str,
    status: &str,
    error: Option<&str>,
) -> Result<(), AppError> {
    // Inside a group fan-out a pipeline only finishes its own replica; the
    // queue finishes the run once every replica is done.
    if let Some(server_id) = crate::servers::replica::current() {
        close_steps(state, run_id, Some(&server_id), status).await;
        append(
            state,
            run_id,
            if status == "failed" { "error" } else { "info" },
            error.unwrap_or("Deployment completed on this server"),
        )
        .await?;
        state
            .queue_runtime
            .log_filters
            .lock()
            .await
            .remove(&filter_key(run_id, Some(&server_id)));
        return Ok(());
    }
    close_steps(state, run_id, None, status).await;
    append(
        state,
        run_id,
        if status == "failed" { "error" } else { "info" },
        error.unwrap_or("Deployment completed"),
    )
    .await?;
    state.queue_runtime.log_filters.lock().await.remove(run_id);
    state.queue_runtime.log_buffers.close(run_id).await;
    // Read the in-flight phase before it is overwritten: a `failed` run that was
    // still `building` is a build error, one already `deploying` failed at
    // rollout. Notifications need the distinction and nothing else records it.
    let phase = crate::notifications::repository::run_phase(state, run_id)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| "building".to_string());
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("UPDATE application_deployment_runs SET status = ?1, finished_at = ?2, last_error = ?3 WHERE id = ?4")
        .bind(status)
        .bind(now_ms())
        .bind(error)
        .bind(run_id)
        .execute(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    crate::notifications::notify_deployment_finished(state, run_id, status, &phase, error).await;
    Ok(())
}

/// Deploy pipelines live in a `tokio::spawn`, so a restart kills them without
/// ever writing a terminal status. Anything still `deploying` at boot is a
/// leftover from the previous process, not work in flight.
///
/// Runs *after* `servers::monitor::reconcile_at_boot`, which has already moved every
/// application whose container is actually up out of `deploying`. Whatever is still
/// `deploying` here either never finished or sits on a server that did not answer at
/// boot; both are honestly reported as failed, and the monitor corrects the second case
/// as soon as the server answers.
pub async fn fail_interrupted(state: &SharedState) -> Result<(), AppError> {
    const MESSAGE: &str = "Deployment was interrupted because the server restarted";
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(
        "SELECT id FROM application_deployment_runs WHERE status IN ('building', 'deploying')",
    )
    .fetch_all(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let mut run_ids: Vec<String> = Vec::new();
    for row in &rows {
        run_ids.push(
            row.try_get(0)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        );
    }
    for run_id in &run_ids {
        finish(state, run_id, "failed", Some(MESSAGE)).await?;
    }
    sqlx::query("UPDATE application_deployments SET status = 'failed', last_error = ?1, updated_at = ?2 WHERE status = 'deploying'")
        .bind(MESSAGE)
        .bind(now_ms())
        .execute(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    if !run_ids.is_empty() {
        tracing::warn!(
            count = run_ids.len(),
            "marked interrupted deployments as failed"
        );
    }
    Ok(())
}

const RUN_COLUMNS: &str = "r.id, r.source_type, r.build_method, r.status, r.started_at, r.finished_at, r.last_error, r.commit_sha, r.commit_message, r.commit_author_name, r.commit_author_email, r.triggered_by, r.branch, j.id, u.id, u.name, u.email";
const RUN_JOINS: &str =
    "LEFT JOIN deployment_jobs j ON j.run_id = r.id LEFT JOIN users u ON u.id = r.created_by";

fn row_to_run(row: &sqlx::sqlite::SqliteRow) -> Result<DeploymentRunDto, AppError> {
    let started_at: i64 = row
        .try_get(4)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let finished_at: Option<i64> = row
        .try_get(5)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let duration_ms = finished_at.map(|f| f - started_at);
    Ok(DeploymentRunDto {
        id: row
            .try_get(0)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        source_type: row
            .try_get(1)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        build_method: row
            .try_get(2)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        status: row
            .try_get(3)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        started_at,
        finished_at,
        duration_ms,
        last_error: row
            .try_get(6)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        commit_sha: row
            .try_get(7)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        commit_message: row
            .try_get(8)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        commit_author_name: row
            .try_get(9)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        commit_author_email: row
            .try_get(10)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        triggered_by: row
            .try_get(11)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        branch: row
            .try_get(12)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        job_id: row
            .try_get(13)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        replicas: Vec::new(),
        actor: match row
            .try_get::<Option<String>, _>(14)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?
        {
            Some(id) => Some(DeploymentRunActorDto {
                id,
                name: row
                    .try_get(15)
                    .map_err(|e| AppError::DatabaseError(e.to_string()))?,
                email: row
                    .try_get(16)
                    .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            }),
            None => None,
        },
    })
}

/// Deletes one run and its logs (cascade). Refuses to delete a run that's
/// still `deploying` - stop it first so the flusher/log_buffers registry
/// isn't torn out from under an in-flight build.
pub async fn delete_run(
    state: &SharedState,
    application_id: &str,
    run_id: &str,
) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(
        "SELECT status FROM application_deployment_runs WHERE id = ?1 AND application_id = ?2",
    )
    .bind(run_id)
    .bind(application_id)
    .fetch_optional(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?
    .ok_or(AppError::ResourceNotFound)?;
    let status: String = row
        .try_get(0)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    if status == "deploying" {
        return Err(AppError::ValidationError(
            "Stop the deployment before deleting it".into(),
        ));
    }
    sqlx::query("DELETE FROM application_deployment_runs WHERE id = ?1 AND application_id = ?2")
        .bind(run_id)
        .bind(application_id)
        .execute(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok(())
}

/// Deletes every finished run for the application. Runs still `deploying`
/// are left alone (same reasoning as `delete_run`).
pub async fn delete_all(state: &SharedState, application_id: &str) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("DELETE FROM application_deployment_runs WHERE application_id = ?1 AND status NOT IN ('building', 'deploying')")
        .bind(application_id)
        .execute(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok(())
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct GlobalHistoryDeleteResult {
    pub deployment_runs_deleted: i64,
    pub deployment_log_lines_deleted: i64,
}

/// Deletes completed history for every application. Active deploying runs
/// remain so their queues can safely finish and persist final log lines.
pub async fn delete_all_completed(
    state: &SharedState,
) -> Result<GlobalHistoryDeleteResult, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let deployment_runs_deleted: i64 = sqlx::query(
        "SELECT COUNT(*) FROM application_deployment_runs WHERE status NOT IN ('building', 'deploying')",
    )
    .fetch_one(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?
    .try_get(0)
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let deployment_log_lines_deleted: i64 = sqlx::query(
        "SELECT COUNT(*) FROM application_deployment_run_logs WHERE run_id IN (SELECT id FROM application_deployment_runs WHERE status NOT IN ('building', 'deploying'))",
    )
    .fetch_one(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?
    .try_get(0)
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    sqlx::query(
        "DELETE FROM application_deployment_runs WHERE status NOT IN ('building', 'deploying')",
    )
    .execute(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok(GlobalHistoryDeleteResult {
        deployment_runs_deleted,
        deployment_log_lines_deleted,
    })
}

pub async fn list(
    state: &SharedState,
    application_id: &str,
) -> Result<Vec<DeploymentRunDto>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(&format!(
        "SELECT {RUN_COLUMNS} FROM application_deployment_runs r {RUN_JOINS} WHERE r.application_id = ?1 ORDER BY r.started_at DESC, r.id DESC LIMIT 10"
    ))
    .bind(application_id)
    .fetch_all(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    drop(conn);
    let mut runs = rows.iter().map(row_to_run).collect::<Result<Vec<_>, _>>()?;
    let ids = runs.iter().map(|run| run.id.clone()).collect::<Vec<_>>();
    let mut replicas = replicas_for(state, &ids).await?;
    for run in &mut runs {
        run.replicas = replicas.remove(&run.id).unwrap_or_default();
    }
    Ok(runs)
}

pub async fn get(
    state: &SharedState,
    application_id: &str,
    run_id: &str,
    tail: Option<usize>,
) -> Result<DeploymentRunDetailDto, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(&format!(
        "SELECT {RUN_COLUMNS} FROM application_deployment_runs r {RUN_JOINS} WHERE r.id = ?1 AND r.application_id = ?2"
    ))
    .bind(run_id)
    .bind(application_id)
    .fetch_optional(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?
    .ok_or(AppError::ResourceNotFound)?;
    let mut run = row_to_run(&row)?;
    run.replicas = replicas_for(state, std::slice::from_ref(&run.id))
        .await?
        .remove(&run.id)
        .unwrap_or_default();
    let tail = tail.unwrap_or(DEFAULT_LOG_TAIL).clamp(1, MAX_LOG_TAIL) as i64;
    let total_log_count: i64 =
        sqlx::query("SELECT COUNT(*) FROM application_deployment_run_logs WHERE run_id = ?1")
            .bind(run_id)
            .fetch_one(&conn)
            .await
            .map_err(|e| AppError::DatabaseError(e.to_string()))?
            .try_get(0)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let logs = sqlx::query(
        // The tail applies per replica, so a group run shows the end of every server's log.
        "SELECT sequence, level, message, created_at, server_id FROM (SELECT sequence, level, message, created_at, server_id, ROW_NUMBER() OVER (PARTITION BY server_id ORDER BY sequence DESC) AS rn FROM application_deployment_run_logs WHERE run_id = ?1) WHERE rn <= ?2 ORDER BY sequence",
    )
    .bind(run_id)
    .bind(tail)
    .fetch_all(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let mut filters: std::collections::HashMap<
        Option<String>,
        crate::deployment::log_filter::LogFilter,
    > = std::collections::HashMap::new();
    let mut entries = Vec::new();
    for row in &logs {
        let server_id: Option<String> = row
            .try_get(4)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        let message = crate::deployment::log_filter::strip_ansi_sequences(
            &row.try_get::<String, _>(2)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        );
        let Some(message) = filters
            .entry(server_id.clone())
            .or_default()
            .filter(&message)
        else {
            continue;
        };
        entries.push(DeploymentRunLogDto {
            sequence: row
                .try_get(0)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            level: row
                .try_get(1)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            message: crate::deployment::log_filter::truncate_line(message),
            created_at: row
                .try_get(3)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            server_id,
        });
    }
    drop(conn);
    Ok(DeploymentRunDetailDto {
        steps: steps_for(state, run_id).await?,
        run,
        truncated: total_log_count > entries.len() as i64,
        total_log_count,
        logs: entries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::state::AppState;
    use std::{
        collections::{HashMap, HashSet},
        sync::{Arc, Mutex},
    };

    use uuid::Uuid;

    async fn test_state() -> (SharedState, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!("litepod-history-{}.db", Uuid::now_v7()));
        let db = crate::testing::sqlite_pool_at(&path).await;
        crate::migrations::run(&db).await;
        let state: SharedState = Arc::new(AppState {
            started_at: chrono::Local::now(),
            db,
            caddy_url: String::new(),
            caddy_admin_url: String::new(),
            caddyfile_path: std::path::PathBuf::new(),
            caddy_server_upstream: String::new(),
            podman_socket: String::new(),
            pending_execs: Mutex::new(HashMap::new()),
            data_dir: std::path::PathBuf::new(),
            data_volume: None,
            host_data_dir: std::path::PathBuf::new(),
            podman_network: String::new(),
            public_url: String::new(),
            oauth_states: Mutex::new(HashMap::new()),
            public_ip: "203.0.113.7".into(),
            is_cloud: false,
            polar: None,
            dev_mode: false,
            port_range: (10_000, 20_000),
            reserved_ports: HashSet::new(),
            volume_access: Default::default(),
            podman_host: Default::default(),
            queue_runtime: Default::default(),
            version: String::new(),
            git_sha: String::new(),
            image_repo: String::new(),
            image_tag: String::new(),
            update_channel: Default::default(),
            version_manifest_url: String::new(),
            update_interval_secs: 0,
            container_name: String::new(),
            caddy_container: String::new(),
            stack_env_file: None,
            update_state: Default::default(),
            backup_lock: Default::default(),
            rate_limiter: tokio::sync::RwLock::new(
                crate::security::rate_limit::RateLimiterState::from_config(Default::default()),
            ),
            ephemeral: Default::default(),
        });
        let conn = state.db.clone();
        sqlx::query("INSERT INTO users (id, email, hashed_password, name) VALUES (?1, ?2, ?3, ?4)")
            .bind("user-1")
            .bind("user@example.com")
            .bind("hash")
            .bind("User")
            .execute(&conn)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO organizations (id, name, slug, created_by) VALUES (?1, ?2, ?3, ?4)",
        )
        .bind("organization-1")
        .bind("Organization")
        .bind("organization")
        .bind("user-1")
        .execute(&conn)
        .await
        .unwrap();
        sqlx::query("INSERT INTO projects (id, organization_id, name, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)")
            .bind("project-1")
            .bind("organization-1")
            .bind("Project")
            .bind(1_i64)
            .bind(1_i64)
            .execute(&conn)
            .await
            .unwrap();
        sqlx::query("INSERT INTO environments (id, project_id, name, is_production, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)")
            .bind("environment-1")
            .bind("project-1")
            .bind("prod")
            .bind(1_i64)
            .bind(1_i64)
            .bind(1_i64)
            .execute(&conn)
            .await
            .unwrap();
        sqlx::query("INSERT INTO applications (id, environment_id, name, server_id, created_at, updated_at) VALUES (?1, ?2, ?3, 'local', ?4, ?5)")
            .bind("application-1")
            .bind("environment-1")
            .bind("Application")
            .bind(1_i64)
            .bind(1_i64)
            .execute(&conn)
            .await
            .unwrap();
        (state, path)
    }

    fn cleanup(path: std::path::PathBuf) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{}", path.display(), suffix));
        }
    }

    #[tokio::test]
    async fn a_run_moves_from_building_through_deploying_to_succeeded() {
        let (state, path) = test_state().await;
        let run_id = start(
            &state,
            "application-1",
            "git",
            "docker",
            "manual",
            Some("main"),
            None,
        )
        .await
        .unwrap();

        let detail = get(&state, "application-1", &run_id, None).await.unwrap();
        assert_eq!(detail.run.status, "building");

        set_status(&state, &run_id, "deploying").await.unwrap();
        let detail = get(&state, "application-1", &run_id, None).await.unwrap();
        assert_eq!(detail.run.status, "deploying");

        finish(&state, &run_id, "succeeded", None).await.unwrap();
        let detail = get(&state, "application-1", &run_id, None).await.unwrap();
        assert_eq!(detail.run.status, "succeeded");

        cleanup(path);
    }

    #[tokio::test]
    async fn append_then_finish_flushes_all_lines_before_returning() {
        let (state, path) = test_state().await;
        let run_id = start(
            &state,
            "application-1",
            "git",
            "docker",
            "manual",
            Some("main"),
            None,
        )
        .await
        .unwrap();

        for i in 0..10 {
            append(&state, &run_id, "info", &format!("line {i}"))
                .await
                .unwrap();
        }
        finish(&state, &run_id, "succeeded", None).await.unwrap();

        let detail = get(&state, "application-1", &run_id, None).await.unwrap();
        assert_eq!(detail.run.status, "succeeded");
        assert_eq!(detail.logs.len(), 12);
        assert_eq!(detail.logs[0].message, "Deployment started");
        assert_eq!(detail.logs[11].message, "Deployment completed");
        let sequences: Vec<i64> = detail.logs.iter().map(|l| l.sequence).collect();
        assert_eq!(
            sequences,
            (0..12).collect::<Vec<_>>(),
            "sequence must be contiguous, no gaps from dropped/reordered batches"
        );
        cleanup(path);
    }

    #[tokio::test]
    async fn two_concurrent_runs_do_not_cross_contaminate_sequences() {
        let (state, path) = test_state().await;
        let run_a = start(
            &state,
            "application-1",
            "git",
            "docker",
            "manual",
            Some("main"),
            None,
        )
        .await
        .unwrap();
        let run_b = start(
            &state,
            "application-1",
            "git",
            "docker",
            "manual",
            Some("main"),
            None,
        )
        .await
        .unwrap();

        append(&state, &run_a, "info", "a-1").await.unwrap();
        append(&state, &run_b, "info", "b-1").await.unwrap();
        append(&state, &run_a, "info", "a-2").await.unwrap();
        finish(&state, &run_a, "succeeded", None).await.unwrap();
        finish(&state, &run_b, "succeeded", None).await.unwrap();

        let detail_a = get(&state, "application-1", &run_a, None).await.unwrap();
        let detail_b = get(&state, "application-1", &run_b, None).await.unwrap();
        assert_eq!(
            detail_a
                .logs
                .iter()
                .map(|l| l.message.clone())
                .collect::<Vec<_>>(),
            vec!["Deployment started", "a-1", "a-2", "Deployment completed"]
        );
        assert_eq!(
            detail_b
                .logs
                .iter()
                .map(|l| l.message.clone())
                .collect::<Vec<_>>(),
            vec!["Deployment started", "b-1", "Deployment completed"]
        );
        cleanup(path);
    }

    #[tokio::test]
    async fn delete_all_completed_removes_logs_but_preserves_active_deployments() {
        let (state, path) = test_state().await;
        let completed = start(
            &state,
            "application-1",
            "git",
            "docker",
            "manual",
            Some("main"),
            None,
        )
        .await
        .unwrap();
        append(&state, &completed, "info", "completed line")
            .await
            .unwrap();
        finish(&state, &completed, "succeeded", None).await.unwrap();
        let active = start(
            &state,
            "application-1",
            "git",
            "docker",
            "manual",
            Some("main"),
            None,
        )
        .await
        .unwrap();

        let deleted = delete_all_completed(&state).await.unwrap();

        assert_eq!(deleted.deployment_runs_deleted, 1);
        assert_eq!(deleted.deployment_log_lines_deleted, 3);
        assert!(
            get(&state, "application-1", &completed, None)
                .await
                .is_err()
        );
        assert_eq!(
            get(&state, "application-1", &active, None)
                .await
                .unwrap()
                .run
                .status,
            "building"
        );
        cleanup(path);
    }

    #[tokio::test]
    async fn get_returns_a_chronological_bounded_tail_with_metadata() {
        let (state, path) = test_state().await;
        let run_id = start(
            &state,
            "application-1",
            "git",
            "docker",
            "manual",
            Some("main"),
            None,
        )
        .await
        .unwrap();

        for i in 0..12 {
            append(&state, &run_id, "info", &format!("line {i}"))
                .await
                .unwrap();
        }
        finish(&state, &run_id, "succeeded", None).await.unwrap();

        let detail = get(&state, "application-1", &run_id, Some(3))
            .await
            .unwrap();
        assert_eq!(detail.total_log_count, 14);
        assert!(detail.truncated);
        assert_eq!(
            detail
                .logs
                .iter()
                .map(|line| line.message.as_str())
                .collect::<Vec<_>>(),
            vec!["line 10", "line 11", "Deployment completed"]
        );
        cleanup(path);
    }

    #[tokio::test]
    async fn run_actor_is_returned_and_survives_user_deletion() {
        let (state, path) = test_state().await;
        let conn = state.db.clone();
        sqlx::query("INSERT INTO users (id, email, hashed_password, name) VALUES (?1, ?2, ?3, ?4)")
            .bind("operator-1")
            .bind("operator@example.com")
            .bind("hash")
            .bind("Operator")
            .execute(&conn)
            .await
            .unwrap();

        let run_id = start(
            &state,
            "application-1",
            "compose",
            "compose",
            "manual_start",
            None,
            Some("operator-1"),
        )
        .await
        .unwrap();
        finish(&state, &run_id, "succeeded", None).await.unwrap();

        let run = get(&state, "application-1", &run_id, None)
            .await
            .unwrap()
            .run;
        let actor = run.actor.expect("actor should be joined");
        assert_eq!(actor.id, "operator-1");
        assert_eq!(actor.name, "Operator");
        assert_eq!(actor.email, "operator@example.com");

        sqlx::query("DELETE FROM users WHERE id = 'operator-1'")
            .execute(&conn)
            .await
            .unwrap();
        let run = get(&state, "application-1", &run_id, None)
            .await
            .unwrap()
            .run;
        assert!(run.actor.is_none());
        assert_eq!(run.triggered_by.as_deref(), Some("manual_start"));

        cleanup(path);
    }

    #[tokio::test]
    async fn commit_author_is_returned() {
        let (state, path) = test_state().await;
        let run_id = start(
            &state,
            "application-1",
            "git",
            "railpack",
            "webhook",
            Some("main"),
            None,
        )
        .await
        .unwrap();

        set_commit(
            &state,
            &run_id,
            "a1b2c3d",
            "Ship identity",
            Some("Ada Lovelace"),
            Some("ada@example.com"),
        )
        .await
        .unwrap();

        let run = get(&state, "application-1", &run_id, None)
            .await
            .unwrap()
            .run;
        assert_eq!(run.commit_author_name.as_deref(), Some("Ada Lovelace"));
        assert_eq!(run.commit_author_email.as_deref(), Some("ada@example.com"));

        cleanup(path);
    }

    fn step_states(detail: &DeploymentRunDetailDto) -> Vec<(String, String)> {
        detail
            .steps
            .iter()
            .map(|step| (step.name.clone(), step.status.clone()))
            .collect()
    }

    fn pairs(expected: &[(&str, &str)]) -> Vec<(String, String)> {
        expected
            .iter()
            .map(|(name, status)| (name.to_string(), status.to_string()))
            .collect()
    }

    #[tokio::test]
    async fn steps_advance_through_the_pipeline_and_close_on_success() {
        let (state, path) = test_state().await;
        let run_id = start(
            &state,
            "application-1",
            "git",
            "docker",
            "manual",
            None,
            None,
        )
        .await
        .unwrap();
        plan_steps(&state, &run_id, planned_steps("git", "docker"))
            .await
            .unwrap();
        let detail = get(&state, "application-1", &run_id, None).await.unwrap();
        assert_eq!(
            step_states(&detail),
            pairs(&[
                ("clone", "running"),
                ("build", "pending"),
                ("start", "pending"),
                ("health_check", "pending"),
                ("route", "pending"),
            ])
        );
        assert!(detail.steps[0].started_at.is_some());

        for_run(&run_id, async {
            step(&state, "build").await;
            // Jumping ahead skips the step that never ran.
            step(&state, "health_check").await;
        })
        .await;
        finish(&state, &run_id, "succeeded", None).await.unwrap();

        let detail = get(&state, "application-1", &run_id, None).await.unwrap();
        assert_eq!(
            step_states(&detail),
            pairs(&[
                ("clone", "succeeded"),
                ("build", "succeeded"),
                ("start", "skipped"),
                ("health_check", "succeeded"),
                ("route", "skipped"),
            ])
        );
        assert!(detail.steps.iter().all(|step| step.server_id.is_none()));
        assert!(detail.steps[1].finished_at.is_some());
        assert!(detail.steps[4].finished_at.is_none());
        cleanup(path);
    }

    #[tokio::test]
    async fn a_failed_run_fails_its_running_step_and_skips_the_rest() {
        let (state, path) = test_state().await;
        let run_id = start(
            &state,
            "application-1",
            "image",
            "image",
            "manual",
            None,
            None,
        )
        .await
        .unwrap();
        plan_steps(&state, &run_id, planned_steps("image", "image"))
            .await
            .unwrap();
        for_run(&run_id, step(&state, "start")).await;
        finish(&state, &run_id, "failed", Some("boom"))
            .await
            .unwrap();

        let detail = get(&state, "application-1", &run_id, None).await.unwrap();
        assert_eq!(
            step_states(&detail),
            pairs(&[
                ("pull", "succeeded"),
                ("start", "failed"),
                ("health_check", "skipped"),
                ("route", "skipped"),
            ])
        );
        cleanup(path);
    }

    #[tokio::test]
    async fn steps_outside_a_run_are_ignored_and_unplanned_ones_are_appended() {
        let (state, path) = test_state().await;
        let run_id = start(
            &state,
            "application-1",
            "static",
            "static",
            "manual",
            None,
            None,
        )
        .await
        .unwrap();
        // Lifecycle actions run outside the queue and record nothing.
        step(&state, "start").await;
        assert!(steps_for(&state, &run_id).await.unwrap().is_empty());

        plan_steps(&state, &run_id, planned_steps("static", "static"))
            .await
            .unwrap();
        for_run(&run_id, step(&state, "cleanup")).await;
        let detail = get(&state, "application-1", &run_id, None).await.unwrap();
        assert_eq!(
            step_states(&detail),
            pairs(&[
                ("prepare", "succeeded"),
                ("start", "skipped"),
                ("route", "skipped"),
                ("cleanup", "running"),
            ])
        );
        cleanup(path);
    }

    /// Follows the order `compose_standalone::pipeline::deploy_started` and
    /// `rollout::promote` record steps in, and checks each change reaches live
    /// viewers as a `steps` event on the job's stream.
    #[tokio::test]
    async fn a_compose_run_records_every_step_and_streams_each_change() {
        let (state, path) = test_state().await;
        let run_id = start(
            &state,
            "application-1",
            "compose",
            "compose",
            "manual",
            None,
            None,
        )
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO deployment_jobs (id, application_id, project_id, environment_id, organization_id, source_type, source_snapshot, build_method, build_snapshot, status, queued_at, run_id) VALUES ('job-1', 'application-1', 'project-1', 'environment-1', 'organization-1', 'compose', '{}', 'compose', '{}', 'building', 1, ?1)",
        )
        .bind(&run_id)
        .execute(&state.db)
        .await
        .unwrap();
        let mut events = state.queue_runtime.events.subscribe();

        plan_steps(&state, &run_id, planned_steps("compose", "compose"))
            .await
            .unwrap();
        for_run(&run_id, async {
            for name in ["pull", "start", "health_check", "route"] {
                step(&state, name).await;
            }
        })
        .await;
        finish(&state, &run_id, "succeeded", None).await.unwrap();

        let detail = get(&state, "application-1", &run_id, None).await.unwrap();
        assert_eq!(
            step_states(&detail),
            pairs(&[
                ("prepare", "succeeded"),
                ("pull", "succeeded"),
                ("start", "succeeded"),
                ("health_check", "succeeded"),
                ("route", "succeeded"),
            ])
        );
        let mut step_events = Vec::new();
        while let Ok(event) = events.try_recv() {
            if event.kind == "steps" {
                assert_eq!(event.job_id, "job-1");
                step_events
                    .push(serde_json::from_str::<serde_json::Value>(&event.payload).unwrap());
            }
        }
        // plan (prepare running), four step changes, then the close on finish.
        assert_eq!(step_events.len(), 6);
        assert_eq!(step_events[2][2]["status"], "running");
        assert_eq!(step_events[2][2]["name"], "start");
        cleanup(path);
    }

    #[tokio::test]
    async fn replicas_keep_their_own_steps() {
        let (state, path) = test_state().await;
        let run_id = start(
            &state,
            "application-1",
            "image",
            "image",
            "manual",
            None,
            None,
        )
        .await
        .unwrap();
        for server in ["server-a", "server-b"] {
            crate::servers::replica::for_replica(
                server,
                plan_steps(&state, &run_id, planned_steps("image", "image")),
            )
            .await
            .unwrap();
        }
        crate::servers::replica::for_replica("server-a", async {
            for_run(&run_id, step(&state, "start")).await;
            finish(&state, &run_id, "failed", Some("boom"))
                .await
                .unwrap();
        })
        .await;

        let steps = steps_for(&state, &run_id).await.unwrap();
        let of = |server: &str| {
            steps
                .iter()
                .filter(|step| step.server_id.as_deref() == Some(server))
                .map(|step| step.status.as_str())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            of("server-a"),
            ["succeeded", "failed", "skipped", "skipped"]
        );
        assert_eq!(of("server-b"), ["running", "pending", "pending", "pending"]);
        cleanup(path);
    }

    #[tokio::test]
    async fn lifecycle_runs_share_the_ten_entry_retention_limit() {
        let (state, path) = test_state().await;
        let triggers = [
            "manual_deploy",
            "manual_start",
            "manual_stop",
            "manual_remove",
        ];
        for index in 0..11 {
            let run_id = start(
                &state,
                "application-1",
                "compose",
                "compose",
                triggers[index % triggers.len()],
                None,
                Some("user-1"),
            )
            .await
            .unwrap();
            finish(&state, &run_id, "succeeded", None).await.unwrap();
        }

        let runs = list(&state, "application-1").await.unwrap();
        assert_eq!(runs.len(), 10);
        let conn = state.db.clone();
        let count: i64 = sqlx::query(
            "SELECT COUNT(*) FROM application_deployment_runs WHERE application_id = 'application-1'",
        )
        .fetch_one(&conn)
        .await
        .unwrap()
        .try_get(0)
        .unwrap();
        assert_eq!(count, 10);

        cleanup(path);
    }
}
