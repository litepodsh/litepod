use sqlx::Row;

use crate::state::state::SharedState;
use crate::utils::security::digest_token as digest;

pub struct SessionRow {
    #[allow(dead_code)]
    pub id: String,
    pub user_id: String,
    /// SHA-256 of the token the client holds, never the token itself. See
    /// `migrations/0007_hash_session_tokens.sql`.
    #[allow(dead_code)]
    pub token_digest: String,
    #[allow(dead_code)]
    pub expires_at: String,
}

/// `token` is the plaintext handed to the client; only its digest is stored.
pub async fn create_session(
    state: &SharedState,
    id: &str,
    user_id: &str,
    token: &str,
    expires_at: &str,
    user_agent: Option<&str>,
) -> Result<(), String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "INSERT INTO sessions (id, user_id, token_digest, expires_at, user_agent) VALUES (?1, ?2, ?3, ?4, ?5)",
    )
    .bind(id)
    .bind(user_id)
    .bind(digest(token))
    .bind(expires_at)
    .bind(user_agent)
    .execute(&conn)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Same as `create_session`, recording which SSO connection and subject the
/// session belongs to (for IdP logout and back-channel logout).
pub async fn create_sso_session(
    state: &SharedState,
    id: &str,
    user_id: &str,
    token: &str,
    expires_at: &str,
    user_agent: Option<&str>,
    sso: &crate::ee::sso::SsoSession,
) -> Result<(), String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "INSERT INTO sessions
            (id, user_id, token_digest, expires_at, user_agent, sso_connection_id, sso_subject, id_token)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )
    .bind(id)
    .bind(user_id)
    .bind(digest(token))
    .bind(expires_at)
    .bind(user_agent)
    .bind(&sso.connection_id)
    .bind(&sso.subject)
    .bind(&sso.id_token_encrypted)
    .execute(&conn)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub struct SsoOrigin {
    pub connection_id: String,
    pub id_token_encrypted: Option<String>,
}

/// The SSO origin of the session holding `token`; `None` for password
/// sessions, unknown tokens, or a database error (logout must not fail).
pub async fn sso_origin_by_token(state: &SharedState, token: &str) -> Option<SsoOrigin> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(
        "SELECT sso_connection_id, id_token FROM sessions
          WHERE token_digest = ?1 AND sso_connection_id IS NOT NULL",
    )
    .bind(digest(token))
    .fetch_optional(&conn)
    .await
    .ok()??;
    Some(SsoOrigin {
        connection_id: row.try_get(0).ok()?,
        id_token_encrypted: row.try_get(1).ok()?,
    })
}

/// Back-channel logout: every session this IdP subject has through this connection.
pub async fn revoke_for_subject(
    state: &SharedState,
    connection_id: &str,
    subject: &str,
) -> Result<u64, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("DELETE FROM sessions WHERE sso_connection_id = ?1 AND sso_subject = ?2")
        .bind(connection_id)
        .bind(subject)
        .execute(&conn)
        .await
        .map(|r| r.rows_affected())
        .map_err(|e| e.to_string())
}

/// Result-returning variant: distinguishes "no row" (Ok(None)) from a database
/// error (Err). The middleware uses this so a transient DB outage (e.g. the
/// brief database maintenance) maps to 503 instead of
/// being misreported as 401 "session expired" and triggering a logout.
pub async fn get_session_by_token_result(
    state: &SharedState,
    token: &str,
) -> Result<Option<SessionRow>, crate::dtos::response::AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(
        "SELECT id, user_id, token_digest, expires_at FROM sessions WHERE token_digest = ?1",
    )
    .bind(digest(token))
    .fetch_optional(&conn)
    .await
    .map_err(|e| crate::dtos::response::AppError::DatabaseError(e.to_string()))?;

    Ok(row.map(|row| SessionRow {
        id: row.try_get(0).unwrap(),
        user_id: row.try_get(1).unwrap(),
        token_digest: row.try_get(2).unwrap(),
        expires_at: row.try_get(3).unwrap(),
    }))
}

pub async fn revoke_session(state: &SharedState, token: &str) -> Result<(), String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("DELETE FROM sessions WHERE token_digest = ?1")
        .bind(digest(token))
        .execute(&conn)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn revoke_session_by_id(
    state: &SharedState,
    session_id: &str,
    user_id: &str,
) -> Result<u64, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let affected = sqlx::query("DELETE FROM sessions WHERE id = ?1 AND user_id = ?2")
        .bind(session_id)
        .bind(user_id)
        .execute(&conn)
        .await
        .map_err(|e| e.to_string())?
        .rows_affected();
    Ok(affected)
}

/// Revoke every session this user has except the one making the request.
///
/// Used after a password change: the point is to evict anyone who got in with
/// the old password, without logging out the browser doing the changing. Pass
/// `None` to evict everything, which is what a password *reset* does — there the
/// person holding the old password may be exactly who we are locking out.
pub async fn revoke_all_for_user_except(
    state: &SharedState,
    user_id: &str,
    keep_token: Option<&str>,
) -> Result<u64, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let affected = match keep_token {
        Some(token) => {
            sqlx::query("DELETE FROM sessions WHERE user_id = ?1 AND token_digest != ?2")
                .bind(user_id)
                .bind(digest(token))
                .execute(&conn)
                .await
        }
        None => {
            sqlx::query("DELETE FROM sessions WHERE user_id = ?1")
                .bind(user_id)
                .execute(&conn)
                .await
        }
    };
    affected
        .map(|result| result.rows_affected())
        .map_err(|e| e.to_string())
}

pub async fn revoke_all_for_user(state: &SharedState, user_id: &str) -> Result<u64, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let affected = sqlx::query("DELETE FROM sessions WHERE user_id = ?1")
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
    async fn sso_sessions_use_the_connection_ttl_and_can_be_revoked_by_subject() {
        let (state, path) = test_state().await;
        let sso = crate::ee::sso::SsoSession {
            connection_id: "c1".into(),
            subject: "sub-ana".into(),
            ttl_minutes: 60,
            id_token_encrypted: Some("enc".into()),
        };
        let token = crate::cqrs::commands::auth::create_session(&state, "editor", None, Some(&sso))
            .await
            .unwrap();
        let password_token =
            crate::cqrs::commands::auth::create_session(&state, "editor", None, None)
                .await
                .unwrap();

        let minutes_left = |token: String| {
            let state = state.clone();
            async move {
                let row = get_session_by_token_result(&state, &token)
                    .await
                    .unwrap()
                    .unwrap();
                let at =
                    chrono::NaiveDateTime::parse_from_str(&row.expires_at, "%Y-%m-%d %H:%M:%S")
                        .unwrap();
                (at - chrono::Utc::now().naive_utc()).num_minutes()
            }
        };
        assert!(
            (58..=60).contains(&minutes_left(token.clone()).await),
            "SSO: connection TTL"
        );
        assert!(
            (4318..=4320).contains(&minutes_left(password_token.clone()).await),
            "password: 3 days"
        );

        let origin = sso_origin_by_token(&state, &token).await.unwrap();
        assert_eq!(origin.connection_id, "c1");
        assert_eq!(origin.id_token_encrypted.as_deref(), Some("enc"));
        assert!(sso_origin_by_token(&state, &password_token).await.is_none());

        assert_eq!(
            revoke_for_subject(&state, "c1", "sub-ana").await.unwrap(),
            1
        );
        assert!(
            get_session_by_token_result(&state, &token)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            get_session_by_token_result(&state, &password_token)
                .await
                .unwrap()
                .is_some()
        );
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn the_plaintext_token_is_never_written_to_the_database() {
        let (state, path) = test_state().await;
        let token = generate_token();
        create_session(
            &state,
            "session-1",
            "editor",
            &token,
            "2999-01-01 00:00:00",
            None,
        )
        .await
        .unwrap();

        let stored: String = sqlx::query("SELECT token_digest FROM sessions WHERE id = ?1")
            .bind("session-1")
            .fetch_one(&state.db)
            .await
            .unwrap()
            .try_get(0)
            .unwrap();

        assert_ne!(stored, token, "the cookie value itself must not be stored");
        assert_eq!(stored, crate::utils::security::digest_token(&token));

        // And the digest is still what a lookup resolves by.
        let found = get_session_by_token_result(&state, &token).await.unwrap();
        assert_eq!(found.map(|row| row.user_id).as_deref(), Some("editor"));

        cleanup_database(&path);
    }

    #[tokio::test]
    async fn revoking_by_token_and_keeping_the_current_one_both_work_on_digests() {
        let (state, path) = test_state().await;
        let keep = generate_token();
        let other = generate_token();
        create_session(&state, "keep", "editor", &keep, "2999-01-01 00:00:00", None)
            .await
            .unwrap();
        create_session(
            &state,
            "other",
            "editor",
            &other,
            "2999-01-01 00:00:00",
            None,
        )
        .await
        .unwrap();

        revoke_all_for_user_except(&state, "editor", Some(&keep))
            .await
            .unwrap();

        assert!(
            get_session_by_token_result(&state, &keep)
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            get_session_by_token_result(&state, &other)
                .await
                .unwrap()
                .is_none()
        );

        revoke_session(&state, &keep).await.unwrap();
        assert!(
            get_session_by_token_result(&state, &keep)
                .await
                .unwrap()
                .is_none()
        );

        cleanup_database(&path);
    }
}
