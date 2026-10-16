use std::io;
use std::path::Path;

use chrono::Utc;
use serde::Serialize;
use uuid::Uuid;

use crate::backup::{config, s3 as s3_client};
use crate::dtos::response::AppError;
use crate::state::state::SharedState;

#[derive(Debug, Serialize)]
pub struct BackupRecordDto {
    pub id: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub status: String,
    pub size_bytes: Option<i64>,
    pub s3_key: Option<String>,
    pub error_message: Option<String>,
    pub triggered_by: String,
    pub created_by: Option<String>,
}

const COLUMNS: &str = "id, started_at, finished_at, status, size_bytes, s3_key, error_message, triggered_by, created_by";

fn row_to_record(row: &sqlx::sqlite::SqliteRow) -> Result<BackupRecordDto, AppError> {
    use sqlx::Row;
    Ok(BackupRecordDto {
        id: row
            .try_get(0)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        started_at: row
            .try_get(1)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        finished_at: row
            .try_get(2)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        status: row
            .try_get(3)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        size_bytes: row
            .try_get(4)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        s3_key: row
            .try_get(5)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        error_message: row
            .try_get(6)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        triggered_by: row
            .try_get(7)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        created_by: row
            .try_get(8)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
    })
}

pub async fn list(state: &SharedState) -> Result<Vec<BackupRecordDto>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(&format!(
        "SELECT {COLUMNS} FROM backup_history ORDER BY started_at DESC"
    ))
    .fetch_all(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    rows.iter().map(row_to_record).collect()
}

pub async fn get(state: &SharedState, id: &str) -> Result<Option<BackupRecordDto>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let Some(row) = sqlx::query(&format!(
        "SELECT {COLUMNS} FROM backup_history WHERE id = ?1"
    ))
    .bind(id)
    .fetch_optional(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?
    else {
        return Ok(None);
    };
    Ok(Some(row_to_record(&row)?))
}

async fn insert_running(
    state: &SharedState,
    id: &str,
    started_at: i64,
    triggered_by: &str,
    created_by: Option<&str>,
) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "INSERT INTO backup_history (id, started_at, status, triggered_by, created_by) VALUES (?1, ?2, 'running', ?3, ?4)",
    )
    .bind(id)
    .bind(started_at)
    .bind(triggered_by)
    .bind(created_by)
    .execute(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok(())
}

async fn finish(
    state: &SharedState,
    id: &str,
    status: &str,
    size_bytes: Option<i64>,
    s3_key: Option<&str>,
    error_message: Option<&str>,
) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE backup_history SET status = ?1, finished_at = ?2, size_bytes = ?3, s3_key = ?4, error_message = ?5 WHERE id = ?6",
    )
    .bind(status)
    .bind(Utc::now().timestamp_millis())
    .bind(size_bytes)
    .bind(s3_key)
    .bind(error_message)
    .bind(id)
    .execute(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok(())
}

/// Deploy pipelines live in a `tokio::spawn`, so a restart kills them without
/// ever writing a terminal status. Anything still `running` at boot is a
/// leftover from the previous process, not work in flight. Mirrors
/// `deployment::history::fail_interrupted`.
pub async fn fail_interrupted(state: &SharedState) -> Result<(), AppError> {
    const MESSAGE: &str = "Backup was interrupted because the server restarted";
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE backup_history SET status = 'failed', error_message = ?1, finished_at = ?2 WHERE status = 'running'",
    )
    .bind(MESSAGE)
    .bind(Utc::now().timestamp_millis())
    .execute(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok(())
}

/// Validates the S3 config exists, claims the backup/restore lock, inserts
/// the `running` row, and spawns the actual pipeline in the background so
/// the HTTP request returns immediately instead of blocking for the full
/// stop/tar/upload/start cycle.
pub async fn start(
    state: &SharedState,
    triggered_by: &str,
    created_by: Option<String>,
) -> Result<BackupRecordDto, AppError> {
    let Some(s3_config) = config::load(state).await? else {
        return Err(AppError::ValidationError(
            "Configure S3 storage before running a backup".into(),
        ));
    };
    let guard = state.backup_lock.clone().try_lock_owned().map_err(|_| {
        AppError::ResourceInUse("A backup or restore is already in progress".into())
    })?;

    let id = Uuid::now_v7().to_string();
    let started_at = Utc::now().timestamp_millis();
    insert_running(state, &id, started_at, triggered_by, created_by.as_deref()).await?;
    let record = get(state, &id).await?.ok_or(AppError::ResourceNotFound)?;

    let state_bg = state.clone();
    let id_bg = id.clone();
    let triggered_by_bg = triggered_by.to_string();
    tokio::spawn(async move {
        let _guard = guard;
        let outcome = upload_snapshot(&state_bg, &s3_config, &id_bg).await;
        let (status, size_bytes, s3_key, error_message) = match outcome {
            Ok((size_bytes, s3_key)) => ("success", Some(size_bytes as i64), Some(s3_key), None),
            Err(error) => {
                tracing::error!(backup_id = %id_bg, error = %error, "backup failed");
                ("failed", None, None, Some(error.to_string()))
            }
        };
        if let Err(error) = finish(
            &state_bg,
            &id_bg,
            status,
            size_bytes,
            s3_key.as_deref(),
            error_message.as_deref(),
        )
        .await
        {
            tracing::error!(backup_id = %id_bg, error = %error, "could not record backup outcome");
        }
        crate::notifications::notify_backup_finished(
            &state_bg,
            status == "success",
            &triggered_by_bg,
            size_bytes,
            s3_key.as_deref(),
            Some(Utc::now().timestamp_millis() - started_at),
            error_message.as_deref(),
        )
        .await;
    });

    Ok(record)
}

async fn upload_snapshot(
    state: &SharedState,
    config: &config::S3Config,
    id: &str,
) -> Result<(usize, String), AppError> {
    // Dump through the embedded Turso connection rather than a helper
    // container. This keeps backups inside the API process: no database stop,
    // no CLI install race. Compose files and application artifacts come from
    // data_dir; the stack .env comes from its mounted stack path.
    let dump_bytes = dump_db_to_sql(state).await?;
    let data_dir = state.data_dir.clone();
    let stack_env_file = state.stack_env_file.clone();
    let metadata_json = build_backup_metadata(id)
        .map_err(|error| AppError::InternalError(format!("Backup metadata encode: {error}")))?;
    let tar_bytes = tokio::task::spawn_blocking(move || {
        build_backup_tar(
            &dump_bytes,
            &metadata_json,
            &data_dir,
            stack_env_file.as_deref(),
        )
    })
    .await
    .map_err(|error| AppError::InternalError(format!("Backup tar build panicked: {error}")))??;

    let bucket = s3_client::client(config)?;
    let stamp = Utc::now().format("%Y-%m-%dT%H-%M-%SZ").to_string();
    let s3_key = format!("backups/{stamp}_{id}.tar");
    let reader = io::Cursor::new(tar_bytes);
    let mut reader = tokio::io::BufReader::new(reader);
    let response = bucket
        .put_object_stream(&mut reader, &s3_key)
        .await
        .map_err(|error| AppError::InternalError(format!("S3 upload failed: {error}")))?;
    Ok((response.uploaded_bytes(), s3_key))
}

/// Walks `sqlite_master` to emit a `.dump`-compatible SQL script. Format is
/// the same one the `sqlite3` CLI produces: `PRAGMA foreign_keys=OFF; BEGIN;
/// <CREATE statements for tables / indexes / triggers / views>;
/// <INSERT statements for each table's data>; COMMIT;`. The output is
/// safe to replay via `sqlite3 < dump.sql` against an empty database.
///
/// Every read runs inside one read transaction, so the dump is a single
/// consistent snapshot: a write landing mid-dump can't leave a child row
/// whose parent was read before it existed.
pub(crate) async fn dump_db_to_sql(state: &SharedState) -> Result<Vec<u8>, AppError> {
    use sqlx::Row;
    use std::io::Write;

    let pool = crate::git::repositories::db::get_conn(state).await;
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let mut out = Vec::new();
    out.extend_from_slice(b"PRAGMA foreign_keys=OFF;\nBEGIN TRANSACTION;\n");

    let rows = sqlx::query(
        "SELECT type, name, sql FROM sqlite_master \
         WHERE name NOT LIKE 'sqlite_%' AND sql IS NOT NULL \
         ORDER BY CASE type \
            WHEN 'table' THEN 1 WHEN 'index' THEN 2 \
            WHEN 'trigger' THEN 3 WHEN 'view' THEN 4 ELSE 5 END, name",
    )
    .fetch_all(&mut *tx)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;

    for row in &rows {
        let obj_type: String = row
            .try_get(0)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        let name: String = row
            .try_get(1)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        let sql: String = row
            .try_get(2)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;

        writeln!(out, "{};", sql)
            .map_err(|e| AppError::InternalError(format!("dump write: {e}")))?;

        if obj_type == "table" {
            dump_table_data(&mut tx, &mut out, &name).await?;
        }
    }

    out.extend_from_slice(b"COMMIT;\n");
    // Read-only: rolling back just releases the snapshot.
    tx.rollback()
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok(out)
}

async fn dump_table_data(
    conn: &mut sqlx::SqliteConnection,
    out: &mut Vec<u8>,
    name: &str,
) -> Result<(), AppError> {
    use sqlx::{Row, TypeInfo, ValueRef};
    use std::io::Write;

    let escaped = name.replace('"', "\"\"");
    let rows = sqlx::query(&format!("SELECT * FROM \"{escaped}\""))
        .fetch_all(&mut *conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    for row in &rows {
        let count = row.columns().len();
        let mut values = Vec::with_capacity(count);
        for i in 0..count {
            let raw = row
                .try_get_raw(i)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?;
            values.push(if raw.is_null() {
                "NULL".to_string()
            } else {
                match raw.type_info().name() {
                    "INTEGER" | "BOOLEAN" => {
                        let value: i64 = row
                            .try_get(i)
                            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
                        value.to_string()
                    }
                    "REAL" | "FLOAT" | "DOUBLE" => {
                        let value: f64 = row
                            .try_get(i)
                            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
                        format!("{value}")
                    }
                    "BLOB" => {
                        let value: Vec<u8> = row
                            .try_get(i)
                            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
                        format_sql_blob(&value)
                    }
                    // TEXT, and anything else SQLite's dynamic typing hands
                    // back (a column's declared type is only affinity, not
                    // a guarantee) — decode as text, matching turso's own
                    // fallback behavior for unrecognized value kinds.
                    _ => {
                        let value: String = row
                            .try_get(i)
                            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
                        format!("'{}'", value.replace('\'', "''"))
                    }
                }
            });
        }
        writeln!(
            out,
            "INSERT INTO \"{escaped}\" VALUES({});",
            values.join(",")
        )
        .map_err(|e| AppError::InternalError(format!("dump write: {e}")))?;
    }
    Ok(())
}

fn format_sql_blob(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut hex = String::with_capacity(bytes.len() * 2 + 3);
    hex.push_str("X'");
    for byte in bytes {
        let _ = write!(hex, "{byte:02x}");
    }
    hex.push('\'');
    hex
}

#[derive(Debug, Serialize)]
struct BackupMetadata {
    schema: u32,
    backup_id: String,
    created_at: String,
    litepod_version: String,
    litepod_git_sha: String,
    api_version: &'static str,
    deployment_mode: &'static str,
    is_self_hosted: bool,
    caddy_image: String,
    dragonfly_image: String,
}

/// Capture environment-of-origin for forensics and restore sanity. Every field
/// has a sane default so a backup produced from an under-configured host is
/// still parseable: an empty `LITEPOD_VERSION` falls back to `env!("CARGO_PKG_VERSION")`
/// at the API binary level (or `"unknown"` if that also isn't set).
fn build_backup_metadata(backup_id: &str) -> Result<Vec<u8>, serde_json::Error> {
    let is_cloud = matches!(std::env::var("IS_CLOUD").as_deref(), Ok("true") | Ok("1"));
    let api_version = env!("CARGO_PKG_VERSION");
    let metadata = BackupMetadata {
        schema: 1,
        backup_id: backup_id.to_string(),
        created_at: Utc::now().to_rfc3339(),
        litepod_version: std::env::var("LITEPOD_VERSION")
            .unwrap_or_else(|_| api_version.to_string()),
        litepod_git_sha: std::env::var("LITEPOD_GIT_SHA").unwrap_or_else(|_| "unknown".to_string()),
        api_version,
        deployment_mode: if is_cloud { "cloud" } else { "self-hosted" },
        is_self_hosted: !is_cloud,
        caddy_image: std::env::var("CADDY_IMAGE_TAG").unwrap_or_else(|_| "2.11.4".to_string()),
        dragonfly_image: std::env::var("DRAGONFLY_IMAGE_TAG")
            .unwrap_or_else(|_| "v1.40.1".to_string()),
    };
    serde_json::to_vec_pretty(&metadata)
}

fn build_backup_tar(
    dump_bytes: &[u8],
    metadata_json: &[u8],
    data_dir: &Path,
    stack_env_file: Option<&Path>,
) -> Result<Vec<u8>, AppError> {
    let mut builder = tar::Builder::new(Vec::new());

    // 1. Database dump
    let mut header = tar::Header::new_gnu();
    header
        .set_path("db.sql")
        .map_err(|error| AppError::InternalError(format!("Backup tar header: {error}")))?;
    header.set_size(dump_bytes.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    builder
        .append(&header, dump_bytes)
        .map_err(|error| AppError::InternalError(format!("Backup tar append: {error}")))?;

    // 2. Backup metadata: litepod version + deployment mode + stack versions.
    let mut header = tar::Header::new_gnu();
    header
        .set_path("metadata.json")
        .map_err(|error| AppError::InternalError(format!("Backup tar header: {error}")))?;
    header.set_size(metadata_json.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    builder
        .append(&header, metadata_json)
        .map_err(|error| AppError::InternalError(format!("Backup tar append: {error}")))?;

    // 3. Compose files
    let compose_root = data_dir.join("compose");
    if compose_root.is_dir() {
        builder
            .append_dir_all("compose", &compose_root)
            .map_err(|error| {
                AppError::InternalError(format!("Backup tar append compose: {error}"))
            })?;
    }

    // 4. Application artifacts (static HTML/ZIP uploads)
    let apps_root = data_dir.join("applications");
    if apps_root.is_dir() {
        builder
            .append_dir_all("applications", &apps_root)
            .map_err(|error| {
                AppError::InternalError(format!("Backup tar append applications: {error}"))
            })?;
    }

    // 5. Uploaded icons (organization/application, see `crate::icons`)
    let icons_root = data_dir.join("icons");
    if icons_root.is_dir() {
        builder
            .append_dir_all("icons", &icons_root)
            .map_err(|error| {
                AppError::InternalError(format!("Backup tar append icons: {error}"))
            })?;
    }

    // 6. Stack .env. It lives beside the Compose stack, not in data_dir.
    if let Some(stack_env) = stack_env_file.filter(|path| path.is_file()) {
        builder
            .append_path_with_name(stack_env, "stack.env")
            .map_err(|error| {
                AppError::InternalError(format!("Backup tar append stack.env: {error}"))
            })?;
    }

    builder
        .into_inner()
        .map_err(|error| AppError::InternalError(format!("Backup tar finalize: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_tar_includes_icons_and_stack_env() {
        let root = std::env::temp_dir().join(format!("litepod-backup-{}", uuid::Uuid::now_v7()));
        let data_dir = root.join("data");
        let stack_env = root.join(".env");
        std::fs::create_dir_all(data_dir.join("icons")).unwrap();
        std::fs::write(data_dir.join("icons/logo.svg"), "icon").unwrap();
        std::fs::write(&stack_env, "ENCRYPTION_KEY=keep").unwrap();

        let metadata = br#"{"schema":1,"litepod_version":"test"}"#;
        let tar = build_backup_tar(b"SELECT 1;", metadata, &data_dir, Some(&stack_env)).unwrap();
        let mut archive = tar::Archive::new(std::io::Cursor::new(tar));
        let paths: Vec<_> = archive
            .entries()
            .unwrap()
            .map(|entry| entry.unwrap().path().unwrap().into_owned())
            .collect();

        assert!(paths.iter().any(|path| path == Path::new("db.sql")));
        assert!(paths.iter().any(|path| path == Path::new("metadata.json")));
        assert!(paths.iter().any(|path| path == Path::new("icons/logo.svg")));
        assert!(paths.iter().any(|path| path == Path::new("stack.env")));
        let _ = std::fs::remove_dir_all(root);
    }
}
