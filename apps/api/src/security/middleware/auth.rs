use axum::{
    extract::Request, extract::State, http::StatusCode, middleware::Next, response::Response,
};
use axum_extra::extract::cookie::CookieJar;
use chrono::Utc;

use crate::git::repositories::{personal_access_token, session, user};
use crate::state::state::SharedState;

#[derive(Clone, Debug)]
pub struct AuthUser {
    pub user_id: String,
    pub email: String,
    pub name: String,
    pub is_sysadmin: bool,
    /// Set when this request authenticated via `Authorization: Bearer` (a
    /// personal access token) instead of the session cookie. `None` for a
    /// normal browser session. Carried on `AuthUser` — rather than looked up
    /// separately — so every existing caller that already has an `AuthUser`
    /// (e.g. for an audit log entry) gets token attribution for free.
    pub token_id: Option<String>,
    pub token_name: Option<String>,
}

/// The scopes of the personal access token that authenticated this request.
/// `org_scope.rs` reads this to cap the role a token can act with; absent
/// entirely for a cookie-authenticated request.
#[derive(Clone, Debug)]
pub struct TokenScopes(pub Vec<String>);

pub async fn auth_middleware(
    State(state): State<SharedState>,
    jar: CookieJar,
    mut request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let token = jar.get("session_token").map(|c| c.value().to_string());

    let (auth_user, token_scopes) = if let Some(token) = token {
        // All three lookups can fail with a database error if maintenance makes
        // the local file briefly unavailable.
        // window of a backup snapshot. Map those to 503 so the client doesn't
        // mistake them for a missing/invalid session and log the user out.
        let session_row = session::get_session_by_token_result(&state, &token)
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
            .ok_or(StatusCode::UNAUTHORIZED)?;

        let now = Utc::now().naive_utc();
        let expires =
            chrono::NaiveDateTime::parse_from_str(&session_row.expires_at, "%Y-%m-%d %H:%M:%S")
                .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        if now > expires {
            let _ = session::revoke_session(&state, &token).await;
            return Err(StatusCode::UNAUTHORIZED);
        }

        let user_row = user::get_user_by_id_result(&state, &session_row.user_id)
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
            .ok_or(StatusCode::UNAUTHORIZED)?;

        if !user::is_active(&user_row) {
            return Err(StatusCode::UNAUTHORIZED);
        }

        let is_sysadmin = user::is_sysadmin(&state, &user_row.id).await;

        (
            AuthUser {
                user_id: user_row.id,
                email: user_row.email,
                name: user_row.name,
                is_sysadmin,
                token_id: None,
                token_name: None,
            },
            None,
        )
    } else if let Some(bearer) = bearer_token(&request) {
        let token_row = personal_access_token::find_active_by_digest(&state, &bearer)
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
            .ok_or(StatusCode::UNAUTHORIZED)?;

        if let Some(expires_at) = &token_row.expires_at {
            let now = Utc::now().naive_utc();
            let expires = chrono::NaiveDateTime::parse_from_str(expires_at, "%Y-%m-%d %H:%M:%S")
                .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
            if now > expires {
                let _ = personal_access_token::revoke(&state, &token_row.id).await;
                return Err(StatusCode::UNAUTHORIZED);
            }
        }

        let user_row = user::get_user_by_id_result(&state, &token_row.user_id)
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
            .ok_or(StatusCode::UNAUTHORIZED)?;

        if !user::is_active(&user_row) {
            return Err(StatusCode::UNAUTHORIZED);
        }

        let is_sysadmin = user::is_sysadmin(&state, &user_row.id).await;
        let scopes: Vec<String> = serde_json::from_str(&token_row.scopes).unwrap_or_default();

        personal_access_token::touch_last_used(&state, &token_row.id).await;

        (
            AuthUser {
                user_id: user_row.id,
                email: user_row.email,
                name: user_row.name,
                is_sysadmin,
                token_id: Some(token_row.id),
                token_name: Some(token_row.name),
            },
            Some(TokenScopes(scopes)),
        )
    } else {
        return Err(StatusCode::UNAUTHORIZED);
    };

    request.extensions_mut().insert(auth_user);
    if let Some(scopes) = token_scopes {
        request.extensions_mut().insert(scopes);
    }
    Ok(next.run(request).await)
}

fn bearer_token(request: &Request) -> Option<String> {
    let value = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    value.strip_prefix("Bearer ").map(|s| s.trim().to_string())
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{StatusCode, header};
    use axum::routing::get;
    use axum::{Extension, Router, middleware};
    use tower::ServiceExt;

    use super::*;
    use crate::git::repositories::personal_access_token;
    use crate::testing::{cleanup_database, test_state};
    use crate::utils::security::generate_token;

    fn probe_router(state: SharedState) -> Router {
        async fn whoami(Extension(user): Extension<AuthUser>) -> String {
            user.token_id.unwrap_or_default()
        }
        Router::new()
            .route("/probe", get(whoami))
            .layer(middleware::from_fn_with_state(
                state.clone(),
                auth_middleware,
            ))
            .with_state(state)
    }

    fn bearer_request(token: &str) -> Request<Body> {
        Request::builder()
            .uri("/probe")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap()
    }

    #[tokio::test]
    async fn a_valid_bearer_token_authenticates_and_carries_its_own_id() {
        let (state, path) = test_state().await;
        let token = generate_token();
        personal_access_token::create(
            &state,
            "token-1",
            "editor",
            "ci",
            &token,
            "lp_abc",
            "[\"read\"]",
            None,
        )
        .await
        .unwrap();

        let response = probe_router(state.clone())
            .oneshot(bearer_request(&token))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(std::str::from_utf8(&body).unwrap(), "token-1");

        cleanup_database(&path);
    }

    #[tokio::test]
    async fn a_garbage_bearer_token_is_rejected() {
        let (state, path) = test_state().await;
        let response = probe_router(state)
            .oneshot(bearer_request("not-a-real-token"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn an_expired_bearer_token_is_rejected_and_revoked() {
        let (state, path) = test_state().await;
        let token = generate_token();
        personal_access_token::create(
            &state,
            "token-1",
            "editor",
            "ci",
            &token,
            "lp_abc",
            "[\"read\"]",
            Some("2000-01-01 00:00:00"),
        )
        .await
        .unwrap();

        let response = probe_router(state.clone())
            .oneshot(bearer_request(&token))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(
            personal_access_token::find_active_by_digest(&state, &token)
                .await
                .unwrap()
                .is_none(),
            "an expired token should be revoked on use, same as an expired session"
        );

        cleanup_database(&path);
    }
}
