use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use axum_extra::extract::cookie::CookieJar;

use crate::cqrs::commands::account as account_commands;
use crate::cqrs::queries::account as account_queries;
use crate::dtos::account::{
    ChangePasswordRequest, DeactivateAccountRequest, RegenerateCodesRequest, TotpDisableRequest,
    TotpVerifyRequest, UpdateAccountRequest,
};
use crate::dtos::response::{AppError, created, ok};
use crate::git::repositories::user;
use crate::security::middleware::auth::AuthUser;
use crate::state::state::SharedState;
use crate::utils::cookie::{SESSION_COOKIE, clear_session_cookie};

fn current_session_token(jar: &CookieJar) -> String {
    jar.get(SESSION_COOKIE)
        .map(|c| c.value().to_string())
        .unwrap_or_default()
}

/// GET /api/v1/account
#[utoipa::path(
    get, path = "/api/v1/account", tag = "account",
    responses((status = 200, description = "Account profile"), (status = 401, description = "Unauthorized"))
)]
pub async fn get_account_handler(
    State(state): State<SharedState>,
    Extension(auth_user): Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    let profile = account_queries::profile(&state, &auth_user.user_id).await?;
    Ok(ok(profile))
}

/// PATCH /api/v1/account
#[utoipa::path(
    patch, path = "/api/v1/account", tag = "account", request_body = UpdateAccountRequest,
    responses((status = 200, description = "Account updated"), (status = 401, description = "Unauthorized"))
)]
pub async fn patch_account_handler(
    State(state): State<SharedState>,
    Extension(auth_user): Extension<AuthUser>,
    Json(req): Json<UpdateAccountRequest>,
) -> Result<impl IntoResponse, AppError> {
    // Check name uniqueness if changed
    let current = user::get_user_by_id(&state, &auth_user.user_id)
        .await
        .ok_or(AppError::UserNotFound)?;

    if req.name != current.name && user::get_user_by_name(&state, &req.name).await.is_some() {
        return Err(AppError::AlreadyExists);
    }

    user::update_user(&state, &auth_user.user_id, Some(&req.name)).await?;
    let _ = crate::security::security_events::record(
        &state,
        Some(&auth_user.user_id),
        &auth_user,
        "account.updated",
        Some(&req.name),
    )
    .await;

    let profile = account_queries::profile(&state, &auth_user.user_id).await?;
    Ok(ok(profile))
}

/// DELETE /api/v1/account
#[utoipa::path(
    delete, path = "/api/v1/account", tag = "account", request_body = DeactivateAccountRequest,
    responses((status = 204, description = "Account deactivated"), (status = 401, description = "Unauthorized"), (status = 422, description = "Confirmation, password, or subscription prevents deactivation"))
)]
pub async fn deactivate_account_handler(
    State(state): State<SharedState>,
    Extension(auth_user): Extension<AuthUser>,
    jar: CookieJar,
    Json(req): Json<DeactivateAccountRequest>,
) -> Result<(CookieJar, StatusCode), AppError> {
    if req.confirmation != "ELIMINAR" {
        return Err(AppError::ValidationError(
            "Type ELIMINAR to deactivate your account".into(),
        ));
    }
    if auth_user.is_sysadmin {
        return Err(AppError::InsufficientPermissions);
    }
    if crate::billing::repository::get_account(&state, &auth_user.user_id)
        .await?
        .is_some_and(|account| matches!(account.status.as_str(), "active" | "trialing"))
    {
        return Err(AppError::ValidationError(
            "Cancel your active subscription before deactivating this account".into(),
        ));
    }

    account_commands::deactivate_account(&state, &auth_user.user_id, &req.current_password).await?;
    let _ = crate::security::security_events::record(
        &state,
        Some(&auth_user.user_id),
        &auth_user,
        "account.deactivated",
        None,
    )
    .await;
    Ok((
        clear_session_cookie(&state, jar).await,
        StatusCode::NO_CONTENT,
    ))
}

/// POST /api/v1/account/change-password
#[utoipa::path(
    post, path = "/api/v1/account/change-password", tag = "account", request_body = ChangePasswordRequest,
    responses((status = 200, description = "Password changed"), (status = 401, description = "Unauthorized"))
)]
pub async fn change_password_handler(
    State(state): State<SharedState>,
    Extension(auth_user): Extension<AuthUser>,
    jar: axum_extra::extract::cookie::CookieJar,
    Json(req): Json<ChangePasswordRequest>,
) -> Result<impl IntoResponse, AppError> {
    // Everything but this browser's own session gets revoked, so the user isn't
    // logged out of the tab they just used to change the password.
    let current_token = jar
        .get(crate::utils::cookie::SESSION_COOKIE)
        .map(|cookie| cookie.value().to_string());
    account_commands::change_password(
        &state,
        &auth_user.user_id,
        &req.current_password,
        &req.new_password,
        &req.confirm_password,
        current_token.as_deref(),
    )
    .await?;
    let _ = crate::security::security_events::record(
        &state,
        Some(&auth_user.user_id),
        &auth_user,
        "password.changed",
        None,
    )
    .await;
    Ok(ok(serde_json::json!({})))
}

/// POST /api/v1/account/totp/setup
#[utoipa::path(
    post, path = "/api/v1/account/totp/setup", tag = "account",
    responses((status = 200, description = "TOTP setup created"), (status = 401, description = "Unauthorized"))
)]
pub async fn totp_setup_handler(
    State(state): State<SharedState>,
    Extension(auth_user): Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    let user_row = user::get_user_by_id(&state, &auth_user.user_id)
        .await
        .ok_or(AppError::UserNotFound)?;

    let resp = account_commands::totp_setup(&state, &auth_user.user_id, &user_row.email).await?;
    Ok(ok(resp))
}

/// POST /api/v1/account/totp/verify
#[utoipa::path(
    post, path = "/api/v1/account/totp/verify", tag = "account", request_body = TotpVerifyRequest,
    responses((status = 201, description = "TOTP enabled"), (status = 401, description = "Unauthorized"))
)]
pub async fn totp_verify_handler(
    State(state): State<SharedState>,
    Extension(auth_user): Extension<AuthUser>,
    Json(req): Json<TotpVerifyRequest>,
) -> Result<impl IntoResponse, AppError> {
    let resp =
        account_commands::totp_verify_and_enable(&state, &auth_user.user_id, &req.code).await?;
    let _ = crate::security::security_events::record(
        &state,
        Some(&auth_user.user_id),
        &auth_user,
        "two_factor.enabled",
        None,
    )
    .await;
    Ok(created(resp))
}

/// POST /api/v1/account/totp/disable
#[utoipa::path(
    post, path = "/api/v1/account/totp/disable", tag = "account", request_body = TotpDisableRequest,
    responses((status = 200, description = "TOTP disabled"), (status = 401, description = "Unauthorized"))
)]
pub async fn totp_disable_handler(
    State(state): State<SharedState>,
    Extension(auth_user): Extension<AuthUser>,
    Json(req): Json<TotpDisableRequest>,
) -> Result<impl IntoResponse, AppError> {
    account_commands::totp_disable(&state, &auth_user.user_id, &req.code).await?;
    let _ = crate::security::security_events::record(
        &state,
        Some(&auth_user.user_id),
        &auth_user,
        "two_factor.disabled",
        None,
    )
    .await;
    Ok(ok(serde_json::json!({})))
}

/// POST /api/v1/account/recovery-codes
#[utoipa::path(
    post, path = "/api/v1/account/recovery-codes", tag = "account", request_body = RegenerateCodesRequest,
    responses((status = 200, description = "Recovery codes regenerated"), (status = 401, description = "Unauthorized"))
)]
pub async fn regenerate_codes_handler(
    State(state): State<SharedState>,
    Extension(auth_user): Extension<AuthUser>,
    Json(req): Json<RegenerateCodesRequest>,
) -> Result<impl IntoResponse, AppError> {
    let resp =
        account_commands::regenerate_recovery_codes(&state, &auth_user.user_id, &req.code).await?;
    let _ = crate::security::security_events::record(
        &state,
        Some(&auth_user.user_id),
        &auth_user,
        "recovery_codes.regenerated",
        None,
    )
    .await;
    Ok(ok(resp))
}

/// GET /api/v1/account/sessions
#[utoipa::path(
    get, path = "/api/v1/account/sessions", tag = "account",
    responses((status = 200, description = "Account sessions"), (status = 401, description = "Unauthorized"))
)]
pub async fn get_sessions_handler(
    State(state): State<SharedState>,
    Extension(auth_user): Extension<AuthUser>,
    jar: CookieJar,
) -> Result<impl IntoResponse, AppError> {
    let current_token = current_session_token(&jar);

    let sessions = account_queries::sessions(&state, &auth_user.user_id, &current_token).await?;
    Ok(ok(sessions))
}

/// DELETE /api/v1/account/sessions/{session_id}
#[utoipa::path(
    delete, path = "/api/v1/account/sessions/{session_id}", tag = "account",
    params(("session_id" = String, Path, description = "Session identifier")),
    responses((status = 204, description = "Session revoked"), (status = 401, description = "Unauthorized"))
)]
pub async fn delete_session_handler(
    State(state): State<SharedState>,
    Extension(auth_user): Extension<AuthUser>,
    Path(session_id): Path<String>,
    jar: CookieJar,
) -> Result<impl IntoResponse, AppError> {
    let current_token = current_session_token(&jar);

    // Lookup token for this session
    let conn = crate::git::repositories::db::get_conn(&state).await;
    let token_digest = {
        let row = sqlx::query("SELECT token_digest FROM sessions WHERE id = ?1 AND user_id = ?2")
            .bind(session_id.clone())
            .bind(auth_user.user_id.clone())
            .fetch_optional(&conn)
            .await?;

        match row {
            Some(row) => {
                use sqlx::Row;
                row.try_get::<String, _>(0).unwrap_or_default()
            }
            None => {
                // Not a browser session: an MCP agent grant revokes as a whole —
                // its tokens and its session record close together.
                let grant = crate::git::repositories::oauth::take_grant(
                    &state,
                    &session_id,
                    &auth_user.user_id,
                )
                .await
                .map_err(AppError::DatabaseError)?;
                return match grant {
                    Some(code_digest) => {
                        crate::git::repositories::oauth::revoke_by_code(&state, &code_digest)
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
                    None => Err(AppError::ValidationError("Session not found".into())),
                };
            }
        }
    };

    // The database holds digests now, so the caller's cookie has to be digested
    // before it can be compared against a row.
    if token_digest == crate::utils::security::digest_token(&current_token) {
        return Err(AppError::ValidationError(
            "Cannot revoke your current session".into(),
        ));
    }

    // Revoke by deleting from sessions table
    crate::git::repositories::db::retry_on_stale_snapshot(|| async {
        sqlx::query("DELETE FROM sessions WHERE id = ?1 AND user_id = ?2")
            .bind(session_id.clone())
            .bind(auth_user.user_id.clone())
            .execute(&conn)
            .await
            .map_err(Into::into)
    })
    .await?;

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

/// DELETE /api/v1/account/sessions — revoke every session for the current user,
/// including the active one. The response clears the session cookie so the
/// caller is logged out immediately.
#[utoipa::path(
    delete, path = "/api/v1/account/sessions", tag = "account",
    responses((status = 204, description = "All sessions revoked"), (status = 401, description = "Unauthorized"))
)]
pub async fn delete_all_sessions_handler(
    State(state): State<SharedState>,
    Extension(auth_user): Extension<AuthUser>,
    jar: CookieJar,
) -> Result<(CookieJar, StatusCode), AppError> {
    let _ = account_commands::revoke_all_sessions(&state, &auth_user.user_id).await?;
    let _ = crate::security::security_events::record(
        &state,
        Some(&auth_user.user_id),
        &auth_user,
        "sessions.revoked_all",
        None,
    )
    .await;
    let jar = clear_session_cookie(&state, jar).await;
    Ok((jar, StatusCode::NO_CONTENT))
}

/// DELETE /api/v1/account/sessions/others — revoke every other session for
/// the current user. The caller's own session is untouched, so they stay
/// logged in on this device.
#[utoipa::path(
    delete, path = "/api/v1/account/sessions/others", tag = "account",
    responses((status = 204, description = "Other sessions revoked"), (status = 401, description = "Unauthorized"))
)]
pub async fn delete_other_sessions_handler(
    State(state): State<SharedState>,
    Extension(auth_user): Extension<AuthUser>,
    jar: CookieJar,
) -> Result<StatusCode, AppError> {
    let current_token = current_session_token(&jar);
    let _ =
        account_commands::revoke_other_sessions(&state, &auth_user.user_id, &current_token).await?;
    let _ = crate::security::security_events::record(
        &state,
        Some(&auth_user.user_id),
        &auth_user,
        "sessions.revoked_others",
        None,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}
