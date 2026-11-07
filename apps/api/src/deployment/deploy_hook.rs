//! Keyless deploy hook.
//!
//! `POST /deploy/<token>` triggers a deploy with no session and no API key — the unguessable
//! token in the URL is the credential. Only its SHA-256 digest is stored, so the URL is shown
//! once, when it is created or rotated; rotating revokes a leaked URL.

use chrono::Utc;
use sqlx::Row;

use crate::utils::security::digest_token as digest;
use crate::{dtos::response::AppError, state::state::SharedState};

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

/// 32 random bytes, base64url — the whole security of the endpoint rests on this being unguessable.
fn generate_token() -> String {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    use rand::Rng;
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn hook_url(public_url: &str, token: &str) -> String {
    format!("{}/deploy/{token}", public_url.trim_end_matches('/'))
}

/// Whether the application has a deploy URL. Only its digest is stored, so the URL
/// itself is shown once, by `rotate_token`.
pub async fn has_token(state: &SharedState, application_id: &str) -> Result<bool, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query("SELECT deploy_token_digest IS NOT NULL FROM applications WHERE id = ?1")
        .bind(application_id)
        .fetch_optional(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?
        .ok_or(AppError::ApplicationNotFound)?;
    row.try_get(0)
        .map_err(|e| AppError::DatabaseError(e.to_string()))
}

/// New token, invalidating the previous one. Returned once: only its digest is kept.
pub async fn rotate_token(state: &SharedState, application_id: &str) -> Result<String, AppError> {
    let token = generate_token();
    let token_digest = digest(&token);
    crate::git::repositories::db::retry_on_stale_snapshot(|| async {
        let conn = crate::git::repositories::db::get_conn(state).await;
        let changed = sqlx::query(
            "UPDATE applications SET deploy_token_digest = ?1, deploy_token = NULL, updated_at = ?2 WHERE id = ?3",
        )
        .bind(token_digest.clone())
        .bind(now_ms())
        .bind(application_id)
        .execute(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?
        .rows_affected();
        if changed == 0 {
            return Err(AppError::ApplicationNotFound);
        }
        Ok(())
    })
    .await?;
    Ok(token)
}

/// Resolves a token to its application. An unknown token is reported as not-found without
/// revealing whether any application exists.
pub async fn application_for_token(state: &SharedState, token: &str) -> Result<String, AppError> {
    if token.trim().is_empty() {
        return Err(AppError::ApplicationNotFound);
    }
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query("SELECT id FROM applications WHERE deploy_token_digest = ?1")
        .bind(digest(token))
        .fetch_optional(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?
        .ok_or(AppError::ApplicationNotFound)?;
    row.try_get(0)
        .map_err(|e| AppError::DatabaseError(e.to_string()))
}

/// Hashes tokens stored in the clear before migration 0091, so existing deploy URLs
/// keep working. Runs at boot; once done it finds nothing.
pub async fn hash_legacy_tokens(pool: &sqlx::SqlitePool) -> Result<u64, sqlx::Error> {
    let rows = sqlx::query("SELECT id, deploy_token FROM applications WHERE deploy_token IS NOT NULL")
        .fetch_all(pool)
        .await?;
    let mut hashed = 0;
    for row in rows {
        let id: String = row.try_get(0)?;
        let token: String = row.try_get(1)?;
        let token_digest = (!token.is_empty()).then(|| digest(&token));
        hashed += sqlx::query(
            "UPDATE applications SET deploy_token_digest = ?1, deploy_token = NULL WHERE id = ?2",
        )
        .bind(token_digest)
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected();
    }
    Ok(hashed)
}

/// Queues a deploy for any application regardless of `deploy_type`. The queue
/// serializes per-app so two consecutive calls to this hook cannot race on the
/// same container.
pub async fn trigger(
    state: &SharedState,
    application_id: &str,
) -> Result<super::queue::DeploymentJobDto, AppError> {
    super::enqueue_for_application(state, application_id, "deploy_hook", None).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn trigger_enqueues_for_static_application() {
        let (state, _path) = crate::testing::test_state_for(false).await;
        let conn = crate::git::repositories::db::get_conn(&state).await;
        sqlx::query("INSERT INTO projects (id, organization_id, name, created_at, updated_at) VALUES ('project-1', 'organization-a', 'P', 1, 1)").execute(&conn).await.unwrap();
        sqlx::query("INSERT INTO environments (id, project_id, name, is_production, created_at, updated_at) VALUES ('env-1', 'project-1', 'prod', 1, 1, 1)").execute(&conn).await.unwrap();
        sqlx::query("INSERT INTO applications (id, environment_id, name, deploy_type, build_method, container_port, railpack_project_path, server_id, created_at, updated_at) VALUES ('app-1', 'env-1', 'A', 'static', 'docker', 80, '.', 'local', 1, 1)").execute(&conn).await.unwrap();

        let result = trigger(&state, "app-1").await;
        // Static without an artifact is rejected; the important thing is that
        // the dispatch did not bounce on the `git`-only guard.
        assert!(result.is_err());
        let message = format!("{:?}", result.unwrap_err());
        assert!(
            !message.contains("Deploy hooks only apply to git"),
            "deploy hook must not be restricted to git: {message}"
        );
    }
}
