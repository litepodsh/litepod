//! Backups of the named volumes an application mounts — Compose volumes and
//! managed volumes alike, keyed by Podman volume name. A run does
//! `podman volume export` on the server the application runs on and uploads
//! the tar to the policy's S3 destination under
//! `volumes/{application_id}/{volume_name}/`. Restore either imports into a new
//! detached managed volume (to check the files first) or replaces the original
//! volume with the application stopped, after saving its current contents.
//!
//! Policy and retention mirror `database_backup::run` on purpose.

use std::ffi::OsString;
use std::time::Duration;

use chrono::Utc;
use sqlx::Row;
use tokio::io::{AsyncRead, AsyncWriteExt};

use crate::backup::stream::{feed_stdin, object_reader, put_reader, stdout_to, upload_stdout};

use crate::database_backup::run::app_org_context;
use crate::deployment::managed_volumes;
use crate::dtos::response::AppError;
use crate::dtos::volume_backup::{
    AppVolumeBackupsDto, NewVolumeFromTarRequest, RestoreFromS3Request, RestoreTarget,
    RestoreVolumeBackupRequest, S3TarDto, UpdateVolumeBackupPolicyRequest, VolumeBackupDto,
    VolumeBackupPolicyDto, VolumeRestoreResultDto,
};
use crate::podman::containers::volume_workspace::{HELPER_IMAGE, remote_volume_mount};
use crate::state::state::SharedState;

const TRANSFER_BUDGET: Duration = Duration::from_secs(1800);

const POLICY_COLUMNS: &str = "volume_name, enabled, cron_expression, retention_count, next_run_at, updated_at, updated_by, s3_target_id, stop_app";
const BACKUP_COLUMNS: &str = "id, volume_name, status, triggered_by, started_at, finished_at, size_bytes, s3_key, error_message, s3_target_id, group_id";

fn db_err(e: sqlx::Error) -> AppError {
    AppError::DatabaseError(e.to_string())
}

fn read_policy_row(row: &sqlx::sqlite::SqliteRow) -> Result<VolumeBackupPolicyDto, AppError> {
    Ok(VolumeBackupPolicyDto {
        volume_name: row.try_get(0).map_err(db_err)?,
        enabled: row.try_get::<i64, _>(1).map_err(db_err)? != 0,
        cron_expression: row.try_get(2).map_err(db_err)?,
        retention_count: row.try_get(3).map_err(db_err)?,
        next_run_at: row.try_get(4).map_err(db_err)?,
        updated_at: row.try_get(5).map_err(db_err)?,
        updated_by: row.try_get(6).map_err(db_err)?,
        s3_target_id: row.try_get(7).map_err(db_err)?,
        default_available: false,
        stop_app: row.try_get::<i64, _>(8).map_err(db_err)? != 0,
    })
}

fn read_backup_row(row: &sqlx::sqlite::SqliteRow) -> Result<VolumeBackupDto, AppError> {
    Ok(VolumeBackupDto {
        id: row.try_get(0).map_err(db_err)?,
        volume_name: row.try_get(1).map_err(db_err)?,
        status: row.try_get(2).map_err(db_err)?,
        triggered_by: row.try_get(3).map_err(db_err)?,
        started_at: row.try_get(4).map_err(db_err)?,
        finished_at: row.try_get(5).map_err(db_err)?,
        size_bytes: row.try_get(6).map_err(db_err)?,
        s3_key: row.try_get(7).map_err(db_err)?,
        error_message: row.try_get(8).map_err(db_err)?,
        s3_target_id: row.try_get(9).map_err(db_err)?,
        group_id: row.try_get(10).map_err(db_err)?,
    })
}

/// Marks runs left `running` by a restart as failed, like `backup::run`.
pub async fn fail_interrupted(state: &SharedState) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE volume_backups SET status = 'failed', finished_at = ?1, \
         error_message = 'Backup was interrupted because litepod restarted' WHERE status = 'running'",
    )
    .bind(Utc::now().timestamp_millis())
    .execute(&conn)
    .await
    .map_err(db_err)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Which volumes belong to the application
// ---------------------------------------------------------------------------

/// Where the volume is mounted, as seen now: `(mount_path, compose_service)`.
#[derive(Debug, Clone, Default)]
struct Mount {
    path: Option<String>,
    service: Option<String>,
}

/// Trust boundary: `volume_name` comes from the URL. It must be a named volume
/// this application mounts right now, or one of its managed volumes (which
/// covers detached ones and a stopped app). A server group is refused: every
/// member has its own copy of the volume.
async fn app_volume(
    state: &SharedState,
    application_id: &str,
    volume_name: &str,
) -> Result<Mount, AppError> {
    remote_volume_mount(volume_name).map_err(|_| AppError::ResourceNotFound)?;
    if app_is_group(state, application_id).await? {
        return Err(AppError::ValidationError(
            "Volume backups are not available for applications on a server group yet".into(),
        ));
    }
    // A stopped app has no live mounts; that is not an error here.
    let live = crate::deployment::volumes(state, application_id)
        .await
        .unwrap_or_default();
    if let Some(mount) = live
        .into_iter()
        .find(|mount| mount.mount_type == "volume" && mount.source == volume_name)
    {
        return Ok(Mount {
            path: Some(mount.destination),
            service: mount.services.into_iter().next(),
        });
    }
    if let Some(volume) =
        managed_volumes::find_by_podman_name(state, application_id, volume_name).await?
    {
        return Ok(Mount {
            path: Some(volume.mount_path),
            service: volume.compose_service,
        });
    }
    Err(AppError::ResourceNotFound)
}

// ---------------------------------------------------------------------------
// Policy
// ---------------------------------------------------------------------------

async fn read_policy(
    state: &SharedState,
    application_id: &str,
    volume_name: &str,
) -> Result<VolumeBackupPolicyDto, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "INSERT OR IGNORE INTO volume_backup_policies \
         (application_id, volume_name, enabled, cron_expression, retention_count, updated_at) \
         VALUES (?1, ?2, 0, '0 0 3 * * *', 7, ?3)",
    )
    .bind(application_id)
    .bind(volume_name)
    .bind(Utc::now().timestamp_millis())
    .execute(&conn)
    .await
    .map_err(db_err)?;
    let row = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {POLICY_COLUMNS} FROM volume_backup_policies WHERE application_id = ?1 AND volume_name = ?2"
    )))
    .bind(application_id)
    .bind(volume_name)
    .fetch_one(&conn)
    .await
    .map_err(db_err)?;
    let mut policy = read_policy_row(&row)?;
    policy.default_available =
        !state.is_cloud && crate::backup::config::load(state).await?.is_some();
    Ok(policy)
}

/// Next run, whole seconds: the shared helper keeps the save time's
/// milliseconds, and volumes saved together must come due in the same
/// scheduler tick to run as one group.
pub fn next_run_at(cron_expression: &str) -> Result<i64, AppError> {
    crate::scheduled_tasks::next_run_at(cron_expression).map(|millis| millis - millis.rem_euclid(1000))
}

/// Reads the policy, creating a default (disabled) row on first access.
pub async fn get_policy(
    state: &SharedState,
    application_id: &str,
    volume_name: &str,
) -> Result<VolumeBackupPolicyDto, AppError> {
    app_volume(state, application_id, volume_name).await?;
    read_policy(state, application_id, volume_name).await
}

/// Same rules as `database_backup::run::set_policy`: retention ≥ 1, a valid
/// cron when enabled, a target of the app's own organization, and on cloud a
/// target is required to enable a schedule.
pub async fn set_policy(
    state: &SharedState,
    application_id: &str,
    volume_name: &str,
    req: &UpdateVolumeBackupPolicyRequest,
    updated_by: Option<&str>,
) -> Result<VolumeBackupPolicyDto, AppError> {
    get_policy(state, application_id, volume_name).await?;
    if req.retention_count < 1 {
        return Err(AppError::ValidationError(
            "Retention must keep at least one backup".into(),
        ));
    }
    let next_run_at = if req.enabled {
        Some(next_run_at(&req.cron_expression)?)
    } else {
        None
    };
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
    if state.is_cloud && req.enabled && target_id.is_none() {
        return Err(AppError::ValidationError(
            "Choose an S3 target before enabling scheduled backups".into(),
        ));
    }
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE volume_backup_policies SET enabled = ?1, cron_expression = ?2, \
         retention_count = ?3, next_run_at = ?4, updated_at = ?5, updated_by = ?6, \
         s3_target_id = ?7, stop_app = ?10 WHERE application_id = ?8 AND volume_name = ?9",
    )
    .bind(req.enabled)
    .bind(req.cron_expression.trim())
    .bind(req.retention_count)
    .bind(next_run_at)
    .bind(Utc::now().timestamp_millis())
    .bind(updated_by)
    .bind(target_id)
    .bind(application_id)
    .bind(volume_name)
    .bind(req.stop_app)
    .execute(&conn)
    .await
    .map_err(db_err)?;
    read_policy(state, application_id, volume_name).await
}

// ---------------------------------------------------------------------------
// Backup run
// ---------------------------------------------------------------------------

/// Inserts a `running` row unless one already runs for this volume.
async fn claim_run(
    state: &SharedState,
    application_id: &str,
    volume_name: &str,
    mount: &Mount,
    triggered_by: &str,
    s3_target_id: Option<&str>,
    group_id: Option<&str>,
) -> Result<VolumeBackupDto, AppError> {
    let id = uuid::Uuid::now_v7().to_string();
    let started_at = Utc::now().timestamp_millis();
    let conn = crate::git::repositories::db::get_conn(state).await;
    let inserted = sqlx::query(
        "INSERT INTO volume_backups (id, application_id, volume_name, mount_path, compose_service, status, triggered_by, started_at, s3_target_id, group_id) \
         SELECT ?1, ?2, ?3, ?4, ?5, 'running', ?6, ?7, ?8, ?9 \
         WHERE NOT EXISTS (SELECT 1 FROM volume_backups WHERE application_id = ?2 AND volume_name = ?3 AND status = 'running')",
    )
    .bind(&id)
    .bind(application_id)
    .bind(volume_name)
    .bind(&mount.path)
    .bind(&mount.service)
    .bind(triggered_by)
    .bind(started_at)
    .bind(s3_target_id)
    .bind(group_id)
    .execute(&conn)
    .await
    .map_err(db_err)?;
    if inserted.rows_affected() == 0 {
        return Err(AppError::ValidationError(
            "A backup or restore of this volume is already running".into(),
        ));
    }
    Ok(VolumeBackupDto {
        id,
        volume_name: volume_name.to_string(),
        status: "running".into(),
        triggered_by: triggered_by.to_string(),
        started_at,
        finished_at: None,
        size_bytes: None,
        s3_key: None,
        error_message: None,
        s3_target_id: s3_target_id.map(str::to_string),
        group_id: group_id.map(str::to_string),
    })
}

async fn finish_run(
    state: &SharedState,
    backup_id: &str,
    outcome: &Result<(i64, String), AppError>,
) {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let finished_at = Utc::now().timestamp_millis();
    let query = match outcome {
        Ok((size, key)) => sqlx::query(
            "UPDATE volume_backups SET status = 'succeeded', finished_at = ?1, size_bytes = ?2, s3_key = ?3 WHERE id = ?4",
        )
        .bind(finished_at)
        .bind(*size)
        .bind(key.clone()),
        Err(error) => sqlx::query(
            "UPDATE volume_backups SET status = 'failed', finished_at = ?1, error_message = ?2 WHERE id = ?3",
        )
        .bind(finished_at)
        .bind(error.to_string()),
    };
    query.bind(backup_id).execute(&conn).await.ok();
}

/// Exports the volume on its own server and uploads the tar. Returns `(size, key)`.
async fn export_and_upload(
    state: &SharedState,
    application_id: &str,
    volume_name: &str,
    backup_id: &str,
    s3_target_id: Option<&str>,
    stop_app: bool,
) -> Result<(i64, String), AppError> {
    let runtime =
        crate::servers::runtime::ServerRuntime::for_application(state, application_id).await?;
    let (organization_id, application_name) = app_org_context(state, application_id).await?;
    let label =
        match managed_volumes::find_by_podman_name(state, application_id, volume_name).await? {
            Some(volume) => volume.name,
            None => volume_label(volume_name).to_string(),
        };
    let s3_key = object_key(&application_name, &label, Utc::now(), backup_id);
    let config = crate::backup::target::resolve(state, &organization_id, s3_target_id).await?;
    // Plain PUT, no x-amz-meta-* headers: RustFS rejects rust-s3's signature
    // once custom headers are added. The key itself names app, volume and time.
    let bucket = crate::backup::s3::client(&config)?;
    let export_args = [
        OsString::from("volume"),
        OsString::from("export"),
        OsString::from(volume_name),
    ];

    let was_running = stop_app && stop_if_running(state, application_id).await?;
    if !was_running {
        let export = runtime.podman_stream(&export_args).await?;
        let size =
            upload_stdout(&bucket, &s3_key, export, TRANSFER_BUDGET, "Volume export").await?;
        return Ok((size, s3_key));
    }

    // Only the export runs with the app down: spool it to local disk, start the
    // app again, then upload from the file.
    let spool = std::env::temp_dir().join(format!("litepod-volume-{backup_id}.tar"));
    let exported = async {
        let mut file = tokio::fs::File::create(&spool).await.map_err(spool_error)?;
        let export = runtime.podman_stream(&export_args).await?;
        stdout_to(export, &mut file, TRANSFER_BUDGET, "Volume export").await?;
        file.flush().await.map_err(spool_error)
    }
    .await;
    if let Err(error) = crate::deployment::start(state, application_id, None).await {
        tracing::warn!(%application_id, %error, "could not start the application again after a volume backup");
    }
    let uploaded = match exported {
        Ok(()) => match tokio::fs::File::open(&spool).await {
            Ok(mut file) => put_reader(&bucket, &s3_key, &mut file).await,
            Err(error) => Err(spool_error(error)),
        },
        Err(error) => Err(error),
    };
    let _ = tokio::fs::remove_file(&spool).await;
    if uploaded.is_err() {
        let _ = bucket.delete_object(&s3_key).await;
    }
    Ok((uploaded?, s3_key))
}

fn spool_error(error: std::io::Error) -> AppError {
    AppError::InternalError(format!(
        "Could not stage the volume archive on local disk: {error}"
    ))
}

/// The Compose volume without its `<project>-compose_` prefix, which only
/// repeats the application's id: `app-ac9c…-compose_forgejo_data` → `forgejo_data`.
fn volume_label(volume_name: &str) -> &str {
    volume_name
        .split_once("-compose_")
        .map_or(volume_name, |(_, rest)| rest)
}

/// Keeps a name readable but safe in an S3 key and an HTTP header value.
fn key_segment(value: &str) -> String {
    let cleaned: String = value
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches('-');
    if cleaned.is_empty() {
        "volume".into()
    } else {
        cleaned.to_string()
    }
}

/// `volumes/<app>/<volume>/<app>_<volume>_<YYYY-MM-DD>_<HH-MM-SS>Z_<id>.tar`:
/// readable in any S3 browser, sorted by time inside a folder. The short run
/// id keeps two same-named apps backing up in the same second apart.
fn object_key(
    application_name: &str,
    volume_label: &str,
    now: chrono::DateTime<Utc>,
    backup_id: &str,
) -> String {
    let app = key_segment(application_name);
    let volume = key_segment(volume_label);
    let short_id = &backup_id[backup_id.len().saturating_sub(6)..];
    format!(
        "volumes/{app}/{volume}/{app}_{volume}_{}_{short_id}.tar",
        now.format("%Y-%m-%d_%H-%M-%SZ")
    )
}

/// Deletes S3 objects + rows beyond the retention window. `pre-restore`
/// safety copies are not counted, so a restore never pushes out the backup it
/// came from.
async fn enforce_retention(
    state: &SharedState,
    application_id: &str,
    volume_name: &str,
) -> Result<(), AppError> {
    let retention = read_policy(state, application_id, volume_name)
        .await?
        .retention_count
        .max(1) as usize;
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(
        "SELECT id FROM volume_backups WHERE application_id = ?1 AND volume_name = ?2 \
         AND status = 'succeeded' AND triggered_by <> 'pre-restore' ORDER BY started_at DESC",
    )
    .bind(application_id)
    .bind(volume_name)
    .fetch_all(&conn)
    .await
    .map_err(db_err)?;
    for row in rows.iter().skip(retention) {
        let id: String = row.try_get(0).map_err(db_err)?;
        // One failed delete must not abort the rest of the cleanup.
        let _ = remove_backup(state, application_id, volume_name, &id).await;
    }
    Ok(())
}

struct BackupRow {
    s3_key: Option<String>,
    s3_target_id: Option<String>,
    mount: Mount,
}

async fn backup_row(
    state: &SharedState,
    application_id: &str,
    volume_name: &str,
    backup_id: &str,
) -> Result<BackupRow, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(
        "SELECT s3_key, s3_target_id, mount_path, compose_service, status FROM volume_backups \
         WHERE id = ?1 AND application_id = ?2 AND volume_name = ?3",
    )
    .bind(backup_id)
    .bind(application_id)
    .bind(volume_name)
    .fetch_optional(&conn)
    .await
    .map_err(db_err)?
    .ok_or(AppError::ResourceNotFound)?;
    let status: String = row.try_get(4).map_err(db_err)?;
    if status == "running" {
        return Err(AppError::ValidationError(
            "This backup is still running".into(),
        ));
    }
    Ok(BackupRow {
        s3_key: row.try_get(0).map_err(db_err)?,
        s3_target_id: row.try_get(1).map_err(db_err)?,
        mount: Mount {
            path: row.try_get(2).map_err(db_err)?,
            service: row.try_get(3).map_err(db_err)?,
        },
    })
}

/// Deletes one backup: its S3 object (in the target it was uploaded to) + row.
/// History stays reachable after the volume is gone, so this does not require
/// the volume to still be mounted.
pub async fn remove_backup(
    state: &SharedState,
    application_id: &str,
    volume_name: &str,
    backup_id: &str,
) -> Result<(), AppError> {
    let row = backup_row(state, application_id, volume_name, backup_id).await?;
    let (organization_id, _) = app_org_context(state, application_id).await?;
    if let Some(key) = row.s3_key
        && let Ok(config) = crate::backup::target::resolve_recorded(
            state,
            &organization_id,
            row.s3_target_id.as_deref(),
        )
        .await
        && let Ok(bucket) = crate::backup::s3::client(&config)
    {
        let _ = bucket.delete_object(&key).await;
    }
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "DELETE FROM volume_backups WHERE id = ?1 AND application_id = ?2 AND volume_name = ?3",
    )
    .bind(backup_id)
    .bind(application_id)
    .bind(volume_name)
    .execute(&conn)
    .await
    .map_err(db_err)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Restore
// ---------------------------------------------------------------------------

fn default_restore_name(source: &str, now: chrono::DateTime<Utc>) -> String {
    // Leave room for the suffix inside the 100-character volume name limit.
    let base: String = source.chars().take(70).collect();
    format!("{base}-restore-{}", now.format("%Y%m%d-%H%M%S"))
}

async fn download(
    state: &SharedState,
    application_id: &str,
    row: &BackupRow,
) -> Result<impl AsyncRead + Unpin + Send + use<>, AppError> {
    let key = row.s3_key.as_deref().ok_or_else(|| {
        AppError::ValidationError("Only a successful backup can be restored".into())
    })?;
    let (organization_id, _) = app_org_context(state, application_id).await?;
    let config = crate::backup::target::resolve_recorded(
        state,
        &organization_id,
        row.s3_target_id.as_deref(),
    )
    .await?;
    object_reader(&*crate::backup::s3::client(&config)?, key).await
}

/// Downloads a backup to local disk, so a restore starts replacing files only
/// once the whole archive is safely here, without holding it in memory.
async fn spool_download(
    state: &SharedState,
    application_id: &str,
    row: &BackupRow,
    path: &std::path::Path,
) -> Result<(), AppError> {
    let mut file = tokio::fs::File::create(path).await.map_err(spool_error)?;
    tokio::io::copy(&mut download(state, application_id, row).await?, &mut file)
        .await
        .map_err(|error| AppError::InternalError(format!("S3 download failed: {error}")))?;
    file.flush().await.map_err(spool_error)
}

fn spool_path() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("litepod-restore-{}.tar", uuid::Uuid::now_v7()))
}

/// `podman volume import <name> -` fed from `tar`, so the archive never sits
/// in memory or in a staging file on the server.
async fn import<R: AsyncRead + Unpin + ?Sized>(
    state: &SharedState,
    application_id: &str,
    volume_name: &str,
    tar: &mut R,
) -> Result<(), AppError> {
    let command = crate::servers::runtime::ServerRuntime::for_application(state, application_id)
        .await?
        .podman_stream_with_stdin(&[
            OsString::from("volume"),
            OsString::from("import"),
            OsString::from(volume_name),
            OsString::from("-"),
        ])
        .await?;
    feed_stdin(command, tar, TRANSFER_BUDGET, "Volume import").await
}

/// Restores a succeeded backup. Synchronous like database restore: the
/// requester needs the outcome.
pub async fn restore_backup(
    state: &SharedState,
    application_id: &str,
    volume_name: &str,
    backup_id: &str,
    req: &RestoreVolumeBackupRequest,
    triggered_by: &str,
) -> Result<VolumeRestoreResultDto, AppError> {
    let row = backup_row(state, application_id, volume_name, backup_id).await?;
    let result = match req.target {
        RestoreTarget::NewVolume => {
            restore_to_new_volume(state, application_id, volume_name, &row, req)
                .await
                .map(Some)
        }
        RestoreTarget::InPlace => restore_in_place(state, application_id, volume_name, &row)
            .await
            .map(|_| None),
    };
    let outcome = match &result {
        Ok(_) => Ok(0),
        Err(error) => Err(AppError::InternalError(error.to_string())),
    };
    notify(
        state,
        application_id,
        volume_name,
        "restore",
        triggered_by,
        &outcome,
    )
    .await;
    Ok(VolumeRestoreResultDto { volume: result? })
}

/// Imports into a new detached managed volume at the backup's mount path, so
/// the files can be browsed before anything changes for the app.
async fn restore_to_new_volume(
    state: &SharedState,
    application_id: &str,
    volume_name: &str,
    row: &BackupRow,
    req: &RestoreVolumeBackupRequest,
) -> Result<crate::dtos::deployment::ManagedVolumeDto, AppError> {
    // Explicit placement (from "Add volume"), else where the backup came from.
    let mut mount = app_volume(state, application_id, volume_name)
        .await
        .unwrap_or_else(|_| row.mount.clone());
    if let Some(path) = req.mount_path.as_deref().filter(|p| !p.trim().is_empty()) {
        mount = Mount {
            path: Some(managed_volumes::normalize_mount_path(path)?),
            service: req
                .compose_service
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
        };
    }
    let name = req.name.as_deref();
    let display = managed_volumes::find_by_podman_name(state, application_id, volume_name)
        .await?
        .map(|volume| volume.name)
        .unwrap_or_else(|| volume_name.to_string());
    let name = match name.map(str::trim).filter(|n| !n.is_empty()) {
        Some(name) => managed_volumes::normalize_name(name)?,
        None => default_restore_name(&display, Utc::now()),
    };
    ensure_name_free(state, application_id, &name).await?;
    let mut tar = download(state, application_id, row).await?;
    volume_from_tar(
        state,
        application_id,
        name,
        mount.path.unwrap_or_else(|| "/data".into()),
        mount.service,
        &mut tar,
    )
    .await
}

async fn ensure_name_free(
    state: &SharedState,
    application_id: &str,
    name: &str,
) -> Result<(), AppError> {
    if managed_volumes::list(state, application_id)
        .await?
        .iter()
        .any(|volume| volume.name == name)
    {
        return Err(AppError::ValidationError(
            "A volume with that name already exists".into(),
        ));
    }
    Ok(())
}

/// Creates a detached managed volume and imports `tar` into it.
async fn volume_from_tar<R: AsyncRead + Unpin + ?Sized>(
    state: &SharedState,
    application_id: &str,
    name: String,
    mount_path: String,
    compose_service: Option<String>,
    tar: &mut R,
) -> Result<crate::dtos::deployment::ManagedVolumeDto, AppError> {
    let restored = managed_volumes::provision(
        state,
        application_id,
        name,
        mount_path,
        compose_service,
        false,
    )
    .await?;
    let podman_name = managed_volumes::get_with_podman_name(state, application_id, &restored.id)
        .await?
        .1;
    if let Err(error) = import(state, application_id, &podman_name, tar).await {
        // Do not leave a half-filled volume behind that looks like a good restore.
        let _ = managed_volumes::delete(state, application_id, &restored.id).await;
        return Err(error);
    }
    Ok(restored)
}

/// Validates the placement of a volume created from an outside `.tar`.
async fn new_volume_placement(
    state: &SharedState,
    application_id: &str,
    req: &NewVolumeFromTarRequest,
) -> Result<(String, String, Option<String>), AppError> {
    if app_is_group(state, application_id).await? {
        return Err(AppError::ValidationError(
            "Volume backups are not available for applications on a server group yet".into(),
        ));
    }
    let name = managed_volumes::normalize_name(&req.name)?;
    ensure_name_free(state, application_id, &name).await?;
    let mount_path = managed_volumes::normalize_mount_path(&req.mount_path)?;
    let service = req
        .compose_service
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    Ok((name, mount_path, service))
}

async fn app_is_group(state: &SharedState, application_id: &str) -> Result<bool, AppError> {
    Ok(
        crate::servers::replica::target_server_id(state, application_id)
            .await?
            .is_none(),
    )
}

/// `.tar` objects in an organization's S3 destination (`None`: the default
/// storage), newest first, so a volume can come from a backup this install
/// did not record (another server, an older install).
pub async fn list_s3_tars(
    state: &SharedState,
    application_id: &str,
    s3_target_id: Option<&str>,
) -> Result<Vec<S3TarDto>, AppError> {
    let (organization_id, _) = app_org_context(state, application_id).await?;
    let config = crate::backup::target::resolve(state, &organization_id, s3_target_id).await?;
    let pages = crate::backup::s3::client(&config)?
        .list(String::new(), None)
        .await
        .map_err(|error| {
            AppError::ValidationError(format!("Could not list the bucket: {error}"))
        })?;
    let mut tars: Vec<S3TarDto> = pages
        .into_iter()
        .flat_map(|page| page.contents)
        .filter(|object| object.key.ends_with(".tar"))
        .map(|object| S3TarDto {
            key: object.key,
            size_bytes: object.size as i64,
            last_modified: object.last_modified,
        })
        .collect();
    // RFC 3339 strings sort by time.
    tars.sort_by(|a, b| b.last_modified.cmp(&a.last_modified));
    tars.truncate(500);
    Ok(tars)
}

/// Creates a detached volume from a `.tar` in one of the organization's S3
/// destinations. `resolve` refuses another organization's target.
pub async fn restore_from_s3(
    state: &SharedState,
    application_id: &str,
    req: &RestoreFromS3Request,
) -> Result<crate::dtos::deployment::ManagedVolumeDto, AppError> {
    if !req.key.ends_with(".tar") {
        return Err(AppError::ValidationError("Choose a .tar file".into()));
    }
    let (name, mount_path, service) =
        new_volume_placement(state, application_id, &req.volume).await?;
    let (organization_id, _) = app_org_context(state, application_id).await?;
    let config =
        crate::backup::target::resolve(state, &organization_id, req.s3_target_id.as_deref())
            .await?;
    let mut tar = object_reader(&*crate::backup::s3::client(&config)?, &req.key).await?;
    volume_from_tar(state, application_id, name, mount_path, service, &mut tar).await
}

/// Creates a detached volume from an uploaded `.tar` (`podman volume export`
/// format, or any plain tar of the files). A file that is not a tar fails the
/// import and leaves nothing behind.
pub async fn restore_from_upload(
    state: &SharedState,
    application_id: &str,
    req: &NewVolumeFromTarRequest,
    tar: Vec<u8>,
) -> Result<crate::dtos::deployment::ManagedVolumeDto, AppError> {
    if tar.is_empty() {
        return Err(AppError::ValidationError(
            "The uploaded file is empty".into(),
        ));
    }
    let (name, mount_path, service) = new_volume_placement(state, application_id, req).await?;
    volume_from_tar(
        state,
        application_id,
        name,
        mount_path,
        service,
        &mut tar.as_slice(),
    )
    .await
}

/// Replaces the original volume's contents. Order matters for data safety:
/// download first to local disk (nothing changed if S3 fails), stop the app,
/// save the current contents as a `pre-restore` backup (abort if that fails),
/// empty the volume, import, and always start the app again.
async fn restore_in_place(
    state: &SharedState,
    application_id: &str,
    volume_name: &str,
    row: &BackupRow,
) -> Result<(), AppError> {
    let mount = app_volume(state, application_id, volume_name).await?;
    let spool = spool_path();
    let result = async {
        spool_download(state, application_id, row, &spool).await?;
        replace_contents(state, application_id, volume_name, mount, &spool).await
    }
    .await;
    let _ = tokio::fs::remove_file(&spool).await;
    result
}

/// The part of [`restore_in_place`] that runs once the archive is on disk.
async fn replace_contents(
    state: &SharedState,
    application_id: &str,
    volume_name: &str,
    mount: Mount,
    spool: &std::path::Path,
) -> Result<(), AppError> {
    let policy = read_policy(state, application_id, volume_name).await?;
    let (organization_id, _) = app_org_context(state, application_id).await?;
    crate::backup::target::resolve(state, &organization_id, policy.s3_target_id.as_deref()).await?;
    // Claim before stopping so a concurrent backup cannot race the restore.
    let safety = claim_run(
        state,
        application_id,
        volume_name,
        &mount,
        "pre-restore",
        policy.s3_target_id.as_deref(),
        None,
    )
    .await?;

    let mut was_running = false;
    let replaced = async {
        match stop_if_running(state, application_id).await {
            Ok(running) => was_running = running,
            Err(error) => {
                let message =
                    format!("Restore cancelled: the application could not be stopped ({error})");
                finish_run(
                    state,
                    &safety.id,
                    &Err(AppError::InternalError(message.clone())),
                )
                .await;
                return Err(AppError::InternalError(message));
            }
        }
        let saved = export_and_upload(
            state,
            application_id,
            volume_name,
            &safety.id,
            policy.s3_target_id.as_deref(),
            // The app is already stopped here.
            false,
        )
        .await;
        finish_run(state, &safety.id, &saved).await;
        saved.map_err(|error| {
            AppError::InternalError(format!(
                "Restore cancelled: the current files could not be saved first ({error})"
            ))
        })?;
        empty_volume(state, application_id, volume_name).await?;
        let mut tar = tokio::fs::File::open(spool).await.map_err(spool_error)?;
        import(state, application_id, volume_name, &mut tar).await
    }
    .await;
    // Started even after a failure: the app must not stay down because of us.
    // An app the user had stopped stays stopped.
    let started = if was_running {
        crate::deployment::start(state, application_id, None)
            .await
            .map(|_| ())
    } else {
        Ok(())
    };
    replaced?;
    started
}

/// Stops the application when it is running; returns whether it was, so the
/// caller starts it again only in that case.
async fn stop_if_running(state: &SharedState, application_id: &str) -> Result<bool, AppError> {
    if crate::deployment::deployment(state, application_id)
        .await?
        .status
        != "running"
    {
        return Ok(false);
    }
    crate::deployment::stop(state, application_id, None).await?;
    Ok(true)
}

/// Deletes everything inside the volume with the same throwaway helper
/// container the file browser uses (`podman volume import` only adds files).
async fn empty_volume(
    state: &SharedState,
    application_id: &str,
    volume_name: &str,
) -> Result<(), AppError> {
    crate::servers::runtime::ServerRuntime::for_application(state, application_id)
        .await?
        .podman_with_timeout(
            &[
                OsString::from("run"),
                OsString::from("--rm"),
                OsString::from("-v"),
                remote_volume_mount(volume_name)?,
                OsString::from(HELPER_IMAGE),
                OsString::from("find"),
                OsString::from("/workspace"),
                OsString::from("-mindepth"),
                OsString::from("1"),
                OsString::from("-delete"),
            ],
            TRANSFER_BUDGET,
        )
        .await
        .map(|_| ())
        .map_err(|error| AppError::InternalError(format!("Could not empty the volume: {error}")))
}

// ---------------------------------------------------------------------------
// Whole application: every named volume in one run
// ---------------------------------------------------------------------------

/// The named volumes the application mounts right now, one per name.
async fn live_named_volumes(
    state: &SharedState,
    application_id: &str,
) -> Result<Vec<(String, Mount)>, AppError> {
    if app_is_group(state, application_id).await? {
        return Err(AppError::ValidationError(
            "Volume backups are not available for applications on a server group yet".into(),
        ));
    }
    let mut volumes: Vec<(String, Mount)> = Vec::new();
    for mount in crate::deployment::volumes(state, application_id).await? {
        if mount.mount_type == "volume" && !volumes.iter().any(|(name, _)| *name == mount.source) {
            volumes.push((
                mount.source,
                Mount {
                    path: Some(mount.destination),
                    service: mount.services.into_iter().next(),
                },
            ));
        }
    }
    if volumes.is_empty() {
        return Err(AppError::ValidationError(
            "The application mounts no named volumes right now. Start it, then try again".into(),
        ));
    }
    Ok(volumes)
}

/// Claims a `running` row per volume under one `group_id`; if any volume is
/// busy, the rows already claimed are released as failed and nothing runs.
async fn claim_group(
    state: &SharedState,
    application_id: &str,
    volumes: &[(String, Mount, Option<String>)],
    triggered_by: &str,
    group_id: &str,
) -> Result<Vec<VolumeBackupDto>, AppError> {
    let mut claimed = Vec::new();
    for (volume_name, mount, s3_target_id) in volumes {
        match claim_run(
            state,
            application_id,
            volume_name,
            mount,
            triggered_by,
            s3_target_id.as_deref(),
            Some(group_id),
        )
        .await
        {
            Ok(dto) => claimed.push(dto),
            Err(error) => {
                let reason = AppError::ValidationError(format!(
                    "Cancelled: {} is busy ({error})",
                    volume_label(volume_name)
                ));
                for dto in &claimed {
                    finish_run(
                        state,
                        &dto.id,
                        &Err(AppError::InternalError(reason.to_string())),
                    )
                    .await;
                }
                return Err(reason);
            }
        }
    }
    Ok(claimed)
}

/// Every backup of the application, newest first. Rows of one run share
/// `group_id`; rows without one (pre-restore copies, older runs) stand alone.
pub async fn list_app_runs(
    state: &SharedState,
    application_id: &str,
) -> Result<AppVolumeBackupsDto, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {BACKUP_COLUMNS} FROM volume_backups \
         WHERE application_id = ?1 ORDER BY started_at DESC LIMIT 300"
    )))
    .bind(application_id)
    .fetch_all(&conn)
    .await
    .map_err(db_err)?;
    Ok(AppVolumeBackupsDto {
        default_available: !state.is_cloud && crate::backup::config::load(state).await?.is_some(),
        runs: rows.iter().map(read_backup_row).collect::<Result<_, _>>()?,
    })
}

/// Backs up the picked named volumes the application mounts (all of them when
/// `only` is empty) in one run, stopping the app once around all the exports
/// when `stop_app`. Returns the `group_id`.
/// Volumes are exported and uploaded one at a time, so memory holds one tar.
pub async fn request_app_run(
    state: &SharedState,
    application_id: &str,
    s3_target_id: Option<&str>,
    stop_app: bool,
    only: &[String],
    triggered_by: &str,
) -> Result<String, AppError> {
    let mut volumes = live_named_volumes(state, application_id).await?;
    // Trust boundary: names come from the request; only the app's live volumes count.
    if let Some(unknown) = only
        .iter()
        .find(|name| !volumes.iter().any(|(live, _)| live == *name))
    {
        return Err(AppError::ValidationError(format!(
            "{} is not a named volume this application mounts",
            volume_label(unknown)
        )));
    }
    if !only.is_empty() {
        volumes.retain(|(name, _)| only.contains(name));
    }
    run_group(state, application_id, volumes, s3_target_id, stop_app, triggered_by).await
}

/// The scheduler's entry: every due volume of the application in one run, with
/// the first policy's destination and stop switch (the UI saves the same
/// settings on every volume). A stopped app has no live mounts, so the volumes
/// are not re-checked against them; the policies were authorized when saved.
pub async fn request_scheduled_run(
    state: &SharedState,
    application_id: &str,
    volume_names: &[String],
) -> Result<String, AppError> {
    let first = volume_names.first().ok_or(AppError::ResourceNotFound)?;
    let policy = read_policy(state, application_id, first).await?;
    let mut volumes = Vec::new();
    for name in volume_names {
        let mount = app_volume(state, application_id, name)
            .await
            .unwrap_or_default();
        volumes.push((name.clone(), mount));
    }
    run_group(
        state,
        application_id,
        volumes,
        policy.s3_target_id.as_deref(),
        policy.stop_app,
        "scheduled",
    )
    .await
}

async fn run_group(
    state: &SharedState,
    application_id: &str,
    volumes: Vec<(String, Mount)>,
    s3_target_id: Option<&str>,
    stop_app: bool,
    triggered_by: &str,
) -> Result<String, AppError> {
    let target = s3_target_id.map(str::trim).filter(|id| !id.is_empty());
    // `resolve` also refuses another organization's target.
    let (organization_id, _) = app_org_context(state, application_id).await?;
    crate::backup::target::resolve(state, &organization_id, target).await?;
    let group_id = uuid::Uuid::now_v7().to_string();
    let with_target: Vec<_> = volumes
        .into_iter()
        .map(|(name, mount)| (name, mount, target.map(str::to_string)))
        .collect();
    let claimed = claim_group(state, application_id, &with_target, triggered_by, &group_id).await?;

    let state = state.clone();
    let application_id = application_id.to_string();
    let target = target.map(str::to_string);
    let triggered_by = triggered_by.to_string();
    tokio::spawn(async move {
        let was_running = match stop_app {
            true => stop_if_running(&state, &application_id).await,
            false => Ok(false),
        };
        let mut total = 0;
        let mut failures = Vec::new();
        for dto in &claimed {
            let outcome = match &was_running {
                Err(error) => Err(AppError::InternalError(format!(
                    "The application could not be stopped: {error}"
                ))),
                Ok(_) => {
                    export_and_upload(
                        &state,
                        &application_id,
                        &dto.volume_name,
                        &dto.id,
                        target.as_deref(),
                        false,
                    )
                    .await
                }
            };
            finish_run(&state, &dto.id, &outcome).await;
            match outcome {
                Ok((size, _)) => total += size,
                Err(error) => failures.push(format!("{}: {error}", volume_label(&dto.volume_name))),
            }
        }
        if let Ok(true) = was_running
            && let Err(error) = crate::deployment::start(&state, &application_id, None).await
        {
            tracing::warn!(%application_id, %error, "could not start the application again after a volume backup");
        }
        for dto in &claimed {
            let _ = enforce_retention(&state, &application_id, &dto.volume_name).await;
        }
        let outcome = if failures.is_empty() {
            Ok(total)
        } else {
            Err(AppError::InternalError(failures.join("; ")))
        };
        let label = format!("all volumes ({})", claimed.len());
        notify(
            &state,
            &application_id,
            &label,
            "backup",
            &triggered_by,
            &outcome,
        )
        .await;
    });
    Ok(group_id)
}

/// Restores every volume of a whole-application run in place. Data safety
/// order: download every tar to local disk first (nothing changes if one is
/// missing), stop
/// the app once, save every volume's current files as a `pre-restore` run
/// (abort before replacing anything if one fails), then empty + import each,
/// and start the app again if it was running.
pub async fn restore_app_run(
    state: &SharedState,
    application_id: &str,
    group_id: &str,
    triggered_by: &str,
) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(
        "SELECT id, volume_name, status FROM volume_backups \
         WHERE application_id = ?1 AND group_id = ?2",
    )
    .bind(application_id)
    .bind(group_id)
    .fetch_all(&conn)
    .await
    .map_err(db_err)?;
    if rows.is_empty() {
        return Err(AppError::ResourceNotFound);
    }
    // Completeness first, so a partial run gets this message and not a lookup error.
    for row in &rows {
        if row.try_get::<String, _>(2).map_err(db_err)? != "succeeded" {
            return Err(AppError::ValidationError(
                "Not every volume of this run was backed up; restore the volumes one by one instead"
                    .into(),
            ));
        }
    }
    let mut members = Vec::new();
    for row in &rows {
        let id: String = row.try_get(0).map_err(db_err)?;
        let volume_name: String = row.try_get(1).map_err(db_err)?;
        let mount = app_volume(state, application_id, &volume_name).await?;
        let backup = backup_row(state, application_id, &volume_name, &id).await?;
        members.push((volume_name, mount, backup));
    }

    let spools: Vec<_> = members.iter().map(|_| spool_path()).collect();
    let result = async {
        for ((_, _, backup), spool) in members.iter().zip(&spools) {
            spool_download(state, application_id, backup, spool).await?;
        }
        let safety_group = uuid::Uuid::now_v7().to_string();
        let safety_set: Vec<_> = members
            .iter()
            .map(|(name, mount, backup)| (name.clone(), mount.clone(), backup.s3_target_id.clone()))
            .collect();
        let (organization_id, _) = app_org_context(state, application_id).await?;
        for (_, _, target) in &safety_set {
            crate::backup::target::resolve(state, &organization_id, target.as_deref()).await?;
        }
        let safety = claim_group(state, application_id, &safety_set, "pre-restore", &safety_group).await?;

        let was_running = match stop_if_running(state, application_id).await {
            Ok(running) => running,
            Err(error) => {
                let message = format!("Restore cancelled: the application could not be stopped ({error})");
                for dto in &safety {
                    finish_run(state, &dto.id, &Err(AppError::InternalError(message.clone()))).await;
                }
                return Err(AppError::InternalError(message));
            }
        };
        let replaced = async {
            for (index, dto) in safety.iter().enumerate() {
                let saved = export_and_upload(
                    state,
                    application_id,
                    &dto.volume_name,
                    &dto.id,
                    safety_set[index].2.as_deref(),
                    false,
                )
                .await;
                finish_run(state, &dto.id, &saved).await;
                if let Err(error) = saved {
                    for rest in &safety[index + 1..] {
                        finish_run(state, &rest.id, &Err(AppError::InternalError("Cancelled".into()))).await;
                    }
                    return Err(AppError::InternalError(format!(
                        "Restore cancelled, nothing was replaced: the current files of {} could not be saved first ({error})",
                        volume_label(&dto.volume_name)
                    )));
                }
            }
            let mut done = Vec::new();
            for ((volume_name, _, _), spool) in members.iter().zip(&spools) {
                let replaced = async {
                    empty_volume(state, application_id, volume_name).await?;
                    let mut tar = tokio::fs::File::open(spool).await.map_err(spool_error)?;
                    import(state, application_id, volume_name, &mut tar).await
                }
                .await;
                if let Err(error) = replaced {
                    return Err(AppError::InternalError(format!(
                        "{} could not be restored ({error}). Restored so far: {}. Every volume's previous files are in the \"before restore\" run",
                        volume_label(volume_name),
                        if done.is_empty() { "none".to_string() } else { done.join(", ") }
                    )));
                }
                done.push(volume_label(volume_name).to_string());
            }
            Ok(())
        }
        .await;
        // Started even after a failure; an app the user had stopped stays stopped.
        let started = if was_running {
            crate::deployment::start(state, application_id, None).await.map(|_| ())
        } else {
            Ok(())
        };
        replaced?;
        started
    }
    .await;
    for spool in &spools {
        let _ = tokio::fs::remove_file(spool).await;
    }
    let outcome = match &result {
        Ok(_) => Ok(0),
        Err(error) => Err(AppError::InternalError(error.to_string())),
    };
    let label = format!("all volumes ({})", members.len());
    notify(
        state,
        application_id,
        &label,
        "restore",
        triggered_by,
        &outcome,
    )
    .await;
    result
}

// ---------------------------------------------------------------------------
// Notifications
// ---------------------------------------------------------------------------

async fn notify(
    state: &SharedState,
    application_id: &str,
    volume_name: &str,
    operation: &str,
    triggered_by: &str,
    outcome: &Result<i64, AppError>,
) {
    let Ok((organization_id, application_name)) = app_org_context(state, application_id).await
    else {
        return;
    };
    let error = outcome.as_ref().err().map(ToString::to_string);
    crate::notifications::emit::notify_volume_backup(
        state,
        &organization_id,
        &application_name,
        volume_name,
        operation,
        outcome.is_ok(),
        triggered_by,
        outcome.as_ref().ok().copied().filter(|size| *size > 0),
        error.as_deref(),
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{cleanup_database, seed_application, test_state_for};
    use chrono::TimeZone;

    async fn insert_managed_volume(state: &SharedState, application_id: &str, podman_name: &str) {
        let conn = crate::git::repositories::db::get_conn(state).await;
        sqlx::query(
            "INSERT INTO application_managed_volumes (id, application_id, name, podman_name, mount_path, attached, created_at, updated_at) \
             VALUES (?1, ?2, 'uploads', ?1, '/data', 0, 1, 1)",
        )
        .bind(podman_name)
        .bind(application_id)
        .execute(&conn)
        .await
        .unwrap();
    }

    fn policy(enabled: bool, retention_count: i64) -> UpdateVolumeBackupPolicyRequest {
        UpdateVolumeBackupPolicyRequest {
            enabled,
            cron_expression: "0 0 3 * * *".into(),
            retention_count,
            s3_target_id: None,
            stop_app: false,
        }
    }

    #[tokio::test]
    async fn only_the_apps_own_volumes_get_a_policy() {
        let (state, path) = test_state_for(false).await;
        let (_, _, app) = seed_application(&state, "organization-a").await;
        let (_, _, other_app) = seed_application(&state, "organization-b").await;
        insert_managed_volume(&state, &app, "litepod-managed-1").await;

        let created = get_policy(&state, &app, "litepod-managed-1").await.unwrap();
        assert!(!created.enabled);
        assert!(
            !created.default_available,
            "no default S3 storage in a fresh install"
        );
        assert_eq!(created.retention_count, 7);
        for (app_id, volume) in [
            (&other_app, "litepod-managed-1"),
            (&app, "someone-elses-volume"),
            (&app, "../../etc"),
        ] {
            assert!(
                matches!(
                    get_policy(&state, app_id, volume).await,
                    Err(AppError::ResourceNotFound)
                ),
                "{volume}"
            );
        }
        assert!(matches!(
            set_policy(&state, &app, "litepod-managed-1", &policy(false, 0), None).await,
            Err(AppError::ValidationError(_))
        ));
        let saved = set_policy(
            &state,
            &app,
            "litepod-managed-1",
            &policy(true, 3),
            Some("me"),
        )
        .await
        .unwrap();
        assert!(saved.enabled && saved.next_run_at.is_some());
        assert_eq!(saved.retention_count, 3);
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn outside_tars_need_a_valid_free_placement() {
        let (state, path) = test_state_for(false).await;
        let (_, _, app) = seed_application(&state, "organization-a").await;
        insert_managed_volume(&state, &app, "litepod-managed-1").await;
        let request = |name: &str, mount_path: &str| NewVolumeFromTarRequest {
            name: name.into(),
            mount_path: mount_path.into(),
            compose_service: None,
        };
        let tar = b"not empty".to_vec();
        for (req, bytes) in [
            (request("fresh", "/data"), Vec::new()),
            (request("fresh", "relative/path"), tar.clone()),
            (request("fresh", "/"), tar.clone()),
            (request("uploads", "/data"), tar.clone()),
        ] {
            let result = restore_from_upload(&state, &app, &req, bytes).await;
            assert!(
                matches!(result, Err(AppError::ValidationError(_))),
                "{} {}: {result:?}",
                req.name,
                req.mount_path
            );
        }
        let not_a_tar = RestoreFromS3Request {
            s3_target_id: None,
            key: "volumes/app/secrets.env".into(),
            volume: request("fresh", "/data"),
        };
        assert!(matches!(
            restore_from_s3(&state, &app, &not_a_tar).await,
            Err(AppError::ValidationError(_))
        ));
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn whole_app_runs_restore_only_when_complete_and_claims_are_all_or_nothing() {
        let (state, path) = test_state_for(false).await;
        let (_, _, app) = seed_application(&state, "organization-a").await;
        let conn = crate::git::repositories::db::get_conn(&state).await;
        for (id, volume, status) in [("b1", "vol-a", "succeeded"), ("b2", "vol-b", "failed")] {
            sqlx::query(
                "INSERT INTO volume_backups (id, application_id, volume_name, status, triggered_by, started_at, group_id) \
                 VALUES (?1, ?2, ?3, ?4, 'me', 1, 'g1')",
            )
            .bind(id)
            .bind(&app)
            .bind(volume)
            .bind(status)
            .execute(&conn)
            .await
            .unwrap();
        }
        assert!(matches!(
            restore_app_run(&state, &app, "missing", "me").await,
            Err(AppError::ResourceNotFound)
        ));
        assert!(matches!(
            restore_app_run(&state, &app, "g1", "me").await,
            Err(AppError::ValidationError(_))
        ));
        let runs = list_app_runs(&state, &app).await.unwrap();
        assert_eq!(runs.runs.len(), 2);
        assert!(
            runs.runs
                .iter()
                .all(|run| run.group_id.as_deref() == Some("g1"))
        );

        // vol-y is busy: vol-x's claim is released, nothing stays `running`.
        let busy = Mount::default();
        claim_run(&state, &app, "vol-y", &busy, "me", None, None)
            .await
            .unwrap();
        let set = vec![
            ("vol-x".to_string(), Mount::default(), None),
            ("vol-y".to_string(), Mount::default(), None),
        ];
        assert!(claim_group(&state, &app, &set, "me", "g2").await.is_err());
        let running: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM volume_backups WHERE group_id = 'g2' AND status = 'running'",
        )
        .fetch_one(&conn)
        .await
        .unwrap();
        assert_eq!(running, 0);
        cleanup_database(&path);
    }

    #[test]
    fn object_keys_are_readable_and_dated() {
        let now = Utc.with_ymd_and_hms(2026, 11, 20, 3, 4, 5).unwrap();
        assert_eq!(
            object_key(
                "forgejo-postgres-bx69y31c",
                volume_label("app-ac9c533011c3-compose_forgejo_data"),
                now,
                "019a-0000-7000-8000-abcdef123456",
            ),
            "volumes/forgejo-postgres-bx69y31c/forgejo_data/forgejo-postgres-bx69y31c_forgejo_data_2026-11-20_03-04-05Z_123456.tar"
        );
        assert_eq!(volume_label("plain_volume"), "plain_volume");
        assert_eq!(key_segment("Mi App / uploads"), "Mi-App---uploads");
        assert_eq!(key_segment("ñ"), "volume");
    }

    #[test]
    fn default_restore_name_fits_the_volume_name_limit() {
        let now = Utc.with_ymd_and_hms(2026, 11, 20, 3, 4, 5).unwrap();
        assert_eq!(
            default_restore_name("uploads", now),
            "uploads-restore-20261120-030405"
        );
        let name = default_restore_name(&"x".repeat(100), now);
        assert!(
            managed_volumes::normalize_name(&name).is_ok(),
            "{} chars",
            name.len()
        );
    }
}
