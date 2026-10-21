//! The upload half of `deploy_static_site`.
//!
//! The agent gets a URL with a one-time token when it proposes, and `curl`s
//! the site there. The bytes wait under the action's id until a person
//! confirms; nothing is created from them before that. The token is the only
//! credential, so it dies with the proposal: pending, unexpired, this action.

use std::path::PathBuf;

use sha2::{Digest, Sha256};
use sqlx::Row;

use crate::{dtos::response::AppError, state::state::SharedState};

use super::lifecycle::now_ms;
use super::types::*;

pub(super) fn hash_token(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn staging_dir(state: &SharedState, action_id: &str) -> PathBuf {
    state.data_dir.join("assistant_uploads").join(action_id)
}

/// Where the agent sends the site, ready to paste into `curl -T`.
pub async fn upload_url(state: &SharedState, action_id: &str, token: &str) -> String {
    format!(
        "{}/api/v1/assistant/uploads/{action_id}/{token}",
        crate::handler::settings::public_base_url(state)
            .await
            .trim_end_matches('/')
    )
}

/// Keeps an upload for a pending `deploy_static_site`. A second upload
/// replaces the first, so a wrong zip is fixed by sending the right one.
pub async fn stage_upload(
    state: &SharedState,
    action_id: &str,
    token: &str,
    bytes: &[u8],
) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(
        "SELECT arguments, status, expires_at FROM assistant_actions WHERE id = ?1 AND tool = ?2",
    )
    .bind(action_id)
    .bind(DEPLOY_STATIC_SITE)
    .fetch_optional(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?
    .ok_or(AppError::ResourceNotFound)?;
    let arguments: String = row
        .try_get(0)
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    let status: String = row
        .try_get(1)
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    let expires_at: i64 = row
        .try_get(2)
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    let site: StaticSite = serde_json::from_str(&arguments)
        .map_err(|error| AppError::InternalError(error.to_string()))?;
    // A wrong token answers like a missing action: nothing to learn by guessing.
    if site.upload_token_hash.is_empty() || site.upload_token_hash != hash_token(token) {
        return Err(AppError::ResourceNotFound);
    }
    if status != "pending" || now_ms() > expires_at {
        return Err(AppError::ValidationError(
            "This upload link is no longer open. Propose the site again.".into(),
        ));
    }

    let filename = match crate::deployment::file_kind("upload", bytes)? {
        "zip" => "site.zip",
        _ => "index.html",
    };
    let dir = staging_dir(state, action_id);
    let _ = tokio::fs::remove_dir_all(&dir).await;
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|error| AppError::InternalError(error.to_string()))?;
    tokio::fs::write(dir.join(filename), bytes)
        .await
        .map_err(|error| AppError::InternalError(error.to_string()))?;
    Ok(())
}

/// The upload route's own limit, so a site is refused the same either way.
const MAX_SITE_BYTES: usize = 10 * 1024 * 1024;

/// A site sent inside the tool call instead of to the upload URL, for an agent
/// whose sandbox cannot reach this host: `zip_base64`, or `files` as path to
/// text content, zipped here. `None` when the call carries neither.
pub fn inline_site(arguments: &serde_json::Value) -> Result<Option<Vec<u8>>, AppError> {
    use base64::Engine;
    use std::io::{Cursor, Write};

    if let Some(encoded) = arguments.get("zip_base64").and_then(|value| value.as_str()) {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded.trim())
            .map_err(|error| {
                AppError::ValidationError(format!("`zip_base64` is not base64: {error}"))
            })?;
        return sized(bytes).map(Some);
    }
    let Some(files) = arguments.get("files").and_then(|value| value.as_object()) else {
        return Ok(None);
    };
    if !files.contains_key("index.html") {
        return Err(AppError::ValidationError(
            "`files` needs an `index.html` at the root".into(),
        ));
    }
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (path, content) in files {
        let Some(content) = content.as_str() else {
            return Err(AppError::ValidationError(format!(
                "`files.{path}` must be the file's text"
            )));
        };
        // Checked again when the site is unpacked; refusing here keeps a bad
        // path from ever becoming a proposal.
        if path.starts_with('/') || path.split(['/', '\\']).any(|part| part == "..") {
            return Err(AppError::ValidationError(format!(
                "`{path}` must be a relative path inside the site"
            )));
        }
        zip.start_file(path.as_str(), options)
            .map_err(|error| AppError::InternalError(error.to_string()))?;
        zip.write_all(content.as_bytes())
            .map_err(|error| AppError::InternalError(error.to_string()))?;
    }
    let bytes = zip
        .finish()
        .map_err(|error| AppError::InternalError(error.to_string()))?
        .into_inner();
    sized(bytes).map(Some)
}

fn sized(bytes: Vec<u8>) -> Result<Vec<u8>, AppError> {
    if bytes.is_empty() {
        return Err(AppError::ValidationError("The site is empty".into()));
    }
    if bytes.len() > MAX_SITE_BYTES {
        return Err(AppError::ValidationError(
            "The site is larger than 10 MB, the upload limit.".into(),
        ));
    }
    Ok(bytes)
}

/// The staged site, as `(filename, bytes)`; `None` until the agent uploads.
pub(super) async fn staged(state: &SharedState, action_id: &str) -> Option<(String, Vec<u8>)> {
    let dir = staging_dir(state, action_id);
    for filename in ["site.zip", "index.html"] {
        if let Ok(bytes) = tokio::fs::read(dir.join(filename)).await {
            return Some((filename.to_owned(), bytes));
        }
    }
    None
}

// shortcut: an upload whose proposal expires without anyone opening it again
// stays on disk; sweep `assistant_uploads` by age if that ever adds up.
pub(super) async fn discard(state: &SharedState, action_id: &str) {
    let _ = tokio::fs::remove_dir_all(staging_dir(state, action_id)).await;
}
