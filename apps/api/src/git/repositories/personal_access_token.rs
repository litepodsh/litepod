use sqlx::Row;

use crate::state::state::SharedState;
use crate::utils::security::digest_token as digest;

pub struct TokenRow {
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub scopes: String,
    pub expires_at: Option<String>,
}

pub struct TokenListItem {
    pub id: String,
    pub name: String,
    pub token_prefix: String,
    pub scopes: String,
    pub expires_at: Option<String>,
    pub last_used_at: Option<String>,
    pub created_at: String,
}

/// `token` is the plaintext handed to the user once; only its digest is stored.
pub async fn create(
    state: &SharedState,
    id: &str,
    user_id: &str,
    name: &str,
    token: &str,
    token_prefix: &str,
    scopes: &str,
    expires_at: Option<&str>,
) -> Result<(), String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "INSERT INTO personal_access_tokens (id, user_id, name, token_prefix, token_digest, scopes, expires_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )
    .bind(id)
    .bind(user_id)
    .bind(name)
    .bind(token_prefix)
    .bind(digest(token))
    .bind(scopes)
    .bind(expires_at)
    .execute(&conn)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn list_for_user(
    state: &SharedState,
    user_id: &str,
) -> Result<Vec<TokenListItem>, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(
        "SELECT id, name, token_prefix, scopes, expires_at, last_used_at, created_at FROM personal_access_tokens WHERE user_id = ?1 AND revoked_at IS NULL ORDER BY created_at DESC",
    )
    .bind(user_id)
    .fetch_all(&conn)
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows
        .iter()
        .map(|row| TokenListItem {
            id: row.try_get(0).unwrap(),
            name: row.try_get(1).unwrap(),
            token_prefix: row.try_get(2).unwrap(),
            scopes: row.try_get(3).unwrap(),
            expires_at: row.try_get(4).unwrap(),
            last_used_at: row.try_get(5).unwrap(),
            created_at: row.try_get(6).unwrap(),
        })
        .collect())
}

/// Excludes revoked and expired tokens — an expired token is treated exactly
/// like an invalid one, same as a session past `expires_at`.
pub async fn find_active_by_digest(
    state: &SharedState,
    token: &str,
) -> Result<Option<TokenRow>, crate::dtos::response::AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(
        "SELECT id, user_id, name, scopes, expires_at FROM personal_access_tokens WHERE token_digest = ?1 AND revoked_at IS NULL",
    )
    .bind(digest(token))
    .fetch_optional(&conn)
    .await
    .map_err(|e| crate::dtos::response::AppError::DatabaseError(e.to_string()))?;

    Ok(row.map(|row| TokenRow {
        id: row.try_get(0).unwrap(),
        user_id: row.try_get(1).unwrap(),
        name: row.try_get(2).unwrap(),
        scopes: row.try_get(3).unwrap(),
        expires_at: row.try_get(4).unwrap(),
    }))
}

pub async fn touch_last_used(state: &SharedState, id: &str) {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let _ = sqlx::query(
        "UPDATE personal_access_tokens SET last_used_at = strftime('%Y-%m-%d %H:%M:%S', 'now') WHERE id = ?1",
    )
    .bind(id)
    .execute(&conn)
    .await;
}

/// Marks a token expired-by-time as revoked, same treatment `auth_middleware`
/// already gives a session past `expires_at`.
pub async fn revoke(state: &SharedState, id: &str) -> Result<(), String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE personal_access_tokens SET revoked_at = strftime('%Y-%m-%d %H:%M:%S', 'now') WHERE id = ?1",
    )
    .bind(id)
    .execute(&conn)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Revokes only if `id` belongs to `user_id` — returns the number of rows
/// affected so the handler can tell "revoked" from "not yours / doesn't exist"
/// without a separate lookup.
pub async fn revoke_for_user(state: &SharedState, id: &str, user_id: &str) -> Result<u64, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let affected = sqlx::query(
        "UPDATE personal_access_tokens SET revoked_at = strftime('%Y-%m-%d %H:%M:%S', 'now') WHERE id = ?1 AND user_id = ?2 AND revoked_at IS NULL",
    )
    .bind(id)
    .bind(user_id)
    .execute(&conn)
    .await
    .map_err(|e| e.to_string())?
    .rows_affected();
    Ok(affected)
}

/// Revokes every active token for a user in one statement — used by "Revoke
/// all tokens", not by anything else, so a single query beats fetching ids
/// first to loop over.
pub async fn revoke_all_for_user(state: &SharedState, user_id: &str) -> Result<u64, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let affected = sqlx::query(
        "UPDATE personal_access_tokens SET revoked_at = strftime('%Y-%m-%d %H:%M:%S', 'now') WHERE user_id = ?1 AND revoked_at IS NULL",
    )
    .bind(user_id)
    .execute(&conn)
    .await
    .map_err(|e| e.to_string())?
    .rows_affected();
    Ok(affected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{cleanup_database, test_state};
    use crate::utils::security::generate_token;

    #[tokio::test]
    async fn the_plaintext_token_is_never_written_to_the_database() {
        let (state, path) = test_state().await;
        let token = generate_token();
        create(
            &state,
            "token-1",
            "editor",
            "ci",
            &token,
            "lp_1234567890",
            "[\"read\"]",
            None,
        )
        .await
        .unwrap();

        let stored: String =
            sqlx::query("SELECT token_digest FROM personal_access_tokens WHERE id = ?1")
                .bind("token-1")
                .fetch_one(&state.db)
                .await
                .unwrap()
                .try_get(0)
                .unwrap();

        assert_ne!(stored, token);
        assert_eq!(stored, crate::utils::security::digest_token(&token));

        let found = find_active_by_digest(&state, &token).await.unwrap();
        assert_eq!(found.map(|row| row.user_id).as_deref(), Some("editor"));

        cleanup_database(&path);
    }

    #[tokio::test]
    async fn revoking_someone_elses_token_affects_nothing() {
        let (state, path) = test_state().await;
        let token = generate_token();
        create(
            &state,
            "token-1",
            "editor",
            "ci",
            &token,
            "lp_1234567890",
            "[\"read\"]",
            None,
        )
        .await
        .unwrap();

        let affected = revoke_for_user(&state, "token-1", "viewer").await.unwrap();
        assert_eq!(affected, 0);
        assert!(
            find_active_by_digest(&state, &token)
                .await
                .unwrap()
                .is_some()
        );

        let affected = revoke_for_user(&state, "token-1", "editor").await.unwrap();
        assert_eq!(affected, 1);
        assert!(
            find_active_by_digest(&state, &token)
                .await
                .unwrap()
                .is_none()
        );

        cleanup_database(&path);
    }
}
