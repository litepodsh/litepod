//! GitLab OAuth (cloud + self-hosted) and webhook verification.
//!
//! The flow is the standard OAuth2 authorization-code grant, identical for gitlab.com and a
//! self-hosted instance — only `host_url` changes. Tokens are stored AES-GCM encrypted with
//! the same key as `password_encrypted` so the existing key derivation covers them.
//!
//! GitLab webhooks differ from GitHub's: they use a plain shared secret in the
//! `X-Gitlab-Token` header (no HMAC). The user pastes the same secret into their GitLab
//! webhook settings, and the backend compares it constant-time on every delivery.

use chrono::Utc;
use sqlx::Row;
use uuid::Uuid;

use crate::{dtos::response::AppError, state::state::SharedState, utils::security};

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

/// Refresh the access token this far before expiry so a clone never starts with one about
/// to die. GitLab's default access-token lifetime is 2 hours.
const TOKEN_SKEW_MS: i64 = 300_000;
/// 10 minutes — GitLab rejects the OAuth state if it lingers, and a stuck entry just blocks
/// reconnection for the organization.
const NONCE_TTL_MS: i64 = 600_000;
const SCOPES: &str = "api";

const OAUTH_COLUMNS: &str = "oauth_client_id, oauth_client_secret_encrypted, oauth_redirect_uri, \
     oauth_access_token_encrypted, oauth_refresh_token_encrypted, oauth_expires_at, \
     oauth_scopes, oauth_user_id, oauth_user_login, oauth_connected_at, webhook_secret, \
     provider_type, host_url";

#[derive(Debug, Clone, serde::Serialize)]
pub struct GitlabOauthStatus {
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
    #[allow(dead_code)] // re-populated once the redirect-uri validation path lands.
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

/// CSRF nonce payload parked in `AppState::oauth_states` between our redirect and GitLab's.
#[derive(serde::Serialize, serde::Deserialize)]
struct StateEntry {
    organization_id: String,
    user_id: String,
    provider_id: String,
    purpose: String,
    created_at: i64,
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
    // Prune on write so abandoned flows do not accumulate across providers.
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

/// Scoped lookup for every call that originates from an organization-authenticated request —
/// the org filter is what stops a caller from reaching another organization's provider by id.
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

/// Unscoped lookup for callers that already resolved `provider_id` through a trusted internal
/// path — deploy-time credential resolution keyed off the application's own configured
/// provider — rather than from attacker-supplied request input. Never call this from a handler.
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

fn host_base(host_url: &str) -> String {
    let trimmed = host_url.trim().trim_end_matches('/');
    if trimmed.is_empty() || trimmed == "https://gitlab.com" {
        "https://gitlab.com".to_string()
    } else {
        trimmed.to_string()
    }
}

fn api_base(host_url: &str) -> String {
    format!("{}/api/v4", host_base(host_url))
}

fn authorize_url(host_url: &str) -> String {
    format!("{}/oauth/authorize", host_base(host_url))
}

fn token_url(host_url: &str) -> String {
    format!("{}/oauth/token", host_base(host_url))
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
            "Set a server domain in Settings before connecting GitLab — GitLab must reach this server for the OAuth callback and webhook".into(),
        ));
    }
    Ok(url)
}

/// Where litepod redirects the browser after the user approves the GitLab OAuth app.
pub fn redirect_uri(public_url: &str, provider_id: &str) -> String {
    format!(
        "{}/api/v1/git-providers/{provider_id}/gitlab-oauth/callback",
        public_url.trim_end_matches('/')
    )
}

/// Unified git webhook URL — used by Gitea/Forgejo providers and any future provider that
/// funnels through the single `/api/v1/webhooks/git/{provider_id}` route. GitHub providers
/// have their own app-level webhook URL.
pub fn git_webhook_url(public_url: &str, provider_id: &str) -> String {
    format!(
        "{}/api/v1/webhooks/git/{provider_id}",
        public_url.trim_end_matches('/')
    )
}

/// Webhook secret shown once in the UI after the provider is created. GitLab sends the same
/// value in `X-Gitlab-Token`; constant-time compare on every delivery.
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

/// Called by `create()` / `update()` when the user pastes client_id + client_secret. Generates
/// a webhook secret if missing so the user can copy all three values into GitLab at once.
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
    // Webhook secret: generate once so it is stable across UI reloads.
    let _ = ensure_webhook_secret(state, provider_id, organization_id).await?;
    Ok(())
}

pub async fn status(
    state: &SharedState,
    provider_id: &str,
    organization_id: &str,
) -> Result<GitlabOauthStatus, AppError> {
    let row = load_row(state, provider_id, organization_id).await?;
    Ok(GitlabOauthStatus {
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

/// Builds the GitLab authorize URL and parks a CSRF state. The browser is then redirected to
/// `authorize_url`, and GitLab calls back into our public route with `code` + `state`.
pub async fn begin_authorize(
    state: &SharedState,
    organization_id: &str,
    user_id: &str,
    provider_id: &str,
) -> Result<String, AppError> {
    let row = load_row(state, provider_id, organization_id).await?;
    if row.provider_type != "gitlab" {
        return Err(AppError::ValidationError(
            "GitLab OAuth is only available for GitLab providers".into(),
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
            purpose: "gitlab_oauth".to_string(),
            created_at: now_ms(),
        },
    )?;
    let url = format!(
        "{}?client_id={}&redirect_uri={}&response_type=code&state={}&scope={}",
        authorize_url(&row.host_url),
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
    #[allow(dead_code)] // captured for debugging; not consumed yet.
    token_type: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct GitlabUser {
    id: i64,
    username: String,
}

/// Validates the `state` nonce, exchanges the `code` for tokens, fetches the GitLab user
/// profile, and persists everything encrypted. Returns the provider_id so the handler can
/// redirect the browser back into the UI.
pub async fn complete_authorize(
    state: &SharedState,
    code: &str,
    state_token: &str,
) -> Result<String, AppError> {
    let entry = take_state(state, state_token, "gitlab_oauth")?;
    let provider_id = entry.provider_id;
    // The state entry was parked by `begin_authorize` after that call already checked the
    // caller's membership in `entry.organization_id` — reusing it here (instead of trusting
    // anything from the callback request, which carries no session) keeps this load scoped to
    // the same organization the flow started in.
    let row = load_row(state, &provider_id, &entry.organization_id).await?;
    let client_id = row.oauth_client_id.ok_or_else(|| {
        AppError::ValidationError(
            "Client ID is missing — save the GitLab App credentials first".into(),
        )
    })?;
    let client_secret = row
        .oauth_client_secret_encrypted
        .as_deref()
        .ok_or_else(|| {
            AppError::ValidationError(
                "Client secret is missing — save the GitLab App credentials first".into(),
            )
        })?;
    let key = security::get_encryption_key();
    let client_secret =
        security::decrypt_data(client_secret, &key).map_err(AppError::InternalError)?;
    let public = public_url(state).await?;
    let redirect = redirect_uri(&public, &provider_id);

    let token_endpoint = token_url(&row.host_url);
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
        .map_err(|e| AppError::ValidationError(format!("Could not reach GitLab: {e}")))?;

    if !token_response.status().is_success() {
        let status = token_response.status();
        let body = token_response.text().await.unwrap_or_default();
        return Err(AppError::ValidationError(format!(
            "GitLab rejected the authorization code ({status}): {}",
            body.chars().take(200).collect::<String>()
        )));
    }
    let token: TokenResponse = token_response
        .json()
        .await
        .map_err(|e| AppError::ValidationError(format!("Unexpected GitLab response: {e}")))?;

    // GitLab returns the same error for "code already used" and "code expired" — the state
    // nonce is single-use, so a replay here means the same browser tab finished twice and we
    // should drop this attempt without overwriting an already-stored token.
    let user = fetch_user(
        &client,
        &api_base(&row.host_url),
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
) -> Result<GitlabUser, AppError> {
    let user_endpoint = format!("{api_base}/user");
    guard_outbound_url(&user_endpoint, allow_private).await?;
    let response = client
        .get(user_endpoint)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| AppError::ValidationError(format!("Could not reach GitLab: {e}")))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(AppError::ValidationError(format!(
            "GitLab /user returned {status}: {}",
            body.chars().take(200).collect::<String>()
        )));
    }
    response
        .json::<GitlabUser>()
        .await
        .map_err(|e| AppError::ValidationError(format!("Unexpected GitLab /user response: {e}")))
}

async fn persist_tokens(
    state: &SharedState,
    provider_id: &str,
    token: &TokenResponse,
    user: &GitlabUser,
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
    .bind(user.username.clone())
    .bind(now_ms())
    .bind(now_ms())
    .bind(provider_id)
    .execute(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok(())
}

/// Returns a usable access token, refreshing the stored one when it is past the safety skew.
/// All callers go through this so a single refresh-on-demand path covers clones, list, and
/// any future API surface.
pub async fn access_token(state: &SharedState, provider_id: &str) -> Result<String, AppError> {
    let row = load_row_unscoped(state, provider_id).await?;
    if row.provider_type != "gitlab" {
        return Err(AppError::ValidationError(
            "Provider is not a GitLab type".into(),
        ));
    }
    let needs_refresh = match row.oauth_expires_at {
        Some(expires) => expires <= now_ms() + TOKEN_SKEW_MS,
        None => true,
    };
    if needs_refresh {
        if row.oauth_refresh_token_encrypted.is_none() {
            return Err(AppError::ValidationError(
                "GitLab connection expired and no refresh token — reconnect".into(),
            ));
        }
        refresh(state, provider_id, &row).await?;
    }
    let after = load_row_unscoped(state, provider_id).await?;
    let ciphertext = after
        .oauth_access_token_encrypted
        .ok_or_else(|| AppError::ValidationError("GitLab is not connected".into()))?;
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

    let token_endpoint = token_url(&row.host_url);
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
            AppError::ValidationError(format!("Could not reach GitLab to refresh: {e}"))
        })?;
    let status = response.status();
    if status == reqwest::StatusCode::BAD_REQUEST || status == reqwest::StatusCode::UNAUTHORIZED {
        // Refresh token rejected — wipe the stored tokens so the UI can show a clear
        // "reconnect" state instead of repeatedly retrying.
        clear_tokens(state, provider_id).await?;
        return Err(AppError::ValidationError(
            "GitLab session expired — reconnect".into(),
        ));
    }
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(AppError::ValidationError(format!(
            "GitLab token refresh failed ({status}): {}",
            body.chars().take(200).collect::<String>()
        )));
    }
    let token: TokenResponse = response
        .json()
        .await
        .map_err(|e| AppError::ValidationError(format!("Unexpected GitLab response: {e}")))?;
    // GitLab rotates the refresh token on each refresh; persist the new one.
    let user_id = row.oauth_user_id.clone().unwrap_or_default();
    let user_login = row.oauth_user_login.clone().unwrap_or_default();
    let user = GitlabUser {
        id: user_id.parse().unwrap_or(0),
        username: user_login,
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

/// Removes the stored tokens. The OAuth client credentials + webhook secret are kept so
/// the user does not have to re-paste them to reconnect.
pub async fn disconnect(
    state: &SharedState,
    provider_id: &str,
    organization_id: &str,
) -> Result<(), AppError> {
    let row = load_row(state, provider_id, organization_id).await?;
    if row.provider_type != "gitlab" {
        return Err(AppError::ValidationError(
            "Provider is not a GitLab type".into(),
        ));
    }
    clear_tokens(state, provider_id).await
}

/// Constant-time check of the `X-Gitlab-Token` header against the stored webhook secret.
/// Returns the same error whether the provider is unknown or the token mismatches so the
/// endpoint does not confirm which provider ids exist. Unauthenticated route — `provider_id`
/// from the URL *is* the scope, there is no session to check an organization against.
pub async fn verify_webhook(
    state: &SharedState,
    provider_id: &str,
    header_value: Option<&str>,
) -> Result<(), AppError> {
    let row = load_row_unscoped(state, provider_id).await?;
    if row.provider_type != "gitlab" {
        return Err(AppError::WebhookVerificationFailed);
    }
    let secret = row
        .webhook_secret
        .as_deref()
        .ok_or(AppError::WebhookVerificationFailed)?;
    let supplied = header_value.unwrap_or_default();
    if secret.len() != supplied.len() {
        return Err(AppError::WebhookVerificationFailed);
    }
    let mut diff = 0u8;
    for (a, b) in secret.as_bytes().iter().zip(supplied.as_bytes().iter()) {
        diff |= a ^ b;
    }
    if diff != 0 {
        return Err(AppError::WebhookVerificationFailed);
    }
    Ok(())
}

fn urlencoded(input: &str) -> String {
    // Minimal URL-encoding for the params we use (no whitespace, no unicode, alnum + `._~-`).
    // GitLab rejects the authorize call on malformed values, so being conservative matters.
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
    use crate::{migrations, state::state::AppState};
    use std::{
        collections::{HashMap, HashSet},
        sync::{Arc, Mutex},
    };

    async fn test_state() -> (SharedState, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!("litepod-gitlab-oauth-{}.db", Uuid::now_v7()));
        let db = crate::testing::sqlite_pool_at(&path).await;
        migrations::run(&db).await;
        let state: SharedState = Arc::new(AppState {
            started_at: chrono::Local::now(),
            db,
            caddy_url: String::new(),
            caddy_admin_url: String::new(),
            caddyfile_path: std::path::PathBuf::new(),
            caddy_server_upstream: String::new(),
            podman_socket: String::new(),
            pending_execs: Mutex::new(HashMap::new()),
            data_dir: std::path::PathBuf::new(),
            data_volume: None,
            host_data_dir: std::path::PathBuf::new(),
            podman_network: String::new(),
            public_url: "https://litepod.example.test".into(),
            oauth_states: Mutex::new(HashMap::new()),
            public_ip: "203.0.113.7".into(),
            is_cloud: false,
            polar: None,
            dev_mode: true,
            port_range: (10_000, 20_000),
            reserved_ports: HashSet::new(),
            volume_access: Default::default(),
            podman_host: Default::default(),
            queue_runtime: Default::default(),
            version: String::new(),
            git_sha: String::new(),
            image_repo: String::new(),
            image_tag: String::new(),
            update_channel: Default::default(),
            version_manifest_url: String::new(),
            update_interval_secs: 0,
            container_name: String::new(),
            caddy_container: String::new(),
            stack_env_file: None,
            update_state: Default::default(),
            backup_lock: Default::default(),
            rate_limiter: tokio::sync::RwLock::new(
                crate::security::rate_limit::RateLimiterState::from_config(Default::default()),
            ),
            ephemeral: Default::default(),
        });
        let conn = state.db.clone();
        sqlx::query("INSERT INTO users (id, email, hashed_password, name) VALUES (?1, ?2, ?3, ?4)")
            .bind("user-1")
            .bind("user@example.com")
            .bind("hash")
            .bind("User")
            .execute(&conn)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO organizations (id, name, slug, created_by) VALUES (?1, ?2, ?3, ?4)",
        )
        .bind("organization-1")
        .bind("Organization")
        .bind("organization")
        .bind("user-1")
        .execute(&conn)
        .await
        .unwrap();
        (state, path)
    }

    fn cleanup(path: std::path::PathBuf) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{}", path.display(), suffix));
        }
    }

    async fn insert_gitlab_provider(state: &SharedState, name: &str) -> String {
        let conn = crate::git::repositories::db::get_conn(state).await;
        let id = Uuid::now_v7().to_string();
        sqlx::query(
            "INSERT INTO git_providers (id, organization_id, name, provider_type, host_url, username, password_encrypted, auth_method, kind, created_at, updated_at) \
             VALUES (?1, 'organization-1', ?2, 'gitlab', 'https://gitlab.example.test', 'oauth-pending', 'placeholder', 'token', 'credentials', 1, 1)",
        )
        .bind(id.clone())
        .bind(name)
        .execute(&conn)
        .await
        .unwrap();
        id
    }

    #[test]
    fn urlencoded_preserves_unreserved_chars() {
        assert_eq!(urlencoded("abc-DEF_1.2~3"), "abc-DEF_1.2~3");
        assert_eq!(urlencoded("a b/c?d"), "a%20b%2Fc%3Fd");
        assert_eq!(urlencoded("café"), "caf%C3%A9");
    }

    #[test]
    fn authorize_url_defaults_to_gitlab_com_when_host_blank() {
        assert_eq!(authorize_url(""), "https://gitlab.com/oauth/authorize");
        assert_eq!(
            authorize_url("https://gitlab.com"),
            "https://gitlab.com/oauth/authorize"
        );
        assert_eq!(
            authorize_url("https://gitlab.example.test/"),
            "https://gitlab.example.test/oauth/authorize"
        );
    }

    #[test]
    fn api_base_uses_host_for_self_hosted() {
        assert_eq!(api_base(""), "https://gitlab.com/api/v4");
        assert_eq!(api_base("https://gitlab.com"), "https://gitlab.com/api/v4");
        assert_eq!(
            api_base("https://gitlab.example.test"),
            "https://gitlab.example.test/api/v4"
        );
    }

    #[tokio::test]
    async fn webhook_secret_is_generated_once_and_reused() {
        let (state, path) = test_state().await;
        let provider_id = insert_gitlab_provider(&state, "Lab").await;
        let first = ensure_webhook_secret(&state, &provider_id, "organization-1")
            .await
            .unwrap();
        assert!(!first.is_empty());
        let second = ensure_webhook_secret(&state, &provider_id, "organization-1")
            .await
            .unwrap();
        assert_eq!(first, second, "secret must be stable across calls");
        cleanup(path);
    }

    #[tokio::test]
    async fn webhook_verify_accepts_matching_token_and_rejects_mismatch() {
        let (state, path) = test_state().await;
        let provider_id = insert_gitlab_provider(&state, "Lab").await;
        let secret = ensure_webhook_secret(&state, &provider_id, "organization-1")
            .await
            .unwrap();
        assert!(
            verify_webhook(&state, &provider_id, Some(&secret))
                .await
                .is_ok()
        );
        assert!(
            verify_webhook(&state, &provider_id, Some("wrong-token"))
                .await
                .is_err()
        );
        assert!(verify_webhook(&state, &provider_id, None).await.is_err());
        // Missing token must fail with the same error as a wrong one.
        assert!(matches!(
            verify_webhook(&state, &provider_id, None).await,
            Err(AppError::WebhookVerificationFailed)
        ));
        cleanup(path);
    }

    #[tokio::test]
    async fn webhook_verify_rejects_non_gitlab_provider() {
        let (state, path) = test_state().await;
        let conn = crate::git::repositories::db::get_conn(&state).await;
        let id = Uuid::now_v7().to_string();
        sqlx::query(
            "INSERT INTO git_providers (id, organization_id, name, provider_type, host_url, username, password_encrypted, auth_method, kind, created_at, updated_at, webhook_secret) \
             VALUES (?1, 'organization-1', 'GH', 'github', 'https://github.com', 'x', 'p', 'token', 'credentials', 1, 1, 's3cr3t')",
        )
        .bind(id.clone())
        .execute(&conn)
        .await
        .unwrap();
        assert!(matches!(
            verify_webhook(&state, &id, Some("s3cr3t")).await,
            Err(AppError::WebhookVerificationFailed)
        ));
        cleanup(path);
    }

    #[tokio::test]
    async fn save_client_credentials_writes_encrypted_secret_and_redirect() {
        let (state, path) = test_state().await;
        let provider_id = insert_gitlab_provider(&state, "Lab").await;
        save_client_credentials(
            &state,
            &provider_id,
            "organization-1",
            "client-id-123",
            "super-secret",
        )
        .await
        .unwrap();

        let row = load_row(&state, &provider_id, "organization-1")
            .await
            .unwrap();
        assert_eq!(row.oauth_client_id.as_deref(), Some("client-id-123"));
        // Stored secret must be encrypted, not plaintext.
        let ciphertext = row.oauth_client_secret_encrypted.unwrap();
        assert!(!ciphertext.contains("super-secret"));
        assert_eq!(row.oauth_redirect_uri.as_deref(), Some(format!("https://litepod.example.test/api/v1/git-providers/{provider_id}/gitlab-oauth/callback").as_str()));
        // Webhook secret is generated as a side effect so the UI can show it right after.
        assert!(row.webhook_secret.is_some());
        cleanup(path);
    }

    #[tokio::test]
    async fn begin_authorize_uses_gitlab_com_when_host_blank_and_self_hosted_otherwise() {
        let (state, path) = test_state().await;
        let provider_id = insert_gitlab_provider(&state, "Lab").await;
        save_client_credentials(
            &state,
            &provider_id,
            "organization-1",
            "client-id-123",
            "secret",
        )
        .await
        .unwrap();

        let url = begin_authorize(&state, "organization-1", "user-1", &provider_id)
            .await
            .unwrap();
        assert!(
            url.starts_with("https://gitlab.example.test/oauth/authorize?"),
            "got {url}"
        );
        assert!(url.contains("client_id=client-id-123"), "got {url}");
        assert!(url.contains("response_type=code"));
        assert!(url.contains("scope=api"));
        // State is a random nonce — assert it was parked so the callback can validate it.
        let guard = state.oauth_states.lock().unwrap();
        assert_eq!(guard.len(), 1);
        cleanup(path);
    }

    #[tokio::test]
    async fn begin_authorize_rejects_non_gitlab_provider() {
        let (state, path) = test_state().await;
        let conn = crate::git::repositories::db::get_conn(&state).await;
        let id = Uuid::now_v7().to_string();
        sqlx::query(
            "INSERT INTO git_providers (id, organization_id, name, provider_type, host_url, username, password_encrypted, auth_method, kind, created_at, updated_at) \
             VALUES (?1, 'organization-1', 'GH', 'github', '', 'x', 'p', 'token', 'credentials', 1, 1)",
        )
        .bind(id.clone())
        .execute(&conn)
        .await
        .unwrap();
        let result = begin_authorize(&state, "organization-1", "user-1", &id).await;
        assert!(
            matches!(result, Err(AppError::ValidationError(_))),
            "got {result:?}"
        );
        cleanup(path);
    }

    #[tokio::test]
    async fn state_nonce_is_single_use() {
        let (state, path) = test_state().await;
        let provider_id = insert_gitlab_provider(&state, "Lab").await;
        save_client_credentials(
            &state,
            &provider_id,
            "organization-1",
            "client-id-123",
            "secret",
        )
        .await
        .unwrap();
        let url = begin_authorize(&state, "organization-1", "user-1", &provider_id)
            .await
            .unwrap();
        let token = url
            .split("state=")
            .nth(1)
            .unwrap()
            .split('&')
            .next()
            .unwrap()
            .to_string();

        // First take consumes the nonce.
        let first = take_state(&state, &token, "gitlab_oauth");
        assert!(first.is_ok());
        // Second take is rejected with the same validation error so replays are visible.
        let second = take_state(&state, &token, "gitlab_oauth");
        assert!(matches!(second, Err(AppError::ValidationError(_))));
        cleanup(path);
    }

    #[tokio::test]
    async fn state_nonce_rejects_wrong_purpose() {
        let (state, path) = test_state().await;
        let token = put_state(
            &state,
            &StateEntry {
                organization_id: "organization-1".into(),
                user_id: "user-1".into(),
                provider_id: "p".into(),
                purpose: "github_manifest".into(),
                created_at: now_ms(),
            },
        )
        .unwrap();
        // Even with a fresh nonce, the wrong purpose must be rejected so a callback cannot
        // be triggered for one flow by replaying another's state.
        assert!(matches!(
            take_state(&state, &token, "gitlab_oauth"),
            Err(AppError::ValidationError(_))
        ));
        cleanup(path);
    }

    #[tokio::test]
    async fn status_reports_disconnected_when_no_tokens() {
        let (state, path) = test_state().await;
        let provider_id = insert_gitlab_provider(&state, "Lab").await;
        let status = status(&state, &provider_id, "organization-1")
            .await
            .unwrap();
        assert!(!status.connected);
        assert_eq!(status.user_login, None);
        assert!(!status.has_client_credentials);
        assert!(!status.has_webhook_secret);
        cleanup(path);
    }

    #[tokio::test]
    async fn org_scoped_lookups_reject_a_provider_from_another_organization() {
        let (state, path) = test_state().await;
        let conn = crate::git::repositories::db::get_conn(&state).await;
        sqlx::query(
            "INSERT INTO organizations (id, name, slug, created_by) VALUES ('organization-2', 'Other', 'other', 'user-1')",
        )
        .execute(&conn)
        .await
        .unwrap();
        let provider_id = insert_gitlab_provider(&state, "Lab").await; // lives in organization-1

        assert!(matches!(
            status(&state, &provider_id, "organization-2").await,
            Err(AppError::ProviderNotFound)
        ));
        assert!(matches!(
            ensure_webhook_secret(&state, &provider_id, "organization-2").await,
            Err(AppError::ProviderNotFound)
        ));
        assert!(matches!(
            disconnect(&state, &provider_id, "organization-2").await,
            Err(AppError::ProviderNotFound)
        ));
        // The owning organization can still read/act on its own provider.
        assert!(status(&state, &provider_id, "organization-1").await.is_ok());
        cleanup(path);
    }
}
