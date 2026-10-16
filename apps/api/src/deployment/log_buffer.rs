use std::{
    collections::HashMap,
    sync::atomic::{AtomicI64, Ordering},
};

use sqlx::Row;
use tokio::{
    sync::{Mutex, mpsc},
    task::JoinHandle,
    time::{Duration, interval},
};
use uuid::Uuid;

use crate::{dtos::response::AppError, state::state::SharedState};

const CHANNEL_CAPACITY: usize = 500;
const FLUSH_INTERVAL: Duration = Duration::from_millis(200);
const FLUSH_BATCH_SIZE: usize = 200;
const MAX_STORED_LOG_LINES: i64 = 2_000;

struct LogLine {
    run_id: String,
    sequence: i64,
    level: String,
    message: String,
    created_at: i64,
    server_id: Option<String>,
}

struct LogBufferHandle {
    sender: mpsc::Sender<LogLine>,
    flusher: JoinHandle<()>,
    job_id: Option<String>,
    next_sequence: AtomicI64,
    /// Lines persisted so far per replica (`None` for single-server runs), so
    /// one chatty server cannot use up the storage cap of the others.
    stored: HashMap<Option<String>, i64>,
}

/// Buffers per-run deployment log lines in memory and flushes them to the
/// DB in batches instead of one round-trip per line. A build step that
/// emits thousands of lines in seconds (e.g. `bun install`) would otherwise
/// serialize thousands of individual writes against the shared remote DB
/// connection, starving unrelated requests.
pub struct LogBufferRegistry {
    buffers: Mutex<HashMap<String, LogBufferHandle>>,
    max_stored_lines: i64,
}

impl Default for LogBufferRegistry {
    fn default() -> Self {
        Self {
            buffers: Mutex::new(HashMap::new()),
            max_stored_lines: MAX_STORED_LOG_LINES,
        }
    }
}

impl LogBufferRegistry {
    #[cfg(test)]
    fn with_max_stored_lines(max_stored_lines: i64) -> Self {
        Self {
            buffers: Mutex::new(HashMap::new()),
            max_stored_lines,
        }
    }

    /// Enqueues a log line for `run_id`, lazily creating the run's buffer
    /// and flusher task on first use. Returns the assigned sequence number
    /// and the run's job_id (if any), for SSE event routing.
    pub async fn enqueue(
        &self,
        state: &SharedState,
        run_id: &str,
        level: &str,
        message: &str,
        created_at: i64,
        server_id: Option<&str>,
    ) -> Result<Option<(i64, Option<String>)>, AppError> {
        let mut buffers = self.buffers.lock().await;
        if !buffers.contains_key(run_id) {
            let (job_id, start_sequence) = resolve_run_start(state, run_id).await?;
            let (sender, receiver) = mpsc::channel(CHANNEL_CAPACITY);
            let flusher = tokio::spawn(flush_loop(state.clone(), receiver));
            buffers.insert(
                run_id.to_string(),
                LogBufferHandle {
                    sender,
                    flusher,
                    job_id,
                    next_sequence: AtomicI64::new(start_sequence),
                    stored: HashMap::from([(None, start_sequence)]),
                },
            );
        }
        let handle = buffers
            .get_mut(run_id)
            .expect("just inserted or already present");
        let sequence = handle.next_sequence.fetch_add(1, Ordering::SeqCst);
        let stored = handle
            .stored
            .entry(server_id.map(str::to_string))
            .or_insert(0);
        let count = *stored;
        *stored += 1;
        if count >= self.max_stored_lines {
            if count == self.max_stored_lines {
                tracing::warn!(
                    run_id,
                    server_id,
                    max_lines = self.max_stored_lines,
                    "deployment log storage limit reached; live output continues but is no longer stored"
                );
            }
            // Live output outlives persistence: the viewer keeps seeing the
            // build, it just cannot be recovered by a refresh.
            return Ok(Some((sequence, handle.job_id.clone())));
        }
        let line = LogLine {
            run_id: run_id.to_string(),
            sequence,
            level: level.to_string(),
            message: message.to_string(),
            created_at,
            server_id: server_id.map(str::to_string),
        };
        if handle.sender.try_send(line).is_err() {
            tracing::warn!(
                run_id,
                "deployment log buffer full or closed, dropping line"
            );
        }
        Ok(Some((sequence, handle.job_id.clone())))
    }

    /// Links a job to an already-open buffer. `enqueue` resolves the job
    /// lazily when it first creates the buffer, but a run's first log line is
    /// written before its `deployment_jobs` row exists, so that lookup finds
    /// nothing and would otherwise suppress SSE events for the whole run.
    pub async fn attach_job(&self, run_id: &str, job_id: &str) {
        if let Some(handle) = self.buffers.lock().await.get_mut(run_id) {
            handle.job_id = Some(job_id.to_string());
        }
    }

    /// Closes the run's channel and waits for the flusher to drain its
    /// remaining lines before returning.
    pub async fn close(&self, run_id: &str) {
        let handle = self.buffers.lock().await.remove(run_id);
        if let Some(handle) = handle {
            drop(handle.sender);
            let _ = handle.flusher.await;
        }
    }
}

async fn resolve_run_start(
    state: &SharedState,
    run_id: &str,
) -> Result<(Option<String>, i64), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let seq_row = sqlx::query(
        "SELECT COALESCE(MAX(sequence), -1) + 1 FROM application_deployment_run_logs WHERE run_id = ?1",
    )
    .bind(run_id)
    .fetch_one(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let start_sequence: i64 = seq_row
        .try_get(0)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let job_row = sqlx::query("SELECT id FROM deployment_jobs WHERE run_id = ?1")
        .bind(run_id)
        .fetch_optional(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let job_id: Option<String> = match job_row {
        Some(row) => Some(
            row.try_get(0)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        ),
        None => None,
    };
    Ok((job_id, start_sequence))
}

async fn flush_loop(state: SharedState, mut receiver: mpsc::Receiver<LogLine>) {
    let mut ticker = interval(FLUSH_INTERVAL);
    let mut pending: Vec<LogLine> = Vec::with_capacity(FLUSH_BATCH_SIZE);
    loop {
        tokio::select! {
            line = receiver.recv() => {
                match line {
                    Some(line) => {
                        pending.push(line);
                        if pending.len() >= FLUSH_BATCH_SIZE {
                            flush(&state, &mut pending).await;
                        }
                    }
                    None => {
                        flush(&state, &mut pending).await;
                        break;
                    }
                }
            }
            _ = ticker.tick() => {
                if !pending.is_empty() {
                    flush(&state, &mut pending).await;
                }
            }
        }
    }
}

async fn flush(state: &SharedState, pending: &mut Vec<LogLine>) {
    if pending.is_empty() {
        return;
    }
    let placeholders: Vec<String> = (0..pending.len())
        .map(|i| {
            let base = i * 7;
            format!(
                "(?{},?{},?{},?{},?{},?{},?{})",
                base + 1,
                base + 2,
                base + 3,
                base + 4,
                base + 5,
                base + 6,
                base + 7
            )
        })
        .collect();
    let sql = format!(
        "INSERT INTO application_deployment_run_logs (id, run_id, sequence, level, message, created_at, server_id) VALUES {}",
        placeholders.join(",")
    );
    let mut query = sqlx::query(&sql);
    for line in pending.iter() {
        query = query
            .bind(Uuid::now_v7().to_string())
            .bind(line.run_id.clone())
            .bind(line.sequence)
            .bind(line.level.clone())
            .bind(line.message.clone())
            .bind(line.created_at)
            .bind(line.server_id.clone());
    }
    let conn = crate::git::repositories::db::get_conn(state).await;
    if let Err(error) = query.execute(&conn).await {
        tracing::error!(%error, count = pending.len(), "failed to flush deployment log batch");
    }
    pending.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_state() -> (SharedState, std::path::PathBuf) {
        let (state, path) = crate::testing::test_state_for(false).await;
        let conn = crate::git::repositories::db::get_conn(&state).await;
        sqlx::query("INSERT INTO projects (id, organization_id, name, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)")
            .bind("project-1")
            .bind("organization-a")
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
        sqlx::query("INSERT INTO application_deployment_runs (id, application_id, source_type, build_method, status, started_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)")
            .bind("run-1")
            .bind("application-1")
            .bind("git")
            .bind("docker")
            .bind("deploying")
            .bind(1_i64)
            .execute(&conn)
            .await
            .unwrap();
        (state, path)
    }

    fn cleanup(path: std::path::PathBuf) {
        crate::testing::cleanup_database(&path);
    }

    #[tokio::test]
    async fn enqueue_assigns_contiguous_sequence_and_flushes_on_close() {
        let (state, path) = test_state().await;
        let registry = LogBufferRegistry::default();

        for i in 0..5 {
            let (sequence, job_id) = registry
                .enqueue(
                    &state,
                    "run-1",
                    "info",
                    &format!("line {i}"),
                    1_000 + i,
                    None,
                )
                .await
                .unwrap()
                .unwrap();
            assert_eq!(sequence, i);
            assert_eq!(job_id, None, "run-1 has no deployment_jobs row");
        }
        registry.close("run-1").await;

        let conn = crate::git::repositories::db::get_conn(&state).await;
        let rows = sqlx::query("SELECT sequence, message FROM application_deployment_run_logs WHERE run_id = 'run-1' ORDER BY sequence")
            .fetch_all(&conn)
            .await
            .unwrap();
        let mut seen = Vec::new();
        for row in &rows {
            let sequence: i64 = row.try_get(0).unwrap();
            let message: String = row.try_get(1).unwrap();
            seen.push((sequence, message));
        }
        assert_eq!(
            seen,
            vec![
                (0, "line 0".into()),
                (1, "line 1".into()),
                (2, "line 2".into()),
                (3, "line 3".into()),
                (4, "line 4".into())
            ]
        );
        cleanup(path);
    }

    #[tokio::test]
    async fn enqueue_resolves_job_id_once_when_a_job_row_exists() {
        let (state, path) = test_state().await;
        let conn = crate::git::repositories::db::get_conn(&state).await;
        sqlx::query(
            "INSERT INTO deployment_jobs (id, application_id, project_id, environment_id, organization_id, source_type, source_snapshot, build_method, build_snapshot, status, queued_at, run_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        )
        .bind("job-1")
        .bind("application-1")
        .bind("project-1")
        .bind("environment-1")
        .bind("organization-a")
        .bind("git")
        .bind("{}")
        .bind("docker")
        .bind("{}")
        .bind("building")
        .bind(1_i64)
        .bind("run-1")
        .execute(&conn)
        .await
        .unwrap();

        let registry = LogBufferRegistry::default();
        let (_, job_id) = registry
            .enqueue(&state, "run-1", "info", "hello", 1_000, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(job_id, Some("job-1".into()));
        registry.close("run-1").await;
        cleanup(path);
    }

    #[tokio::test]
    async fn keeps_emitting_past_the_storage_cap_without_persisting() {
        let (state, path) = test_state().await;
        let registry = LogBufferRegistry::with_max_stored_lines(2);

        for i in 0..4 {
            let emitted = registry
                .enqueue(
                    &state,
                    "run-1",
                    "info",
                    &format!("line {i}"),
                    1_000 + i,
                    None,
                )
                .await
                .unwrap();
            assert_eq!(
                emitted.map(|(sequence, _)| sequence),
                Some(i),
                "every line keeps streaming, capped or not"
            );
        }
        registry.close("run-1").await;

        let conn = crate::git::repositories::db::get_conn(&state).await;
        let rows = sqlx::query(
            "SELECT message FROM application_deployment_run_logs WHERE run_id = 'run-1' ORDER BY sequence",
        )
        .fetch_all(&conn)
        .await
        .unwrap();
        let mut stored = Vec::new();
        for row in &rows {
            let message: String = row.try_get(0).unwrap();
            stored.push(message);
        }
        assert_eq!(stored, vec!["line 0".to_string(), "line 1".to_string()]);
        cleanup(path);
    }

    #[tokio::test]
    async fn storage_cap_applies_per_replica() {
        let (state, path) = test_state().await;
        let registry = LogBufferRegistry::with_max_stored_lines(2);

        for i in 0..4 {
            registry
                .enqueue(
                    &state,
                    "run-1",
                    "info",
                    &format!("a{i}"),
                    1_000 + i,
                    Some("srv-a"),
                )
                .await
                .unwrap();
        }
        for i in 0..2 {
            registry
                .enqueue(
                    &state,
                    "run-1",
                    "info",
                    &format!("b{i}"),
                    2_000 + i,
                    Some("srv-b"),
                )
                .await
                .unwrap();
        }
        registry.close("run-1").await;

        let conn = crate::git::repositories::db::get_conn(&state).await;
        let rows = sqlx::query(
            "SELECT message FROM application_deployment_run_logs WHERE run_id = 'run-1' ORDER BY sequence",
        )
        .fetch_all(&conn)
        .await
        .unwrap();
        let stored: Vec<String> = rows.iter().map(|row| row.try_get(0).unwrap()).collect();
        assert_eq!(
            stored,
            vec!["a0", "a1", "b0", "b1"],
            "a chatty server leaves room for the others"
        );
        cleanup(path);
    }

    #[tokio::test]
    async fn attach_job_links_a_job_created_after_the_first_log_line() {
        let (state, path) = test_state().await;
        let registry = LogBufferRegistry::default();

        // The run's first line is written by `history::start`, before the job row exists.
        let (_, job_id) = registry
            .enqueue(&state, "run-1", "info", "Deployment started", 1_000, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(job_id, None, "the job row does not exist yet");

        let conn = crate::git::repositories::db::get_conn(&state).await;
        sqlx::query(
            "INSERT INTO deployment_jobs (id, application_id, project_id, environment_id, organization_id, source_type, source_snapshot, build_method, build_snapshot, status, queued_at, run_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        )
        .bind("job-1")
        .bind("application-1")
        .bind("project-1")
        .bind("environment-1")
        .bind("organization-a")
        .bind("git")
        .bind("{}")
        .bind("docker")
        .bind("{}")
        .bind("queued")
        .bind(1_i64)
        .bind("run-1")
        .execute(&conn)
        .await
        .unwrap();
        registry.attach_job("run-1", "job-1").await;

        let (_, job_id) = registry
            .enqueue(&state, "run-1", "info", "Cloning repository", 1_001, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(job_id, Some("job-1".into()));

        registry.close("run-1").await;
        cleanup(path);
    }

    #[tokio::test]
    async fn resumes_sequence_after_existing_rows() {
        let (state, path) = test_state().await;
        let conn = crate::git::repositories::db::get_conn(&state).await;
        sqlx::query("INSERT INTO application_deployment_run_logs (id, run_id, sequence, level, message, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)")
            .bind("log-1")
            .bind("run-1")
            .bind(0_i64)
            .bind("info")
            .bind("existing")
            .bind(1_i64)
            .execute(&conn)
            .await
            .unwrap();

        let registry = LogBufferRegistry::default();
        let (sequence, _) = registry
            .enqueue(&state, "run-1", "info", "next", 2_000, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(sequence, 1);
        registry.close("run-1").await;
        cleanup(path);
    }
}
