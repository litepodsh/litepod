use axum::{
    extract::{Request, State},
    http::{Method, StatusCode},
    middleware::Next,
    response::Response,
};

use crate::state::state::SharedState;

pub fn cors_allowed_origins() -> Vec<String> {
    let origins_str = std::env::var("CORS_ORIGINS")
        .unwrap_or_else(|_| "http://localhost:5173,http://localhost:6002".to_string());
    origins_str
        .split(',')
        .map(|s| normalize_origin(s.trim()))
        .collect()
}

/// The request's `Origin` when it is one of the allowed UI origins, so a
/// browser can be sent back to the UI it came from (e.g. `localhost:6002` in
/// dev while `PUBLIC_URL` is a tunnel to the API).
pub fn trusted_request_origin(headers: &axum::http::HeaderMap) -> Option<String> {
    let origin = normalize_origin(headers.get("origin")?.to_str().ok()?);
    cors_allowed_origins().contains(&origin).then_some(origin)
}

/// Strips a trailing `/` and the scheme's default port (`:80` on `http://`,
/// `:443` on `https://`) so config values like `PUBLIC_URL=http://1.2.3.4:80`
/// still match the `Origin: http://1.2.3.4` header browsers actually send
/// (they omit the default port). Anything that isn't a recognized default
/// port pairing is left untouched.
fn normalize_origin(origin: &str) -> String {
    let origin = origin.trim_end_matches('/');
    if let Some(host) = origin
        .strip_prefix("http://")
        .and_then(|rest| rest.strip_suffix(":80"))
    {
        format!("http://{host}")
    } else if let Some(host) = origin
        .strip_prefix("https://")
        .and_then(|rest| rest.strip_suffix(":443"))
    {
        format!("https://{host}")
    } else {
        origin.to_string()
    }
}

/// Path prefixes hit server-to-server by third parties (Polar, git providers,
/// keyless deploy hooks). They carry no browser Origin/Referer and are
/// authenticated by their own signature/shared-secret/single-use token
/// instead, so the CSRF origin check doesn't apply to them.
const ORIGIN_GUARD_EXEMPT_PREFIXES: &[&str] = &[
    "/api/v1/webhooks/polar",
    "/api/v1/webhooks/git/",
    "/deploy/",
    // Called server-to-server by the IdP's provisioning service (no Origin) and
    // authenticated by a bearer token, never a cookie, so there is nothing to forge.
    "/scim/v2/",
    // The IdP's browser POST (cross-site by design); trust comes from the signed
    // assertion and the single-use request id, not from a cookie.
    "/api/v1/auth/saml/acs",
    // OIDC back-channel logout: the IdP calls server to server (no Origin);
    // trust comes from the signed logout token, never a cookie.
    "/api/v1/auth/sso/backchannel/",
];

/// The MCP endpoint (`handler::mcp`), called by the user's own agent with no
/// browser Origin.
fn is_mcp_path(path: &str) -> bool {
    path.strip_prefix("/api/v1/organizations/")
        .and_then(|rest| rest.strip_suffix("/mcp"))
        .is_some_and(|organization| !organization.is_empty() && !organization.contains('/'))
}

/// Authenticated by a bearer token and nothing else. A forged cross-site
/// request cannot carry one: a browser only attaches `Authorization` to a
/// cross-origin request after a CORS preflight, which `cors_allowed_origins`
/// restricts. A session cookie alongside it disqualifies the request, because
/// `auth_middleware` would authenticate the cookie, not the token.
fn bearer_only(request: &Request) -> bool {
    let headers = request.headers();
    let bearer = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("Bearer "));
    let session_cookie = headers
        .get_all(axum::http::header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .any(|cookies| {
            cookies
                .split(';')
                .any(|cookie| cookie.trim_start().starts_with("session_token="))
        });
    bearer && !session_cookie
}

pub async fn origin_guard(
    State(state): State<SharedState>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    if matches!(
        request.method(),
        &Method::GET | &Method::HEAD | &Method::OPTIONS
    ) {
        return Ok(next.run(request).await);
    }

    let path = request.uri().path();
    if ORIGIN_GUARD_EXEMPT_PREFIXES
        .iter()
        .any(|prefix| path.starts_with(prefix))
    {
        return Ok(next.run(request).await);
    }
    if is_mcp_path(path) && bearer_only(&request) {
        return Ok(next.run(request).await);
    }

    // CORS_ORIGINS is fixed at container start from PUBLIC_URL. When the operator later sets a
    // server domain in settings, requests now arrive with that domain's Origin and would 403.
    // public_base_url() resolves the configured domain (falling back to PUBLIC_URL), so the
    // active domain is always allowed without a redeploy.
    let mut allowed_origins = cors_allowed_origins();
    allowed_origins.push(normalize_origin(
        &crate::handler::settings::public_base_url(&state).await,
    ));

    let origin = request
        .headers()
        .get(axum::http::header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .or_else(|| {
            request
                .headers()
                .get(axum::http::header::REFERER)
                .and_then(|v| v.to_str().ok())
                .and_then(|referer| {
                    referer
                        .trim_end_matches('/')
                        .rsplit_once('/')
                        .map(|(base, _)| base)
                })
        });

    match origin {
        Some(origin)
            if allowed_origins
                .iter()
                .any(|a| a == &normalize_origin(origin)) =>
        {
            Ok(next.run(request).await)
        }
        _ => {
            tracing::warn!(
                origin = origin.unwrap_or("<none>"),
                allowed_origins = %allowed_origins.join(","),
                method = %request.method(),
                path = %request.uri().path(),
                "origin_guard: rejected request, Origin/Referer not in CORS_ORIGINS"
            );
            Err(StatusCode::FORBIDDEN)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trusted_request_origin_accepts_only_allowed_origins() {
        let header = |value: &str| {
            let mut headers = axum::http::HeaderMap::new();
            headers.insert("origin", value.parse().unwrap());
            headers
        };
        let allowed = cors_allowed_origins();
        let first = allowed.first().expect("at least one allowed origin");
        assert_eq!(
            trusted_request_origin(&header(first)).as_deref(),
            Some(first.as_str())
        );
        assert_eq!(
            trusted_request_origin(&header("https://evil.example")),
            None
        );
        assert_eq!(trusted_request_origin(&axum::http::HeaderMap::new()), None);
    }

    #[test]
    fn strips_default_http_port() {
        assert_eq!(normalize_origin("http://1.2.3.4:80"), "http://1.2.3.4");
    }

    #[test]
    fn strips_default_https_port() {
        assert_eq!(
            normalize_origin("https://example.com:443"),
            "https://example.com"
        );
    }

    #[test]
    fn strips_trailing_slash() {
        assert_eq!(
            normalize_origin("http://example.com/"),
            "http://example.com"
        );
    }

    #[test]
    fn leaves_non_default_port_untouched() {
        assert_eq!(
            normalize_origin("http://localhost:5173"),
            "http://localhost:5173"
        );
    }

    #[test]
    fn leaves_bare_origin_untouched() {
        assert_eq!(
            normalize_origin("https://example.com"),
            "https://example.com"
        );
    }

    /// The IdP's provisioning service sends no Origin; SCIM must still get through,
    /// while any other unsafe request without one stays refused.
    #[tokio::test]
    async fn scim_is_exempt_but_other_originless_writes_are_not() {
        use axum::{Router, body::Body, routing::post};
        use tower::ServiceExt;
        let (state, path) = crate::testing::test_state().await;
        let app = Router::new()
            .route("/scim/v2/{org}/Users", post(|| async { "ok" }))
            .route("/api/v1/auth/sso/backchannel/{id}", post(|| async { "ok" }))
            .route("/api/v1/anything", post(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                origin_guard,
            ))
            .with_state(state);
        let status = |uri: &'static str| {
            let app = app.clone();
            async move {
                app.oneshot(Request::post(uri).body(Body::empty()).unwrap())
                    .await
                    .unwrap()
                    .status()
            }
        };
        assert_eq!(status("/scim/v2/org/Users").await, StatusCode::OK);
        assert_eq!(
            status("/api/v1/auth/sso/backchannel/c1").await,
            StatusCode::OK
        );
        assert_eq!(status("/api/v1/anything").await, StatusCode::FORBIDDEN);
        crate::testing::cleanup_database(&path);
    }

    /// An MCP client sends a token and no Origin; it must get through. The same
    /// request carrying a session cookie, or aimed at any other route, must not.
    #[tokio::test]
    async fn mcp_with_a_bearer_token_only_is_exempt() {
        use axum::{Router, body::Body, routing::post};
        use tower::ServiceExt;
        let (state, path) = crate::testing::test_state().await;
        let app = Router::new()
            .route("/api/v1/organizations/{org}/mcp", post(|| async { "ok" }))
            .route(
                "/api/v1/organizations/{org}/projects",
                post(|| async { "ok" }),
            )
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                origin_guard,
            ))
            .with_state(state);
        let status = |uri: &'static str, bearer: bool, cookie: bool| {
            let app = app.clone();
            async move {
                let mut request = Request::post(uri);
                if bearer {
                    request = request.header("authorization", "Bearer lp_test");
                }
                if cookie {
                    request = request.header("cookie", "theme=dark; session_token=abc");
                }
                app.oneshot(request.body(Body::empty()).unwrap())
                    .await
                    .unwrap()
                    .status()
            }
        };
        let mcp = "/api/v1/organizations/org/mcp";
        assert_eq!(status(mcp, true, false).await, StatusCode::OK);
        assert_eq!(status(mcp, true, true).await, StatusCode::FORBIDDEN);
        assert_eq!(status(mcp, false, false).await, StatusCode::FORBIDDEN);
        assert_eq!(
            status("/api/v1/organizations/org/projects", true, false).await,
            StatusCode::FORBIDDEN
        );
        crate::testing::cleanup_database(&path);
    }

    #[test]
    fn only_the_mcp_route_is_an_mcp_path() {
        assert!(is_mcp_path("/api/v1/organizations/org-1/mcp"));
        assert!(!is_mcp_path("/api/v1/organizations//mcp"));
        assert!(!is_mcp_path("/api/v1/organizations/org/projects/p/mcp"));
        assert!(!is_mcp_path("/api/v1/organizations/org/mcp/extra"));
    }

    #[test]
    fn does_not_strip_https_port_from_http_origin() {
        // :443 on a plain http:// origin is not a default-port pairing - leave it be.
        assert_eq!(
            normalize_origin("http://example.com:443"),
            "http://example.com:443"
        );
    }
}
