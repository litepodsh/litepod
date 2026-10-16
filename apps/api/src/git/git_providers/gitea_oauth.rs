//! Gitea/Forgejo OAuth (self-hosted) and credential resolution.
//!
//! The flow mirrors `gitlab_oauth` — the standard OAuth2 authorization-code
//! grant against the instance's `/login/oauth` endpoints — with the Gitea REST
//! API shape (base `/api/v1`, user object with a `login` field) and the
//! `gitea_oauth` auth method. Gitea and Forgejo share the same endpoints, so
//! one module serves both `provider_type`s.
//!
//! Gitea/Forgejo webhooks are already implemented (HMAC-SHA256 over the body
//! via `X-Gitea-Signature`, `webhooks::handle_gitea`); this module only adds
//! the OAuth connect flow and token access for git operations.

use chrono::Utc;
use sqlx::Row;
use uuid::Uuid;

use crate::{dtos::response::AppError, state::state::SharedState, utils::security};

const PROVIDER_TYPES: [&str; 2] = ["gitea", "forgejo"];

/// Gitea grants personal-access-style scopes per request. Repository access is
/// implied for the user's own repos, but the API list (used by the repo picker)
/// needs repo + organization read scopes.
const SCOPES: &str = "read:user read:organization read:repository";

const TOKEN_SKEW_MS: i64 = 300_000;
const NONCE_TTL_MS: i64 = 600_000;

const OAUTH_COLUMNS: &str = "oauth_client_id, oauth_client_secret_encrypted, oauth_redirect_uri, \
     oauth_access_token_encrypted, oauth_refresh_token_encrypted, oauth_expires_at, \
     oauth_scopes, oauth_user_id, oauth_user_login, oauth_connected_at, webhook_secret, \
     provider_type, host_url";

#[derive(Debug, Clone, serde::Serialize)]
pub struct GiteaOauthStatus {
    pub connected: bool,
    pub user_login: Option<String>,
    pub user_id: Option<String>,
    pub scopes: Option<String>,
    pub connected_at: Option<i64>,
    pub expires_at: Option<i64>,
    pub has_client_credentials: bool,
    pub has_webhook_secret: bool,
}

#[derive(Debug, Clone)]
struct OauthRow {
    oauth_client_id: Option<String>,
    oauth_client_secret_encrypted: Option<String>,
    #[allow(dead_code)]
    oauth_redirect_uri: Option<String>,
    oauth_access_token_encrypted: Option<String>,
    oauth_refresh_token_encrypted: Option<String>,
    oauth_expires_at: Option<i64>,
    oauth_scopes: Option<String>,
    oauth_user_id: Option<String>,
    oauth_user_login: Option<String>,
    oauth_connected_at: Option<i64>,
    webhook_secret: Option<String>,
    provider_type: String,
    host_url: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct StateEntry {
    organization_id: String,
    user_id: String,
    provider_id: String,
    purpose: String,
    created_at: i64,
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

fn client() -> Result<reqwest::Client, AppError> {
    reqwest::Client::builder()
        .user_agent("litepod")
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| AppError::InternalError(format!("HTTP client build failed: {e}")))
}

fn nonce() -> String {
    use rand::Rng;
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    URL_SAFE_NO_PAD.encode(bytes)
}

fn put_state(state: &SharedState, entry: &StateEntry) -> Result<String, AppError> {
    let token = nonce();
    let encoded =
        serde_json::to_string(entry).map_err(|e| AppError::InternalError(e.to_string()))?;
    let mut guard = state
        .oauth_states
        .lock()
        .map_err(|_| AppError::InternalError("State lock poisoned".into()))?;
    let cutoff = now_ms() - NONCE_TTL_MS;
    guard.retain(|_, value| {
        serde_json::from_str::<StateEntry>(value).is_ok_and(|entry| entry.created_at >= cutoff)
    });
    guard.insert(token.clone(), encoded);
    Ok(token)
}

fn take_state(state: &SharedState, token: &str, purpose: &str) -> Result<StateEntry, AppError> {
    let mut guard = state
        .oauth_states
        .lock()
        .map_err(|_| AppError::InternalError("State lock poisoned".into()))?;
    let raw = guard.remove(token).ok_or_else(|| {
        AppError::ValidationError("This link has expired or was already used — start again".into())
    })?;
    let entry: StateEntry =
        serde_json::from_str(&raw).map_err(|e| AppError::InternalError(e.to_string()))?;
    if entry.purpose != purpose || entry.created_at < now_ms() - NONCE_TTL_MS {
        return Err(AppError::ValidationError(
            "This link has expired or was already used — start again".into(),
        ));
    }
    Ok(entry)
}

fn parse_oauth_row(row: &sqlx::sqlite::SqliteRow) -> Result<OauthRow, AppError> {
    let read = |index: usize| -> Result<String, AppError> {
        row.try_get(index)
            .map_err(|e| AppError::DatabaseError(e.to_string()))
    };
    let read_opt = |index: usize| -> Result<Option<String>, AppError> {
        row.try_get(index)
            .map_err(|e| AppError::DatabaseError(e.to_string()))
    };
    let read_opt_i64 = |index: usize| -> Result<Option<i64>, AppError> {
        row.try_get(index)
            .map_err(|e| AppError::DatabaseError(e.to_string()))
    };
    Ok(OauthRow {
        oauth_client_id: read_opt(0)?,
        oauth_client_secret_encrypted: read_opt(1)?,
        oauth_redirect_uri: read_opt(2)?,
        oauth_access_token_encrypted: read_opt(3)?,
        oauth_refresh_token_encrypted: read_opt(4)?,
        oauth_expires_at: read_opt_i64(5)?,
        oauth_scopes: read_opt(6)?,
        oauth_user_id: read_opt(7)?,
        oauth_user_login: read_opt(8)?,
        oauth_connected_at: read_opt_i64(9)?,
        webhook_secret: read_opt(10)?,
        provider_type: read(11)?,
        host_url: read(12)?,
    })
}

async fn load_row(
    state: &SharedState,
    provider_id: &str,
    organization_id: &str,
) -> Result<OauthRow, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(&format!(
        "SELECT {OAUTH_COLUMNS} FROM git_providers WHERE id = ?1 AND organization_id = ?2"
    ))
    .bind(provider_id)
    .bind(organization_id)
    .fetch_optional(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?
    .ok_or(AppError::ProviderNotFound)?;
    parse_oauth_row(&row)
}

async fn load_row_unscoped(state: &SharedState, provider_id: &str) -> Result<OauthRow, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(&format!(
        "SELECT {OAUTH_COLUMNS} FROM git_providers WHERE id = ?1"
    ))
    .bind(provider_id)
    .fetch_optional(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?
    .ok_or(AppError::ProviderNotFound)?;
    parse_oauth_row(&row)
}

fn require_gitea_host(host_url: &str) -> Result<String, AppError> {
    let trimmed = host_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(AppError::ValidationError(
            "Host URL is required for Gitea/Forgejo providers".into(),
        ));
    }
    Ok(trimmed.to_string())
}

fn api_base(host_url: &str) -> Result<String, AppError> {
    Ok(format!("{}/api/v1", require_gitea_host(host_url)?))
}

fn authorize_url(host_url: &str) -> Result<String, AppError> {
    Ok(format!(
        "{}/login/oauth/authorize",
        require_gitea_host(host_url)?
    ))
}

fn token_url(host_url: &str) -> Result<String, AppError> {
    Ok(format!(
        "{}/login/oauth/access_token",
        require_gitea_host(host_url)?
    ))
}

async fn guard_outbound_url(raw: &str, allow_private: bool) -> Result<(), AppError> {
    let url =
        crate::security::ssrf::parse_https_url_with_port(raw).map_err(AppError::ValidationError)?;
    crate::security::ssrf::assert_host(&url, allow_private)
        .await
        .map_err(AppError::ValidationError)
}

fn random_token(bytes: usize) -> String {
    use rand::Rng;
    let mut buf = vec![0u8; bytes];
    rand::rng().fill_bytes(&mut buf);
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    URL_SAFE_NO_PAD.encode(buf)
}

pub async fn public_url(state: &SharedState) -> Result<String, AppError> {
    let url = crate::handler::settings::public_base_url(state).await;
    let is_loopback =
        url.contains("localhost") || url.contains("127.0.0.1") || url.contains("0.0.0.0");
    if is_loopback && !state.dev_mode {
        return Err(AppError::ValidationError(
            "Set a server domain in Settings before connecting Gitea/Forgejo — the instance must reach this server for the OAuth callback and webhook".into(),
        ));
    }
    Ok(url)
}

/// Where litepod redirects the browser after the user approves the Gitea OAuth app.
pub fn redirect_uri(public_url: &str, provider_id: &str) -> String {
    format!(
        "{}/api/v1/git-providers/{provider_id}/gitea-oauth/callback",
        public_url.trim_end_matches('/')
    )
}

/// Gitea/Forgejo webhooks HMAC-sign the body; the same unified route as GitLab.
pub fn git_webhook_url(public_url: &str, provider_id: &str) -> String {
    format!(
        "{}/api/v1/webhooks/git/{provider_id}",
        public_url.trim_end_matches('/')
    )
}

/// Webhook secret shared with the Gitea/Forgejo instance (the HMAC key).
pub async fn ensure_webhook_secret(
    state: &SharedState,
    provider_id: &str,
    organization_id: &str,
) -> Result<String, AppError> {
    let row = load_row(state, provider_id, organization_id).await?;
    if let Some(secret) = row.webhook_secret.as_ref().filter(|s| !s.is_empty()) {
        return Ok(secret.clone());
    }
    let secret = random_token(32);
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE git_providers SET webhook_secret = ?1, updated_at = ?2 WHERE id = ?3 AND organization_id = ?4",
    )
    .bind(secret.clone())
    .bind(now_ms())
    .bind(provider_id)
    .bind(organization_id)
    .execute(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok(secret)
}

/// Called by `create()` / `update()` when the user pastes client_id + client_secret.
pub async fn save_client_credentials(
    state: &SharedState,
    provider_id: &str,
    organization_id: &str,
    client_id: &str,
    client_secret: &str,
) -> Result<(), AppError> {
    if client_id.trim().is_empty() {
        return Err(AppError::ValidationError("Client ID is required".into()));
    }
    if client_secret.trim().is_empty() {
        return Err(AppError::ValidationError(
            "Client secret is required".into(),
        ));
    }
    let key = security::get_encryption_key();
    let encrypted = security::encrypt_data(client_secret, &key).map_err(AppError::InternalError)?;
    let public = public_url(state).await?;
    let redirect = redirect_uri(&public, provider_id);
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE git_providers SET \
            oauth_client_id = ?1, \
            oauth_client_secret_encrypted = ?2, \
            oauth_redirect_uri = ?3, \
            updated_at = ?4 \
         WHERE id = ?5 AND organization_id = ?6",
    )
    .bind(client_id.trim())
    .bind(encrypted)
    .bind(redirect)
    .bind(now_ms())
    .bind(provider_id)
    .bind(organization_id)
    .execute(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let _ = ensure_webhook_secret(state, provider_id, organization_id).await?;
    Ok(())
}

pub async fn status(
    state: &SharedState,
    provider_id: &str,
    organization_id: &str,
) -> Result<GiteaOauthStatus, AppError> {
    let row = load_row(state, provider_id, organization_id).await?;
    Ok(GiteaOauthStatus {
        connected: row
            .oauth_access_token_encrypted
            .as_ref()
            .is_some_and(|s| !s.is_empty()),
        user_login: row.oauth_user_login,
        user_id: row.oauth_user_id,
        scopes: row.oauth_scopes,
        connected_at: row.oauth_connected_at,
        expires_at: row.oauth_expires_at,
        has_client_credentials: row.oauth_client_id.as_ref().is_some_and(|s| !s.is_empty())
            && row
                .oauth_client_secret_encrypted
                .as_ref()
                .is_some_and(|s| !s.is_empty()),
        has_webhook_secret: row.webhook_secret.as_ref().is_some_and(|s| !s.is_empty()),
    })
}

/// Builds the instance's authorize URL and parks a CSRF state. Serves both
/// `gitea` and `forgejo` provider types.
pub async fn begin_authorize(
    state: &SharedState,
    organization_id: &str,
    user_id: &str,
    provider_id: &str,
) -> Result<String, AppError> {
    let row = load_row(state, provider_id, organization_id).await?;
    if !PROVIDER_TYPES.contains(&row.provider_type.as_str()) {
        return Err(AppError::ValidationError(
            "Gitea OAuth is only available for Gitea or Forgejo providers".into(),
        ));
    }
    let client_id = row.oauth_client_id.ok_or_else(|| {
        AppError::ValidationError("Save the client ID and secret before connecting".into())
    })?;
    let public = public_url(state).await?;
    let redirect = redirect_uri(&public, provider_id);
    let token = put_state(
        state,
        &StateEntry {
            organization_id: organization_id.to_string(),
            user_id: user_id.to_string(),
            provider_id: provider_id.to_string(),
            purpose: "gitea_oauth".to_string(),
            created_at: now_ms(),
        },
    )?;
    let url = format!(
        "{}?client_id={}&redirect_uri={}&response_type=code&state={}&scope={}",
        authorize_url(&row.host_url)?,
        urlencoded(&client_id),
        urlencoded(&redirect),
        urlencoded(&token),
        urlencoded(SCOPES),
    );
    Ok(url)
}

#[derive(Debug, serde::Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    token_type: Option<String>,
}

/// Gitea's `/api/v1/user` returns `login` (not `username`).
#[derive(Debug, serde::Deserialize)]
struct GiteaUser {
    id: i64,
    login: String,
}

/// Validates the `state` nonce, exchanges the `code` for tokens, fetches the
/// user profile, and persists everything encrypted. Returns the provider_id.
pub async fn complete_authorize(
    state: &SharedState,
    code: &str,
    state_token: &str,
) -> Result<String, AppError> {
    let entry = take_state(state, state_token, "gitea_oauth")?;
    let provider_id = entry.provider_id;
    let row = load_row(state, &provider_id, &entry.organization_id).await?;
    if !PROVIDER_TYPES.contains(&row.provider_type.as_str()) {
        return Err(AppError::ValidationError(
            "Provider is not a Gitea/Forgejo type".into(),
        ));
    }
    let client_id = row.oauth_client_id.ok_or_else(|| {
        AppError::ValidationError(
            "Client ID is missing — save the OAuth App credentials first".into(),
        )
    })?;
    let client_secret = row
        .oauth_client_secret_encrypted
        .as_deref()
        .ok_or_else(|| {
            AppError::ValidationError(
                "Client secret is missing — save the OAuth App credentials first".into(),
            )
        })?;
    let key = security::get_encryption_key();
    let client_secret =
        security::decrypt_data(client_secret, &key).map_err(AppError::InternalError)?;
    let public = public_url(state).await?;
    let redirect = redirect_uri(&public, &provider_id);

    let token_endpoint = token_url(&row.host_url)?;
    let allow_private = !row.host_url.trim().is_empty()
        && crate::git::git_providers::allows_private_addresses(
            state.is_cloud,
            &row.provider_type,
            &row.host_url,
        );
    guard_outbound_url(&token_endpoint, allow_private).await?;
    let client = client()?;
    let token_response = client
        .post(token_endpoint)
        .form(&[
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.as_str()),
            ("code", code),
            ("grant_type", "authorization_code"),
            ("redirect_uri", redirect.as_str()),
        ])
        .send()
        .await
        .map_err(|e| {
            AppError::ValidationError(format!("Could not reach the Gitea instance: {e}"))
        })?;

    if !token_response.status().is_success() {
        let status = token_response.status();
        let body = token_response.text().await.unwrap_or_default();
        return Err(AppError::ValidationError(format!(
            "The Gitea instance rejected the authorization code ({status}): {}",
            body.chars().take(200).collect::<String>()
        )));
    }
    let token: TokenResponse = token_response
        .json()
        .await
        .map_err(|e| AppError::ValidationError(format!("Unexpected Gitea response: {e}")))?;

    let user = fetch_user(
        &client,
        &api_base(&row.host_url)?,
        &token.access_token,
        allow_private,
    )
    .await?;

    persist_tokens(state, &provider_id, &token, &user).await?;
    Ok(provider_id)
}

async fn fetch_user(
    client: &reqwest::Client,
    api_base: &str,
    access_token: &str,
    allow_private: bool,
) -> Result<GiteaUser, AppError> {
    let user_endpoint = format!("{api_base}/user");
    guard_outbound_url(&user_endpoint, allow_private).await?;
    let response = client
        .get(user_endpoint)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| {
            AppError::ValidationError(format!("Could not reach the Gitea instance: {e}"))
        })?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(AppError::ValidationError(format!(
            "Gitea /api/v1/user returned {status}: {}",
            body.chars().take(200).collect::<String>()
        )));
    }
    response
        .json::<GiteaUser>()
        .await
        .map_err(|e| AppError::ValidationError(format!("Unexpected Gitea /user response: {e}")))
}

async fn persist_tokens(
    state: &SharedState,
    provider_id: &str,
    token: &TokenResponse,
    user: &GiteaUser,
) -> Result<(), AppError> {
    let key = security::get_encryption_key();
    let access_enc =
        security::encrypt_data(&token.access_token, &key).map_err(AppError::InternalError)?;
    let refresh_enc = token
        .refresh_token
        .as_deref()
        .map(|raw| security::encrypt_data(raw, &key))
        .transpose()
        .map_err(AppError::InternalError)?;
    let expires_at = token.expires_in.map(|seconds| now_ms() + seconds * 1000);
    let scopes = token.scope.clone().or_else(|| Some(SCOPES.to_string()));
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE git_providers SET \
            oauth_access_token_encrypted = ?1, \
            oauth_refresh_token_encrypted = ?2, \
            oauth_expires_at = ?3, \
            oauth_scopes = ?4, \
            oauth_user_id = ?5, \
            oauth_user_login = ?6, \
            oauth_connected_at = ?7, \
            updated_at = ?8 \
         WHERE id = ?9",
    )
    .bind(access_enc)
    .bind(refresh_enc)
    .bind(expires_at)
    .bind(scopes)
    .bind(user.id.to_string())
    .bind(user.login.clone())
    .bind(now_ms())
    .bind(now_ms())
    .bind(provider_id)
    .execute(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok(())
}

/// Returns a usable access token, refreshing the stored one when past the skew.
pub async fn access_token(state: &SharedState, provider_id: &str) -> Result<String, AppError> {
    let row = load_row_unscoped(state, provider_id).await?;
    if !PROVIDER_TYPES.contains(&row.provider_type.as_str()) {
        return Err(AppError::ValidationError(
            "Provider is not a Gitea/Forgejo type".into(),
        ));
    }
    let needs_refresh = match row.oauth_expires_at {
        Some(expires) => expires <= now_ms() + TOKEN_SKEW_MS,
        None => false,
    };
    if needs_refresh {
        if row.oauth_refresh_token_encrypted.is_none() {
            return Err(AppError::ValidationError(
                "Gitea connection expired and no refresh token — reconnect".into(),
            ));
        }
        refresh(state, provider_id, &row).await?;
    }
    let after = load_row_unscoped(state, provider_id).await?;
    let ciphertext = after
        .oauth_access_token_encrypted
        .ok_or_else(|| AppError::ValidationError("Gitea is not connected".into()))?;
    let key = security::get_encryption_key();
    security::decrypt_data(&ciphertext, &key).map_err(AppError::InternalError)
}

async fn refresh(state: &SharedState, provider_id: &str, row: &OauthRow) -> Result<(), AppError> {
    let client_id = row
        .oauth_client_id
        .clone()
        .ok_or_else(|| AppError::ValidationError("Client ID missing".into()))?;
    let client_secret_enc = row
        .oauth_client_secret_encrypted
        .clone()
        .ok_or_else(|| AppError::ValidationError("Client secret missing".into()))?;
    let refresh_enc = row
        .oauth_refresh_token_encrypted
        .clone()
        .ok_or_else(|| AppError::ValidationError("No refresh token — reconnect".into()))?;
    let key = security::get_encryption_key();
    let client_secret =
        security::decrypt_data(&client_secret_enc, &key).map_err(AppError::InternalError)?;
    let refresh_token =
        security::decrypt_data(&refresh_enc, &key).map_err(AppError::InternalError)?;

    let token_endpoint = token_url(&row.host_url)?;
    let allow_private = !row.host_url.trim().is_empty()
        && crate::git::git_providers::allows_private_addresses(
            state.is_cloud,
            &row.provider_type,
            &row.host_url,
        );
    guard_outbound_url(&token_endpoint, allow_private).await?;
    let http = client()?;
    let response = http
        .post(token_endpoint)
        .form(&[
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.as_str()),
            ("refresh_token", refresh_token.as_str()),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .await
        .map_err(|e| {
            AppError::ValidationError(format!(
                "Could not reach the Gitea instance to refresh: {e}"
            ))
        })?;
    let status = response.status();
    if status == reqwest::StatusCode::BAD_REQUEST || status == reqwest::StatusCode::UNAUTHORIZED {
        clear_tokens(state, provider_id).await?;
        return Err(AppError::ValidationError(
            "Gitea session expired — reconnect".into(),
        ));
    }
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(AppError::ValidationError(format!(
            "Gitea token refresh failed ({status}): {}",
            body.chars().take(200).collect::<String>()
        )));
    }
    let token: TokenResponse = response
        .json()
        .await
        .map_err(|e| AppError::ValidationError(format!("Unexpected Gitea response: {e}")))?;
    let user_id = row.oauth_user_id.clone().unwrap_or_default();
    let user_login = row.oauth_user_login.clone().unwrap_or_default();
    let user = GiteaUser {
        id: user_id.parse().unwrap_or(0),
        login: user_login,
    };
    persist_tokens(state, provider_id, &token, &user).await?;
    Ok(())
}

async fn clear_tokens(state: &SharedState, provider_id: &str) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE git_providers SET \
            oauth_access_token_encrypted = NULL, \
            oauth_refresh_token_encrypted = NULL, \
            oauth_expires_at = NULL, \
            oauth_user_id = NULL, \
            oauth_user_login = NULL, \
            oauth_connected_at = NULL, \
            updated_at = ?1 \
         WHERE id = ?2",
    )
    .bind(now_ms())
    .bind(provider_id)
    .execute(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok(())
}

/// Removes the stored tokens. OAuth client credentials + webhook secret are kept.
pub async fn disconnect(
    state: &SharedState,
    provider_id: &str,
    organization_id: &str,
) -> Result<(), AppError> {
    let row = load_row(state, provider_id, organization_id).await?;
    if !PROVIDER_TYPES.contains(&row.provider_type.as_str()) {
        return Err(AppError::ValidationError(
            "Provider is not a Gitea/Forgejo type".into(),
        ));
    }
    clear_tokens(state, provider_id).await
}

fn urlencoded(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            _ => out.push_str(&format!("%{:02X}", byte)),
        }
    }
    out
}

#[allow(dead_code)]
pub fn test_provider_id() -> String {
    Uuid::now_v7().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_base_strips_trailing_slash() {
        assert_eq!(
            require_gitea_host("https://git.example.com/").unwrap(),
            "https://git.example.com"
        );
        assert!(require_gitea_host("   ").is_err());
    }

    #[test]
    fn url_base_endpoints_match_gitea_shapes() {
        assert_eq!(
            authorize_url("https://git.example.com").unwrap(),
            "https://git.example.com/login/oauth/authorize"
        );
        assert_eq!(
            token_url("https://git.example.com").unwrap(),
            "https://git.example.com/login/oauth/access_token"
        );
        assert_eq!(
            api_base("https://git.example.com").unwrap(),
            "https://git.example.com/api/v1"
        );
    }

    #[test]
    fn urlencoded_handles_scopes_and_reserved_chars() {
        assert_eq!(
            urlencoded("read:user read:organization"),
            "read%3Auser%20read%3Aorganization"
        );
        assert_eq!(urlencoded("abc._~-ABC"), "abc._~-ABC");
    }

    #[test]
    fn provider_types_are_gitea_and_forgejo() {
        assert!(PROVIDER_TYPES.contains(&"gitea"));
        assert!(PROVIDER_TYPES.contains(&"forgejo"));
        assert_eq!(PROVIDER_TYPES.len(), 2);
    }
}
