use axum::{
    Extension,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use axum_extra::extract::cookie::CookieJar;

use crate::cqrs::queries::account as account_queries;
use crate::dtos::response::{AppError, ok};
use crate::git::repositories::session;
use crate::security::middleware::auth::AuthUser;
use crate::state::state::SharedState;
use crate::utils::cookie::{SESSION_COOKIE, clear_session_cookie};

/// GET /api/v1/sessions
#[utoipa::path(
    get, path = "/api/v1/sessions", tag = "sessions",
    responses((status = 200, description = "List active sessions"), (status = 401, description = "Unauthorized"))
)]
pub async fn list_sessions_handler(
    State(state): State<SharedState>,
    Extension(auth_user): Extension<AuthUser>,
    jar: CookieJar,
) -> Result<impl IntoResponse, AppError> {
    let current_token = jar
        .get(SESSION_COOKIE)
        .map(|c| c.value().to_string())
        .unwrap_or_default();
    let sessions = account_queries::sessions(&state, &auth_user.user_id, &current_token).await?;
    Ok(ok(sessions))
}

/// POST /api/v1/sessions/current/revoke
/// Revokes the caller's own session. Always returns 204; the cookie is cleared
/// whether or not the session row still existed (idempotent).
#[utoipa::path(
    post, path = "/api/v1/sessions/current/revoke", tag = "sessions",
    responses((status = 204, description = "Session revoked"))
)]
pub async fn revoke_current_session_handler(
    State(state): State<SharedState>,
    jar: CookieJar,
) -> Result<impl IntoResponse, AppError> {
    if let Some(cookie) = jar.get(SESSION_COOKIE) {
        let _ = session::revoke_session(&state, cookie.value()).await;
    }
    Ok((
        clear_session_cookie(&state, jar).await,
        StatusCode::NO_CONTENT,
    ))
}

/// POST /api/v1/sessions/revoke-all
/// Revokes every session for the authenticated user, including the caller's
/// own. Clears the session cookie so the caller is logged out immediately.
#[utoipa::path(
    post, path = "/api/v1/sessions/revoke-all", tag = "sessions",
    responses((status = 204, description = "All sessions revoked"), (status = 401, description = "Unauthorized"))
)]
pub async fn revoke_all_sessions_handler(
    State(state): State<SharedState>,
    Extension(auth_user): Extension<AuthUser>,
    jar: CookieJar,
) -> Result<impl IntoResponse, AppError> {
    let _ = session::revoke_all_for_user(&state, &auth_user.user_id)
        .await
        .map_err(AppError::DatabaseError)?;
    let _ = crate::security::security_events::record(
        &state,
        Some(&auth_user.user_id),
        &auth_user,
        "sessions.revoked_all",
        None,
    )
    .await;
    Ok((
        clear_session_cookie(&state, jar).await,
        StatusCode::NO_CONTENT,
    ))
}

/// DELETE /api/v1/sessions/{session_id}
/// Revokes a single other session belonging to the user. Refuses to revoke the
/// caller's own session — use `POST /api/v1/sessions/current/revoke` for that.
#[utoipa::path(
    delete, path = "/api/v1/sessions/{session_id}", tag = "sessions",
    params(("session_id" = String, Path, description = "Session identifier")),
    responses(
        (status = 204, description = "Session revoked"),
        (status = 400, description = "Cannot revoke current session"),
        (status = 404, description = "Session not found"),
        (status = 401, description = "Unauthorized")
    )
)]
pub async fn delete_session_handler(
    State(state): State<SharedState>,
    Extension(auth_user): Extension<AuthUser>,
    Path(session_id): Path<String>,
    jar: CookieJar,
) -> Result<impl IntoResponse, AppError> {
    let current_token = jar
        .get(SESSION_COOKIE)
        .map(|c| c.value().to_string())
        .unwrap_or_default();

    let conn = crate::git::repositories::db::get_conn(&state).await;
    let row = sqlx::query("SELECT token_digest FROM sessions WHERE id = ?1 AND user_id = ?2")
        .bind(session_id.clone())
        .bind(auth_user.user_id.clone())
        .fetch_optional(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;

    let token_digest = match row {
        Some(row) => {
            use sqlx::Row;
            row.try_get::<String, _>(0).unwrap_or_default()
        }
        None => return Err(AppError::ValidationError("Session not found".into())),
    };

    // The database holds digests now, so the caller's cookie has to be digested
    // before it can be compared against a row.
    if token_digest == crate::utils::security::digest_token(&current_token) {
        return Err(AppError::ValidationError(
            "Cannot revoke your current session".into(),
        ));
    }

    let _ = session::revoke_session_by_id(&state, &session_id, &auth_user.user_id)
        .await
        .map_err(AppError::DatabaseError)?;
    let _ = crate::security::security_events::record(
        &state,
        Some(&auth_user.user_id),
        &auth_user,
        "session.revoked",
        Some(&session_id),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}
