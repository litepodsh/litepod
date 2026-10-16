//! GitHub App support: manifest creation, installation, and short-lived installation tokens.
//!
//! Unlike credential providers, nothing here stores a long-lived git secret. The app's private
//! key mints a 10-minute RS256 JWT, the JWT mints a 1-hour installation token, and that token is
//! what clones and lists repositories. Tokens are cached in `github_apps` so restarts do not
//! re-mint (GitHub rate-limits token creation).

use chrono::Utc;
use sqlx::Row;
use uuid::Uuid;

use crate::{
    dtos::{git_provider::GitRepositoryDto, response::AppError},
    state::state::SharedState,
    utils::security,
};

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

/// Refresh this far before expiry so a clone never starts with a token about to die.
const TOKEN_SKEW_MS: i64 = 300_000;
/// GitHub rejects app JWTs with a lifetime over 10 minutes.
const JWT_TTL_SECONDS: i64 = 540;
const NONCE_TTL_MS: i64 = 600_000;

const APP_COLUMNS: &str =
    "app_id, slug, owner_login, owner_type, html_url, api_base, web_base, installation_id, status";

#[derive(Debug, Clone)]
pub struct GithubApp {
    pub git_provider_id: String,
    pub app_id: Option<i64>,
    pub slug: Option<String>,
    pub owner_login: String,
    pub owner_type: String,
    pub html_url: String,
    pub api_base: String,
    pub web_base: String,
    pub installation_id: Option<i64>,
    pub status: String,
}

/// CSRF nonce payload parked in `AppState::oauth_states` between our redirect and GitHub's.
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
        .build()
        .map_err(|e| AppError::InternalError(format!("HTTP client build failed: {e}")))
}

/// Random 32-byte base64url nonce. Single use, consumed by the matching callback.
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
    // Prune on write so abandoned flows cannot accumulate.
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

/// GitHub has to reach this host, so loopback only makes sense while developing. The URL comes
/// from the server domain in settings, falling back to the LITEPOD_PUBLIC_URL env var.
async fn validate_public_url(state: &SharedState) -> Result<String, AppError> {
    let url = crate::handler::settings::public_base_url(state).await;
    let is_loopback =
        url.contains("localhost") || url.contains("127.0.0.1") || url.contains("0.0.0.0");
    if is_loopback && !state.dev_mode {
        return Err(AppError::ValidationError(
            "Set a server domain in Settings before connecting GitHub — GitHub must reach this server for callbacks and webhooks".into(),
        ));
    }
    Ok(url)
}

/// `github_apps` has no `organization_id` of its own — it joins through the `git_providers`
/// row it belongs to, the same way every other provider-scoped lookup in this module does.
pub async fn get(
    state: &SharedState,
    provider_id: &str,
    organization_id: &str,
) -> Result<GithubApp, AppError> {
    use sqlx::Row;
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(&format!(
        "SELECT {APP_COLUMNS} FROM github_apps ga \
         INNER JOIN git_providers gp ON gp.id = ga.git_provider_id \
         WHERE ga.git_provider_id = ?1 AND gp.organization_id = ?2"
    ))
    .bind(provider_id)
    .bind(organization_id)
    .fetch_optional(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?
    .ok_or(AppError::ProviderNotFound)?;
    let read = |index: usize| -> Result<String, AppError> {
        row.try_get(index)
            .map_err(|e| AppError::DatabaseError(e.to_string()))
    };
    Ok(GithubApp {
        git_provider_id: provider_id.to_string(),
        app_id: row
            .try_get::<Option<i64>, _>(0)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        slug: row
            .try_get::<Option<String>, _>(1)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        owner_login: read(2)?,
        owner_type: read(3)?,
        html_url: read(4)?,
        api_base: read(5)?,
        web_base: read(6)?,
        installation_id: row
            .try_get::<Option<i64>, _>(7)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        status: read(8)?,
    })
}

/// The URL GitHub posts push events to. Per-provider so the webhook secret to verify against is
/// known before any verification work happens. Must match the unified route registered in
/// `router.rs` (`/api/v1/webhooks/git/{provider_id}`) — this used to say `/webhooks/github/`,
/// a path that was never routed, so every GitHub App push 404'd and never reached the handler.
pub fn webhook_url(public_url: &str, provider_id: &str) -> String {
    format!(
        "{}/api/v1/webhooks/git/{provider_id}",
        public_url.trim_end_matches('/')
    )
}

pub struct ManifestForm {
    pub post_url: String,
    pub manifest: String,
    pub state: String,
    pub provider_id: String,
}

/// Creates the placeholder rows and builds the form GitHub's manifest endpoint expects.
///
/// The provider row is written *before* the redirect so the manifest can embed a stable webhook
/// URL — GitHub registers the hook at creation time, not afterwards.
pub async fn begin_manifest(
    state: &SharedState,
    organization_id: &str,
    user_id: &str,
    name: &str,
    organization: Option<&str>,
) -> Result<ManifestForm, AppError> {
    let public_url = validate_public_url(state).await?;
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::ValidationError(
            "Provider name is required".into(),
        ));
    }

    let provider_id = Uuid::now_v7().to_string();
    let timestamp = now_ms();
    let conn = crate::git::repositories::db::get_conn(state).await;
    // App rows cannot go through create(): username/password_encrypted are NOT NULL and create()
    // rejects an empty password. username is filled with the app slug once GitHub returns it.
    sqlx::query(
        "INSERT INTO git_providers (id, organization_id, name, provider_type, host_url, username, password_encrypted, auth_method, kind, created_at, updated_at) \
         VALUES (?1, ?2, ?3, 'github', 'https://github.com', '', '', 'token', 'github_app', ?4, ?4)",
    )
    .bind(provider_id.clone())
    .bind(organization_id)
    .bind(name)
    .bind(timestamp)
    .execute(&conn)
    .await
    .map_err(|e| {
        if e.to_string().to_ascii_lowercase().contains("unique") { AppError::AlreadyExists } else { AppError::DatabaseError(e.to_string()) }
    })?;
    sqlx::query(
        "INSERT INTO github_apps (git_provider_id, owner_login, owner_type, status, created_at, updated_at) VALUES (?1, ?2, ?3, 'pending_manifest', ?4, ?4)",
    )
    .bind(provider_id.clone())
    .bind(organization.unwrap_or_default())
    .bind(if organization.is_some() { "organization" } else { "user" })
    .bind(timestamp)
    .execute(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;

    let token = put_state(
        state,
        &StateEntry {
            organization_id: organization_id.into(),
            user_id: user_id.into(),
            provider_id: provider_id.clone(),
            purpose: "manifest".into(),
            created_at: timestamp,
        },
    )?;

    // contents+metadata read is everything a clone needs. Wider permissions would force every
    // user to re-approve the app later, so they stay out until a feature actually needs them.
    // setup_url is what GitHub actually redirects to after install/update (with installation_id,
    // setup_action, and state as query params) — callback_urls only applies to the separate
    // "request user authorization (OAuth) during installation" flow, which this app doesn't use.
    let manifest = serde_json::json!({
        "name": format!("litepod-{}", &provider_id[..8.min(provider_id.len())]),
        "url": public_url,
        "hook_attributes": { "url": webhook_url(&public_url, &provider_id), "active": true },
        "redirect_url": format!("{public_url}/api/v1/github-app/manifest/callback"),
        "setup_url": format!("{public_url}/api/v1/github-app/install/callback"),
        "setup_on_update": true,
        "public": false,
        "default_permissions": { "contents": "read", "metadata": "read" },
        "default_events": ["push"],
    });

    let post_url = match organization {
        Some(org) if !org.trim().is_empty() => format!(
            "https://github.com/organizations/{}/settings/apps/new?state={token}",
            org.trim()
        ),
        _ => format!("https://github.com/settings/apps/new?state={token}"),
    };

    Ok(ManifestForm {
        post_url,
        manifest: serde_json::to_string(&manifest)
            .map_err(|e| AppError::InternalError(e.to_string()))?,
        state: token,
        provider_id,
    })
}

/// Exchanges the single-use manifest code for the app's credentials. The code itself is the
/// credential here — the request carries no auth header.
pub async fn complete_manifest(
    state: &SharedState,
    code: &str,
    state_token: &str,
) -> Result<String, AppError> {
    let entry = take_state(state, state_token, "manifest")?;
    let response = client()?
        .post(format!(
            "https://api.github.com/app-manifests/{code}/conversions"
        ))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| AppError::ValidationError(format!("GitHub could not be reached: {e}")))?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(AppError::ValidationError(format!(
            "GitHub rejected the app manifest ({status})"
        )));
    }
    let payload: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| AppError::InternalError(format!("Unexpected GitHub response: {e}")))?;

    let key = security::get_encryption_key();
    let encrypt = |value: &str| -> Result<String, AppError> {
        if value.is_empty() {
            return Ok(String::new());
        }
        security::encrypt_data(value, &key).map_err(AppError::InternalError)
    };
    let text = |field: &str| {
        payload
            .get(field)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string()
    };

    let slug = text("slug");
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE github_apps SET app_id = ?1, slug = ?2, owner_login = ?3, owner_type = ?4, html_url = ?5, private_key_encrypted = ?6, \
         webhook_secret_encrypted = ?7, client_id = ?8, client_secret_encrypted = ?9, status = 'pending_install', updated_at = ?10 \
         WHERE git_provider_id = ?11",
    )
    .bind(payload.get("id").and_then(|v| v.as_i64()))
    .bind(slug.clone())
    .bind(payload.get("owner").and_then(|v| v.get("login")).and_then(|v| v.as_str()).unwrap_or_default())
    .bind(match payload.get("owner").and_then(|v| v.get("type")).and_then(|v| v.as_str()) { Some("Organization") => "organization", _ => "user" })
    .bind(text("html_url"))
    .bind(encrypt(&text("pem"))?)
    .bind(encrypt(&text("webhook_secret"))?)
    .bind(text("client_id"))
    .bind(encrypt(&text("client_secret"))?)
    .bind(now_ms())
    .bind(entry.provider_id.clone())
    .execute(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    sqlx::query("UPDATE git_providers SET username = ?1, updated_at = ?2 WHERE id = ?3")
        .bind(slug)
        .bind(now_ms())
        .bind(entry.provider_id.clone())
        .execute(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;

    Ok(entry.provider_id)
}

/// Where the user goes to pick which account or org the app may access.
pub async fn install_url(
    state: &SharedState,
    organization_id: &str,
    user_id: &str,
    provider_id: &str,
) -> Result<String, AppError> {
    let app = get(state, provider_id, organization_id).await?;
    let slug = app
        .slug
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::ValidationError("Finish creating the GitHub App first".into()))?;
    let token = put_state(
        state,
        &StateEntry {
            organization_id: organization_id.into(),
            user_id: user_id.into(),
            provider_id: provider_id.into(),
            purpose: "install".into(),
            created_at: now_ms(),
        },
    )?;
    Ok(format!(
        "{}/apps/{slug}/installations/new?state={token}",
        app.web_base.trim_end_matches('/')
    ))
}

pub async fn complete_install(
    state: &SharedState,
    state_token: &str,
    installation_id: i64,
) -> Result<String, AppError> {
    let entry = take_state(state, state_token, "install")?;
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE github_apps SET installation_id = ?1, status = 'ready', updated_at = ?2 WHERE git_provider_id = ?3",
    )
    .bind(installation_id)
    .bind(now_ms())
    .bind(entry.provider_id.clone())
    .execute(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok(entry.provider_id)
}

/// RS256 JWT proving app identity. Back-dated 60s for clock skew; GitHub caps the lifetime at 10 min.
fn app_jwt(app_id: i64, pem: &str) -> Result<String, AppError> {
    #[derive(serde::Serialize)]
    struct Claims {
        iat: i64,
        exp: i64,
        iss: String,
    }
    let now = Utc::now().timestamp();
    let key = jsonwebtoken::EncodingKey::from_rsa_pem(pem.as_bytes())
        .map_err(|e| AppError::InternalError(format!("Invalid GitHub App private key: {e}")))?;
    jsonwebtoken::encode(
        &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256),
        &Claims {
            iat: now - 60,
            exp: now + JWT_TTL_SECONDS,
            iss: app_id.to_string(),
        },
        &key,
    )
    .map_err(|e| AppError::InternalError(format!("Could not sign GitHub App token: {e}")))
}

/// Cached installation token, minting a new one when the stored one is missing or near expiry.
pub async fn installation_token(
    state: &SharedState,
    provider_id: &str,
) -> Result<String, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(
        "SELECT app_id, installation_id, private_key_encrypted, installation_token_encrypted, installation_token_expires_at, api_base, status \
         FROM github_apps WHERE git_provider_id = ?1",
    )
    .bind(provider_id)
    .fetch_optional(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?
    .ok_or(AppError::ProviderNotFound)?;
    let db = |e: sqlx::Error| AppError::DatabaseError(e.to_string());
    let app_id: Option<i64> = row.try_get(0).map_err(db)?;
    let installation_id: Option<i64> = row.try_get(1).map_err(db)?;
    let private_key_encrypted: String = row.try_get(2).map_err(db)?;
    let cached: Option<String> = row.try_get(3).map_err(db)?;
    let expires_at: Option<i64> = row.try_get(4).map_err(db)?;
    let api_base: String = row.try_get(5).map_err(db)?;
    let status: String = row.try_get(6).map_err(db)?;

    if status != "ready" {
        return Err(AppError::ValidationError(
            "Finish installing the GitHub App before deploying from it".into(),
        ));
    }
    let key = security::get_encryption_key();
    if let (Some(cached), Some(expires_at)) =
        (cached.as_deref().filter(|t| !t.is_empty()), expires_at)
        && expires_at - now_ms() > TOKEN_SKEW_MS
        && let Ok(token) = security::decrypt_data(cached, &key)
    {
        return Ok(token);
    }

    let app_id = app_id.ok_or_else(|| {
        AppError::ValidationError("GitHub App is missing its app id — reconnect it".into())
    })?;
    let installation_id = installation_id
        .ok_or_else(|| AppError::ValidationError("GitHub App is not installed yet".into()))?;
    let pem =
        security::decrypt_data(&private_key_encrypted, &key).map_err(AppError::InternalError)?;
    let jwt = app_jwt(app_id, &pem)?;

    let response = client()?
        .post(format!(
            "{}/app/installations/{installation_id}/access_tokens",
            api_base.trim_end_matches('/')
        ))
        .header("Accept", "application/vnd.github+json")
        .bearer_auth(&jwt)
        .send()
        .await
        .map_err(|e| AppError::ValidationError(format!("GitHub could not be reached: {e}")))?;
    let status_code = response.status();
    let body = response.text().await.unwrap_or_default();
    if status_code == reqwest::StatusCode::NOT_FOUND {
        // The installation was removed on GitHub's side; without this the provider would keep
        // failing with an opaque 404 forever.
        let _ = sqlx::query("UPDATE github_apps SET status = 'pending_install', installation_id = NULL, updated_at = ?1 WHERE git_provider_id = ?2")
            .bind(now_ms())
            .bind(provider_id)
            .execute(&conn)
            .await;
        return Err(AppError::ValidationError(
            "The GitHub App installation no longer exists — install it again".into(),
        ));
    }
    if !status_code.is_success() {
        return Err(AppError::ValidationError(format!(
            "GitHub rejected the installation token request ({status_code})"
        )));
    }
    let payload: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| AppError::InternalError(format!("Unexpected GitHub response: {e}")))?;
    let token = payload
        .get("token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::InternalError("GitHub returned no installation token".into()))?
        .to_string();
    let expires_ms = payload
        .get("expires_at")
        .and_then(|v| v.as_str())
        .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
        .map(|v| v.timestamp_millis())
        .unwrap_or_else(|| now_ms() + 3_600_000);

    let encrypted = security::encrypt_data(&token, &key).map_err(AppError::InternalError)?;
    sqlx::query(
        "UPDATE github_apps SET installation_token_encrypted = ?1, installation_token_expires_at = ?2, updated_at = ?3 WHERE git_provider_id = ?4",
    )
    .bind(encrypted)
    .bind(expires_ms)
    .bind(now_ms())
    .bind(provider_id)
    .execute(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok(token)
}

/// Repositories the installation can see. Note the response is an envelope, not a bare array,
/// so the credential-provider paging helper cannot be reused here.
pub async fn installation_repositories(
    state: &SharedState,
    provider_id: &str,
    organization_id: &str,
) -> Result<Vec<GitRepositoryDto>, AppError> {
    let app = get(state, provider_id, organization_id).await?;
    let token = installation_token(state, provider_id).await?;
    let client = client()?;
    let mut repositories = Vec::new();

    for page in 1..=10 {
        let response = client
            .get(format!(
                "{}/installation/repositories?per_page=100&page={page}",
                app.api_base.trim_end_matches('/')
            ))
            .header("Accept", "application/vnd.github+json")
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| {
                AppError::ValidationError(super::sanitize_git_error(&e.to_string(), &token))
            })?;
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(AppError::ValidationError(format!(
                "GitHub returned {status} listing repositories"
            )));
        }
        let payload: serde_json::Value = serde_json::from_str(&body)
            .map_err(|e| AppError::InternalError(format!("Unexpected GitHub response: {e}")))?;
        let items = payload
            .get("repositories")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let count = items.len();
        repositories.extend(super::map_repositories("github", &items));
        if count < 100 {
            break;
        }
    }

    repositories.sort_by(|a, b| a.full_name.to_lowercase().cmp(&b.full_name.to_lowercase()));
    Ok(repositories)
}

/// Decrypted webhook secret for signature verification. Never leaves the backend.
pub async fn webhook_secret(state: &SharedState, provider_id: &str) -> Result<String, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row =
        sqlx::query("SELECT webhook_secret_encrypted FROM github_apps WHERE git_provider_id = ?1")
            .bind(provider_id)
            .fetch_optional(&conn)
            .await
            .map_err(|e| AppError::DatabaseError(e.to_string()))?
            .ok_or(AppError::WebhookVerificationFailed)?;
    let encrypted: String = row
        .try_get(0)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    if encrypted.is_empty() {
        return Err(AppError::WebhookVerificationFailed);
    }
    security::decrypt_data(&encrypted, &security::get_encryption_key())
        .map_err(AppError::InternalError)
}
