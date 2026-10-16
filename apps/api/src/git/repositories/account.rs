use sqlx::Row;

use crate::state::state::SharedState;
use crate::utils::security;

/// Store encrypted TOTP secret for a user.
pub async fn set_totp_secret(
    state: &SharedState,
    user_id: &str,
    encrypted_secret: &str,
) -> Result<(), String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("UPDATE users SET totp_secret = ?1 WHERE id = ?2")
        .bind(encrypted_secret)
        .bind(user_id)
        .execute(&conn)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Get the encrypted TOTP secret for a user.
pub async fn get_totp_secret(state: &SharedState, user_id: &str) -> Option<String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query("SELECT totp_secret FROM users WHERE id = ?1")
        .bind(user_id)
        .fetch_optional(&conn)
        .await
        .ok()?;
    row.and_then(|row| row.try_get::<String, _>(0).ok())
}

/// Enable or disable TOTP for a user.
pub async fn set_totp_enabled(
    state: &SharedState,
    user_id: &str,
    enabled: bool,
) -> Result<(), String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("UPDATE users SET totp_enabled = ?1 WHERE id = ?2")
        .bind(enabled as i32)
        .bind(user_id)
        .execute(&conn)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Check if TOTP is enabled for the user.
pub async fn is_totp_enabled(
    state: &SharedState,
    user_id: &str,
) -> Result<bool, crate::dtos::response::AppError> {
    use crate::dtos::response::AppError;
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query("SELECT totp_enabled FROM users WHERE id = ?1")
        .bind(user_id)
        .fetch_optional(&conn)
        .await?
        .ok_or(AppError::UserNotFound)?;
    // Missing, unreadable, or invalid state must never disable the second factor.
    match row.try_get::<Option<i64>, _>(0)? {
        Some(0) => Ok(false),
        Some(1) => Ok(true),
        _ => Err(AppError::InternalError("Invalid TOTP enabled state".into())),
    }
}

/// Store recovery codes as SHA-256 digests.
///
/// They used to be stored with `encrypt_data`, i.e. recoverable by anyone with
/// the database and `ENCRYPTION_KEY`. A recovery code is only ever checked, so
/// it belongs in the same shape as invitation and email-verification tokens: a
/// digest that proves a match and reveals nothing else.
pub async fn save_recovery_codes(
    state: &SharedState,
    user_id: &str,
    codes: &[String],
) -> Result<(), String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    for code in codes {
        let id = uuid::Uuid::now_v7().to_string();
        sqlx::query("INSERT INTO recovery_codes (id, user_id, code_digest) VALUES (?1, ?2, ?3)")
            .bind(id)
            .bind(user_id)
            .bind(security::digest_token(code))
            .execute(&conn)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Spend one recovery code. `true` if it existed, belonged to this user and was
/// unused; `false` otherwise.
///
/// A single conditional UPDATE, which does three jobs at once: the match happens
/// inside SQLite against an indexed digest rather than by decrypting every stored
/// code and comparing with `==` (which leaked match length through timing), and
/// the `used = 0` predicate makes the claim atomic, so two concurrent requests
/// cannot both spend the same code.
pub async fn claim_recovery_code(
    state: &SharedState,
    user_id: &str,
    code: &str,
) -> Result<bool, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let affected = sqlx::query(
        "UPDATE recovery_codes SET used = 1 WHERE user_id = ?1 AND code_digest = ?2 AND used = 0",
    )
    .bind(user_id)
    .bind(security::digest_token(code))
    .execute(&conn)
    .await
    .map_err(|e| e.to_string())?
    .rows_affected();
    Ok(affected > 0)
}

/// Count unused recovery codes.
pub async fn count_unused_recovery_codes(state: &SharedState, user_id: &str) -> i64 {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query("SELECT COUNT(*) FROM recovery_codes WHERE user_id = ?1 AND used = 0")
        .bind(user_id)
        .fetch_optional(&conn)
        .await;
    match row {
        Ok(row) => row
            .and_then(|row| row.try_get::<i64, _>(0).ok())
            .unwrap_or(0),
        Err(_) => 0,
    }
}

/// Delete all recovery codes for a user.
pub async fn delete_recovery_codes(state: &SharedState, user_id: &str) -> Result<(), String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("DELETE FROM recovery_codes WHERE user_id = ?1")
        .bind(user_id)
        .execute(&conn)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// List all sessions for a user (most recent first).
/// Returns (id, token, created_at, expires_at, user_agent).
pub async fn list_sessions(
    state: &SharedState,
    user_id: &str,
) -> Vec<(String, String, String, String, Option<String>)> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = match sqlx::query(
        "SELECT id, token_digest, created_at, expires_at, user_agent FROM sessions WHERE user_id = ?1 ORDER BY created_at DESC",
    )
    .bind(user_id)
    .fetch_all(&conn)
    .await
    {
        Ok(rows) => rows,
        Err(_) => return vec![],
    };
    let mut sessions = Vec::new();
    for row in &rows {
        sessions.push((
            row.try_get::<String, _>(0).unwrap_or_default(),
            row.try_get::<String, _>(1).unwrap_or_default(),
            row.try_get::<String, _>(2).unwrap_or_default(),
            row.try_get::<String, _>(3).unwrap_or_default(),
            row.try_get::<Option<String>, _>(4).unwrap_or(None),
        ));
    }
    sessions
}

#[cfg(test)]
mod security_tests {
    use super::*;

    #[tokio::test]
    async fn missing_null_and_invalid_totp_state_fail_closed() {
        let (state, _path) = crate::testing::test_state().await;
        assert!(is_totp_enabled(&state, "missing").await.is_err());
        for value in [None, Some(2_i64)] {
            sqlx::query("UPDATE users SET totp_enabled = ?1 WHERE id = 'viewer'")
                .bind(value)
                .execute(&state.db)
                .await
                .unwrap();
            assert!(is_totp_enabled(&state, "viewer").await.is_err());
        }
        for enabled in [false, true] {
            set_totp_enabled(&state, "viewer", enabled).await.unwrap();
            assert_eq!(is_totp_enabled(&state, "viewer").await.unwrap(), enabled);
        }
    }
}
