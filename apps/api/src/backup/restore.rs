use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::Serialize;
use uuid::Uuid;

use crate::backup::{config, run, s3 as s3_client};
use crate::dtos::response::AppError;
use crate::state::state::SharedState;

#[derive(Debug, Serialize)]
pub struct RestoreRecordDto {
    pub id: String,
    pub backup_history_id: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub status: String,
    pub error_message: Option<String>,
    pub created_by: Option<String>,
}

const COLUMNS: &str =
    "id, backup_history_id, started_at, finished_at, status, error_message, created_by";

fn row_to_record(row: &sqlx::sqlite::SqliteRow) -> Result<RestoreRecordDto, AppError> {
    use sqlx::Row;
    Ok(RestoreRecordDto {
        id: row
            .try_get(0)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        backup_history_id: row
            .try_get(1)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        started_at: row
            .try_get(2)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        finished_at: row
            .try_get(3)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        status: row
            .try_get(4)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        error_message: row
            .try_get(5)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        created_by: row
            .try_get(6)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
    })
}

pub async fn list(state: &SharedState) -> Result<Vec<RestoreRecordDto>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(&format!(
        "SELECT {COLUMNS} FROM restore_history ORDER BY started_at DESC"
    ))
    .fetch_all(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    rows.iter().map(row_to_record).collect()
}

async fn get(state: &SharedState, id: &str) -> Result<Option<RestoreRecordDto>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let Some(row) = sqlx::query(&format!(
        "SELECT {COLUMNS} FROM restore_history WHERE id = ?1"
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
    backup_history_id: &str,
    started_at: i64,
    created_by: Option<&str>,
) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "INSERT INTO restore_history (id, backup_history_id, started_at, status, created_by) VALUES (?1, ?2, ?3, 'running', ?4)",
    )
    .bind(id)
    .bind(backup_history_id)
    .bind(started_at)
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
    error_message: Option<&str>,
) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE restore_history SET status = ?1, finished_at = ?2, error_message = ?3 WHERE id = ?4",
    )
    .bind(status)
    .bind(Utc::now().timestamp_millis())
    .bind(error_message)
    .bind(id)
    .execute(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok(())
}

pub async fn fail_interrupted(state: &SharedState) -> Result<(), AppError> {
    const MESSAGE: &str = "Restore was interrupted because the server restarted";
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE restore_history SET status = 'failed', error_message = ?1, finished_at = ?2 WHERE status = 'running'",
    )
    .bind(MESSAGE)
    .bind(Utc::now().timestamp_millis())
    .execute(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok(())
}

/// Validates the target backup, claims the shared backup/restore lock, and
/// spawns the destructive pipeline in the background. Restoring overwrites
/// the whole embedded database — including `s3_config`/`backup_history` — so
/// the chosen backup's S3 key is logged up front: it is the only place that
/// key survives once the restore completes and the history table reverts to
/// whatever it looked like at snapshot time.
pub async fn start(
    state: &SharedState,
    backup_id: &str,
    created_by: Option<String>,
) -> Result<RestoreRecordDto, AppError> {
    let Some(s3_config) = config::load(state).await? else {
        return Err(AppError::ValidationError(
            "Configure S3 storage before restoring a backup".into(),
        ));
    };
    let Some(backup) = run::get(state, backup_id).await? else {
        return Err(AppError::ResourceNotFound);
    };
    if backup.status != "success" {
        return Err(AppError::ValidationError(
            "Only a successful backup can be restored".into(),
        ));
    }
    let Some(s3_key) = backup.s3_key.clone() else {
        return Err(AppError::ValidationError(
            "This backup has no stored object to restore".into(),
        ));
    };

    let guard = state.backup_lock.clone().try_lock_owned().map_err(|_| {
        AppError::ResourceInUse("A backup or restore is already in progress".into())
    })?;

    tracing::warn!(backup_id, s3_key = %s3_key, "starting restore — this backup's S3 key will not be visible in history once the restore completes");

    let id = Uuid::now_v7().to_string();
    let started_at = Utc::now().timestamp_millis();
    insert_running(state, &id, backup_id, started_at, created_by.as_deref()).await?;
    let record = get(state, &id).await?.ok_or(AppError::ResourceNotFound)?;

    let state_bg = state.clone();
    let id_bg = id.clone();
    tokio::spawn(async move {
        let _guard = guard;
        let outcome = run_pipeline(
            &state_bg.db,
            &state_bg.data_dir,
            state_bg.stack_env_file.as_deref(),
            &s3_config,
            &s3_key,
        )
        .await;
        let result = match outcome {
            Ok(()) => finish(&state_bg, &id_bg, "success", None).await,
            Err(error) => {
                tracing::error!(restore_id = %id_bg, error = %error, "restore failed");
                finish(&state_bg, &id_bg, "failed", Some(&error.to_string())).await
            }
        };
        if let Err(error) = result {
            tracing::error!(restore_id = %id_bg, error = %error, "could not record restore outcome");
        }
    });

    Ok(record)
}

/// Download, extract and apply one backup. Takes its targets explicitly (not
/// `SharedState`) so the offline `litepod-api restore` CLI can drive it too.
pub(crate) async fn run_pipeline(
    pool: &sqlx::SqlitePool,
    data_dir: &Path,
    stack_env_file: Option<&Path>,
    config: &config::S3Config,
    s3_key: &str,
) -> Result<(), AppError> {
    // Download-before-apply: fetch the whole tar first so a truncated or
    // corrupt download never touches the live database or compose tree.
    let scratch_dir = data_dir.join("restore-tmp");
    tokio::fs::create_dir_all(&scratch_dir)
        .await
        .map_err(|error| {
            AppError::InternalError(format!("Could not prepare restore scratch dir: {error}"))
        })?;
    let downloaded_path = scratch_dir.join(format!("{}.tar", Uuid::now_v7()));
    let db_sql_path = scratch_dir.join("db.sql");

    if let Err(error) = download(config, s3_key, &downloaded_path).await {
        let _ = tokio::fs::remove_file(&downloaded_path).await;
        return Err(error);
    }

    // Route the tar: `db.sql` lands in scratch; `compose/...`, `applications/...`,
    // and `icons/...` land below data_dir; `stack.env` returns to the stack.
    // Filesystem writes happen before the
    // database is touched — losing existing files is fine because they
    // came from a live backup moments earlier.
    let compose_dest = data_dir.join("compose");
    let apps_dest = data_dir.join("applications");
    let icons_dest = data_dir.join("icons");
    let extract_result = extract_tar_routes(
        &downloaded_path,
        &compose_dest,
        &apps_dest,
        &icons_dest,
        stack_env_file,
        &db_sql_path,
    )
    .await;
    if let Err(error) = extract_result {
        let _ = tokio::fs::remove_file(&downloaded_path).await;
        let _ = tokio::fs::remove_file(&db_sql_path).await;
        return Err(error);
    }
    let _ = tokio::fs::remove_file(&downloaded_path).await;

    // Apply dump inside an exclusive embedded transaction. Existing schema
    // objects are removed and the transaction rolls back if replay fails.
    let apply_result = apply_db_dump(pool, &db_sql_path).await;
    let _ = tokio::fs::remove_file(&db_sql_path).await;
    apply_result
}

async fn download(
    config: &config::S3Config,
    s3_key: &str,
    downloaded_path: &Path,
) -> Result<(), AppError> {
    let bucket = s3_client::client(config)?;
    let mut file = tokio::fs::File::create(downloaded_path)
        .await
        .map_err(|error| {
            AppError::InternalError(format!("Could not create restore scratch file: {error}"))
        })?;
    bucket
        .get_object_to_writer(s3_key, &mut file)
        .await
        .map_err(|error| {
            AppError::InternalError(format!("Could not download backup from S3: {error}"))
        })?;
    Ok(())
}

/// Routes a backup tar's entries to their destinations: `db.sql` → scratch,
/// `compose/`, `applications/`, and `icons/` → data_dir; `stack.env` → stack.
/// Unknown top-level entries are skipped so a tar
/// can grow new keys without breaking older restore code.
async fn extract_tar_routes(
    tar_path: &Path,
    compose_dest: &Path,
    apps_dest: &Path,
    icons_dest: &Path,
    stack_env_dest: Option<&Path>,
    db_dest: &Path,
) -> Result<(), AppError> {
    let tar_path = tar_path.to_path_buf();
    let compose_dest = compose_dest.to_path_buf();
    let apps_dest = apps_dest.to_path_buf();
    let icons_dest = icons_dest.to_path_buf();
    let stack_env_dest = stack_env_dest.map(Path::to_path_buf);
    let db_dest = db_dest.to_path_buf();
    tokio::task::spawn_blocking(move || {
        extract_tar_blocking(
            &tar_path,
            &compose_dest,
            &apps_dest,
            &icons_dest,
            stack_env_dest.as_deref(),
            &db_dest,
        )
    })
    .await
    .map_err(|error| AppError::InternalError(format!("Restore extract task panicked: {error}")))?
}

fn extract_tar_blocking(
    tar_path: &Path,
    compose_dest: &Path,
    apps_dest: &Path,
    icons_dest: &Path,
    stack_env_dest: Option<&Path>,
    db_dest: &Path,
) -> Result<(), AppError> {
    let file = std::fs::File::open(tar_path).map_err(|error| {
        AppError::InternalError(format!("Could not open downloaded backup: {error}"))
    })?;
    let mut archive = tar::Archive::new(file);
    let mut saw_db_sql = false;

    for entry in archive.entries().map_err(|error| {
        AppError::ValidationError(format!("Downloaded backup is not a valid archive: {error}"))
    })? {
        let mut entry = entry.map_err(|error| {
            AppError::ValidationError(format!("Downloaded backup is not a valid archive: {error}"))
        })?;
        let entry_path = entry
            .path()
            .map_err(|error| {
                AppError::ValidationError(format!(
                    "Downloaded backup has an invalid entry path: {error}"
                ))
            })?
            .into_owned();
        let mut components = entry_path.components();
        let Some(head) = components.next() else {
            continue;
        };
        let rest: PathBuf = components.collect();

        // db.sql → scratch dir for DB replay
        if head.as_os_str() == "db.sql" {
            if rest.as_os_str().is_empty() {
                let mut out = std::fs::File::create(db_dest).map_err(|error| {
                    AppError::InternalError(format!("Could not create db.sql target: {error}"))
                })?;
                std::io::copy(&mut entry, &mut out).map_err(|error| {
                    AppError::InternalError(format!("Could not extract db.sql: {error}"))
                })?;
                saw_db_sql = true;
            }
            continue;
        }

        // compose/ → data_dir/compose/
        if head.as_os_str() == "compose" {
            if rest.as_os_str().is_empty() {
                continue;
            }
            extract_entry(&mut entry, &compose_dest.join(&rest))?;
            continue;
        }

        // applications/ → data_dir/applications/
        if head.as_os_str() == "applications" {
            if rest.as_os_str().is_empty() {
                continue;
            }
            extract_entry(&mut entry, &apps_dest.join(&rest))?;
            continue;
        }

        if head.as_os_str() == "icons" {
            if rest.as_os_str().is_empty() {
                continue;
            }
            extract_entry(&mut entry, &icons_dest.join(&rest))?;
            continue;
        }

        if head.as_os_str() == "stack.env" && rest.as_os_str().is_empty() {
            if let Some(stack_env_dest) = stack_env_dest {
                let mut backup = String::new();
                std::io::Read::read_to_string(&mut entry, &mut backup).map_err(|error| {
                    AppError::InternalError(format!("Could not read stack.env: {error}"))
                })?;
                crate::updater::stack_env::adopt_secrets(stack_env_dest, &backup).map_err(
                    |error| {
                        AppError::InternalError(format!("Could not restore stack.env: {error}"))
                    },
                )?;
            }
            continue;
        }

        if head.as_os_str() == "metadata.json" && rest.as_os_str().is_empty() {
            log_backup_metadata(&mut entry);
            continue;
        }

        // Future top-level keys land here — skipped silently so newer
        // backups don't break older restore code.
    }

    if !saw_db_sql {
        return Err(AppError::ValidationError(
            "Downloaded backup does not contain a db.sql entry".into(),
        ));
    }
    Ok(())
}

fn log_backup_metadata(entry: &mut tar::Entry<std::fs::File>) {
    let mut body = Vec::new();
    if std::io::copy(entry, &mut body).is_err() {
        return;
    }
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&body) else {
        return;
    };
    tracing::info!(
        litepod_version = value
            .get("litepod_version")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown"),
        litepod_git_sha = value
            .get("litepod_git_sha")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown"),
        deployment_mode = value
            .get("deployment_mode")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown"),
        caddy_image = value
            .get("caddy_image")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown"),
        dragonfly_image = value
            .get("dragonfly_image")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown"),
        "backup metadata"
    );
}

fn extract_entry(entry: &mut tar::Entry<std::fs::File>, dest: &Path) -> Result<(), AppError> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            AppError::InternalError(format!(
                "Could not create dir {}: {error}",
                parent.display()
            ))
        })?;
    }
    let entry_type = entry.header().entry_type();
    if entry_type.is_dir() {
        std::fs::create_dir_all(dest).map_err(|error| {
            AppError::InternalError(format!("Could not create {}: {error}", dest.display()))
        })?;
    } else {
        let mut out = std::fs::File::create(dest).map_err(|error| {
            AppError::InternalError(format!("Could not create {}: {error}", dest.display()))
        })?;
        std::io::copy(entry, &mut out).map_err(|error| {
            AppError::InternalError(format!("Could not write {}: {error}", dest.display()))
        })?;
    }
    Ok(())
}

async fn apply_db_dump(pool: &sqlx::SqlitePool, db_sql_path: &Path) -> Result<(), AppError> {
    let sql = tokio::fs::read_to_string(db_sql_path)
        .await
        .map_err(|error| {
            AppError::InternalError(format!("Could not read db.sql for restore: {error}"))
        })?;
    let pool = pool.clone();
    let mut conn = pool
        .acquire()
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    // Foreign keys are switched off on this connection below; never hand it
    // back to the pool in that state.
    conn.close_on_drop();

    // Must run before `BEGIN`: SQLite ignores `PRAGMA foreign_keys` inside a
    // transaction, and with enforcement on, the dump's alphabetical table
    // order inserts children (`environments`) before parents (`projects`).
    sqlx::query("PRAGMA foreign_keys = OFF")
        .execute(&mut *conn)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;

    // `BEGIN EXCLUSIVE` rather than sqlx's `Transaction` (which only issues a
    // plain `BEGIN`): nothing else may read or write while the whole database
    // is dropped and replayed from the dump inside this one transaction.
    sqlx::query("BEGIN EXCLUSIVE")
        .execute(&mut *conn)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;

    let outcome = replay_dump(&mut conn, &sql).await;

    match outcome {
        Ok(()) => {
            sqlx::query("COMMIT")
                .execute(&mut *conn)
                .await
                .map_err(|error| AppError::DatabaseError(error.to_string()))?;
        }
        Err(error) => {
            let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
            return Err(error);
        }
    }
    drop(conn);

    // A backup from an older litepod restores an older schema. Bring it up
    // to this binary's schema now, as a restart would, instead of serving
    // requests against missing tables/columns until the next boot.
    tokio::spawn(async move { crate::migrations::run(&pool).await })
        .await
        .map_err(|error| {
            AppError::InternalError(format!(
                "Restored the backup but could not migrate it to this version: {error}"
            ))
        })
}

/// Drops every existing schema object and replays the dump's SQL, all against
/// the connection already holding the `BEGIN EXCLUSIVE` lock. Split out of
/// `apply_db_dump` as a named `async fn` (not an inline `async {}` block) —
/// sqlx's `Executor` impl for `&mut SqliteConnection` otherwise trips a
/// higher-ranked lifetime inference limitation when reused across `.await`
/// points inside an anonymous async block.
async fn replay_dump(conn: &mut sqlx::SqliteConnection, sql: &str) -> Result<(), AppError> {
    use sqlx::Row;

    let object_rows = sqlx::query(
        "SELECT type, name FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY CASE type WHEN 'trigger' THEN 1 WHEN 'view' THEN 2 WHEN 'table' THEN 3 ELSE 4 END",
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;

    let mut objects = Vec::new();
    for row in &object_rows {
        let kind: String = row
            .try_get(0)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?;
        let name: String = row
            .try_get(1)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?;
        let escaped = name.replace('"', "\"\"");
        objects.push(format!("DROP {kind} IF EXISTS \"{escaped}\";"));
    }

    sqlx::query("PRAGMA foreign_keys = OFF;")
        .execute(&mut *conn)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    for statement in &objects {
        sqlx::query(statement)
            .execute(&mut *conn)
            .await
            .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    }
    let replay_sql: String = sql
        .lines()
        .filter(|line| {
            let statement = line.trim();
            statement != "BEGIN TRANSACTION;" && statement != "BEGIN;" && statement != "COMMIT;"
        })
        .map(|line| format!("{line}\n"))
        .collect();
    // Not `raw_sql` (sqlx's multi-statement executor): boxing/awaiting its
    // `Execute<'q>` future inside a `tokio::spawn`ed task trips a known sqlx/rustc
    // HRTB inference limitation ("implementation of `Executor` is not general
    // enough"). Splitting into individual `;`-terminated statements and running
    // each through the same plain `sqlx::query(...).execute(...)` already used
    // everywhere else in this codebase sidesteps it entirely. The splitter tracks
    // single-quoted string state so a `;` inside a string literal (e.g. a CHECK
    // constraint's default value) is never mistaken for a statement boundary —
    // matching how SQLite's own multi-statement parser treats the dump we wrote.
    for statement in split_sql_statements(&replay_sql) {
        sqlx::query(statement)
            .execute(&mut *conn)
            .await
            .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    }

    // A backup taken by a newer litepod carries migrations this binary has
    // never heard of; its schema is ahead of the code. Refuse it (the caller
    // rolls back) rather than run old code against a newer schema.
    let known = crate::migrations::runner::names();
    let restored = sqlx::query("SELECT name FROM _migrations ORDER BY name")
        .fetch_all(&mut *conn)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    for row in &restored {
        let name: String = row
            .try_get(0)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?;
        if !known.contains(&name) {
            return Err(AppError::ValidationError(format!(
                "This backup comes from a newer litepod version (unknown migration {name}). Update litepod, then restore it."
            )));
        }
    }

    let violation = sqlx::query("PRAGMA foreign_key_check")
        .fetch_optional(&mut *conn)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    if let Some(row) = violation {
        let table: String = row
            .try_get(0)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?;
        return Err(AppError::ValidationError(format!(
            "Backup is inconsistent: table {table} has rows pointing at missing parents"
        )));
    }
    Ok(())
}

/// Splits a script of `;`-terminated SQL statements, respecting `'...'` string
/// literals (with `''` as the escaped-quote form) so a semicolon inside a
/// string is never treated as a statement boundary; `--` comments are skipped
/// so an apostrophe in one can't open a string. A `CREATE TRIGGER` body
/// holds its own `;`-terminated statements, so it runs on to its `END;`.
/// Empty/whitespace-only statements are dropped.
fn split_sql_statements(script: &str) -> Vec<&str> {
    let bytes = script.as_bytes();
    let mut statements = Vec::new();
    let mut start = 0usize;
    let mut in_string = false;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'\'' => in_string = !in_string,
            // Schema SQL keeps its `--` comments, and prose like "Paddle
            // subscription's items" must not open a string literal.
            b'-' if !in_string && bytes.get(i + 1) == Some(&b'-') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b';' if !in_string && !inside_trigger(script[start..=i].trim()) => {
                let statement = script[start..=i].trim();
                if !statement.is_empty() {
                    statements.push(statement);
                }
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    let tail = script[start..].trim();
    if !tail.is_empty() {
        statements.push(tail);
    }
    statements
}

/// True while `statement` is a `CREATE TRIGGER` whose body hasn't reached
/// `END;` yet.
fn inside_trigger(statement: &str) -> bool {
    let upper = statement.to_ascii_uppercase();
    upper.starts_with("CREATE TRIGGER") && !upper.ends_with("END;")
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn seed_project_hierarchy(state: &SharedState) {
        for (sql, binds) in [
            (
                "INSERT INTO users (id, email, hashed_password, name) VALUES (?1, ?2, ?3, ?4)",
                vec!["user-1", "user@example.com", "hash", "User"],
            ),
            (
                "INSERT INTO organizations (id, name, slug, created_by) VALUES (?1, ?2, ?3, ?4)",
                vec!["organization-1", "Organization", "organization", "user-1"],
            ),
            (
                "INSERT INTO projects (id, organization_id, name, created_at, updated_at) VALUES (?1, ?2, ?3, 1, 1)",
                vec!["project-1", "organization-1", "Project"],
            ),
            // `environments` sorts before its parent `projects` in the dump.
            (
                "INSERT INTO environments (id, project_id, name, is_production, created_at, updated_at) VALUES (?1, ?2, ?3, 1, 1, 1)",
                vec!["environment-1", "project-1", "prod"],
            ),
        ] {
            let mut query = sqlx::query(sql);
            for bind in binds {
                query = query.bind(bind);
            }
            query.execute(&state.db).await.unwrap();
        }
    }

    async fn restore_dump(state: &SharedState, dump: &[u8]) -> Result<(), AppError> {
        let path = std::env::temp_dir().join(format!("litepod-restore-{}.sql", Uuid::now_v7()));
        std::fs::write(&path, dump).unwrap();
        let result = apply_db_dump(&state.db, &path).await;
        let _ = std::fs::remove_file(&path);
        result
    }

    async fn count(state: &SharedState, sql: &str) -> i64 {
        sqlx::query_scalar(sql).fetch_one(&state.db).await.unwrap()
    }

    #[tokio::test]
    async fn dump_round_trips_through_restore() {
        let (state, path) = crate::testing::test_state().await;
        seed_project_hierarchy(&state).await;
        let triggers = count(
            &state,
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'trigger'",
        )
        .await;
        assert!(triggers > 0, "schema should ship triggers");

        let dump = run::dump_db_to_sql(&state).await.unwrap();
        sqlx::query("DELETE FROM organizations")
            .execute(&state.db)
            .await
            .unwrap();
        assert_eq!(count(&state, "SELECT COUNT(*) FROM environments").await, 0);

        restore_dump(&state, &dump).await.unwrap();

        assert_eq!(count(&state, "SELECT COUNT(*) FROM environments").await, 1);
        assert_eq!(
            count(
                &state,
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'trigger'"
            )
            .await,
            triggers
        );
        assert_eq!(count(&state, "PRAGMA foreign_keys").await, 1);
        crate::testing::cleanup_database(&path);
    }

    #[tokio::test]
    async fn restore_refuses_backup_from_newer_version() {
        let (state, path) = crate::testing::test_state().await;
        seed_project_hierarchy(&state).await;
        sqlx::query("INSERT INTO _migrations (name) VALUES ('9999_from_the_future.sql')")
            .execute(&state.db)
            .await
            .unwrap();
        let dump = run::dump_db_to_sql(&state).await.unwrap();
        sqlx::query("DELETE FROM _migrations WHERE name = '9999_from_the_future.sql'")
            .execute(&state.db)
            .await
            .unwrap();
        sqlx::query("DELETE FROM organizations")
            .execute(&state.db)
            .await
            .unwrap();

        let error = restore_dump(&state, &dump).await.unwrap_err();

        assert!(
            error.to_string().contains("9999_from_the_future.sql"),
            "{error}"
        );
        // Rolled back: the live database is untouched.
        assert_eq!(count(&state, "SELECT COUNT(*) FROM organizations").await, 0);
        crate::testing::cleanup_database(&path);
    }

    #[test]
    fn splitter_keeps_trigger_bodies_whole() {
        let statements = split_sql_statements(
            "CREATE TABLE x(\n    a -- the row's value\n);\nCREATE TRIGGER t BEFORE INSERT ON x\nBEGIN\n    SELECT RAISE(ABORT, 'no; really');\nEND;\nINSERT INTO x VALUES(1);\n",
        );
        assert_eq!(statements.len(), 3, "{statements:?}");
        assert!(statements[1].ends_with("END;"));
    }

    #[test]
    fn restore_extracts_icons_and_stack_env() {
        let root = std::env::temp_dir().join(format!("litepod-restore-{}", Uuid::now_v7()));
        std::fs::create_dir_all(&root).unwrap();
        let archive_path = root.join("backup.tar");
        let source_icon = root.join("source-icon.svg");
        let source_env = root.join("source.env");
        let source_db = root.join("source.sql");
        std::fs::write(&source_icon, "icon").unwrap();
        std::fs::write(&source_env, "ENCRYPTION_KEY=keep").unwrap();
        std::fs::write(&source_db, "SELECT 1;").unwrap();

        let mut archive = tar::Builder::new(std::fs::File::create(&archive_path).unwrap());
        archive.append_path_with_name(&source_db, "db.sql").unwrap();
        archive
            .append_path_with_name(&source_icon, "icons/logo.svg")
            .unwrap();
        archive
            .append_path_with_name(&source_env, "stack.env")
            .unwrap();
        archive.finish().unwrap();

        let compose = root.join("compose");
        let apps = root.join("applications");
        let icons = root.join("icons");
        let stack_env = root.join("stack/.env");
        let db = root.join("restored.sql");
        extract_tar_blocking(
            &archive_path,
            &compose,
            &apps,
            &icons,
            Some(&stack_env),
            &db,
        )
        .unwrap();

        assert_eq!(std::fs::read_to_string(db).unwrap(), "SELECT 1;");
        assert_eq!(
            std::fs::read_to_string(icons.join("logo.svg")).unwrap(),
            "icon"
        );
        assert_eq!(
            std::fs::read_to_string(stack_env).unwrap(),
            "ENCRYPTION_KEY=keep"
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
