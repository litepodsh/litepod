use axum::{
    Extension, Json,
    extract::{ConnectInfo, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use axum_extra::extract::cookie::CookieJar;

use std::net::SocketAddr;

use crate::cqrs::commands::auth;
use crate::cqrs::commands::invitation;
use crate::cqrs::queries::auth as auth_queries;
use crate::dtos::auth::{
    CompleteTotpLoginRequest, ConfirmEmailVerificationRequest, ConfirmEmailVerificationResponse,
    EmailVerificationPendingResponse, LoginErrorResponse, LoginRequest,
    PasswordResetConfirmRequest, PasswordResetRequest, RegisterRequest, RegistrationStatusResponse,
    ResendEmailVerificationRequest, SetupCheckResponse, SetupRequest, TotpChallengeResponse,
};
use crate::dtos::invitation::AcceptInvitationRequest;
use crate::dtos::response::{AppError, accepted, created, ok};
use crate::git::repositories::user;
use crate::security::middleware::auth::AuthUser;
use crate::security::middleware::rate_limit::{
    LOGIN_CAPTCHA_THRESHOLD, client_ip_from_parts, login_failed_attempts,
};
use crate::security::middleware::request_context::RequestContext;
use crate::state::state::SharedState;
use crate::utils::cookie::{SESSION_COOKIE, add_session_cookie, clear_session_cookie};

/// Mirrors a login/logout into the audit log of every organization the user
/// belongs to; `audit_log::record` drops it for orgs with audit logging off.
/// `metadata` (JSON) records how a login happened: method, 2FA, SSO protocol.
pub(crate) async fn audit_auth_event(
    state: &SharedState,
    actor: &AuthUser,
    action: &str,
    metadata: Option<&str>,
    context: Option<&RequestContext>,
) {
    let Ok(memberships) = crate::git::repositories::organization_member::list_memberships_for_user(
        state,
        &actor.user_id,
    )
    .await
    else {
        return;
    };
    for membership in memberships {
        let _ = crate::git::repositories::audit_log::record(
            state,
            &membership.organization_id,
            actor,
            action,
            Some("user"),
            Some(&actor.user_id),
            Some(&actor.email),
            metadata,
            context,
            Some(crate::git::repositories::audit_log::AuditOutcome::Success),
            None,
        )
        .await;
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/auth/setup",
    responses(
        (status = 200, description = "Setup status", body = SetupCheckResponse),
    ),
    tag = "auth"
)]
pub async fn setup_check_handler(
    State(state): State<SharedState>,
) -> Result<impl IntoResponse, AppError> {
    let count = user::count_users(&state)
        .await
        .map_err(|e| AppError::DatabaseError(e))?;
    let setup_required = user::count_sysadmins(&state)
        .await
        .map_err(AppError::DatabaseError)?
        == 0;
    Ok(ok(SetupCheckResponse {
        setup_required,
        user_count: count,
    }))
}

#[utoipa::path(
    get,
    path = "/api/v1/auth/registration-status",
    responses((status = 200, description = "Whether initial setup registration is allowed", body = RegistrationStatusResponse)),
    tag = "auth"
)]
pub async fn registration_status_handler(
    State(state): State<SharedState>,
) -> Result<impl IntoResponse, AppError> {
    let setup_required = user::count_sysadmins(&state)
        .await
        .map_err(AppError::DatabaseError)?
        == 0;
    Ok(ok(RegistrationStatusResponse {
        setup_required,
        public_registration_open: state.is_cloud || setup_required,
        is_cloud: state.is_cloud,
    }))
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/setup",
    request_body = SetupRequest,
    responses(
        (status = 201, description = "Admin account created"),
        (status = 409, description = "Setup already completed"),
    ),
    tag = "auth"
)]
pub async fn setup_handler(
    State(state): State<SharedState>,
    jar: CookieJar,
    headers: HeaderMap,
    Json(req): Json<SetupRequest>,
) -> Result<(CookieJar, axum::response::Response), AppError> {
    if req.password != req.confirm_password {
        return Err(AppError::ValidationError(
            "Passwords do not match".to_string(),
        ));
    }
    let count = user::count_sysadmins(&state)
        .await
        .map_err(|e| AppError::DatabaseError(e))?;
    if count > 0 {
        return Err(AppError::RegistrationClosed);
    }
    let user_agent = headers.get("user-agent").and_then(|v| v.to_str().ok());
    let (resp, token) =
        auth::register(&state, &req.email, &req.password, &req.name, user_agent).await?;
    if !state.is_cloud {
        // Kept for legacy standard installations with the seeded root organization.
        let _ = crate::git::repositories::organization_member::add_member(
            &state,
            "00000000-0000-0000-0000-000000000000",
            &resp.user.user_id,
            1,
        )
        .await;
    }
    let jar = add_session_cookie(&state, jar, token).await;
    Ok((jar, created(resp).into_response()))
}

pub async fn confirm_email_verification_handler(
    State(state): State<SharedState>,
    Json(req): Json<ConfirmEmailVerificationRequest>,
) -> Result<impl IntoResponse, AppError> {
    let result = crate::users::email_verification::confirm(&state, &req.email, &req.code).await?;
    Ok(ok(ConfirmEmailVerificationResponse {
        verified: result.verified,
        reactivated: result.reactivated,
    }))
}

pub async fn resend_email_verification_handler(
    State(state): State<SharedState>,
    Json(req): Json<ResendEmailVerificationRequest>,
) -> Result<impl IntoResponse, AppError> {
    crate::users::email_verification::resend(&state, &req.email).await?;
    Ok(accepted(()))
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/register",
    request_body = RegisterRequest,
    responses(
        (status = 201, description = "User registered"),
        (status = 409, description = "Email already exists"),
    ),
    tag = "auth"
)]
pub async fn register_handler(
    State(state): State<SharedState>,
    jar: CookieJar,
    headers: HeaderMap,
    axum::extract::ConnectInfo(connect_info): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Json(req): Json<RegisterRequest>,
) -> Result<(CookieJar, axum::response::Response), AppError> {
    if req.password != req.confirm_password {
        return Err(AppError::ValidationError(
            "Passwords do not match".to_string(),
        ));
    }
    if state.is_cloud {
        crate::users::legal::require_accepted(true, req.accept_terms)?;
        let turnstile_token = req
            .turnstile_token
            .as_deref()
            .filter(|t| !t.is_empty())
            .ok_or_else(|| {
                tracing::warn!("register: rejected, request had no turnstile_token");
                AppError::TurnstileFailed
            })?;
        let remote_ip = crate::security::middleware::rate_limit::client_ip_from_parts(
            &headers,
            Some(connect_info),
        );
        crate::security::turnstile::verify(turnstile_token, remote_ip).await?;

        let acceptance = crate::users::legal::Acceptance {
            ip: Some(remote_ip.to_string()),
            user_agent: headers
                .get("user-agent")
                .and_then(|v| v.to_str().ok())
                .map(str::to_string),
        };
        let result = crate::users::email_verification::register(
            &state,
            &req.email,
            &req.password,
            &req.username,
            &acceptance,
        )
        .await?;
        return Ok((
            jar,
            accepted(EmailVerificationPendingResponse {
                email: result.email,
                accepted_deliveries: result.accepted_deliveries,
                resend_available_at: result.resend_available_at,
                delivery_failed: result.delivery_failed,
                reactivating: result.reactivating,
            })
            .into_response(),
        ));
    }
    let user_agent = headers.get("user-agent").and_then(|v| v.to_str().ok());
    let (resp, token) =
        auth::register(&state, &req.email, &req.password, &req.username, user_agent).await?;
    let jar = add_session_cookie(&state, jar, token).await;
    Ok((jar, created(resp).into_response()))
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/invited-register",
    request_body = AcceptInvitationRequest,
    responses((status = 201, description = "Invitation accepted and account activated")),
    tag = "auth"
)]
pub async fn invited_register_handler(
    State(state): State<SharedState>,
    jar: CookieJar,
    headers: HeaderMap,
    axum::extract::ConnectInfo(connect_info): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Json(req): Json<AcceptInvitationRequest>,
) -> Result<(CookieJar, impl IntoResponse), AppError> {
    if req.password != req.confirm_password {
        return Err(AppError::ValidationError(
            "Passwords do not match".to_string(),
        ));
    }
    if req.password.len() < 8 {
        return Err(AppError::ValidationError(
            "Password must be at least 8 characters".to_string(),
        ));
    }
    crate::users::legal::require_accepted(state.is_cloud, req.accept_terms)?;
    let user_agent = headers.get("user-agent").and_then(|v| v.to_str().ok());
    let (resp, token) =
        invitation::accept_invitation(&state, &req.token, &req.password, user_agent).await?;
    if state.is_cloud {
        let acceptance = crate::users::legal::Acceptance {
            ip: Some(
                crate::security::middleware::rate_limit::client_ip_from_parts(
                    &headers,
                    Some(connect_info),
                )
                .to_string(),
            ),
            user_agent: user_agent.map(str::to_string),
        };
        crate::users::legal::record(&state, &resp.user.user_id, &acceptance).await?;
    }
    Ok((add_session_cookie(&state, jar, token).await, created(resp)))
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/login",
    request_body = LoginRequest,
    responses(
        (status = 200, description = "Login successful"),
        (status = 401, description = "Invalid credentials"),
    ),
    tag = "auth"
)]
pub async fn login_handler(
    State(state): State<SharedState>,
    jar: CookieJar,
    headers: HeaderMap,
    ConnectInfo(connect_info): ConnectInfo<SocketAddr>,
    context: Option<Extension<RequestContext>>,
    Json(req): Json<LoginRequest>,
) -> Result<(CookieJar, axum::response::Response), AppError> {
    let remote_ip = client_ip_from_parts(&headers, Some(connect_info));
    let prior_failures = login_failed_attempts(&state, remote_ip).await;
    let captcha_required = state.is_cloud && prior_failures >= LOGIN_CAPTCHA_THRESHOLD;

    if captcha_required {
        let verified = match req.turnstile_token.as_deref().filter(|t| !t.is_empty()) {
            Some(token) => crate::security::turnstile::verify(token, remote_ip)
                .await
                .is_ok(),
            None => false,
        };
        if !verified {
            let body = Json(LoginErrorResponse {
                error: AppError::TurnstileFailed.to_string(),
                captcha_required: true,
            });
            return Ok((jar, (StatusCode::FORBIDDEN, body).into_response()));
        }
    }

    let user_agent = headers.get("user-agent").and_then(|v| v.to_str().ok());
    match auth::login(&state, &req.email, &req.password, user_agent).await {
        Ok(auth::LoginOutcome::Authenticated(outcome)) => {
            let (resp, token) = *outcome;
            let actor = crate::security::middleware::auth::AuthUser {
                user_id: resp.user.user_id.clone(),
                email: resp.user.email.clone(),
                name: resp.user.name.clone(),
                is_sysadmin: resp.user.is_sysadmin,
                token_id: None,
                token_name: None,
            };
            let _ = crate::security::security_events::record(
                &state,
                Some(&actor.user_id),
                &actor,
                "auth.login",
                None,
            )
            .await;
            let metadata = serde_json::json!({ "method": "password", "mfa": false }).to_string();
            audit_auth_event(
                &state,
                &actor,
                "auth.login",
                Some(&metadata),
                context.as_deref(),
            )
            .await;
            let jar = add_session_cookie(&state, jar, token).await;
            Ok((jar, ok(resp).into_response()))
        }
        // Password accepted, second factor still owed. Deliberately no cookie on
        // this path — the jar goes back untouched.
        Ok(auth::LoginOutcome::TotpRequired { challenge_token }) => Ok((
            jar,
            ok(TotpChallengeResponse {
                totp_required: true,
                challenge_token,
            })
            .into_response(),
        )),
        Err(AppError::InvalidCredentials) => {
            // `login_ban_middleware` increments the same counter after this response lands,
            // so predict its post-increment value to tell the frontend whether the *next*
            // attempt needs the widget.
            let next_captcha_required =
                state.is_cloud && (prior_failures + 1) >= LOGIN_CAPTCHA_THRESHOLD;
            let _ = crate::security::security_events::record_anonymous(
                &state,
                "auth.login_failed",
                Some(&req.email),
            )
            .await;
            let body = Json(LoginErrorResponse {
                error: AppError::InvalidCredentials.to_string(),
                captcha_required: next_captcha_required,
            });
            Ok((jar, (StatusCode::UNAUTHORIZED, body).into_response()))
        }
        Err(e) => Err(e),
    }
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/login/totp",
    request_body = CompleteTotpLoginRequest,
    responses(
        (status = 200, description = "Second factor accepted; session issued"),
        (status = 400, description = "Wrong or already-used code"),
        (status = 401, description = "Challenge expired, already spent, or unknown"),
    ),
    tag = "auth"
)]
pub async fn login_totp_handler(
    State(state): State<SharedState>,
    jar: CookieJar,
    headers: HeaderMap,
    context: Option<Extension<RequestContext>>,
    Json(req): Json<CompleteTotpLoginRequest>,
) -> Result<(CookieJar, axum::response::Response), AppError> {
    let user_agent = headers.get("user-agent").and_then(|v| v.to_str().ok());
    let (resp, token, mfa_method) =
        auth::complete_totp_login(&state, &req.challenge_token, &req.code, user_agent).await?;
    let actor = crate::security::middleware::auth::AuthUser {
        user_id: resp.user.user_id.clone(),
        email: resp.user.email.clone(),
        name: resp.user.name.clone(),
        is_sysadmin: resp.user.is_sysadmin,
        token_id: None,
        token_name: None,
    };
    let _ = crate::security::security_events::record(
        &state,
        Some(&actor.user_id),
        &actor,
        "auth.login",
        None,
    )
    .await;
    let metadata =
        serde_json::json!({ "method": "password", "mfa": true, "mfa_method": mfa_method })
            .to_string();
    audit_auth_event(
        &state,
        &actor,
        "auth.login",
        Some(&metadata),
        context.as_deref(),
    )
    .await;
    Ok((
        add_session_cookie(&state, jar, token).await,
        ok(resp).into_response(),
    ))
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/password-reset/request",
    request_body = PasswordResetRequest,
    responses((status = 202, description = "Accepted, whether or not the address has an account")),
    tag = "auth"
)]
pub async fn password_reset_request_handler(
    State(state): State<SharedState>,
    Json(req): Json<PasswordResetRequest>,
) -> Result<impl IntoResponse, AppError> {
    let base = crate::handler::settings::public_base_url(&state).await;
    crate::users::password_reset::request(&state, &req.email, &base).await?;
    // Deliberately identical for a known and an unknown address.
    Ok(accepted(serde_json::json!({})))
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/password-reset/confirm",
    request_body = PasswordResetConfirmRequest,
    responses(
        (status = 200, description = "Password replaced; every session revoked"),
        (status = 401, description = "Token expired, already used, or unknown"),
        (status = 422, description = "Passwords do not match or are too short"),
    ),
    tag = "auth"
)]
pub async fn password_reset_confirm_handler(
    State(state): State<SharedState>,
    jar: CookieJar,
    Json(req): Json<PasswordResetConfirmRequest>,
) -> Result<(CookieJar, axum::response::Response), AppError> {
    crate::users::password_reset::confirm(&state, &req.token, &req.password, &req.confirm_password)
        .await?;
    // Every session for the account is gone, including whatever this browser
    // was holding — clear its cookie rather than leave a token that now 401s.
    Ok((
        clear_session_cookie(&state, jar).await,
        ok(serde_json::json!({})).into_response(),
    ))
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/logout",
    responses(
        (status = 200, description = "Logged out", body = LogoutResponse),
    ),
    tag = "auth"
)]
pub async fn logout_handler(
    State(state): State<SharedState>,
    jar: CookieJar,
    context: Option<Extension<RequestContext>>,
) -> Result<(CookieJar, impl IntoResponse), AppError> {
    let mut redirect_url = None;
    let mut reauth = false;
    if let Some(cookie) = jar.get(SESSION_COOKIE) {
        // Read the SSO origin and resolve the user before the row is deleted.
        let origin =
            crate::git::repositories::session::sso_origin_by_token(&state, cookie.value()).await;
        if let Ok(Some(session)) =
            crate::git::repositories::session::get_session_by_token_result(&state, cookie.value())
                .await
            && let Some(user_row) = user::get_user_by_id(&state, &session.user_id).await
        {
            let actor = AuthUser {
                is_sysadmin: user::is_sysadmin(&state, &user_row.id).await,
                user_id: user_row.id,
                email: user_row.email,
                name: user_row.name,
                token_id: None,
                token_name: None,
            };
            audit_auth_event(&state, &actor, "auth.logout", None, context.as_deref()).await;
        }
        // The local session goes first and unconditionally.
        let _ = auth::logout(&state, cookie.value()).await;

        if let Some(origin) = origin {
            redirect_url = idp_logout_url(&state, &origin).await;
            // No way to end the IdP session from here: force credentials next time.
            reauth = redirect_url.is_none();
        }
    }
    let _ = crate::security::security_events::record_anonymous(&state, "auth.logout", None).await;
    let mut jar = clear_session_cookie(&state, jar).await;
    if reauth {
        jar = crate::utils::cookie::add_sso_reauth_cookie(&state, jar).await;
    }
    Ok((jar, ok(LogoutResponse { redirect_url })))
}

#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct LogoutResponse {
    /// Send the browser here to end the identity provider's session too.
    pub redirect_url: Option<String>,
}

/// The IdP's logout URL for an enabled OIDC connection, else `None`.
async fn idp_logout_url(
    state: &SharedState,
    origin: &crate::git::repositories::session::SsoOrigin,
) -> Option<String> {
    let mut conn = state.db.acquire().await.ok()?;
    let connection = crate::ee::sso::connection(&mut conn, &origin.connection_id)
        .await
        .ok()??;
    drop(conn);
    if !connection.enabled || connection.kind != "oidc" {
        return None;
    }
    let public = crate::handler::settings::public_base_url(state).await;
    crate::ee::sso::oidc::end_session_url(
        &connection,
        origin.id_token_encrypted.as_deref(),
        &public,
    )
    .await
}

#[utoipa::path(
    get,
    path = "/api/v1/auth/me",
    responses(
        (status = 200, description = "Current user profile"),
        (status = 401, description = "Unauthorized"),
    ),
    tag = "auth"
)]
pub async fn me_handler(
    State(state): State<SharedState>,
    Extension(auth_user): Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    let resp = auth_queries::me(&state, &auth_user.user_id).await?;
    Ok(ok(resp))
}

#[cfg(test)]
mod tests {
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use super::*;
    use crate::testing::cleanup_database;

    #[tokio::test]
    async fn logout_of_an_sso_session_redirects_to_the_idp_or_sets_reauth() {
        let (state, path, _idp) = crate::ee::sso::oidc::tests::setup().await;
        let sso = crate::ee::sso::SsoSession {
            connection_id: "c1".into(),
            subject: "sub-ana".into(),
            ttl_minutes: 60,
            id_token_encrypted: None,
        };
        let app = axum::Router::new()
            .route("/api/v1/auth/logout", axum::routing::post(logout_handler))
            .with_state(state.clone());
        let logout = |token: String| {
            let app = app.clone();
            async move {
                let response = app
                    .oneshot(
                        Request::post("/api/v1/auth/logout")
                            .header("cookie", format!("{SESSION_COOKIE}={token}"))
                            .body(Body::empty())
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                assert_eq!(response.status(), StatusCode::OK, "logout never fails");
                let cookies: Vec<String> = response
                    .headers()
                    .get_all("set-cookie")
                    .iter()
                    .map(|v| v.to_str().unwrap().to_string())
                    .collect();
                let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
                let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                (body["data"]["redirect_url"].clone(), cookies)
            }
        };
        let reauth_set =
            |cookies: &[String]| cookies.iter().any(|c| c.starts_with("lp_sso_reauth=1"));

        // OIDC with end_session_endpoint: redirect, no reauth cookie.
        let token = auth::create_session(&state, "editor", None, Some(&sso))
            .await
            .unwrap();
        let (redirect, cookies) = logout(token.clone()).await;
        assert!(
            redirect.as_str().unwrap().contains("/logout?"),
            "{redirect}"
        );
        assert!(!reauth_set(&cookies));
        assert!(
            crate::git::repositories::session::get_session_by_token_result(&state, &token)
                .await
                .unwrap()
                .is_none()
        );

        // Connection unusable (disabled here; SAML or no endpoint alike): reauth cookie.
        sqlx::query("UPDATE sso_connections SET enabled = 0")
            .execute(&state.db)
            .await
            .unwrap();
        let token = auth::create_session(&state, "editor", None, Some(&sso))
            .await
            .unwrap();
        let (redirect, cookies) = logout(token).await;
        assert!(redirect.is_null());
        assert!(reauth_set(&cookies));

        // Password session: neither.
        let token = auth::create_session(&state, "editor", None, None)
            .await
            .unwrap();
        let (redirect, cookies) = logout(token).await;
        assert!(redirect.is_null());
        assert!(!reauth_set(&cookies));
        cleanup_database(&path);
    }
}
