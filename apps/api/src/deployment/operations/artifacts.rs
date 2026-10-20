use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};
use sqlx::Row;
use uuid::Uuid;

use crate::{
    dtos::{deployment::DeploymentDto, response::AppError},
    state::state::SharedState,
};

use super::identity::*;
use super::resources::*;

pub(crate) fn file_kind(filename: &str, bytes: &[u8]) -> Result<&'static str, AppError> {
    let zip = bytes.len() >= 4
        && bytes[0..2] == [0x50, 0x4b]
        && matches!(&bytes[2..4], [0x03, 0x04] | [0x05, 0x06] | [0x07, 0x08]);
    if zip {
        return Ok("zip");
    }
    let leading = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]).to_ascii_lowercase();
    let mut rest = leading.trim_start_matches('\u{feff}').trim_start();
    while rest.starts_with("<!--") {
        match rest.find("-->") {
            Some(end) => rest = rest[end + 3..].trim_start(),
            None => break,
        }
    }
    if rest.starts_with("<!doctype html") || rest.starts_with("<html") {
        return Ok("html");
    }
    Err(AppError::ValidationError(format!(
        "{filename}: upload a ZIP archive or a complete HTML document. HTML files must start with `<!DOCTYPE html>` or `<html>` (comments before it are fine) — fragments or snippets are not accepted."
    )))
}

pub(super) fn safe_zip_path(name: &str) -> Result<PathBuf, AppError> {
    let path = Path::new(name);
    if path.is_absolute()
        || path.components().any(|part| {
            matches!(
                part,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(AppError::ValidationError(
            "ZIP archive contains an unsafe path".into(),
        ));
    }
    Ok(path.to_path_buf())
}

pub async fn save_artifact(
    state: &SharedState,
    application_id: &str,
    filename: String,
    bytes: Vec<u8>,
) -> Result<DeploymentDto, AppError> {
    let kind = file_kind(&filename, &bytes)?;
    let id = Uuid::now_v7().to_string();
    let root = app_root(state, application_id).join("artifacts").join(&id);
    std::fs::create_dir_all(&root).map_err(|e| {
        AppError::InternalError(format!("Could not create artifact directory: {e}"))
    })?;
    let result = (|| -> Result<(), AppError> {
        if kind == "zip" {
            let reader = std::io::Cursor::new(&bytes);
            let mut archive = zip::ZipArchive::new(reader)
                .map_err(|_| AppError::ValidationError("Invalid ZIP archive".into()))?;
            let mut index_found = false;
            for i in 0..archive.len() {
                let mut entry = archive
                    .by_index(i)
                    .map_err(|_| AppError::ValidationError("Invalid ZIP entry".into()))?;
                let output = root.join(safe_zip_path(entry.name())?);
                if entry.is_dir() {
                    std::fs::create_dir_all(&output)
                        .map_err(|e| AppError::InternalError(e.to_string()))?;
                    continue;
                }
                if entry.name() == "index.html" {
                    index_found = true;
                }
                if let Some(parent) = output.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| AppError::InternalError(e.to_string()))?;
                }
                let mut file = std::fs::File::create(output)
                    .map_err(|e| AppError::InternalError(e.to_string()))?;
                std::io::copy(&mut entry, &mut file)
                    .map_err(|e| AppError::InternalError(e.to_string()))?;
            }
            if !index_found {
                return Err(AppError::ValidationError(
                    "ZIP archive must contain index.html at its root".into(),
                ));
            }
        } else {
            std::fs::write(root.join("index.html"), &bytes)
                .map_err(|e| AppError::InternalError(e.to_string()))?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        let _ = std::fs::remove_dir_all(&root);
        return Err(error);
    }
    let mut hash = Sha256::new();
    hash.update(&bytes);
    ensure_deployment(state, application_id).await?;
    let conn = crate::git::repositories::db::get_conn(state).await;
    let timestamp = now_ms();
    let relative_path = root
        .strip_prefix(&state.data_dir)
        .unwrap_or(&root)
        .to_string_lossy()
        .to_string();
    let sha256 = hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    sqlx::query("INSERT INTO application_artifacts (id, application_id, filename, kind, byte_size, sha256, relative_path, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)")
        .bind(id.clone())
        .bind(application_id)
        .bind(filename)
        .bind(kind)
        .bind(bytes.len() as i64)
        .bind(sha256)
        .bind(relative_path)
        .bind(timestamp)
        .execute(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    sqlx::query("UPDATE application_deployments SET artifact_id = ?1, status = 'ready', last_error = NULL, updated_at = ?2 WHERE application_id = ?3")
        .bind(id)
        .bind(timestamp)
        .bind(application_id)
        .execute(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    prune_unreferenced_artifacts(state, application_id).await;
    deployment(state, application_id).await
}

/// Deletes every artifact this application no longer needs: not the one
/// currently deployed, and not referenced by any of its retained deployment
/// runs (`history::start` already caps that history at 10). Best-effort - a
/// failed delete here must never fail the upload that just succeeded, and
/// the next upload gets another chance at the same rows.
pub(super) async fn prune_unreferenced_artifacts(state: &SharedState, application_id: &str) {
    use sqlx::Row;
    let conn = crate::git::repositories::db::get_conn(state).await;
    let Ok(rows) = sqlx::query(
        "SELECT id, relative_path FROM application_artifacts \
         WHERE application_id = ?1 \
         AND id NOT IN (SELECT artifact_id FROM application_deployments WHERE application_id = ?1 AND artifact_id IS NOT NULL) \
         AND id NOT IN (SELECT artifact_id FROM application_deployment_runs WHERE application_id = ?1 AND artifact_id IS NOT NULL)",
    )
    .bind(application_id)
    .fetch_all(&conn)
    .await
    else {
        return;
    };
    for row in &rows {
        let (Ok(artifact_id), Ok(relative_path)) =
            (row.try_get::<String, _>(0), row.try_get::<String, _>(1))
        else {
            continue;
        };
        if sqlx::query("DELETE FROM application_artifacts WHERE id = ?1")
            .bind(&artifact_id)
            .execute(&conn)
            .await
            .is_ok()
        {
            let _ = std::fs::remove_dir_all(state.data_dir.join(relative_path));
        }
    }
}

pub async fn delete_artifact(
    state: &SharedState,
    application_id: &str,
) -> Result<DeploymentDto, AppError> {
    ensure_deployment(state, application_id).await?;
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(
        "SELECT a.id, a.relative_path FROM application_deployments d LEFT JOIN application_artifacts a ON a.id = d.artifact_id WHERE d.application_id = ?1",
    )
    .bind(application_id)
    .fetch_optional(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?
    .ok_or(AppError::ApplicationNotFound)?;
    let artifact_id: Option<String> = row
        .try_get(0)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let relative_path: Option<String> = row
        .try_get(1)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let Some(artifact_id) = artifact_id else {
        return Err(AppError::ValidationError("No artifact to remove".into()));
    };
    let timestamp = now_ms();
    sqlx::query(
        "UPDATE application_deployments SET artifact_id = NULL, status = 'not_deployed', updated_at = ?1 WHERE application_id = ?2",
    )
    .bind(timestamp)
    .bind(application_id)
    .execute(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    sqlx::query("DELETE FROM application_artifacts WHERE id = ?1")
        .bind(artifact_id)
        .execute(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    drop(conn);
    if let Some(relative_path) = relative_path {
        let _ = std::fs::remove_dir_all(state.data_dir.join(relative_path));
    }
    deployment(state, application_id).await
}
