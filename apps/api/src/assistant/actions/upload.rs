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
