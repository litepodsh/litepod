//! OAuth 2.1 for the MCP endpoint (`handler::mcp`), per the MCP authorization
//! spec: a client finds Litepod through the 401's `WWW-Authenticate`, reads the
//! two metadata documents, registers itself, sends the user to consent, and
//! trades the code for a token. The token is a personal access token pinned to
//! one organization's MCP route (`auth_middleware`), so nothing past that
//! point knows OAuth exists.

use axum::{
    Extension, Form, Json, Router,
    extract::{Path, Query, Request, State},
    http::{HeaderValue, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use chrono::{Duration, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use url::Url;

use crate::dtos::response::AppError;
use crate::git::repositories::oauth as repo;
use crate::handler::common::require_session_auth;
use crate::security::authz::extract::{RequireOrg, perm};
use crate::security::middleware::auth::AuthUser;
use crate::security::middleware::csrf::mcp_path_organization;
use crate::state::state::SharedState;

const CODE_MINUTES: i64 = 10;
const ACCESS_SECONDS: i64 = 3600;
const REFRESH_DAYS: i64 = 30;
const MAX_REDIRECT_URIS: usize = 10;
const MAX_CLIENT_NAME: usize = 80;

fn timestamp(at: chrono::DateTime<Utc>) -> String {
    at.naive_utc().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// Where the consent page lives. The UI and the API share an origin in
/// production; in dev the UI is its own server, named by `LITEPOD_UI_URL`.
async fn ui_url(state: &SharedState) -> String {
    match std::env::var("LITEPOD_UI_URL") {
        Ok(url) if !url.trim().is_empty() => url.trim().trim_end_matches('/').to_string(),
        _ => base_url(state).await,
    }
}

/// RFC 6749 error body.
fn oauth_error(status: StatusCode, error: &str, description: &str) -> Response {
    (
        status,
        Json(json!({ "error": error, "error_description": description })),
    )
        .into_response()
}

/// `http` on loopback (RFC 8252 native apps) or a private scheme such as
/// `cursor://`: a code sent there stays on the user's device. Never `https`
/// yet: anyone can register, so a remote redirect would hand a code to
/// whoever got the user to approve it. Never a scheme a browser would run or read.
fn acceptable_redirect_uri(value: &str) -> bool {
    let Ok(url) = Url::parse(value) else {
        return false;
    };
    if url.fragment().is_some() {
        return false;
    }
    match url.scheme() {
        // shortcut: no remote redirects at all; claude.ai/ChatGPT need a per-org allowlist first.
        "https" => false,
        "http" => matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")),
        "javascript" | "data" | "file" | "vbscript" | "blob" | "about" | "ws" | "wss" => false,
        _ => true,
    }
}

/// The organization an MCP `resource` URL names. Only the path is read: the
/// host is whatever the client was given, which may be a tunnel or a domain
/// set later in settings.
fn resource_organization(resource: &str) -> Option<String> {
    let url = Url::parse(resource).ok()?;
    mcp_path_organization(url.path()).map(str::to_owned)
}

fn with_query(redirect_uri: &str, pairs: &[(&str, &str)]) -> String {
    let mut url = Url::parse(redirect_uri).expect("redirect_uri validated at registration");
    // An empty value is a parameter the client never sent, such as `state`.
    url.query_pairs_mut()
        .extend_pairs(pairs.iter().filter(|(_, value)| !value.is_empty()));
    url.to_string()
}

async fn base_url(state: &SharedState) -> String {
    crate::handler::settings::public_base_url(state)
        .await
        .trim_end_matches('/')
        .to_string()
}

fn mcp_url(base: &str, organization_id: &str) -> String {
    format!("{base}/api/v1/organizations/{organization_id}/mcp")
}

/// The public half: discovery, registration, authorize and token. No session;
/// each carries its own proof (PKCE, a registered redirect, a single-use code).
pub fn routes() -> Router<SharedState> {
    Router::new()
        .route(
            "/.well-known/oauth-protected-resource/api/v1/organizations/{organization_id}/mcp",
            get(protected_resource_handler),
        )
        .route(
            "/.well-known/oauth-authorization-server",
            get(authorization_server_handler),
        )
        .route("/oauth/register", post(register_handler))
        .route("/oauth/authorize", get(authorize_handler))
        .route("/oauth/token", post(token_handler))
}

/// The session half, for the consent page; merged behind `auth_middleware`.
/// The organization is in the path so `RequireOrg` does the role check.
pub fn session_routes() -> Router<SharedState> {
    Router::new().route(
        "/api/v1/organizations/{organization_id}/oauth/consent",
        get(read_consent_handler).post(consent_handler),
    )
}

#[derive(Deserialize)]
struct RegisterRequest {
    #[serde(default)]
    redirect_uris: Vec<String>,
    client_name: Option<String>,
}

/// POST /oauth/register (RFC 7591). Public clients only: no secret is issued.
async fn register_handler(
    State(state): State<SharedState>,
    Json(request): Json<RegisterRequest>,
) -> Response {
    if request.redirect_uris.is_empty() || request.redirect_uris.len() > MAX_REDIRECT_URIS {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_redirect_uri",
            "Between 1 and 10 redirect_uris are required",
        );
    }
    if let Some(bad) = request
        .redirect_uris
        .iter()
        .find(|uri| !acceptable_redirect_uri(uri))
    {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_redirect_uri",
            &format!("Not an acceptable redirect URI: {bad}"),
        );
    }
    let name: String = request
        .client_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("MCP client")
        .chars()
        .take(MAX_CLIENT_NAME)
        .collect();
    let id = uuid::Uuid::now_v7().to_string();
    let uris = serde_json::to_string(&request.redirect_uris).expect("strings serialize");
    if let Err(error) = repo::create_client(&state, &id, &name, &uris).await {
        tracing::error!(%error, "oauth.register failed");
        return oauth_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            "Could not register the client",
        );
    }
    (
        StatusCode::CREATED,
        Json(json!({
            "client_id": id,
            "client_name": name,
            "redirect_uris": request.redirect_uris,
            "token_endpoint_auth_method": "none",
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
        })),
    )
        .into_response()
}

#[derive(Deserialize)]
struct AuthorizeQuery {
    response_type: Option<String>,
    client_id: Option<String>,
    redirect_uri: Option<String>,
    code_challenge: Option<String>,
    code_challenge_method: Option<String>,
    resource: Option<String>,
}

/// The client and the exact redirect URI it registered. Until both check out
/// nothing may be sent to the redirect URI (RFC 6749 §4.1.2.1).
async fn registered(
    state: &SharedState,
    client_id: Option<&str>,
    redirect_uri: Option<&str>,
) -> Result<repo::OAuthClient, Response> {
    let refuse = |message: &str| (StatusCode::BAD_REQUEST, message.to_string()).into_response();
    let client = match client_id {
        Some(id) => repo::get_client(state, id)
            .await
            .map_err(|_| refuse("Could not read the client"))?,
        None => None,
    }
    .ok_or_else(|| refuse("Unknown client_id"))?;
    match redirect_uri {
        // Rechecked here, not only at registration, so a row from looser rules cannot pass.
        Some(uri)
            if acceptable_redirect_uri(uri)
                && client.redirect_uris.iter().any(|known| known == uri) =>
        {
            Ok(client)
        }
        _ => Err(refuse("redirect_uri is not one this client registered")),
    }
}

/// What both authorize and consent require of the rest of the request, once
/// the redirect URI is trusted. `Err` is the `error_description`.
fn check_request(
    response_type: Option<&str>,
    code_challenge: Option<&str>,
    method: Option<&str>,
) -> Result<(), &'static str> {
    if response_type != Some("code") {
        return Err("response_type must be code");
    }
    if method != Some("S256") {
        return Err("code_challenge_method must be S256");
    }
    // 43..=128 characters of base64url, per RFC 7636.
    match code_challenge {
        Some(challenge) if (43..=128).contains(&challenge.len()) => Ok(()),
        _ => Err("code_challenge is required"),
    }
}

/// GET /oauth/authorize: checks the request, then hands the browser to the
/// consent page with the same query.
async fn authorize_handler(State(state): State<SharedState>, request: Request) -> Response {
    let raw_query = request.uri().query().unwrap_or("").to_string();
    let Ok(Query(query)) = Query::<AuthorizeQuery>::try_from_uri(request.uri()) else {
        return (StatusCode::BAD_REQUEST, "Malformed authorization request").into_response();
    };
    let redirect_uri = query.redirect_uri.as_deref();
    if let Err(refusal) = registered(&state, query.client_id.as_deref(), redirect_uri).await {
        return refusal;
    }
    // A page, not a redirect with `error=`: redirecting on a bad request
    // would let any registered client use Litepod as an open redirector.
    let fail =
        |description: &str| (StatusCode::BAD_REQUEST, description.to_string()).into_response();
    if let Err(description) = check_request(
        query.response_type.as_deref(),
        query.code_challenge.as_deref(),
        query.code_challenge_method.as_deref(),
    ) {
        return fail(description);
    }
    if query
        .resource
        .as_deref()
        .and_then(resource_organization)
        .is_none()
    {
        return fail("resource must be this install's MCP endpoint URL");
    }
    Redirect::to(&format!(
        "{}/oauth/consent?{raw_query}",
        ui_url(&state).await
    ))
    .into_response()
}

#[derive(Deserialize)]
struct ReadConsentQuery {
    client_id: Option<String>,
    redirect_uri: Option<String>,
}

/// GET /api/v1/organizations/{id}/oauth/consent: what the consent page shows.
async fn read_consent_handler(
    State(state): State<SharedState>,
    Extension(user): Extension<AuthUser>,
    RequireOrg(org, _): RequireOrg<perm::AssistantUse>,
    Query(query): Query<ReadConsentQuery>,
) -> Result<Response, AppError> {
    require_session_auth(&user)?;
    crate::handler::mcp::mcp_on(&state, &org.organization_id).await?;
    let client = match registered(
        &state,
        query.client_id.as_deref(),
        query.redirect_uri.as_deref(),
    )
    .await
    {
        Ok(client) => client,
        Err(_) => {
            return Err(AppError::ValidationError(
                "Unknown client or redirect_uri".into(),
            ));
        }
    };
    let organization = crate::git::repositories::organization::get_organization_by_id(
        &state,
        &org.organization_id,
    )
    .await
    .ok_or(AppError::OrganizationNotFound)?;
    let redirect_host = query
        .redirect_uri
        .as_deref()
        .and_then(|uri| Url::parse(uri).ok())
        .map(|url| {
            url.host_str()
                .map(str::to_owned)
                .unwrap_or_else(|| url.scheme().to_owned())
        })
        .unwrap_or_default();
    Ok(Json(json!({
        "client_name": client.name,
        "organization": { "id": organization.id, "name": organization.name },
        "redirect_host": redirect_host,
    }))
    .into_response())
}

#[derive(Deserialize)]
struct ConsentRequest {
    response_type: Option<String>,
    client_id: Option<String>,
    redirect_uri: Option<String>,
    code_challenge: Option<String>,
    code_challenge_method: Option<String>,
    state: Option<String>,
    approve: bool,
}

/// POST /api/v1/organizations/{id}/oauth/consent: the user's answer. Checks
/// the whole request again; the browser need not have come through authorize.
async fn consent_handler(
    State(state): State<SharedState>,
    Extension(user): Extension<AuthUser>,
    RequireOrg(org, _): RequireOrg<perm::AssistantUse>,
    Json(request): Json<ConsentRequest>,
) -> Result<Response, AppError> {
    require_session_auth(&user)?;
    crate::handler::mcp::mcp_on(&state, &org.organization_id).await?;
    let client = match registered(
        &state,
        request.client_id.as_deref(),
        request.redirect_uri.as_deref(),
    )
    .await
    {
        Ok(client) => client,
        Err(_) => {
            return Err(AppError::ValidationError(
                "Unknown client or redirect_uri".into(),
            ));
        }
    };
    let redirect_uri = request
        .redirect_uri
        .as_deref()
        .expect("checked by registered");
    let client_state = request.state.as_deref().unwrap_or("");
    check_request(
        request.response_type.as_deref(),
        request.code_challenge.as_deref(),
        request.code_challenge_method.as_deref(),
    )
    .map_err(|description| AppError::ValidationError(description.into()))?;

    let redirect_to = if request.approve {
        let code = crate::utils::security::generate_token();
        repo::create_code(
            &state,
            &repo::NewCode {
                code: &code,
                client_id: &client.id,
                user_id: &user.user_id,
                organization_id: &org.organization_id,
                redirect_uri,
                code_challenge: request.code_challenge.as_deref().expect("checked"),
                expires_at: &timestamp(Utc::now() + Duration::minutes(CODE_MINUTES)),
            },
        )
        .await
        .map_err(AppError::DatabaseError)?;
        let _ = crate::security::security_events::record(
            &state,
            Some(&user.user_id),
            &user,
            "oauth.consented",
            Some(&client.name),
        )
        .await;
        with_query(redirect_uri, &[("code", &code), ("state", client_state)])
    } else {
        with_query(
            redirect_uri,
            &[("error", "access_denied"), ("state", client_state)],
        )
    };
    Ok(Json(json!({ "redirect_to": redirect_to })).into_response())
}

/// GET /.well-known/oauth-protected-resource/api/v1/organizations/{id}/mcp (RFC 9728)
async fn protected_resource_handler(
    State(state): State<SharedState>,
    Path(organization_id): Path<String>,
) -> Json<Value> {
    let base = base_url(&state).await;
    Json(json!({
        "resource": mcp_url(&base, &organization_id),
        "authorization_servers": [base],
        "bearer_methods_supported": ["header"],
        "scopes_supported": ["mcp"],
    }))
}

/// GET /.well-known/oauth-authorization-server (RFC 8414)
async fn authorization_server_handler(State(state): State<SharedState>) -> Json<Value> {
    let base = base_url(&state).await;
    Json(json!({
        "issuer": base,
        "authorization_endpoint": format!("{base}/oauth/authorize"),
        "token_endpoint": format!("{base}/oauth/token"),
        "registration_endpoint": format!("{base}/oauth/register"),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none"],
        "scopes_supported": ["mcp"],
    }))
}

/// Tells an MCP client that got a 401 where to start OAuth. Layered outside
/// `auth_middleware`, which is what produces the 401.
pub async fn mcp_challenge(
    State(state): State<SharedState>,
    request: Request,
    next: Next,
) -> Response {
    let organization = mcp_path_organization(request.uri().path()).map(str::to_owned);
    let mut response = next.run(request).await;
    if let Some(organization) = organization
        && response.status() == axum::http::StatusCode::UNAUTHORIZED
    {
        let base = base_url(&state).await;
        let value = format!(
            "Bearer resource_metadata=\"{base}/.well-known/oauth-protected-resource/api/v1/organizations/{organization}/mcp\""
        );
        if let Ok(value) = HeaderValue::from_str(&value) {
            response
                .headers_mut()
                .insert(header::WWW_AUTHENTICATE, value);
        }
    }
    response
}

#[derive(Deserialize)]
struct TokenRequest {
    grant_type: Option<String>,
    code: Option<String>,
    redirect_uri: Option<String>,
    client_id: Option<String>,
    code_verifier: Option<String>,
    refresh_token: Option<String>,
}

fn expired(expires_at: &str) -> bool {
    chrono::NaiveDateTime::parse_from_str(expires_at, "%Y-%m-%d %H:%M:%S")
        .map_or(true, |at| Utc::now().naive_utc() > at)
}

/// PKCE S256 (RFC 7636 §4.6).
fn verifier_matches(verifier: &str, challenge: &str) -> bool {
    use base64::Engine;
    use sha2::{Digest, Sha256};
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
        == challenge
}

fn invalid_grant(description: &str) -> Response {
    oauth_error(StatusCode::BAD_REQUEST, "invalid_grant", description)
}

/// POST /oauth/token: a code or a refresh token for a fresh pair.
async fn token_handler(
    State(state): State<SharedState>,
    Form(request): Form<TokenRequest>,
) -> Response {
    let mut response = match request.grant_type.as_deref() {
        Some("authorization_code") => exchange_code(&state, &request).await,
        Some("refresh_token") => refresh(&state, &request).await,
        _ => oauth_error(
            StatusCode::BAD_REQUEST,
            "unsupported_grant_type",
            "grant_type must be authorization_code or refresh_token",
        ),
    };
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

async fn exchange_code(state: &SharedState, request: &TokenRequest) -> Response {
    let (Some(code), Some(verifier)) = (request.code.as_deref(), request.code_verifier.as_deref())
    else {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "code and code_verifier are required",
        );
    };
    let row = match repo::take_code(state, code).await {
        Ok(Some(row)) => row,
        Ok(None) => return invalid_grant("Unknown code"),
        Err(error) => return server_error(&error),
    };
    if row.used {
        // RFC 6749 §4.1.2: a replayed code may mean it leaked, so whatever it produced goes too.
        if let Err(error) = repo::revoke_by_code(state, &row.code_digest).await {
            return server_error(&error);
        }
        return invalid_grant("Code already used");
    }
    if request.client_id.as_deref() != Some(row.client_id.as_str())
        || request.redirect_uri.as_deref() != Some(row.redirect_uri.as_str())
    {
        return invalid_grant("Code was issued to another client or redirect_uri");
    }
    if expired(&row.expires_at) {
        return invalid_grant("Code expired");
    }
    if !verifier_matches(verifier, &row.code_challenge) {
        return invalid_grant("code_verifier does not match");
    }
    mint(
        state,
        &row.client_id,
        &row.user_id,
        &row.organization_id,
        &row.code_digest,
    )
    .await
}

async fn refresh(state: &SharedState, request: &TokenRequest) -> Response {
    let Some(token) = request.refresh_token.as_deref() else {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "refresh_token is required",
        );
    };
    let row = match repo::take_refresh(state, token).await {
        Ok(Some(row)) => row,
        Ok(None) => return invalid_grant("Unknown or used refresh_token"),
        Err(error) => return server_error(&error),
    };
    // Taken either way: the old access token goes with its refresh token.
    if let Err(error) =
        crate::git::repositories::personal_access_token::revoke(state, &row.access_token_id).await
    {
        return server_error(&error);
    }
    if request.client_id.as_deref() != Some(row.client_id.as_str()) {
        return invalid_grant("refresh_token was issued to another client");
    }
    if expired(&row.expires_at) {
        return invalid_grant("refresh_token expired");
    }
    mint(
        state,
        &row.client_id,
        &row.user_id,
        &row.organization_id,
        &row.code_digest,
    )
    .await
}

fn server_error(error: &str) -> Response {
    tracing::error!(%error, "oauth.token failed");
    oauth_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "server_error",
        "Could not issue a token",
    )
}

/// A personal access token pinned to the organization's MCP route, plus the
/// refresh token that stands behind it.
async fn mint(
    state: &SharedState,
    client_id: &str,
    user_id: &str,
    organization_id: &str,
    code_digest: &str,
) -> Response {
    let client_name = match repo::get_client(state, client_id).await {
        Ok(Some(client)) => client.name,
        Ok(None) => return invalid_grant("Client no longer exists"),
        Err(error) => return server_error(&error),
    };
    let access_token_id = uuid::Uuid::now_v7().to_string();
    let access_token = crate::utils::security::generate_token();
    let refresh_token = crate::utils::security::generate_token();
    let now = Utc::now();
    if let Err(error) = crate::git::repositories::personal_access_token::create(
        state,
        &access_token_id,
        user_id,
        &format!("MCP · {client_name}"),
        &access_token,
        &format!("lp_{}", &access_token[..10]),
        "[\"write\"]",
        Some(&timestamp(now + Duration::seconds(ACCESS_SECONDS))),
        Some(organization_id),
    )
    .await
    {
        return server_error(&error);
    }
    if let Err(error) = repo::create_refresh(
        state,
        &repo::NewRefresh {
            token: &refresh_token,
            client_id,
            user_id,
            organization_id,
            access_token_id: &access_token_id,
            code_digest,
            expires_at: &timestamp(now + Duration::days(REFRESH_DAYS)),
        },
    )
    .await
    {
        return server_error(&error);
    }
    Json(json!({
        "access_token": access_token,
        "token_type": "Bearer",
        "expires_in": ACCESS_SECONDS,
        "refresh_token": refresh_token,
    }))
    .into_response()
}

#[cfg(test)]
mod tests {
    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode, header},
        middleware,
        routing::post,
    };
    use serde_json::Value;
    use tower::ServiceExt;

    use super::*;
    use crate::security::middleware::auth::auth_middleware;
    use crate::testing::{cleanup_database, test_state};

    /// The production layering for these routes: public OAuth routes, the MCP
    /// route behind `auth_middleware`, and the challenge around both.
    fn app(state: SharedState) -> Router {
        let mcp = Router::new()
            .route(
                "/api/v1/organizations/{organization_id}/mcp",
                post(crate::handler::mcp::mcp_handler),
            )
            .layer(middleware::from_fn_with_state(
                state.clone(),
                auth_middleware,
            ));
        routes()
            .merge(mcp)
            .merge(session_routes().layer(middleware::from_fn(signed_in)))
            .layer(middleware::from_fn_with_state(state.clone(), mcp_challenge))
            .with_state(state)
    }

    /// Stands in for a session cookie: whoever `x-test-user` names, else nobody.
    async fn signed_in(
        mut request: Request<Body>,
        next: middleware::Next,
    ) -> axum::response::Response {
        if let Some(user) = request
            .headers()
            .get("x-test-user")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
        {
            request
                .extensions_mut()
                .insert(crate::testing::auth_user(&user));
        }
        next.run(request).await
    }

    const REDIRECT: &str = "http://127.0.0.1:33418/callback";
    const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

    async fn enable_mcp(state: &SharedState) {
        sqlx::query("UPDATE organizations SET mcp_enabled = 1 WHERE id = 'organization-a'")
            .execute(&state.db)
            .await
            .unwrap();
    }

    async fn send_json(
        app: &Router,
        method: &str,
        uri: &str,
        user: Option<&str>,
        body: Value,
    ) -> (StatusCode, Value) {
        let mut request = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(user) = user {
            request = request.header("x-test-user", user);
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn register(app: &Router) -> String {
        let (status, client) = send_json(
            app,
            "POST",
            "/oauth/register",
            None,
            json!({ "redirect_uris": [REDIRECT], "client_name": "Claude Code" }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{client}");
        client["client_id"].as_str().unwrap().to_string()
    }

    fn authorize_uri(client_id: &str, redirect_uri: &str) -> String {
        let mut url = Url::parse("http://x/oauth/authorize").unwrap();
        url.query_pairs_mut().extend_pairs([
            ("response_type", "code"),
            ("client_id", client_id),
            ("redirect_uri", redirect_uri),
            ("code_challenge", CHALLENGE),
            ("code_challenge_method", "S256"),
            ("state", "xyz"),
            (
                "resource",
                "http://localhost:6001/api/v1/organizations/organization-a/mcp",
            ),
        ]);
        format!("{}?{}", url.path(), url.query().unwrap())
    }

    fn consent_body(client_id: &str, approve: bool) -> Value {
        json!({
            "response_type": "code",
            "client_id": client_id,
            "redirect_uri": REDIRECT,
            "code_challenge": CHALLENGE,
            "code_challenge_method": "S256",
            "state": "xyz",
            "approve": approve,
        })
    }

    const CONSENT: &str = "/api/v1/organizations/organization-a/oauth/consent";

    /// Registers, consents as `editor`, and returns (client_id, code).
    async fn approved_code(app: &Router) -> (String, String) {
        let client_id = register(app).await;
        let (status, body) = send_json(
            app,
            "POST",
            CONSENT,
            Some("editor"),
            consent_body(&client_id, true),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let redirect = Url::parse(body["redirect_to"].as_str().unwrap()).unwrap();
        let code = redirect
            .query_pairs()
            .find(|(key, _)| key == "code")
            .unwrap()
            .1
            .to_string();
        (client_id, code)
    }

    #[tokio::test]
    async fn register_accepts_loopback_and_private_schemes() {
        let (state, path) = test_state().await;
        let app = app(state);
        let (status, client) = send_json(
            &app,
            "POST",
            "/oauth/register",
            None,
            json!({ "redirect_uris": ["http://localhost:9000/cb", "http://127.0.0.1:33418/callback", "cursor://anysphere.cursor-mcp/oauth/callback"] }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{client}");
        assert_eq!(client["client_name"], "MCP client");
        assert_eq!(client["token_endpoint_auth_method"], "none");
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn register_refuses_anything_off_the_device() {
        let (state, path) = test_state().await;
        let app = app(state);
        for uri in [
            "https://evil.test/cb",
            "https://claude.ai/api/mcp/auth_callback",
            "http://evil.test/cb",
            "javascript:alert(1)",
            "https://ok.test/cb#frag",
        ] {
            let (status, body) = send_json(
                &app,
                "POST",
                "/oauth/register",
                None,
                json!({ "redirect_uris": [uri] }),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
            assert_eq!(body["error"], "invalid_redirect_uri");
        }
        cleanup_database(&path);
    }

    /// A bad request from a registered client is still not a reason to send
    /// the browser anywhere: that would make Litepod an open redirector.
    #[tokio::test]
    async fn an_invalid_authorize_request_is_a_page_not_a_redirect() {
        let (state, path) = test_state().await;
        let app = app(state);
        let client_id = register(&app).await;
        let uri = authorize_uri(&client_id, REDIRECT)
            .replace("response_type=code", "response_type=token");
        let response = app
            .clone()
            .oneshot(Request::get(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(response.headers().get(header::LOCATION).is_none());
        cleanup_database(&path);
    }

    /// Clients registered before the redirect rules tightened are held to them too.
    #[tokio::test]
    async fn authorize_rechecks_a_stored_redirect_uri() {
        let (state, path) = test_state().await;
        crate::git::repositories::oauth::create_client(
            &state,
            "old-client",
            "Old",
            "[\"https://evil.test/cb\"]",
        )
        .await
        .unwrap();
        let response = app(state)
            .oneshot(
                Request::get(authorize_uri("old-client", "https://evil.test/cb"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(response.headers().get(header::LOCATION).is_none());
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn authorize_refuses_an_unregistered_redirect_uri() {
        let (state, path) = test_state().await;
        let app = app(state);
        let client_id = register(&app).await;
        let response = app
            .clone()
            .oneshot(
                Request::get(authorize_uri(&client_id, "http://127.0.0.1:33418/other"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(response.headers().get(header::LOCATION).is_none());
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn authorize_sends_the_browser_to_consent() {
        let (state, path) = test_state().await;
        let ui = ui_url(&state).await;
        let app = app(state);
        let client_id = register(&app).await;
        let response = app
            .clone()
            .oneshot(
                Request::get(authorize_uri(&client_id, REDIRECT))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        let location = response.headers()[header::LOCATION].to_str().unwrap();
        assert!(
            location.starts_with(&format!("{ui}/oauth/consent?")),
            "{location}"
        );
        assert!(location.contains("code_challenge="), "{location}");
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn consent_approve_returns_a_code_for_the_registered_uri() {
        let (state, path) = test_state().await;
        enable_mcp(&state).await;
        let app = app(state);
        let client_id = register(&app).await;

        let (status, shown) = send_json(
            &app,
            "GET",
            &format!(
                "{CONSENT}?client_id={client_id}&redirect_uri={}",
                urlencoding(REDIRECT)
            ),
            Some("editor"),
            Value::Null,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{shown}");
        assert_eq!(shown["client_name"], "Claude Code");
        assert_eq!(shown["organization"]["id"], "organization-a");
        assert_eq!(shown["redirect_host"], "127.0.0.1");

        let (status, body) = send_json(
            &app,
            "POST",
            CONSENT,
            Some("editor"),
            consent_body(&client_id, true),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let redirect_to = body["redirect_to"].as_str().unwrap();
        assert!(
            redirect_to.starts_with(&format!("{REDIRECT}?code=")),
            "{redirect_to}"
        );
        assert!(redirect_to.ends_with("&state=xyz"), "{redirect_to}");

        let (_, denied) = send_json(
            &app,
            "POST",
            CONSENT,
            Some("editor"),
            consent_body(&client_id, false),
        )
        .await;
        assert_eq!(
            denied["redirect_to"],
            format!("{REDIRECT}?error=access_denied&state=xyz")
        );
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn a_viewer_cannot_consent() {
        let (state, path) = test_state().await;
        enable_mcp(&state).await;
        let app = app(state);
        let client_id = register(&app).await;
        let (status, _) = send_json(
            &app,
            "POST",
            CONSENT,
            Some("viewer"),
            consent_body(&client_id, true),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        cleanup_database(&path);
    }

    async fn token(app: &Router, form: &[(&str, &str)]) -> (StatusCode, Value) {
        let body: String = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(form)
            .finish();
        let response = app
            .clone()
            .oneshot(
                Request::post("/oauth/token")
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        if status == StatusCode::OK {
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        }
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn exchange(
        app: &Router,
        client_id: &str,
        code: &str,
        verifier: &str,
    ) -> (StatusCode, Value) {
        token(
            app,
            &[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", REDIRECT),
                ("client_id", client_id),
                ("code_verifier", verifier),
            ],
        )
        .await
    }

    async fn mcp_status(app: &Router, access_token: &str) -> StatusCode {
        app.clone()
            .oneshot(
                Request::post("/api/v1/organizations/organization-a/mcp")
                    .header(header::AUTHORIZATION, format!("Bearer {access_token}"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
    }

    #[tokio::test]
    async fn a_code_becomes_a_token_that_calls_mcp() {
        let (state, path) = test_state().await;
        enable_mcp(&state).await;
        let app = app(state);
        let (client_id, code) = approved_code(&app).await;
        let (status, tokens) = exchange(&app, &client_id, &code, VERIFIER).await;
        assert_eq!(status, StatusCode::OK, "{tokens}");
        assert_eq!(tokens["token_type"], "Bearer");
        assert_eq!(tokens["expires_in"], 3600);
        assert!(tokens["refresh_token"].is_string());
        assert_eq!(
            mcp_status(&app, tokens["access_token"].as_str().unwrap()).await,
            StatusCode::OK
        );
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn a_wrong_verifier_is_invalid_grant() {
        let (state, path) = test_state().await;
        enable_mcp(&state).await;
        let app = app(state);
        let (client_id, code) = approved_code(&app).await;
        let (status, body) = exchange(
            &app,
            &client_id,
            &code,
            "not-the-verifier-not-the-verifier-not-the-verifier",
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "invalid_grant");
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn a_reused_code_is_refused_and_revokes_its_tokens() {
        let (state, path) = test_state().await;
        enable_mcp(&state).await;
        let app = app(state);
        let (client_id, code) = approved_code(&app).await;
        let (_, first) = exchange(&app, &client_id, &code, VERIFIER).await;
        let (status, body) = exchange(&app, &client_id, &code, VERIFIER).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "invalid_grant");
        assert_eq!(
            mcp_status(&app, first["access_token"].as_str().unwrap()).await,
            StatusCode::UNAUTHORIZED
        );
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn an_expired_code_is_invalid_grant() {
        let (state, path) = test_state().await;
        enable_mcp(&state).await;
        let app = app(state.clone());
        let (client_id, code) = approved_code(&app).await;
        sqlx::query("UPDATE oauth_codes SET expires_at = '2000-01-01 00:00:00'")
            .execute(&state.db)
            .await
            .unwrap();
        let (status, body) = exchange(&app, &client_id, &code, VERIFIER).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "invalid_grant");
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn refresh_rotates_and_the_old_refresh_is_dead() {
        let (state, path) = test_state().await;
        enable_mcp(&state).await;
        let app = app(state);
        let (client_id, code) = approved_code(&app).await;
        let (_, first) = exchange(&app, &client_id, &code, VERIFIER).await;
        let refresh = first["refresh_token"].as_str().unwrap();
        let form = [
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh),
            ("client_id", client_id.as_str()),
        ];

        let (status, second) = token(&app, &form).await;
        assert_eq!(status, StatusCode::OK, "{second}");
        assert_ne!(second["refresh_token"], first["refresh_token"]);
        assert_eq!(
            mcp_status(&app, second["access_token"].as_str().unwrap()).await,
            StatusCode::OK
        );
        assert_eq!(
            mcp_status(&app, first["access_token"].as_str().unwrap()).await,
            StatusCode::UNAUTHORIZED
        );

        let (status, body) = token(&app, &form).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "invalid_grant");
        cleanup_database(&path);
    }

    fn urlencoding(value: &str) -> String {
        url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
    }

    async fn get_json(app: &Router, uri: &str) -> (StatusCode, Value) {
        let response = app
            .clone()
            .oneshot(Request::get(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    #[tokio::test]
    async fn metadata_points_at_this_install() {
        let (state, path) = test_state().await;
        let base = base_url(&state).await;
        let app = app(state);

        let (status, resource) = get_json(
            &app,
            "/.well-known/oauth-protected-resource/api/v1/organizations/organization-a/mcp",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            resource["resource"],
            format!("{base}/api/v1/organizations/organization-a/mcp")
        );
        assert_eq!(resource["authorization_servers"][0], base);

        let (status, server) = get_json(&app, "/.well-known/oauth-authorization-server").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(server["issuer"], base);
        assert_eq!(server["token_endpoint"], format!("{base}/oauth/token"));
        assert_eq!(server["code_challenge_methods_supported"][0], "S256");

        cleanup_database(&path);
    }

    #[tokio::test]
    async fn an_unauthenticated_mcp_call_gets_the_challenge() {
        let (state, path) = test_state().await;
        let base = base_url(&state).await;
        let response = app(state)
            .oneshot(
                Request::post("/api/v1/organizations/organization-a/mcp")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            response.headers()[header::WWW_AUTHENTICATE],
            format!(
                "Bearer resource_metadata=\"{base}/.well-known/oauth-protected-resource/api/v1/organizations/organization-a/mcp\""
            )
        );
        cleanup_database(&path);
    }
}
