//! Bitbucket Cloud OAuth and webhook verification.
//!
//! Bitbucket Cloud only - no self-hosted host_url, unlike GitLab. Bitbucket
//! removed App Passwords entirely on 2026-07-28; OAuth consumer (client id +
//! secret, registered once in the workspace's settings) is the only auth
//! method left, so unlike `gitlab_oauth` this is the sole connection path
//! for a Bitbucket provider, not an alternative to a token.
//!
//! The OAuth consumer's callback URL is fixed at registration time in
//! Bitbucket's own UI, not passed per-request - `redirect_uri` is stored only
//! so the UI can show the user the exact value to paste there, never sent on
//! the authorize or token-exchange calls.
//!
//! Bitbucket webhooks HMAC-sign the raw body with the stored secret and send
//! it in `X-Hub-Signature: sha256=<hex>` - the same scheme GitHub uses, so
//! `handler::webhooks::verify_signature` is reused rather than duplicated.
//! (GitLab's webhook model is a plain shared-secret string compare instead,
//! which is why `gitlab_oauth` carries its own `verify_webhook`.)

use chrono::Utc;
use sqlx::Row;

use crate::{dtos::response::AppError, state::state::SharedState, utils::security};

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

const AUTHORIZE_URL: &str = "https://bitbucket.org/site/oauth2/authorize";
const TOKEN_URL: &str = "https://bitbucket.org/site/oauth2/access_token";
const API_BASE: &str = "https://api.bitbucket.org/2.0";
/// Refresh this far before expiry so a clone never starts with a token about
/// to die. Bitbucket access tokens live 2 hours.
const TOKEN_SKEW_MS: i64 = 300_000;
/// 10 minutes - a stuck entry just blocks reconnection for the organization.
const NONCE_TTL_MS: i64 = 600_000;
const SCOPES: &str = "repository account webhook";

const OAUTH_COLUMNS: &str = "oauth_client_id, oauth_client_secret_encrypted, oauth_redirect_uri, \
     oauth_access_token_encrypted, oauth_refresh_token_encrypted, oauth_expires_at, \
     oauth_scopes, oauth_user_id, oauth_user_login, oauth_connected_at, webhook_secret, \
     provider_type";

#[derive(Debug, Clone, serde::Serialize)]
pub struct BitbucketOauthStatus {
    pub connected: bool,
    pub user_login: Option<String>,
    pub user_id: Option<String>,
    pub scopes: Option<String>,
    pub connected_at: Option<i64>,
    pub expires_at: Option<i64>,
    pub has_client_credentials: bool,
    pub has_webhook_secret: bool,
    /// The exact callback URL to paste into the Bitbucket OAuth consumer's
    /// own settings - Bitbucket has no per-request redirect_uri.
    pub redirect_uri: String,
}

#[derive(Debug, Clone)]
struct OauthRow {
    oauth_client_id: Option<String>,
    oauth_client_secret_encrypted: Option<String>,
    #[allow(dead_code)] // display-only; never sent to Bitbucket.
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
}

/// CSRF nonce payload parked in `AppState::oauth_states` between our redirect and Bitbucket's.
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
    })
}

/// Scoped lookup for every call that originates from an organization-authenticated request.
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
/// path. Never call this from a handler.
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

async fn guard_outbound_url(raw: &str) -> Result<(), AppError> {
    let url =
        crate::security::ssrf::parse_https_url_with_port(raw).map_err(AppError::ValidationError)?;
    // Bitbucket Cloud only - never a self-hosted target, so private addresses are never allowed.
    crate::security::ssrf::assert_host(&url, false)
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
            "Set a server domain in Settings before connecting Bitbucket — Bitbucket must reach this server for the OAuth callback and webhook".into(),
        ));
    }
    Ok(url)
}

/// Where litepod redirects the browser after the user approves the Bitbucket OAuth consumer.
/// Shown in the UI for the user to paste into the consumer's own "Callback URL" field -
/// Bitbucket never receives this value from litepod directly.
pub fn redirect_uri(public_url: &str, provider_id: &str) -> String {
    format!(
        "{}/api/v1/git-providers/{provider_id}/bitbucket-oauth/callback",
        public_url.trim_end_matches('/')
    )
}

pub fn git_webhook_url(public_url: &str, provider_id: &str) -> String {
    format!(
        "{}/api/v1/webhooks/git/{provider_id}",
        public_url.trim_end_matches('/')
    )
}

/// Webhook secret shown once in the UI. Bitbucket HMAC-signs the delivery body with it
/// (`X-Hub-Signature`); see `handler::webhooks::verify_signature`.
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

/// The stored webhook secret, unscoped - called from the unauthenticated webhook route where
/// `provider_id` from the URL *is* the scope, mirroring `github_app::webhook_secret`.
pub async fn webhook_secret(state: &SharedState, provider_id: &str) -> Result<String, AppError> {
    let row = load_row_unscoped(state, provider_id).await?;
    if row.provider_type != "bitbucket" {
        return Err(AppError::WebhookVerificationFailed);
    }
    row.webhook_secret
        .filter(|s| !s.is_empty())
        .ok_or(AppError::WebhookVerificationFailed)
}

/// Called by `create()` / `update()` when the user pastes client_id + client_secret. Generates
/// a webhook secret if missing so the user can copy all three values into Bitbucket at once.
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
) -> Result<BitbucketOauthStatus, AppError> {
    let row = load_row(state, provider_id, organization_id).await?;
    let public = public_url(state).await.unwrap_or_default();
    Ok(BitbucketOauthStatus {
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
        redirect_uri: redirect_uri(&public, provider_id),
    })
}

/// Builds the Bitbucket authorize URL and parks a CSRF state. No `redirect_uri` param - the
/// OAuth consumer's callback URL is fixed in Bitbucket's own settings.
pub async fn begin_authorize(
    state: &SharedState,
    organization_id: &str,
    user_id: &str,
    provider_id: &str,
) -> Result<String, AppError> {
    let row = load_row(state, provider_id, organization_id).await?;
    if row.provider_type != "bitbucket" {
        return Err(AppError::ValidationError(
            "Bitbucket OAuth is only available for Bitbucket providers".into(),
        ));
    }
    let client_id = row.oauth_client_id.ok_or_else(|| {
        AppError::ValidationError("Save the client ID and secret before connecting".into())
    })?;
    let token = put_state(
        state,
        &StateEntry {
            organization_id: organization_id.to_string(),
            user_id: user_id.to_string(),
            provider_id: provider_id.to_string(),
            purpose: "bitbucket_oauth".to_string(),
            created_at: now_ms(),
        },
    )?;
    let url = format!(
        "{AUTHORIZE_URL}?client_id={}&response_type=code&state={}",
        urlencoded(&client_id),
        urlencoded(&token),
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
    scopes: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct BitbucketUser {
    uuid: String,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    nickname: Option<String>,
    #[serde(default)]
    display_name: Option<String>,
}

impl BitbucketUser {
    fn login(&self) -> String {
        self.username
            .clone()
            .or_else(|| self.nickname.clone())
            .or_else(|| self.display_name.clone())
            .unwrap_or_else(|| self.uuid.clone())
    }
}

/// Validates the `state` nonce, exchanges the `code` for tokens (HTTP Basic Auth, Bitbucket's
/// documented method - unlike GitLab's client_id/client_secret-in-body form), fetches the
/// Bitbucket user profile, and persists everything encrypted.
pub async fn complete_authorize(
    state: &SharedState,
    code: &str,
    state_token: &str,
) -> Result<String, AppError> {
    let entry = take_state(state, state_token, "bitbucket_oauth")?;
    let provider_id = entry.provider_id;
    let row = load_row(state, &provider_id, &entry.organization_id).await?;
    let client_id = row.oauth_client_id.ok_or_else(|| {
        AppError::ValidationError(
            "Client ID is missing — save the Bitbucket OAuth consumer credentials first".into(),
        )
    })?;
    let client_secret = row
        .oauth_client_secret_encrypted
        .as_deref()
        .ok_or_else(|| {
            AppError::ValidationError(
                "Client secret is missing — save the Bitbucket OAuth consumer credentials first"
                    .into(),
            )
        })?;
    let key = security::get_encryption_key();
    let client_secret =
        security::decrypt_data(client_secret, &key).map_err(AppError::InternalError)?;

    guard_outbound_url(TOKEN_URL).await?;
    let client = client()?;
    let token_response = client
        .post(TOKEN_URL)
        .basic_auth(&client_id, Some(&client_secret))
        .form(&[("grant_type", "authorization_code"), ("code", code)])
        .send()
        .await
        .map_err(|e| AppError::ValidationError(format!("Could not reach Bitbucket: {e}")))?;

    if !token_response.status().is_success() {
        let status = token_response.status();
        let body = token_response.text().await.unwrap_or_default();
        return Err(AppError::ValidationError(format!(
            "Bitbucket rejected the authorization code ({status}): {}",
            body.chars().take(200).collect::<String>()
        )));
    }
    let token: TokenResponse = token_response
        .json()
        .await
        .map_err(|e| AppError::ValidationError(format!("Unexpected Bitbucket response: {e}")))?;

    let user = fetch_user(&client, &token.access_token).await?;
    persist_tokens(state, &provider_id, &token, &user).await?;
    Ok(provider_id)
}

async fn fetch_user(
    client: &reqwest::Client,
    access_token: &str,
) -> Result<BitbucketUser, AppError> {
    let user_endpoint = format!("{API_BASE}/user");
    guard_outbound_url(&user_endpoint).await?;
    let response = client
        .get(user_endpoint)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| AppError::ValidationError(format!("Could not reach Bitbucket: {e}")))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(AppError::ValidationError(format!(
            "Bitbucket /user returned {status}: {}",
            body.chars().take(200).collect::<String>()
        )));
    }
    response
        .json::<BitbucketUser>()
        .await
        .map_err(|e| AppError::ValidationError(format!("Unexpected Bitbucket /user response: {e}")))
}

async fn persist_tokens(
    state: &SharedState,
    provider_id: &str,
    token: &TokenResponse,
    user: &BitbucketUser,
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
    let scopes = token.scopes.clone().or_else(|| Some(SCOPES.to_string()));
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
    .bind(user.uuid.clone())
    .bind(user.login())
    .bind(now_ms())
    .bind(now_ms())
    .bind(provider_id)
    .execute(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok(())
}

/// Returns a usable access token, refreshing the stored one when it is past the safety skew.
pub async fn access_token(state: &SharedState, provider_id: &str) -> Result<String, AppError> {
    let row = load_row_unscoped(state, provider_id).await?;
    if row.provider_type != "bitbucket" {
        return Err(AppError::ValidationError(
            "Provider is not a Bitbucket type".into(),
        ));
    }
    let needs_refresh = match row.oauth_expires_at {
        Some(expires) => expires <= now_ms() + TOKEN_SKEW_MS,
        None => true,
    };
    if needs_refresh {
        if row.oauth_refresh_token_encrypted.is_none() {
            return Err(AppError::ValidationError(
                "Bitbucket connection expired and no refresh token — reconnect".into(),
            ));
        }
        refresh(state, provider_id, &row).await?;
    }
    let after = load_row_unscoped(state, provider_id).await?;
    let ciphertext = after
        .oauth_access_token_encrypted
        .ok_or_else(|| AppError::ValidationError("Bitbucket is not connected".into()))?;
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

    guard_outbound_url(TOKEN_URL).await?;
    let http = client()?;
    let response = http
        .post(TOKEN_URL)
        .basic_auth(&client_id, Some(&client_secret))
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token.as_str()),
        ])
        .send()
        .await
        .map_err(|e| {
            AppError::ValidationError(format!("Could not reach Bitbucket to refresh: {e}"))
        })?;
    let status = response.status();
    if status == reqwest::StatusCode::BAD_REQUEST || status == reqwest::StatusCode::UNAUTHORIZED {
        clear_tokens(state, provider_id).await?;
        return Err(AppError::ValidationError(
            "Bitbucket session expired — reconnect".into(),
        ));
    }
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(AppError::ValidationError(format!(
            "Bitbucket token refresh failed ({status}): {}",
            body.chars().take(200).collect::<String>()
        )));
    }
    let token: TokenResponse = response
        .json()
        .await
        .map_err(|e| AppError::ValidationError(format!("Unexpected Bitbucket response: {e}")))?;
    let user = BitbucketUser {
        uuid: row.oauth_user_id.clone().unwrap_or_default(),
        username: row.oauth_user_login.clone(),
        nickname: None,
        display_name: None,
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

/// Removes the stored tokens. The OAuth client credentials + webhook secret are kept.
pub async fn disconnect(
    state: &SharedState,
    provider_id: &str,
    organization_id: &str,
) -> Result<(), AppError> {
    let row = load_row(state, provider_id, organization_id).await?;
    if row.provider_type != "bitbucket" {
        return Err(AppError::ValidationError(
            "Provider is not a Bitbucket type".into(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{migrations, state::state::AppState};
    use std::{
        collections::{HashMap, HashSet},
        sync::{Arc, Mutex},
    };
    use uuid::Uuid;

    async fn test_state() -> (SharedState, std::path::PathBuf) {
        let path =
            std::env::temp_dir().join(format!("litepod-bitbucket-oauth-{}.db", Uuid::now_v7()));
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

    async fn insert_bitbucket_provider(state: &SharedState, name: &str) -> String {
        let conn = crate::git::repositories::db::get_conn(state).await;
        let id = Uuid::now_v7().to_string();
        sqlx::query(
            "INSERT INTO git_providers (id, organization_id, name, provider_type, host_url, username, password_encrypted, auth_method, kind, created_at, updated_at) \
             VALUES (?1, 'organization-1', ?2, 'bitbucket', '', 'oauth-pending', 'placeholder', 'bitbucket_oauth', 'credentials', 1, 1)",
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
    }

    #[tokio::test]
    async fn webhook_secret_is_generated_once_and_reused() {
        let (state, path) = test_state().await;
        let provider_id = insert_bitbucket_provider(&state, "BB").await;
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
    async fn webhook_secret_lookup_rejects_non_bitbucket_provider() {
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
            webhook_secret(&state, &id).await,
            Err(AppError::WebhookVerificationFailed)
        ));
        cleanup(path);
    }

    #[tokio::test]
    async fn save_client_credentials_writes_encrypted_secret_and_redirect() {
        let (state, path) = test_state().await;
        let provider_id = insert_bitbucket_provider(&state, "BB").await;
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
        let ciphertext = row.oauth_client_secret_encrypted.unwrap();
        assert!(!ciphertext.contains("super-secret"));
        assert_eq!(
            row.oauth_redirect_uri.as_deref(),
            Some(
                format!(
                    "https://litepod.example.test/api/v1/git-providers/{provider_id}/bitbucket-oauth/callback"
                )
                .as_str()
            )
        );
        assert!(row.webhook_secret.is_some());
        cleanup(path);
    }

    #[tokio::test]
    async fn begin_authorize_has_no_redirect_uri_param() {
        let (state, path) = test_state().await;
        let provider_id = insert_bitbucket_provider(&state, "BB").await;
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
            url.starts_with("https://bitbucket.org/site/oauth2/authorize?"),
            "got {url}"
        );
        assert!(url.contains("client_id=client-id-123"), "got {url}");
        assert!(url.contains("response_type=code"));
        assert!(
            !url.contains("redirect_uri"),
            "Bitbucket's callback is fixed in the consumer's own settings: {url}"
        );
        let guard = state.oauth_states.lock().unwrap();
        assert_eq!(guard.len(), 1);
        cleanup(path);
    }

    #[tokio::test]
    async fn begin_authorize_rejects_non_bitbucket_provider() {
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
        let provider_id = insert_bitbucket_provider(&state, "BB").await;
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
        let first = take_state(&state, &token, "bitbucket_oauth");
        assert!(first.is_ok());
        let second = take_state(&state, &token, "bitbucket_oauth");
        assert!(matches!(second, Err(AppError::ValidationError(_))));
        cleanup(path);
    }

    #[tokio::test]
    async fn status_reports_disconnected_when_no_tokens() {
        let (state, path) = test_state().await;
        let provider_id = insert_bitbucket_provider(&state, "BB").await;
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
        let provider_id = insert_bitbucket_provider(&state, "BB").await;

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
        assert!(status(&state, &provider_id, "organization-1").await.is_ok());
        cleanup(path);
    }
}
