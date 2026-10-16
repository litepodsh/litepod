use chrono::Utc;
use futures_util::FutureExt;
use serde::Serialize;
use sqlx::Row;
use std::panic::AssertUnwindSafe;
use std::time::Duration;
use tokio::{
    sync::{Mutex, Notify, broadcast},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    dtos::{deployment::DeploymentRunActorDto, response::AppError},
    git::repositories::db::get_conn,
    state::state::SharedState,
};

const MAX_LIST_LIMIT: usize = 100;

fn is_stream_expired_error(error: &AppError) -> bool {
    matches!(error, AppError::DatabaseError(message) if message.to_ascii_uppercase().contains("STREAM_EXPIRED"))
}

fn stream_expired_backoff(attempt: u32) -> Duration {
    let multiplier = 1_u64 << attempt.min(5);
    Duration::from_millis((250 * multiplier).min(5_000))
}

#[derive(Debug, Clone, Serialize)]
pub struct QueueEvent {
    pub job_id: String,
    pub kind: String,
    pub sequence: Option<i64>,
    pub level: Option<String>,
    pub payload: String,
    /// Replica that produced a `log` event; absent for run-level events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_id: Option<String>,
}

pub struct QueueRuntime {
    pub notify: Notify,
    pub events: broadcast::Sender<QueueEvent>,
    pub cancellations: Mutex<std::collections::HashMap<String, CancellationToken>>,
    pub coordinator: Mutex<Option<JoinHandle<()>>>,
    pub log_buffers: super::log_buffer::LogBufferRegistry,
    pub log_filters: Mutex<std::collections::HashMap<String, super::log_filter::LogFilter>>,
}

impl Default for QueueRuntime {
    fn default() -> Self {
        // A noisy build (e.g. `bun install`) emits log events far faster than a
        // browser drains them; 256 slots overflow routinely and every overflow
        // costs the viewer log lines.
        let (events, _) = broadcast::channel(1024);
        Self {
            notify: Notify::new(),
            events,
            cancellations: Mutex::new(std::collections::HashMap::new()),
            coordinator: Mutex::new(None),
            log_buffers: Default::default(),
            log_filters: Default::default(),
        }
    }
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown panic".to_string()
    }
}

#[derive(Debug, Clone)]
pub struct EnqueueJobInput {
    pub application_id: String,
    pub project_id: String,
    pub environment_id: String,
    pub organization_id: String,
    pub source_type: String,
    pub source_snapshot: String,
    pub build_method: String,
    pub build_snapshot: String,
    pub run_id: String,
}

/// How many replicas of one group deploy run at the same time. Higher fans out
/// faster but multiplies SSH sessions and remote builds in flight.
const REPLICA_CONCURRENCY: usize = 3;

tokio::task_local! {
    static PINNED_SERVER: String;
}

/// Runs `future` so any job it enqueues deploys only `server_id`, the way a
/// "retry this server" action re-runs one replica without touching the others.
pub async fn pinned_to<T>(server_id: &str, future: impl std::future::Future<Output = T>) -> T {
    PINNED_SERVER.scope(server_id.to_string(), future).await
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // richer snapshot fields are filled in by future worker iterations.
pub struct DeploymentJobSnapshot {
    pub id: String,
    pub application_id: String,
    pub project_id: String,
    pub environment_id: String,
    pub organization_id: String,
    pub source_type: String,
    pub source_snapshot: String,
    pub build_method: String,
    pub build_snapshot: String,
    pub run_id: String,
    /// Retry of a single replica of a group-targeted application.
    pub target_server_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeploymentJobDto {
    pub id: String,
    pub application_id: String,
    pub application_name: String,
    pub project_id: String,
    pub project_name: String,
    pub environment_id: String,
    pub organization_id: String,
    pub source_type: String,
    pub build_method: String,
    pub status: String,
    pub queued_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub cancel_requested: bool,
    pub canceled_at: Option<i64>,
    pub builder_container_name: Option<String>,
    pub builder_container_id: Option<String>,
    pub run_id: String,
    pub sanitized_error: Option<String>,
    pub duration_ms: Option<i64>,
    pub commit_sha: Option<String>,
    pub commit_message: Option<String>,
    pub branch: Option<String>,
    pub commit_author_name: Option<String>,
    pub commit_author_email: Option<String>,
    pub triggered_by: Option<String>,
    pub actor: Option<DeploymentRunActorDto>,
}

fn row_to_job(row: &sqlx::sqlite::SqliteRow) -> Result<DeploymentJobDto, AppError> {
    use sqlx::Row;
    let started_at: Option<i64> = row
        .try_get(11)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let finished_at: Option<i64> = row
        .try_get(12)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let duration_ms = finished_at.zip(started_at).map(|(f, s)| f - s);
    Ok(DeploymentJobDto {
        id: row
            .try_get(0)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        application_id: row
            .try_get(1)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        application_name: row
            .try_get(2)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        project_id: row
            .try_get(3)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        project_name: row
            .try_get(4)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        environment_id: row
            .try_get(5)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        organization_id: row
            .try_get(6)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        source_type: row
            .try_get(7)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        build_method: row
            .try_get(8)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        status: row
            .try_get(9)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        queued_at: row
            .try_get(10)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        started_at,
        finished_at,
        cancel_requested: row
            .try_get::<i64, _>(13)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?
            != 0,
        canceled_at: row
            .try_get(14)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        builder_container_name: row
            .try_get(15)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        builder_container_id: row
            .try_get(16)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        run_id: row
            .try_get(17)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        sanitized_error: row
            .try_get(18)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        duration_ms,
        commit_sha: row
            .try_get(19)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        commit_message: row
            .try_get(20)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        branch: row
            .try_get(21)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        commit_author_name: row
            .try_get(22)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        commit_author_email: row
            .try_get(23)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        triggered_by: row
            .try_get(24)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        actor: match row
            .try_get::<Option<String>, _>(25)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?
        {
            Some(id) => Some(DeploymentRunActorDto {
                id,
                name: row
                    .try_get(26)
                    .map_err(|e| AppError::DatabaseError(e.to_string()))?,
                email: row
                    .try_get(27)
                    .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            }),
            None => None,
        },
    })
}

const JOB_COLUMNS: &str = "j.id, j.application_id, a.name, j.project_id, p.name, j.environment_id, j.organization_id, j.source_type, j.build_method, j.status, j.queued_at, j.started_at, j.finished_at, j.cancel_requested, j.canceled_at, j.builder_container_name, j.builder_container_id, j.run_id, j.sanitized_error, r.commit_sha, r.commit_message, r.branch, r.commit_author_name, r.commit_author_email, r.triggered_by, u.id, u.name, u.email";
const JOB_JOINS: &str = "LEFT JOIN applications a ON a.id = j.application_id LEFT JOIN projects p ON p.id = j.project_id LEFT JOIN application_deployment_runs r ON r.id = j.run_id LEFT JOIN users u ON u.id = r.created_by";

pub async fn enqueue(
    state: &SharedState,
    input: EnqueueJobInput,
) -> Result<DeploymentJobDto, AppError> {
    let id = Uuid::now_v7().to_string();
    // The Railpack builder name is deterministic from this deployment run, so it
    // can be shown in the dashboard as soon as the job is queued rather than
    // waiting for the worker to claim it. It must not be application-scoped:
    // independent applications may build concurrently.
    let builder_container_name = (input.build_method == "railpack").then(|| {
        crate::deployment::git::railpack::builder_name(&input.application_id, &input.run_id)
    });
    let run_id = input.run_id.clone();
    let target_server_id = PINNED_SERVER.try_with(|id| id.clone()).ok();
    let conn = get_conn(state).await;
    crate::git::repositories::db::retry_on_stale_snapshot(|| async {
        sqlx::query(
            "INSERT INTO deployment_jobs (id, application_id, project_id, environment_id, organization_id, source_type, source_snapshot, build_method, build_snapshot, status, queued_at, run_id, builder_container_name, target_server_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'queued', ?10, ?11, ?12, ?13)",
        )
        .bind(id.clone())
        .bind(input.application_id.clone())
        .bind(input.project_id.clone())
        .bind(input.environment_id.clone())
        .bind(input.organization_id.clone())
        .bind(input.source_type.clone())
        .bind(input.source_snapshot.clone())
        .bind(input.build_method.clone())
        .bind(input.build_snapshot.clone())
        .bind(now_ms())
        .bind(input.run_id.clone())
        .bind(builder_container_name.clone())
        .bind(target_server_id.clone())
        .execute(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))
    })
    .await?;
    // The run's buffer already exists (its first line was written by
    // `history::start`) and cached a missing job. Link it before the worker
    // starts, so every build line broadcasts.
    state
        .queue_runtime
        .log_buffers
        .attach_job(&run_id, &id)
        .await;
    state.queue_runtime.notify.notify_one();
    let _ = state.queue_runtime.events.send(QueueEvent {
        job_id: id.clone(),
        kind: "status".into(),
        sequence: None,
        level: None,
        payload: "queued".into(),
        server_id: None,
    });
    get(state, &id).await
}

pub async fn get(state: &SharedState, job_id: &str) -> Result<DeploymentJobDto, AppError> {
    let conn = get_conn(state).await;
    sqlx::query(&format!(
        "SELECT {JOB_COLUMNS} FROM deployment_jobs j {JOB_JOINS} WHERE j.id = ?1"
    ))
    .bind(job_id)
    .fetch_optional(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?
    .map(|row| row_to_job(&row))
    .transpose()?
    .ok_or(AppError::ResourceNotFound)
}

pub async fn list_for_organization(
    state: &SharedState,
    organization_id: &str,
    limit: usize,
) -> Result<Vec<DeploymentJobDto>, AppError> {
    let limit = limit.clamp(1, MAX_LIST_LIMIT) as i64;
    let conn = get_conn(state).await;
    let rows = sqlx::query(&format!(
        "SELECT {JOB_COLUMNS} FROM deployment_jobs j {JOB_JOINS} WHERE j.organization_id = ?1 ORDER BY j.queued_at DESC, j.id DESC LIMIT ?2"
    ))
    .bind(organization_id)
    .bind(limit)
    .fetch_all(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    rows.iter().map(row_to_job).collect()
}

/// A second authorization path, separate from `security::authz::extract`: queue routes are
/// keyed on `job_id` alone with no `{organization_id}` segment, so the extractor has nothing to
/// read and the organization, project and environment are recovered from the job row instead.
/// The decision itself is the same one the extractors make (`organization_role` with its
/// SysAdmin bypass, then the caller's permission set at the job's environment).
///
/// Every refusal — unknown job, not a member, missing permission — is `ResourceNotFound`, so
/// the answer never confirms that a job id exists in someone else's organization.
pub async fn ensure_user_access(
    state: &SharedState,
    user: &crate::security::middleware::auth::AuthUser,
    job_id: &str,
    permission: crate::security::authz::Permission,
) -> Result<(), AppError> {
    let conn = get_conn(state).await;
    let row = sqlx::query(
        "SELECT organization_id, project_id, environment_id FROM deployment_jobs WHERE id = ?1",
    )
    .bind(job_id)
    .fetch_optional(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?
    .ok_or(AppError::ResourceNotFound)?;
    let organization_id: String = row.get(0);
    let project_id: String = row.get(1);
    let environment_id: String = row.get(2);

    let role_id = crate::handler::common::organization_role(state, &organization_id, user)
        .await
        .map_err(|_| AppError::ResourceNotFound)?;
    let set = crate::security::authz::set::load(state, user, &organization_id, role_id).await?;
    let target = crate::security::authz::Target::Environment {
        project_id: &project_id,
        environment_id: &environment_id,
    };
    if set.allows(permission, target) {
        Ok(())
    } else {
        Err(AppError::ResourceNotFound)
    }
}

pub async fn claim_next(state: &SharedState) -> Result<Option<DeploymentJobSnapshot>, AppError> {
    crate::git::repositories::db::retry_on_stale_snapshot(|| claim_next_once(state)).await
}

async fn claim_next_once(state: &SharedState) -> Result<Option<DeploymentJobSnapshot>, AppError> {
    let conn = get_conn(state).await;
    let mut tx = conn
        .begin()
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let Some(row) = sqlx::query(
        "SELECT j.id, j.application_id, j.project_id, j.environment_id, j.organization_id, j.source_type, j.source_snapshot, j.build_method, j.build_snapshot, j.run_id, j.target_server_id FROM deployment_jobs j WHERE j.status = 'queued' AND NOT EXISTS (SELECT 1 FROM deployment_jobs earlier WHERE earlier.application_id = j.application_id AND earlier.status IN ('queued', 'building', 'deploying') AND (earlier.queued_at < j.queued_at OR (earlier.queued_at = j.queued_at AND earlier.id < j.id))) AND (SELECT COUNT(*) FROM deployment_jobs active WHERE active.status IN ('building', 'deploying')) < (SELECT max_concurrent_builds FROM build_configuration WHERE id = 1) ORDER BY j.queued_at, j.id LIMIT 1",
    )
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?
    else {
        tx.commit()
            .await
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        return Ok(None);
    };
    let snapshot = DeploymentJobSnapshot {
        id: row
            .try_get(0)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        application_id: row
            .try_get(1)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        project_id: row
            .try_get(2)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        environment_id: row
            .try_get(3)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        organization_id: row
            .try_get(4)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        source_type: row
            .try_get(5)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        source_snapshot: row
            .try_get(6)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        build_method: row
            .try_get(7)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        build_snapshot: row
            .try_get(8)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        run_id: row
            .try_get(9)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        target_server_id: row
            .try_get(10)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
    };
    let changed = sqlx::query("UPDATE deployment_jobs SET status = 'building', started_at = ?1, claimed_at = ?1 WHERE id = ?2 AND status = 'queued'")
        .bind(now_ms())
        .bind(snapshot.id.clone())
        .execute(&mut *tx)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?
        .rows_affected();
    tx.commit()
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    if changed == 1 {
        let token = CancellationToken::new();
        state
            .queue_runtime
            .cancellations
            .lock()
            .await
            .insert(snapshot.id.clone(), token);
        let _ = state.queue_runtime.events.send(QueueEvent {
            job_id: snapshot.id.clone(),
            kind: "status".into(),
            sequence: None,
            level: None,
            payload: "building".into(),
            server_id: None,
        });
        Ok(Some(snapshot))
    } else {
        Ok(None)
    }
}

pub async fn request_cancel(
    state: &SharedState,
    job_id: &str,
) -> Result<DeploymentJobDto, AppError> {
    let conn = get_conn(state).await;
    let timestamp = now_ms();
    sqlx::query("UPDATE deployment_jobs SET status = CASE WHEN status = 'queued' THEN 'canceled' ELSE status END, cancel_requested = 1, canceled_at = CASE WHEN status = 'queued' THEN ?1 ELSE canceled_at END, finished_at = CASE WHEN status = 'queued' THEN ?1 ELSE finished_at END WHERE id = ?2 AND status IN ('queued', 'building', 'deploying')")
        .bind(timestamp)
        .bind(job_id)
        .execute(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    if let Some(token) = state
        .queue_runtime
        .cancellations
        .lock()
        .await
        .get(job_id)
        .cloned()
    {
        token.cancel();
    }
    state.queue_runtime.notify.notify_one();
    let _ = state.queue_runtime.events.send(QueueEvent {
        job_id: job_id.to_owned(),
        kind: "status".into(),
        sequence: None,
        level: None,
        payload: "cancel_requested".into(),
        server_id: None,
    });
    let job = get(state, job_id).await?;
    if job.status == "canceled" {
        let _ = state.queue_runtime.events.send(QueueEvent {
            job_id: job_id.to_owned(),
            kind: "terminal".into(),
            sequence: None,
            level: None,
            payload: "canceled".into(),
            server_id: None,
        });
    }
    Ok(job)
}

pub async fn recover_interrupted(state: &SharedState) -> Result<(), AppError> {
    let conn = get_conn(state).await;
    sqlx::query("UPDATE deployment_jobs SET status = 'failed', finished_at = ?1, sanitized_error = 'Deployment was interrupted because the server restarted' WHERE status IN ('building', 'deploying')")
        .bind(now_ms())
        .execute(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok(())
}

/// Runs one pipeline (one replica, for a group) with its steps planned and the
/// run in scope for `history::step`.
async fn run_source(
    state: &SharedState,
    source_type: &str,
    build_method: &str,
    application_id: &str,
    run_id: &str,
) -> Result<(), String> {
    let steps = crate::deployment::history::planned_steps(source_type, build_method);
    let _ = crate::deployment::history::plan_steps(state, run_id, steps).await;
    crate::deployment::history::for_run(
        run_id,
        run_source_pipeline(state, source_type, application_id, run_id),
    )
    .await
}

async fn run_source_pipeline(
    state: &SharedState,
    source_type: &str,
    application_id: &str,
    run_id: &str,
) -> Result<(), String> {
    match source_type {
        "git" => {
            crate::deployment::git::run(
                state.clone(),
                application_id.to_string(),
                run_id.to_string(),
            )
            .await
        }
        "compose" => {
            crate::deployment::compose_standalone::run(
                state.clone(),
                application_id.to_string(),
                run_id.to_string(),
            )
            .await
        }
        "static" => crate::deployment::static_deploy::run(state, application_id, run_id).await,
        "image" => crate::deployment::image::run(state, application_id, run_id).await,
        other => Err(format!(
            "Source type `{other}` is not executed by the queue worker"
        )),
    }
}

async fn execute_claimed(state: SharedState, job: DeploymentJobSnapshot) {
    let conn = get_conn(&state).await;
    let deploying = sqlx::query(
        "UPDATE deployment_jobs SET status = 'deploying' WHERE id = ?1 AND status = 'building'",
    )
    .bind(job.id.clone())
    .execute(&conn)
    .await
    .map(|result| result.rows_affected());
    if matches!(deploying, Ok(1)) {
        let _ = state.queue_runtime.events.send(QueueEvent {
            job_id: job.id.clone(),
            kind: "status".into(),
            sequence: None,
            level: None,
            payload: "deploying".into(),
            server_id: None,
        });
    }
    let cancellation = state
        .queue_runtime
        .cancellations
        .lock()
        .await
        .get(&job.id)
        .cloned()
        .unwrap_or_else(CancellationToken::new);
    // Boxed so every branch can be raced against cancellation and wrapped in
    // catch_unwind through one shared path below, instead of tripling the
    // panic-handling code across the git/compose/static branches.
    let pipeline_state = state.clone();
    let application_id = job.application_id.clone();
    let run_id = job.run_id.clone();
    let source_type = job.source_type.clone();
    let build_method = job.build_method.clone();
    let target_server_id = job.target_server_id.clone();
    let pipeline: std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send>> =
        Box::pin(async move {
            let target_servers =
                crate::servers::groups::resolve_target_servers(&pipeline_state, &application_id)
                    .await
                    .map_err(|error| error.to_string())?;
            let is_group =
                crate::servers::replica::target_server_id(&pipeline_state, &application_id)
                    .await
                    .map_err(|error| error.to_string())?
                    .is_none();
            // A single-server app never fans out. A group (even of one member)
            // always does, since its runtime only resolves inside a replica scope.
            if !is_group {
                return run_source(
                    &pipeline_state,
                    &source_type,
                    &build_method,
                    &application_id,
                    &run_id,
                )
                .await;
            }
            if target_servers.is_empty() {
                let error = "This server group has no members to deploy to".to_string();
                let _ = crate::deployment::history::finish(
                    &pipeline_state,
                    &run_id,
                    "failed",
                    Some(&error),
                )
                .await;
                return Err(error);
            }

            let ids: Vec<String> = target_servers
                .iter()
                .map(|server| server.id.clone())
                .collect();
            crate::deployment::replicas::reconcile(&pipeline_state, &application_id, &ids)
                .await
                .map_err(|error| error.to_string())?;
            // A retry pins one member; every other replica keeps what it runs.
            let selected: Vec<_> = match &target_server_id {
                Some(pinned) => {
                    let selected: Vec<_> = target_servers
                        .iter()
                        .filter(|server| &server.id == pinned)
                        .collect();
                    if selected.is_empty() {
                        let error =
                            "The server to retry is no longer part of this group".to_string();
                        let _ = crate::deployment::history::finish(
                            &pipeline_state,
                            &run_id,
                            "failed",
                            Some(&error),
                        )
                        .await;
                        return Err(error);
                    }
                    selected
                }
                None => target_servers.iter().collect(),
            };
            for server in &selected {
                let _ = crate::deployment::history::set_replica(
                    &pipeline_state,
                    &run_id,
                    &server.id,
                    &server.name,
                    "queued",
                    None,
                )
                .await;
            }
            let _ = crate::deployment::history::append(
                &pipeline_state,
                &run_id,
                "info",
                &format!(
                    "Deploying to {} server(s), up to {REPLICA_CONCURRENCY} at a time",
                    selected.len()
                ),
            )
            .await;

            // Each replica runs the ordinary single-server pipeline under its own
            // task-local target, at most REPLICA_CONCURRENCY at once.
            let permits = std::sync::Arc::new(tokio::sync::Semaphore::new(REPLICA_CONCURRENCY));
            let replica_runs = selected.iter().map(|server| {
                let permits = permits.clone();
                let state = &pipeline_state;
                let application_id = &application_id;
                let run_id = &run_id;
                let source_type = &source_type;
                let build_method = &build_method;
                async move {
                    let _permit = permits.acquire().await.ok();
                    let _ = crate::deployment::history::set_replica(
                        state,
                        run_id,
                        &server.id,
                        &server.name,
                        "deploying",
                        None,
                    )
                    .await;
                    let _ = crate::deployment::replicas::set_status(
                        state,
                        application_id,
                        &server.id,
                        "deploying",
                        None,
                    )
                    .await;
                    let result = crate::servers::replica::for_replica(
                        &server.id,
                        run_source(state, source_type, build_method, application_id, run_id),
                    )
                    .await;
                    let (run_status, replica_status, error) = match &result {
                        Ok(()) => ("succeeded", "running", None),
                        Err(error) => ("failed", "failed", Some(error.as_str())),
                    };
                    let _ = crate::deployment::history::set_replica(
                        state,
                        run_id,
                        &server.id,
                        &server.name,
                        run_status,
                        error,
                    )
                    .await;
                    let _ = crate::deployment::replicas::set_status(
                        state,
                        application_id,
                        &server.id,
                        replica_status,
                        error,
                    )
                    .await;
                    result.map_err(|error| format!("{}: {error}", server.name))
                }
            });
            let results = futures_util::future::join_all(replica_runs).await;
            let failures: Vec<String> = results.into_iter().filter_map(Result::err).collect();
            let outcome = if failures.is_empty() {
                Ok(())
            } else {
                Err(format!(
                    "{} of {} server(s) failed — {}",
                    failures.len(),
                    selected.len(),
                    failures.join("; ")
                ))
            };
            // The per-replica pipelines only finished their own replica (see
            // `history::finish`); close the run itself now.
            let _ = crate::deployment::history::finish(
                &pipeline_state,
                &run_id,
                if outcome.is_ok() {
                    "succeeded"
                } else {
                    "failed"
                },
                outcome.as_ref().err().map(String::as_str),
            )
            .await;
            outcome
        });
    // A pipeline that panics (instead of returning Err) must not be allowed to
    // unwind out of this spawned task: nothing else observes that, so the job
    // would stay `building`/`deploying` forever and, since jobs are serialized
    // per application, block every future deploy for it.
    let mut panicked = false;
    let result = tokio::select! {
        outcome = AssertUnwindSafe(pipeline).catch_unwind() => match outcome {
            Ok(result) => result,
            Err(payload) => {
                panicked = true;
                let message = panic_message(&payload);
                tracing::error!(job_id = %job.id, panic = %message, "deployment pipeline panicked");
                Err(format!("Deployment failed: internal error ({message})"))
            }
        },
        _ = cancellation.cancelled() => Err("Deployment canceled by user".to_string()),
    };
    let (status, error) = match result {
        Ok(()) => ("succeeded", None),
        Err(error) if cancellation.is_cancelled() => ("canceled", Some(error)),
        Err(error) => ("failed", Some(error)),
    };
    if job.build_method == "railpack" {
        crate::deployment::git::railpack::cleanup_run(&state, &job.application_id, &job.run_id)
            .await;
    }
    if status == "canceled" || panicked {
        let _ =
            crate::deployment::history::finish(&state, &job.run_id, status, error.as_deref()).await;
        // Replicas that were still waiting or deploying never reached their own finish.
        let _ = sqlx::query(
            "UPDATE application_deployment_run_replicas SET status = 'canceled', finished_at = ?1 WHERE run_id = ?2 AND status IN ('queued', 'deploying')",
        )
        .bind(now_ms())
        .bind(&job.run_id)
        .execute(&get_conn(&state).await)
        .await;
    }
    let conn = get_conn(&state).await;
    let _ = sqlx::query("UPDATE deployment_jobs SET status = ?1, finished_at = ?2, sanitized_error = ?3 WHERE id = ?4")
        .bind(status)
        .bind(now_ms())
        .bind(error)
        .bind(job.id.clone())
        .execute(&conn)
        .await;
    let job_id = job.id.clone();
    let _ = state.queue_runtime.events.send(QueueEvent {
        job_id: job_id.clone(),
        kind: "terminal".into(),
        sequence: None,
        level: None,
        payload: status.into(),
        server_id: None,
    });
    state
        .queue_runtime
        .cancellations
        .lock()
        .await
        .remove(&job_id);
}

// ponytail: hardcoded, matches the hardcoded 30-min build timeouts used
// throughout the deploy pipelines (compose, remote/local git, remote/local
// railpack); promote to a build_configuration column if operators need
// longer builds than that timeout allows.
const STUCK_JOB_TIMEOUT_MS: i64 = 40 * 60 * 1000;

/// Backstop for `execute_claimed`: normally every terminal outcome (success,
/// error, cancellation, panic) writes a final status itself. This exists for
/// whatever that still misses — e.g. the whole worker process getting killed
/// hard enough to skip even the panic handler. Without it, a job stuck in
/// `building`/`deploying` blocks every future deploy for that application
/// (jobs are serialized per application) until the API process restarts.
async fn reconcile_stuck(state: &SharedState) -> Result<(), AppError> {
    let conn = get_conn(state).await;
    let cutoff = now_ms() - STUCK_JOB_TIMEOUT_MS;
    let rows = sqlx::query(
        "SELECT id, run_id, application_id, build_method FROM deployment_jobs \
         WHERE status IN ('building', 'deploying') AND started_at IS NOT NULL AND started_at < ?1",
    )
    .bind(cutoff)
    .fetch_all(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    for row in rows {
        let id: String = row
            .try_get(0)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        let run_id: String = row
            .try_get(1)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        let application_id: String = row
            .try_get(2)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        let build_method: String = row
            .try_get(3)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        const MESSAGE: &str = "Deployment watchdog: job exceeded the maximum allowed build/deploy time and was marked failed";
        // Compare-and-swap, same idea as `claim_next_once`: only act if this
        // job is still stuck by the time we get here, so we never clobber a
        // status the real worker resolves in the same instant.
        let changed = sqlx::query(
            "UPDATE deployment_jobs SET status = 'failed', finished_at = ?1, sanitized_error = ?2 \
             WHERE id = ?3 AND status IN ('building', 'deploying')",
        )
        .bind(now_ms())
        .bind(MESSAGE)
        .bind(&id)
        .execute(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?
        .rows_affected();
        if changed != 1 {
            continue;
        }
        tracing::warn!(job_id = %id, %application_id, "deployment watchdog force-failed a stuck job");
        let _ = crate::deployment::history::finish(state, &run_id, "failed", Some(MESSAGE)).await;
        if build_method == "railpack" {
            crate::deployment::git::railpack::cleanup_run(state, &application_id, &run_id).await;
        }
        state.queue_runtime.cancellations.lock().await.remove(&id);
        let _ = state.queue_runtime.events.send(QueueEvent {
            job_id: id,
            kind: "terminal".into(),
            sequence: None,
            level: None,
            payload: "failed".into(),
            server_id: None,
        });
    }
    Ok(())
}

pub async fn start_coordinator(state: SharedState) {
    let mut coordinator = state.queue_runtime.coordinator.lock().await;
    if coordinator.is_some() {
        return;
    }
    let reconciler_state = state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        loop {
            interval.tick().await;
            if let Err(error) = reconcile_stuck(&reconciler_state).await {
                tracing::warn!(%error, "deployment queue watchdog tick failed");
            }
        }
    });
    let worker_state = state.clone();
    *coordinator = Some(tokio::spawn(async move {
        let mut stream_expired_attempts = 0_u32;
        loop {
            match claim_next(&worker_state).await {
                Ok(Some(job)) => {
                    stream_expired_attempts = 0;
                    tokio::spawn(execute_claimed(worker_state.clone(), job));
                }
                Ok(None) => {
                    stream_expired_attempts = 0;
                    tokio::select! {
                        _ = worker_state.queue_runtime.notify.notified() => {},
                        _ = tokio::time::sleep(std::time::Duration::from_millis(500)) => {},
                    }
                }
                Err(error) => {
                    if is_stream_expired_error(&error) {
                        let delay = stream_expired_backoff(stream_expired_attempts);
                        stream_expired_attempts = stream_expired_attempts.saturating_add(1);
                        tracing::warn!(error = %error, attempt = stream_expired_attempts, delay_ms = delay.as_millis(), "deployment queue database stream expired; retrying");
                        tokio::time::sleep(delay).await;
                    } else {
                        tracing::error!(error = %error, "deployment queue claim failed");
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                }
            }
        }
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{deployment::history, migrations, state::state::AppState};
    use std::{
        collections::{HashMap, HashSet},
        sync::{Arc, Mutex},
    };

    #[test]
    fn stream_expiration_is_transient_with_bounded_backoff() {
        assert!(is_stream_expired_error(&AppError::DatabaseError(
            "Hrana: STREAM_EXPIRED".into()
        )));
        assert!(!is_stream_expired_error(&AppError::DatabaseError(
            "constraint failed".into()
        )));
        assert_eq!(stream_expired_backoff(0), Duration::from_millis(250));
        assert_eq!(stream_expired_backoff(20), Duration::from_secs(5));
    }

    async fn test_state() -> (SharedState, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!("litepod-queue-{}.db", Uuid::now_v7()));
        let db = crate::testing::sqlite_pool_at(&path).await;
        migrations::run(&db).await;
        let state: SharedState = Arc::new(AppState {
            started_at: chrono::Local::now(),
            db,
            caddy_url: String::new(),
            caddy_admin_url: String::new(),
            caddyfile_path: Default::default(),
            caddy_server_upstream: String::new(),
            podman_socket: String::new(),
            pending_execs: Mutex::new(HashMap::new()),
            data_dir: Default::default(),
            data_volume: None,
            host_data_dir: Default::default(),
            podman_network: String::new(),
            public_url: String::new(),
            oauth_states: Mutex::new(HashMap::new()),
            public_ip: String::new(),
            is_cloud: false,
            polar: None,
            dev_mode: false,
            port_range: (0, 0),
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
        sqlx::query(
            "INSERT INTO users (id, email, hashed_password, name) VALUES ('user-1', 'user@example.com', 'hash', 'User')",
        )
        .execute(&conn)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO organizations (id, name, slug, created_by) VALUES ('organization-1', 'Organization', 'organization', 'user-1')",
        )
        .execute(&conn)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO projects (id, organization_id, name, created_at, updated_at) VALUES ('project-1', 'organization-1', 'Project', 1, 1)",
        )
        .execute(&conn)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO environments (id, project_id, name, is_production, created_at, updated_at) VALUES ('environment-1', 'project-1', 'production', 1, 1, 1)",
        )
        .execute(&conn)
        .await
        .unwrap();
        for application_id in ["application-1", "application-2"] {
            sqlx::query(
                "INSERT INTO applications (id, environment_id, name, server_id, created_at, updated_at) VALUES (?1, 'environment-1', ?1, 'local', 1, 1)",
            )
            .bind(application_id)
            .execute(&conn)
            .await
            .unwrap();
        }
        (state, path)
    }

    async fn queue_job(state: &SharedState, application_id: &str) -> DeploymentJobDto {
        let run_id = history::start(
            state,
            application_id,
            "git",
            "railpack",
            "manual",
            Some("main"),
            None,
        )
        .await
        .unwrap();
        enqueue(
            state,
            EnqueueJobInput {
                application_id: application_id.into(),
                project_id: "project-1".into(),
                environment_id: "environment-1".into(),
                organization_id: "organization-1".into(),
                source_type: "git".into(),
                source_snapshot: "{}".into(),
                build_method: "railpack".into(),
                build_snapshot: "{}".into(),
                run_id,
            },
        )
        .await
        .unwrap()
    }

    fn cleanup(path: std::path::PathBuf) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{}", path.display(), suffix));
        }
    }

    #[tokio::test]
    async fn same_application_jobs_are_claimed_in_fifo_order() {
        let (state, path) = test_state().await;
        let first = queue_job(&state, "application-1").await;
        let second = queue_job(&state, "application-1").await;
        let third = queue_job(&state, "application-1").await;

        let claimed = claim_next(&state).await.unwrap().unwrap();
        assert_eq!(claimed.id, first.id);
        assert!(claim_next(&state).await.unwrap().is_none());

        let conn = state.db.clone();
        sqlx::query("UPDATE deployment_jobs SET status = 'succeeded' WHERE id = ?1")
            .bind(claimed.id)
            .execute(&conn)
            .await
            .unwrap();
        let claimed = claim_next(&state).await.unwrap().unwrap();
        assert_eq!(claimed.id, second.id);
        sqlx::query("UPDATE deployment_jobs SET status = 'succeeded' WHERE id = ?1")
            .bind(claimed.id)
            .execute(&conn)
            .await
            .unwrap();
        assert_eq!(claim_next(&state).await.unwrap().unwrap().id, third.id);
        cleanup(path);
    }

    #[tokio::test]
    async fn queue_job_includes_commit_author_and_actor() {
        let (state, path) = test_state().await;
        let run_id = history::start(
            &state,
            "application-1",
            "git",
            "railpack",
            "manual_deploy",
            Some("main"),
            Some("user-1"),
        )
        .await
        .unwrap();
        history::set_commit(
            &state,
            &run_id,
            "a1b2c3d",
            "Ship identity",
            Some("Ada Lovelace"),
            Some("ada@example.com"),
        )
        .await
        .unwrap();
        enqueue(
            &state,
            EnqueueJobInput {
                application_id: "application-1".into(),
                project_id: "project-1".into(),
                environment_id: "environment-1".into(),
                organization_id: "organization-1".into(),
                source_type: "git".into(),
                source_snapshot: "{}".into(),
                build_method: "railpack".into(),
                build_snapshot: "{}".into(),
                run_id,
            },
        )
        .await
        .unwrap();

        let jobs = list_for_organization(&state, "organization-1", 50)
            .await
            .unwrap();
        let job = jobs.first().unwrap();
        assert_eq!(job.commit_author_name.as_deref(), Some("Ada Lovelace"));
        assert_eq!(job.commit_author_email.as_deref(), Some("ada@example.com"));
        assert_eq!(job.triggered_by.as_deref(), Some("manual_deploy"));
        assert_eq!(
            job.actor.as_ref().map(|actor| actor.id.as_str()),
            Some("user-1")
        );

        cleanup(path);
    }

    #[tokio::test]
    async fn distinct_applications_can_claim_global_capacity_together() {
        let (state, path) = test_state().await;
        let first = queue_job(&state, "application-1").await;
        let second = queue_job(&state, "application-2").await;
        let conn = state.db.clone();
        sqlx::query("UPDATE build_configuration SET max_concurrent_builds = 2 WHERE id = 1")
            .execute(&conn)
            .await
            .unwrap();

        let claimed_one = claim_next(&state).await.unwrap().unwrap();
        let claimed_two = claim_next(&state).await.unwrap().unwrap();
        assert_eq!(
            std::collections::HashSet::from([claimed_one.id, claimed_two.id]),
            std::collections::HashSet::from([first.id, second.id])
        );
        cleanup(path);
    }

    #[tokio::test]
    async fn queue_access_follows_scoped_permissions_and_hides_refusals() {
        use crate::security::authz::Permission;
        use crate::testing::{
            assign, auth_user, cleanup_database, seed_application, set_base_role, sysadmin_user,
            test_state,
        };
        let (state, path) = test_state().await;
        let (project, environment, application) = seed_application(&state, "organization-a").await;
        sqlx::query(
            "INSERT INTO application_deployment_runs (id, application_id, source_type, build_method, status, started_at) \
             VALUES ('run-1', ?1, 'git', 'docker', 'building', 1)",
        )
        .bind(&application)
        .execute(&state.db)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO deployment_jobs (id, application_id, project_id, environment_id, organization_id, source_type, source_snapshot, build_method, build_snapshot, status, queued_at, run_id) \
             VALUES ('job-1', ?1, ?2, ?3, 'organization-a', 'git', '{}', 'docker', '{}', 'queued', 1, 'run-1')",
        )
        .bind(&application)
        .bind(&project)
        .bind(&environment)
        .execute(&state.db)
        .await
        .unwrap();

        // Base role Member, Viewer on the environment: can watch, cannot cancel.
        set_base_role(&state, "organization-a", "viewer", 4).await;
        let viewer = auth_user("viewer");
        assert!(matches!(
            ensure_user_access(&state, &viewer, "job-1", Permission::AppRead).await,
            Err(AppError::ResourceNotFound)
        ));
        assign(
            &state,
            "organization-a",
            "viewer",
            3,
            "environment",
            &environment,
        )
        .await;
        assert!(
            ensure_user_access(&state, &viewer, "job-1", Permission::AppRead)
                .await
                .is_ok()
        );
        assert!(matches!(
            ensure_user_access(&state, &viewer, "job-1", Permission::AppDeploy).await,
            Err(AppError::ResourceNotFound)
        ));

        // Non-member and unknown job look the same.
        assert!(matches!(
            ensure_user_access(&state, &auth_user("outsider"), "job-1", Permission::AppRead).await,
            Err(AppError::ResourceNotFound)
        ));
        assert!(matches!(
            ensure_user_access(&state, &viewer, "no-such-job", Permission::AppRead).await,
            Err(AppError::ResourceNotFound)
        ));

        // SysAdmin bypass now matches the extractors.
        assert!(
            ensure_user_access(
                &state,
                &sysadmin_user("outsider"),
                "job-1",
                Permission::AppDeploy
            )
            .await
            .is_ok()
        );
        cleanup_database(&path);
    }
}
