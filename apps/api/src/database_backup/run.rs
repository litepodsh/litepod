//! Executes per-database backups: dump inside the running container, extract
//! the bytes via `podman cp`, upload to the policy's S3 destination (a named
//! target or the global config) under `databases/{application_id}/`, enforce
//! retention, and emit the `database_backup` notification. Restore downloads
//! the S3 object from the exact target it was uploaded to, writes it into the
//! container and runs the engine's restore tool.

use std::io::Read;
use std::time::Duration;

use chrono::Utc;
use sqlx::Row;
use tar::Archive;

use crate::database_backup::engine::Engine;
use crate::dtos::database_backup::{
    DatabaseBackupDto, DatabaseBackupPolicyDto, UpdateDatabaseBackupPolicyRequest,
};
use crate::dtos::response::AppError;
use crate::state::state::SharedState;

const CONTAINER_DUMP_PATH: &str = "/tmp/litepod-db-backup";
const CONTAINER_DUMP_DIR: &str = "/tmp";
const CONTAINER_RESTORE_NAME: &str = "litepod-db-restore";
const DUMP_BUDGET: Duration = Duration::from_secs(1800);

const POLICY_COLUMNS: &str = "application_id, enabled, cron_expression, retention_count, next_run_at, updated_at, updated_by, s3_target_id";
const BACKUP_COLUMNS: &str = "id, application_id, status, engine, started_at, finished_at, size_bytes, s3_key, error_message, s3_target_id";

fn exec_argv(container: &str, script: &str) -> Vec<std::ffi::OsString> {
    vec![
        "exec".into(),
        container.into(),
        "bash".into(),
        "-lc".into(),
        script.into(),
    ]
}

fn read_policy_row(row: &sqlx::sqlite::SqliteRow) -> Result<DatabaseBackupPolicyDto, AppError> {
    let err = |e: sqlx::Error| AppError::DatabaseError(e.to_string());
    Ok(DatabaseBackupPolicyDto {
        application_id: row.try_get(0).map_err(err)?,
        enabled: row.try_get::<i64, _>(1).map_err(err)? != 0,
        cron_expression: row.try_get(2).map_err(err)?,
        retention_count: row.try_get(3).map_err(err)?,
        next_run_at: row.try_get(4).map_err(err)?,
        updated_at: row.try_get(5).map_err(err)?,
        updated_by: row.try_get(6).map_err(err)?,
        s3_target_id: row.try_get(7).map_err(err)?,
    })
}

fn read_backup_row(row: &sqlx::sqlite::SqliteRow) -> Result<DatabaseBackupDto, AppError> {
    let err = |e: sqlx::Error| AppError::DatabaseError(e.to_string());
    Ok(DatabaseBackupDto {
        id: row.try_get(0).map_err(err)?,
        application_id: row.try_get(1).map_err(err)?,
        status: row.try_get(2).map_err(err)?,
        engine: row.try_get(3).map_err(err)?,
        started_at: row.try_get(4).map_err(err)?,
        finished_at: row.try_get(5).map_err(err)?,
        size_bytes: row.try_get(6).map_err(err)?,
        s3_key: row.try_get(7).map_err(err)?,
        error_message: row.try_get(8).map_err(err)?,
        s3_target_id: row.try_get(9).map_err(err)?,
    })
}

/// Resolves the organization and name for a database application so scheduled
/// runs (which have no request scope) can still emit org-scoped notifications.
async fn app_org_context(
    state: &SharedState,
    application_id: &str,
) -> Result<(String, String), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(
        "SELECT o.id, a.name FROM applications a \
         JOIN environments e ON e.id = a.environment_id \
         JOIN projects p ON p.id = e.project_id \
         JOIN organizations o ON o.id = p.organization_id \
         WHERE a.id = ?1",
    )
    .bind(application_id)
    .fetch_optional(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    match row {
        Some(row) => Ok((
            row.try_get(0)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            row.try_get(1)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        )),
        None => Err(AppError::ApplicationNotFound),
    }
}

async fn template_kind(
    state: &SharedState,
    application_id: &str,
) -> Result<Option<String>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query("SELECT template_kind FROM applications WHERE id = ?1")
        .bind(application_id)
        .fetch_optional(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    match row {
        Some(row) => row
            .try_get(0)
            .map_err(|e| AppError::DatabaseError(e.to_string())),
        None => Err(AppError::ApplicationNotFound),
    }
}

// ---------------------------------------------------------------------------
// Policy
// ---------------------------------------------------------------------------

/// Reads the policy, creating a default (disabled) row on first access so the
/// UI always has a policy to render.
pub async fn get_policy(
    state: &SharedState,
    application_id: &str,
) -> Result<DatabaseBackupPolicyDto, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let now = Utc::now().timestamp_millis();
    // INSERT OR IGNORE then SELECT keeps the read path single-code even for a
    // brand-new application.
    sqlx::query(
        "INSERT OR IGNORE INTO database_backup_policies \
         (application_id, enabled, cron_expression, retention_count, next_run_at, updated_at, updated_by, s3_target_id) \
         VALUES (?1, 0, '0 0 3 * * *', 7, NULL, ?2, NULL, NULL)",
    )
    .bind(application_id)
    .bind(now)
    .execute(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let row = sqlx::query(&format!(
        "SELECT {POLICY_COLUMNS} FROM database_backup_policies WHERE application_id = ?1"
    ))
    .bind(application_id)
    .fetch_one(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    read_policy_row(&row)
}

/// Validates and upserts the policy. `req.retention_count` must be ≥ 1 and
/// `req.cron_expression` a valid croner expression; enabling computes the next
/// run time so the scheduler has a due date.
pub async fn set_policy(
    state: &SharedState,
    application_id: &str,
    req: &UpdateDatabaseBackupPolicyRequest,
    updated_by: Option<&str>,
) -> Result<DatabaseBackupPolicyDto, AppError> {
    if req.retention_count < 1 {
        return Err(AppError::ValidationError(
            "Retention must keep at least one backup".into(),
        ));
    }
    if req.enabled {
        crate::scheduled_tasks::next_run_at(&req.cron_expression)?;
    }
    // A policy that names a target must name one of its own organization's:
    // a target id from another organization reads exactly like a missing one.
    let target_id = req
        .s3_target_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let (organization_id, _) = app_org_context(state, application_id).await?;
    if let Some(id) = target_id
        && crate::backup::target::get(state, &organization_id, id)
            .await?
            .is_none()
    {
        return Err(AppError::ValidationError(
            "The selected S3 target does not exist".into(),
        ));
    }
    // On cloud there is no global destination to fall back to (it is the
    // operator's bucket), so a schedule without a target could never run.
    if state.is_cloud && req.enabled && target_id.is_none() {
        return Err(AppError::ValidationError(
            "Choose an S3 target before enabling scheduled backups".into(),
        ));
    }
    let conn = crate::git::repositories::db::get_conn(state).await;
    let now = Utc::now().timestamp_millis();
    let next_run_at = if req.enabled {
        Some(crate::scheduled_tasks::next_run_at(&req.cron_expression)?)
    } else {
        None
    };
    sqlx::query(
        "INSERT INTO database_backup_policies \
         (application_id, enabled, cron_expression, retention_count, next_run_at, updated_at, updated_by, s3_target_id) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
         ON CONFLICT(application_id) DO UPDATE SET \
           enabled = excluded.enabled, \
           cron_expression = excluded.cron_expression, \
           retention_count = excluded.retention_count, \
           next_run_at = excluded.next_run_at, \
           updated_at = excluded.updated_at, \
           updated_by = excluded.updated_by, \
           s3_target_id = excluded.s3_target_id",
    )
    .bind(application_id)
    .bind(req.enabled)
    .bind(req.cron_expression.trim())
    .bind(req.retention_count)
    .bind(next_run_at)
    .bind(now)
    .bind(updated_by)
    .bind(target_id)
    .execute(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    get_policy(state, application_id).await
}

// ---------------------------------------------------------------------------
// Backup run
// ---------------------------------------------------------------------------

/// Validates S3 + engine, records a `running` row and executes the run in the
/// background. Returns the running record so the API can answer immediately.
pub async fn request_run(
    state: &SharedState,
    application_id: &str,
    triggered_by: &str,
) -> Result<DatabaseBackupDto, AppError> {
    let policy = get_policy(state, application_id).await?;
    // Fail fast if no destination exists (target or global), and capture which
    // one this run will use so history/restore resolve the exact bucket.
    let target_id = policy.s3_target_id.clone();
    let (organization_id, _) = app_org_context(state, application_id).await?;
    crate::backup::target::resolve(state, &organization_id, target_id.as_deref()).await?;

    let kind = template_kind(state, application_id).await?;
    let engine = Engine::from_kind(kind.as_deref())?;

    let id = uuid::Uuid::now_v7().to_string();
    let started_at = Utc::now().timestamp_millis();
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "INSERT INTO database_backups \
         (id, application_id, status, engine, started_at, s3_target_id) VALUES (?1, ?2, 'running', ?3, ?4, ?5)",
    )
    .bind(&id)
    .bind(application_id)
    .bind(engine.as_str())
    .bind(started_at)
    .bind(&target_id)
    .execute(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;

    let state = state.clone();
    let application_id = application_id.to_string();
    let triggered_by = triggered_by.to_string();
    let dto = DatabaseBackupDto {
        id: id.clone(),
        application_id: application_id.clone(),
        status: "running".into(),
        engine: engine.as_str().to_string(),
        started_at,
        finished_at: None,
        size_bytes: None,
        s3_key: None,
        error_message: None,
        s3_target_id: target_id,
    };
    tokio::spawn(async move {
        execute_run(state, id, application_id, triggered_by).await;
    });

    Ok(dto)
}

async fn execute_run(
    state: SharedState,
    backup_id: String,
    application_id: String,
    triggered_by: String,
) {
    let outcome = perform_backup(&state, &application_id).await;
    let conn = crate::git::repositories::db::get_conn(&state).await;
    let finished_at = Utc::now().timestamp_millis();
    match outcome {
        Ok((size_bytes, s3_key)) => {
            sqlx::query(
                "UPDATE database_backups SET status = 'succeeded', finished_at = ?1, size_bytes = ?2, s3_key = ?3 WHERE id = ?4",
            )
            .bind(finished_at)
            .bind(size_bytes)
            .bind(&s3_key)
            .bind(&backup_id)
            .execute(&conn)
            .await
            .ok();
            let _ = enforce_retention(&state, &application_id).await;
            notify(
                &state,
                &application_id,
                true,
                &triggered_by,
                Some(size_bytes),
                None,
            )
            .await;
        }
        Err(error) => {
            let message = error.to_string();
            sqlx::query(
                "UPDATE database_backups SET status = 'failed', finished_at = ?1, error_message = ?2 WHERE id = ?3",
            )
            .bind(finished_at)
            .bind(&message)
            .bind(&backup_id)
            .execute(&conn)
            .await
            .ok();
            notify(
                &state,
                &application_id,
                false,
                &triggered_by,
                None,
                Some(&message),
            )
            .await;
        }
    }
}

/// Runs the actual dump/unload cycle and returns `(size_bytes, s3_key)`.
async fn perform_backup(
    state: &SharedState,
    application_id: &str,
) -> Result<(i64, String), AppError> {
    let kind = template_kind(state, application_id).await?;
    let engine = Engine::from_kind(kind.as_deref())?;

    let runtime =
        crate::servers::runtime::ServerRuntime::for_application(state, application_id).await?;
    let (container, _) = crate::deployment::identity(state, application_id).await?;

    // Preflight: the image must ship the engine's dump tool. A clean error here
    // beats a misleading podman failure halfway through a run.
    runtime
        .podman_with_timeout(
            &exec_argv(
                &container,
                &format!("command -v {} >/dev/null 2>&1 || exit 127", engine.tool()),
            ),
            Duration::from_secs(60),
        )
        .await
        .map_err(|e| {
            AppError::ValidationError(format!(
                "The {} image does not include the {} tool (or the container is not running): {}",
                engine.as_str(),
                engine.tool(),
                e
            ))
        })?;

    runtime
        .podman_with_timeout(&exec_argv(&container, engine.dump_script()), DUMP_BUDGET)
        .await
        .map_err(|error| {
            AppError::InternalError(format!(
                "Database dump failed inside the container: {error}"
            ))
        })?;

    let tar_bytes = runtime
        .podman_cp_from(&container, CONTAINER_DUMP_PATH, Duration::from_secs(300))
        .await
        .map_err(|error| AppError::InternalError(format!("Could not extract the dump: {error}")))?;
    let dump_bytes = extract_single_entry(&tar_bytes)?;

    let cleanup = exec_argv(&container, &format!("rm -f {}", CONTAINER_DUMP_PATH));
    let _ = runtime
        .podman_with_timeout(&cleanup, Duration::from_secs(30))
        .await;

    // Resolve the destination from the policy: the named target when picked,
    // the global S3 config otherwise.
    let target_id = get_policy(state, application_id).await?.s3_target_id;
    let (organization_id, _) = app_org_context(state, application_id).await?;
    let s3_config =
        crate::backup::target::resolve(state, &organization_id, target_id.as_deref()).await?;
    let bucket = crate::backup::s3::client(&s3_config)?;
    let stamp = Utc::now().format("%Y-%m-%dT%H-%M-%SZ");
    let s3_key = format!("databases/{application_id}/{stamp}.{}", engine.extension());
    let size = dump_bytes.len() as i64;
    let mut reader = std::io::Cursor::new(dump_bytes);
    bucket
        .put_object_stream(&mut reader, &s3_key)
        .await
        .map_err(|error| AppError::InternalError(format!("S3 upload failed: {error}")))?;
    Ok((size, s3_key))
}

/// `podman cp <container>:<path> -` streams a tar; unwrap its single member.
fn extract_single_entry(tar_bytes: &[u8]) -> Result<Vec<u8>, AppError> {
    let mut archive = Archive::new(tar_bytes);
    for entry in archive
        .entries()
        .map_err(|e| AppError::InternalError(format!("Backup tar read failed: {e}")))?
    {
        let mut entry =
            entry.map_err(|e| AppError::InternalError(format!("Backup tar entry failed: {e}")))?;
        if entry.header().entry_type().is_file() {
            let mut bytes = Vec::new();
            entry
                .read_to_end(&mut bytes)
                .map_err(|e| AppError::InternalError(format!("Backup tar extract failed: {e}")))?;
            return Ok(bytes);
        }
    }
    Err(AppError::InternalError(
        "The extracted dump archive contained no file".into(),
    ))
}

/// Builds a single-member tar for `podman cp - container:/tmp` so the restore
/// lands at the fixed `/tmp/litepod-db-restore` path inside the container.
fn build_restore_tar(dump_bytes: &[u8]) -> Result<Vec<u8>, AppError> {
    let mut builder = tar::Builder::new(Vec::new());
    let mut header = tar::Header::new_gnu();
    header.set_size(dump_bytes.len() as u64);
    header.set_mode(0o600);
    header.set_cksum();
    builder
        .append_data(&mut header, CONTAINER_RESTORE_NAME, dump_bytes)
        .map_err(|e| AppError::InternalError(format!("Restore tar failed: {e}")))?;
    let bytes = builder
        .into_inner()
        .map_err(|e| AppError::InternalError(format!("Restore tar finalize failed: {e}")))?;
    Ok(bytes)
}

/// Deletes S3 objects + rows beyond the policy's retention window.
async fn enforce_retention(state: &SharedState, application_id: &str) -> Result<(), AppError> {
    let retention = get_policy(state, application_id)
        .await?
        .retention_count
        .max(1) as usize;
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(
        "SELECT id, s3_key, s3_target_id FROM database_backups \
         WHERE application_id = ?1 AND status = 'succeeded' \
         ORDER BY started_at DESC",
    )
    .bind(application_id)
    .fetch_all(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let mut stale: Vec<(String, String, Option<String>)> = Vec::new();
    for row in rows.iter().skip(retention) {
        let id: String = row
            .try_get(0)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        let key: Option<String> = row
            .try_get(1)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        let s3_target_id: Option<String> = row
            .try_get(2)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        if let Some(key) = key {
            stale.push((id, key, s3_target_id));
        }
    }
    if stale.is_empty() {
        return Ok(());
    }
    let (organization_id, _) = app_org_context(state, application_id).await?;
    for (id, key, s3_target_id) in &stale {
        // Delete each archive against the target it was uploaded to; a missing
        // target falls back to the global config, and any single failure must
        // not abort the rest of the cleanup.
        if let Ok(config) = crate::backup::target::resolve_recorded(
            state,
            &organization_id,
            s3_target_id.as_deref(),
        )
        .await
            && let Ok(bucket) = crate::backup::s3::client(&config)
        {
            let _ = bucket.delete_object(key).await;
        }
        sqlx::query("DELETE FROM database_backups WHERE id = ?1 AND application_id = ?2")
            .bind(id)
            .bind(application_id)
            .execute(&conn)
            .await
            .ok();
    }
    Ok(())
}

/// Deletes one backup: S3 object (when present, in the target it was uploaded
/// to) + row.
pub async fn remove_backup(
    state: &SharedState,
    application_id: &str,
    backup_id: &str,
) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(
        "SELECT s3_key, s3_target_id FROM database_backups WHERE id = ?1 AND application_id = ?2",
    )
    .bind(backup_id)
    .bind(application_id)
    .fetch_optional(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let Some(row) = row else {
        return Err(AppError::ResourceNotFound);
    };
    let s3_key: Option<String> = row
        .try_get(0)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let s3_target_id: Option<String> = row
        .try_get(1)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let (organization_id, _) = app_org_context(state, application_id).await?;
    if let Some(key) = s3_key
        && let Ok(config) = crate::backup::target::resolve_recorded(
            state,
            &organization_id,
            s3_target_id.as_deref(),
        )
        .await
        && let Ok(bucket) = crate::backup::s3::client(&config)
    {
        let _ = bucket.delete_object(&key).await;
    }
    sqlx::query("DELETE FROM database_backups WHERE id = ?1 AND application_id = ?2")
        .bind(backup_id)
        .bind(application_id)
        .execute(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok(())
}

pub async fn list_backups(
    state: &SharedState,
    application_id: &str,
) -> Result<Vec<DatabaseBackupDto>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(&format!(
        "SELECT {BACKUP_COLUMNS} FROM database_backups \
         WHERE application_id = ?1 ORDER BY started_at DESC LIMIT 100"
    ))
    .bind(application_id)
    .fetch_all(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    rows.iter().map(read_backup_row).collect()
}

// ---------------------------------------------------------------------------
// Restore
// ---------------------------------------------------------------------------

/// Restores a succeeded backup into the live container. Synchronous on
/// purpose: a restore is a deliberate, rare operation and the requester needs
/// the outcome.
pub async fn restore_backup(
    state: &SharedState,
    application_id: &str,
    backup_id: &str,
    triggered_by: &str,
) -> Result<(), AppError> {
    let kind = template_kind(state, application_id).await?;
    let engine = Engine::from_kind(kind.as_deref())?;
    if !engine.restore_supported() {
        return Err(AppError::ValidationError(format!(
            "Restore is not supported for {} yet",
            engine.as_str()
        )));
    }
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(
        "SELECT s3_key, s3_target_id FROM database_backups \
         WHERE id = ?1 AND application_id = ?2 AND status = 'succeeded'",
    )
    .bind(backup_id)
    .bind(application_id)
    .fetch_optional(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let Some(row) = row else {
        return Err(AppError::ResourceNotFound);
    };
    let s3_key: String = row
        .try_get(0)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let s3_target_id: Option<String> = row
        .try_get(1)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;

    // Restore from the exact destination the archive was uploaded to.
    let (organization_id, _) = app_org_context(state, application_id).await?;
    let s3_config =
        crate::backup::target::resolve_recorded(state, &organization_id, s3_target_id.as_deref())
            .await?;
    let bucket = crate::backup::s3::client(&s3_config)?;
    let object = bucket
        .get_object(&s3_key)
        .await
        .map_err(|error| AppError::InternalError(format!("S3 download failed: {error}")))?;
    let dump_bytes = object.bytes().to_vec();
    let tar_bytes = build_restore_tar(&dump_bytes)?;

    let runtime =
        crate::servers::runtime::ServerRuntime::for_application(state, application_id).await?;
    let (container, _) = crate::deployment::identity(state, application_id).await?;

    runtime
        .podman_cp_to(
            application_id,
            &container,
            CONTAINER_DUMP_DIR,
            tar_bytes,
            Duration::from_secs(300),
        )
        .await
        .map_err(|error| {
            AppError::InternalError(format!("Could not stage the restore file: {error}"))
        })?;

    runtime
        .podman_with_timeout(&exec_argv(&container, engine.restore_script()), DUMP_BUDGET)
        .await
        .map_err(|error| {
            AppError::InternalError(format!("Restore failed inside the container: {error}"))
        })?;

    let cleanup = exec_argv(
        &container,
        &format!("rm -f /tmp/{}", CONTAINER_RESTORE_NAME),
    );
    let _ = runtime
        .podman_with_timeout(&cleanup, Duration::from_secs(30))
        .await;

    notify(
        state,
        application_id,
        true,
        triggered_by,
        Some(dump_bytes.len() as i64),
        None,
    )
    .await;
    Ok(())
}

// ---------------------------------------------------------------------------
// Notifications
// ---------------------------------------------------------------------------

async fn notify(
    state: &SharedState,
    application_id: &str,
    succeeded: bool,
    triggered_by: &str,
    size_bytes: Option<i64>,
    error: Option<&str>,
) {
    let Ok((organization_id, application_name)) = app_org_context(state, application_id).await
    else {
        return;
    };
    let kind = template_kind(state, application_id)
        .await
        .unwrap_or_default();
    let engine = Engine::from_kind(kind.as_deref())
        .map(|e| e.as_str().to_string())
        .unwrap_or_else(|_| "database".into());
    crate::notifications::emit::notify_database_backup(
        state,
        &organization_id,
        &application_name,
        &engine,
        succeeded,
        triggered_by,
        size_bytes,
        error,
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_single_entry_returns_the_only_file() {
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_size(4);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, "litepod-db-backup", b"data".as_slice())
            .unwrap();
        let tar = builder.into_inner().unwrap();
        assert_eq!(extract_single_entry(&tar).unwrap(), b"data");
    }

    #[test]
    fn extract_single_entry_rejects_an_empty_tar() {
        let mut builder = tar::Builder::new(Vec::new());
        let tar = builder.into_inner().unwrap();
        assert!(extract_single_entry(&tar).is_err());
    }

    #[test]
    fn restore_tar_round_trips_through_extract() {
        let dump: Vec<u8> = (0..=255u8).cycle().take(4096).collect();
        let tar = build_restore_tar(&dump).unwrap();
        assert_eq!(extract_single_entry(&tar).unwrap(), dump);
    }

    #[test]
    fn restore_is_gated_per_engine() {
        assert!(Engine::Postgres.restore_supported());
        assert!(Engine::MariaDb.restore_supported());
        assert!(Engine::MongoDb.restore_supported());
        assert!(!Engine::DragonflyDb.restore_supported());
        assert!(!Engine::KeyDb.restore_supported());
    }

    #[test]
    fn every_dump_script_references_its_own_tool() {
        for engine in [
            Engine::Postgres,
            Engine::MariaDb,
            Engine::MongoDb,
            Engine::DragonflyDb,
            Engine::KeyDb,
        ] {
            assert!(
                engine.dump_script().contains(engine.tool()),
                "{} dump script must invoke {}",
                engine.as_str(),
                engine.tool()
            );
        }
    }

    #[test]
    fn sqlserver_is_explicitly_rejected() {
        assert!(matches!(
            Engine::from_kind(Some("sqlserver")),
            Err(AppError::ValidationError(_))
        ));
        assert!(matches!(
            Engine::from_kind(Some("nonexistent")),
            Err(AppError::ValidationError(_))
        ));
    }
}
