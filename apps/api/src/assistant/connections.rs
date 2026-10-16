//! An organization's model connections: the keys, the endpoints, and the
//! catalog of models each one actually serves.
//!
//! A connection is a key plus a base URL. The key is encrypted with the same
//! AES-256-GCM helper the git providers use and never leaves this module in the
//! clear; what the UI gets back is `api_key_hint`, a redacted string that
//! identifies a key without being one.
//!
//! Two rules shape the rest:
//!
//! - **A save probes first.** Creating or updating a connection fetches the
//!   provider's model list and only writes when that succeeds. This is where the
//!   old boot-time model check went: a bad key now fails in the dialog that set
//!   it, not three screens later inside somebody's conversation.
//! - **Every model is listed.** A model this build cannot run is still returned,
//!   carrying the reason in `blocked`. Hiding it would leave the user hunting
//!   for something that is right there in the provider's own list.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

use crate::{
    assistant::config::{
        ANTHROPIC_VERSION, AssistantConfig, EndpointFamily, KeyPolicy, PROVIDERS, Protocol,
        Provider, endpoint_family_for, provider,
    },
    assistant::{endpoint, redaction},
    dtos::response::AppError,
    state::state::SharedState,
    utils::security,
};

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

/// One model, as the picker shows it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, utoipa::ToSchema)]
pub struct ModelEntry {
    pub id: String,
    /// The provider's own display name when it offers one, else the id.
    pub label: String,
    pub context: Option<i64>,
    pub tools: bool,
    /// `null` when usable. `"endpoint"` when the provider serves it through a
    /// route this build does not speak; `"no-tools"` when it cannot call tools,
    /// which the assistant needs for every useful turn.
    pub blocked: Option<String>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct ConnectionDto {
    pub id: String,
    pub organization_id: String,
    pub name: String,
    pub provider: String,
    pub protocol: String,
    pub base_url: String,
    pub has_key: bool,
    /// A redacted string like `sk-or-v1 ···· 91c7`. Never the key.
    pub api_key_hint: Option<String>,
    pub is_default: bool,
    /// `unverified` | `ok` | `failed`.
    pub status: String,
    /// The provider's own words when a probe failed.
    pub status_detail: Option<String>,
    pub models: Vec<ModelEntry>,
    pub models_fetched_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateConnectionRequest {
    pub name: String,
    /// One of `config::PROVIDERS`.
    pub provider: String,
    /// Overrides the provider's default. Required for the custom-URL kinds.
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    /// Makes this the organization's default. The first connection always is.
    pub is_default: Option<bool>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateConnectionRequest {
    pub name: Option<String>,
    pub base_url: Option<String>,
    /// Replaces the stored key. Omit to keep it.
    pub api_key: Option<String>,
    pub is_default: Option<bool>,
}

const SELECT_COLUMNS: &str = "id, organization_id, name, provider, protocol, base_url, \
     api_key_encrypted, api_key_hint, is_default, status, status_detail, models_json, \
     models_fetched_at, created_at, updated_at";

fn row_to_dto(row: &sqlx::sqlite::SqliteRow) -> Result<ConnectionDto, AppError> {
    let text = |index: usize| -> Result<String, AppError> {
        row.try_get(index)
            .map_err(|e| AppError::DatabaseError(e.to_string()))
    };
    let maybe_text = |index: usize| -> Result<Option<String>, AppError> {
        row.try_get(index)
            .map_err(|e| AppError::DatabaseError(e.to_string()))
    };
    let int = |index: usize| -> Result<i64, AppError> {
        row.try_get(index)
            .map_err(|e| AppError::DatabaseError(e.to_string()))
    };
    let maybe_int = |index: usize| -> Result<Option<i64>, AppError> {
        row.try_get(index)
            .map_err(|e| AppError::DatabaseError(e.to_string()))
    };

    Ok(ConnectionDto {
        id: text(0)?,
        organization_id: text(1)?,
        name: text(2)?,
        provider: text(3)?,
        protocol: text(4)?,
        base_url: text(5)?,
        has_key: maybe_text(6)?.is_some_and(|key| !key.is_empty()),
        api_key_hint: maybe_text(7)?,
        is_default: int(8)? != 0,
        status: text(9)?,
        status_detail: maybe_text(10)?,
        // A catalog that will not parse is an empty one: the connection still
        // lists and can be re-probed, which is what the user needs to fix it.
        models: serde_json::from_str(&text(11)?).unwrap_or_default(),
        models_fetched_at: maybe_int(12)?,
        created_at: int(13)?,
        updated_at: int(14)?,
    })
}

fn map_write_error(error: sqlx::Error) -> AppError {
    let message = error.to_string();
    if message.to_ascii_lowercase().contains("unique") {
        AppError::AlreadyExists
    } else {
        AppError::DatabaseError(message)
    }
}

/// Identifies a key without holding one: enough of the prefix to tell an
/// OpenRouter key from an Anthropic one, enough of the tail to tell two apart.
fn hint(key: &str) -> String {
    if key.chars().count() < 12 {
        return "····".into();
    }
    let head: String = key.chars().take(6).collect();
    let tail: String = key.chars().skip(key.chars().count() - 4).collect();
    format!("{head} ···· {tail}")
}

fn clean_url(url: &str) -> String {
    url.trim().trim_end_matches('/').to_string()
}

/// Refuses a provider kind this edition does not offer. The UI only lists the
/// offered kinds, but the API is the boundary: a hand-written request naming
/// `ollama` on cloud must fail here, not reach the network.
fn require_available(entry: &Provider, is_cloud: bool) -> Result<(), AppError> {
    if entry.available(is_cloud) {
        return Ok(());
    }
    Err(AppError::ValidationError(format!(
        "{} connects to a model server on your own network, which Litepod Cloud cannot reach. \
         Use a hosted provider or an OpenAI-compatible endpoint with a public HTTPS URL.",
        entry.name
    )))
}

/// Checks the parts a row is built from, whichever request supplied them.
fn validate(name: &str, base_url: &str, key: Option<&str>, kind: &str) -> Result<(), AppError> {
    let Some(entry) = provider(kind) else {
        let known: Vec<&str> = PROVIDERS.iter().map(|p| p.id).collect();
        return Err(AppError::ValidationError(format!(
            "`{kind}` is not a provider. Pick one of: {}.",
            known.join(", ")
        )));
    };
    if name.is_empty() || name.chars().count() > 60 {
        return Err(AppError::ValidationError(
            "Name is required and must be 60 characters or fewer".into(),
        ));
    }
    if base_url.is_empty() {
        return Err(AppError::ValidationError(
            "A base URL is required for this provider".into(),
        ));
    }
    if !base_url.starts_with("http://") && !base_url.starts_with("https://") {
        return Err(AppError::ValidationError(
            "The base URL must start with http:// or https://".into(),
        ));
    }
    if entry.key == KeyPolicy::Required && key.is_none_or(str::is_empty) {
        return Err(AppError::ValidationError(format!(
            "{} needs an API key",
            entry.name
        )));
    }
    Ok(())
}

// ── Catalog ──────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct ModelList {
    data: Vec<RawModel>,
}

/// The union of what the providers return. Everything past `id` is optional
/// because only OpenRouter and Anthropic fill any of it in.
#[derive(Deserialize)]
struct RawModel {
    id: String,
    display_name: Option<String>,
    name: Option<String>,
    context_length: Option<i64>,
    /// OpenRouter lists `"tools"` here when the model can call them.
    supported_parameters: Option<Vec<String>>,
}

/// Fetches and normalises a connection's model list.
///
/// Both protocols expose `GET {base}/models`; only the auth header differs.
/// The error string is the provider's own, because a 401 and a closed port are
/// fixed in completely different ways and a generic "failed" says neither.
pub async fn fetch_models(
    config: &AssistantConfig,
    opencode: bool,
) -> Result<Vec<ModelEntry>, String> {
    // Checked here rather than at save time, so a name that resolved publicly
    // when the connection was stored is checked again on every use.
    endpoint::allowed(&config.base_url, config.allow_local).await?;

    // No redirects: `allowed` vetted this host only, and a 302 from a public
    // host to `http://169.254.169.254/` would otherwise be followed unchecked.
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| format!("could not build HTTP client: {error}"))?;
    let mut request = client
        .get(config.models_url())
        .timeout(std::time::Duration::from_secs(20));
    request = match (config.protocol, &config.api_key) {
        (_, None) => request,
        (Protocol::OpenAi, Some(key)) => request.bearer_auth(key),
        (Protocol::Anthropic, Some(key)) => request
            .header("x-api-key", key)
            .header("anthropic-version", ANTHROPIC_VERSION),
    };

    let response = request
        .send()
        .await
        .map_err(|error| format!("{} is unreachable: {error}", config.models_url()))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|error| format!("no body from {}: {error}", config.models_url()))?;
    if !status.is_success() {
        // The body can echo the request, key included, and this string is stored
        // on the row and read by anyone with view access. Redact before
        // truncating: cutting first can leave half a key in the excerpt.
        return Err(format!("{status}: {}", excerpt(&text, config)));
    }

    let parsed: ModelList = serde_json::from_str(&text).map_err(|error| {
        format!(
            "unexpected model list from {}: {error}",
            config.models_url()
        )
    })?;

    let mut models: Vec<ModelEntry> = parsed
        .data
        .into_iter()
        .map(|raw| normalise(raw, config.protocol, opencode))
        .collect();
    models.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(models)
}

/// The first 200 characters of a provider's response, with this connection's
/// own key taken out of them.
fn excerpt(text: &str, config: &AssistantConfig) -> String {
    let scrubbed = match &config.api_key {
        Some(key) => redaction::redact_values(text, &[("API key".into(), key.clone())]),
        None => text.to_owned(),
    };
    scrubbed.chars().take(200).collect()
}

fn normalise(raw: RawModel, _protocol: Protocol, opencode: bool) -> ModelEntry {
    // Only OpenRouter says anything about tools. Where nothing is said, assume
    // yes: a model that turns out not to support them fails loudly on the first
    // call, which is better than hiding half a provider's catalog on a guess.
    let tools = match &raw.supported_parameters {
        Some(parameters) => parameters.iter().any(|entry| entry == "tools"),
        None => true,
    };
    // OpenCode routes some of its own model ids through `/responses` or paths
    // this build does not speak. The Rig provider adapter makes the same call,
    // so a model this build *can* run (`Messages` or
    // `ChatCompletions`) is left unblocked here; the rest are listed-but-blocked
    // so the user can see why their chosen id will not turn.
    let unrunnable = opencode
        && matches!(
            endpoint_family_for(true, &raw.id),
            EndpointFamily::Responses | EndpointFamily::Unsupported
        );
    let blocked = if unrunnable {
        Some("endpoint".to_string())
    } else if !tools {
        Some("no-tools".to_string())
    } else {
        None
    };

    ModelEntry {
        label: raw
            .display_name
            .or(raw.name)
            .unwrap_or_else(|| raw.id.clone()),
        id: raw.id,
        context: raw.context_length,
        tools,
        blocked,
    }
}

// ── Reads ────────────────────────────────────────────────────────────────────

pub async fn list(
    state: &SharedState,
    organization_id: &str,
) -> Result<Vec<ConnectionDto>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(&format!(
        "SELECT {SELECT_COLUMNS} FROM assistant_connections \
         WHERE organization_id = ?1 ORDER BY is_default DESC, name"
    ))
    .bind(organization_id)
    .fetch_all(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;

    rows.iter().map(row_to_dto).collect()
}

/// One connection. The handler reads it before a write so the audit entry can
/// record what the value was as well as what it became.
pub async fn get(
    state: &SharedState,
    organization_id: &str,
    connection_id: &str,
) -> Result<ConnectionDto, AppError> {
    row_to_dto(&row(state, organization_id, connection_id).await?)
}

/// One row, scoped to the organization so a stray id cannot read across a boundary.
async fn row(
    state: &SharedState,
    organization_id: &str,
    connection_id: &str,
) -> Result<sqlx::sqlite::SqliteRow, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(&format!(
        "SELECT {SELECT_COLUMNS} FROM assistant_connections WHERE id = ?1 AND organization_id = ?2"
    ))
    .bind(connection_id)
    .bind(organization_id)
    .fetch_optional(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?
    .ok_or(AppError::ProviderNotFound)
}

/// The decrypted key for a row. Kept private: nothing outside this module has a
/// reason to hold one.
fn decrypt_key(row: &sqlx::sqlite::SqliteRow) -> Result<Option<String>, AppError> {
    let stored: Option<String> = row
        .try_get(6)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    match stored.filter(|key| !key.is_empty()) {
        None => Ok(None),
        Some(encrypted) => {
            let key = security::get_encryption_key();
            security::decrypt_data(&encrypted, &key)
                .map(Some)
                .map_err(AppError::InternalError)
        }
    }
}

/// Builds a callable config from a row plus the model to use.
fn config_from(
    row: &sqlx::sqlite::SqliteRow,
    model: String,
    is_cloud: bool,
) -> Result<AssistantConfig, AppError> {
    let kind: String = row
        .try_get(3)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let base_url: String = row
        .try_get(5)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let protocol: String = row
        .try_get(4)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;

    Ok(AssistantConfig {
        api_key: decrypt_key(row)?,
        base_url,
        model,
        protocol: Protocol::parse(&protocol)
            .ok_or_else(|| AppError::InternalError(format!("unknown protocol `{protocol}`")))?,
        opencode: provider(&kind).is_some_and(|entry| entry.opencode),
        allow_local: provider(&kind).is_some_and(|entry| entry.allows_local(is_cloud)),
    })
}

/// The connection and model one turn should use.
///
/// A conversation pins what it started with, so moving the organization's
/// default never changes the model halfway through an open thread. Only a fresh
/// conversation falls back to the default.
pub async fn resolve(
    state: &SharedState,
    organization_id: &str,
    connection_id: Option<&str>,
    model: Option<&str>,
) -> Result<AssistantConfig, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let by_id = format!(
        "SELECT {SELECT_COLUMNS} FROM assistant_connections WHERE id = ?1 AND organization_id = ?2"
    );
    let by_default = format!(
        "SELECT {SELECT_COLUMNS} FROM assistant_connections \
         WHERE organization_id = ?1 AND is_default = 1"
    );
    let found = match connection_id {
        Some(id) => sqlx::query(&by_id).bind(id).bind(organization_id),
        None => sqlx::query(&by_default).bind(organization_id),
    }
    .fetch_optional(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;

    let found = found.ok_or_else(|| {
        AppError::ValidationError(
            "No model connection yet. An admin can add one in organization settings.".into(),
        )
    })?;

    let chosen = match model.map(str::trim).filter(|model| !model.is_empty()) {
        Some(model) => model.to_string(),
        // No model named: the first usable one in the connection's own catalog.
        None => first_usable(&found)?,
    };
    config_from(&found, chosen, state.is_cloud)
}

fn first_usable(row: &sqlx::sqlite::SqliteRow) -> Result<String, AppError> {
    let raw: String = row
        .try_get(11)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let models: Vec<ModelEntry> = serde_json::from_str(&raw).unwrap_or_default();
    models
        .into_iter()
        .find(|model| model.blocked.is_none())
        .map(|model| model.id)
        .ok_or_else(|| {
            AppError::ValidationError(
                "That connection has no usable model. Reload its models in organization settings."
                    .into(),
            )
        })
}

// ── Writes ───────────────────────────────────────────────────────────────────

async fn count(state: &SharedState, organization_id: &str) -> Result<i64, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("SELECT COUNT(*) FROM assistant_connections WHERE organization_id = ?1")
        .bind(organization_id)
        .fetch_one(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?
        .try_get(0)
        .map_err(|e| AppError::DatabaseError(e.to_string()))
}

/// Clears the current default so the partial unique index does not reject the
/// row about to claim it.
async fn clear_default(state: &SharedState, organization_id: &str) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("UPDATE assistant_connections SET is_default = 0 WHERE organization_id = ?1")
        .bind(organization_id)
        .execute(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok(())
}

pub async fn create(
    state: &SharedState,
    organization_id: &str,
    request: CreateConnectionRequest,
) -> Result<ConnectionDto, AppError> {
    let name = request.name.trim().to_string();
    let kind = request.provider.trim().to_string();
    let entry = provider(&kind)
        .ok_or_else(|| AppError::ValidationError(format!("`{kind}` is not a provider")))?;
    require_available(entry, state.is_cloud)?;
    let base_url = request
        .base_url
        .as_deref()
        .map(clean_url)
        .filter(|url| !url.is_empty())
        .unwrap_or_else(|| entry.base_url.to_string());
    let api_key = request
        .api_key
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty());

    validate(&name, &base_url, api_key.as_deref(), &kind)?;

    // Probe before writing. A key that cannot list models cannot run a turn
    // either, so failing here keeps a broken connection out of the table.
    let config = AssistantConfig {
        api_key: api_key.clone(),
        base_url: base_url.clone(),
        model: String::new(),
        protocol: entry.protocol,
        opencode: entry.opencode,
        allow_local: entry.allows_local(state.is_cloud),
    };
    let models = fetch_models(&config, entry.opencode)
        .await
        .map_err(AppError::ValidationError)?;

    // The first connection is the default whether or not it was asked for:
    // otherwise the assistant stays dark until someone notices a second switch.
    let make_default =
        request.is_default.unwrap_or(false) || count(state, organization_id).await? == 0;
    if make_default {
        clear_default(state, organization_id).await?;
    }

    let encrypted = match &api_key {
        Some(key) => {
            let secret = security::get_encryption_key();
            Some(security::encrypt_data(key, &secret).map_err(AppError::InternalError)?)
        }
        None => None,
    };

    let id = Uuid::now_v7().to_string();
    let timestamp = now_ms();
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "INSERT INTO assistant_connections (id, organization_id, name, provider, protocol, \
         base_url, api_key_encrypted, api_key_hint, is_default, status, status_detail, \
         models_json, models_fetched_at, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'ok', NULL, ?10, ?11, ?12, ?12)",
    )
    .bind(&id)
    .bind(organization_id)
    .bind(&name)
    .bind(&kind)
    .bind(entry.protocol.as_str())
    .bind(&base_url)
    .bind(&encrypted)
    .bind(api_key.as_deref().map(hint))
    .bind(i64::from(make_default))
    .bind(serde_json::to_string(&models).unwrap_or_else(|_| "[]".into()))
    .bind(timestamp)
    .bind(timestamp)
    .execute(&conn)
    .await
    .map_err(map_write_error)?;

    row_to_dto(&row(state, organization_id, &id).await?)
}

pub async fn update(
    state: &SharedState,
    organization_id: &str,
    connection_id: &str,
    request: UpdateConnectionRequest,
) -> Result<ConnectionDto, AppError> {
    let existing = row(state, organization_id, connection_id).await?;
    let current = row_to_dto(&existing)?;
    let entry = provider(&current.provider).ok_or_else(|| {
        AppError::InternalError(format!("unknown provider `{}`", current.provider))
    })?;
    require_available(entry, state.is_cloud)?;

    let name = request
        .name
        .as_deref()
        .map(|name| name.trim().to_string())
        .unwrap_or(current.name);
    let previous_url = current.base_url.clone();
    let base_url = request
        .base_url
        .as_deref()
        .map(clean_url)
        .filter(|url| !url.is_empty())
        .unwrap_or(current.base_url);
    // An omitted key keeps the stored one; an empty string clears it.
    let (api_key, key_changed) = match request.api_key.as_deref().map(str::trim) {
        None => (decrypt_key(&existing)?, false),
        Some("") => (None, true),
        Some(key) => (Some(key.to_string()), true),
    };

    // Moving a connection to a new host must be done by someone holding the
    // key, not merely by someone who can edit the row. Without this, a single
    // PATCH naming only a base URL would post the organization's stored key to
    // any host the caller chose, and the caller would never have to know it.
    if base_url != previous_url && !key_changed && current.has_key {
        return Err(AppError::ValidationError(
            "Enter the API key again to move this connection to a different base URL.".into(),
        ));
    }

    validate(&name, &base_url, api_key.as_deref(), &current.provider)?;

    let config = AssistantConfig {
        api_key: api_key.clone(),
        base_url: base_url.clone(),
        model: String::new(),
        protocol: entry.protocol,
        opencode: entry.opencode,
        allow_local: entry.allows_local(state.is_cloud),
    };
    let models = fetch_models(&config, entry.opencode)
        .await
        .map_err(AppError::ValidationError)?;

    if request.is_default == Some(true) {
        clear_default(state, organization_id).await?;
    }
    let is_default = request.is_default.unwrap_or(current.is_default);

    let encrypted = match (&api_key, key_changed) {
        (Some(key), true) => {
            let secret = security::get_encryption_key();
            Some(security::encrypt_data(key, &secret).map_err(AppError::InternalError)?)
        }
        (None, true) => None,
        // Unchanged: re-bind the stored ciphertext rather than re-encrypting.
        _ => existing
            .try_get(6)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
    };

    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE assistant_connections SET name = ?1, base_url = ?2, api_key_encrypted = ?3, \
         api_key_hint = ?4, is_default = ?5, status = 'ok', status_detail = NULL, \
         models_json = ?6, models_fetched_at = ?7, updated_at = ?7 \
         WHERE id = ?8 AND organization_id = ?9",
    )
    .bind(&name)
    .bind(&base_url)
    .bind(&encrypted)
    .bind(api_key.as_deref().map(hint))
    .bind(i64::from(is_default))
    .bind(serde_json::to_string(&models).unwrap_or_else(|_| "[]".into()))
    .bind(now_ms())
    .bind(connection_id)
    .bind(organization_id)
    .execute(&conn)
    .await
    .map_err(map_write_error)?;

    row_to_dto(&row(state, organization_id, connection_id).await?)
}

/// Re-probes an existing connection and records what came back.
///
/// Unlike a save, a failure is stored rather than refused: the row already
/// exists, and the list is more useful showing why it broke than pretending it
/// is still fine.
pub async fn refresh(
    state: &SharedState,
    organization_id: &str,
    connection_id: &str,
) -> Result<ConnectionDto, AppError> {
    let existing = row(state, organization_id, connection_id).await?;
    let current = row_to_dto(&existing)?;
    let opencode = provider(&current.provider).is_some_and(|entry| entry.opencode);
    let config = config_from(&existing, String::new(), state.is_cloud)?;

    let conn = crate::git::repositories::db::get_conn(state).await;
    match fetch_models(&config, opencode).await {
        Ok(models) => {
            sqlx::query(
                "UPDATE assistant_connections SET status = 'ok', status_detail = NULL, \
                 models_json = ?1, models_fetched_at = ?2, updated_at = ?2 WHERE id = ?3",
            )
            .bind(serde_json::to_string(&models).unwrap_or_else(|_| "[]".into()))
            .bind(now_ms())
            .bind(connection_id)
            .execute(&conn)
            .await
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        }
        Err(detail) => {
            sqlx::query(
                "UPDATE assistant_connections SET status = 'failed', status_detail = ?1, \
                 updated_at = ?2 WHERE id = ?3",
            )
            .bind(detail)
            .bind(now_ms())
            .bind(connection_id)
            .execute(&conn)
            .await
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        }
    }

    row_to_dto(&row(state, organization_id, connection_id).await?)
}

pub async fn delete(
    state: &SharedState,
    organization_id: &str,
    connection_id: &str,
) -> Result<ConnectionDto, AppError> {
    let removed = row_to_dto(&row(state, organization_id, connection_id).await?)?;
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("DELETE FROM assistant_connections WHERE id = ?1 AND organization_id = ?2")
        .bind(connection_id)
        .bind(organization_id)
        .execute(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;

    // Deleting the default would otherwise leave the organization with working
    // connections and no assistant.
    if removed.is_default {
        sqlx::query(
            "UPDATE assistant_connections SET is_default = 1 WHERE id = ( \
                SELECT id FROM assistant_connections WHERE organization_id = ?1 \
                ORDER BY status = 'ok' DESC, created_at LIMIT 1)",
        )
        .bind(organization_id)
        .execute(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    }

    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hint_identifies_a_key_without_being_one() {
        let key = "sk-or-v1-0000000000000091c7";
        let shown = hint(key);
        assert_eq!(shown, "sk-or- ···· 91c7");
        assert!(!key.contains(&shown));
        // Too short to redact usefully: show nothing at all.
        assert_eq!(hint("sk-abc"), "····");
    }

    #[test]
    fn a_provider_that_needs_a_key_refuses_to_save_without_one() {
        assert!(validate("OpenAI", "https://api.openai.com/v1", None, "openai").is_err());
        assert!(
            validate(
                "OpenAI",
                "https://api.openai.com/v1",
                Some("sk-x"),
                "openai"
            )
            .is_ok()
        );
        // Local providers authenticate nothing.
        assert!(validate("Ollama", "http://127.0.0.1:11434/v1", None, "ollama").is_ok());
        // A URL is always required, and always has to be one.
        assert!(validate("Ollama", "", None, "ollama").is_err());
        assert!(validate("Ollama", "127.0.0.1:11434", None, "ollama").is_err());
        assert!(validate("X", "https://x/v1", Some("k"), "not_a_provider").is_err());
    }

    #[test]
    fn a_model_the_build_cannot_run_is_listed_with_the_reason() {
        // /responses is not spoken by this build — the user sees the id with
        // a reason attached rather than a 404 on first turn.
        let raw = RawModel {
            id: "opencode-go/grok-4.6".into(),
            display_name: None,
            name: None,
            context_length: None,
            supported_parameters: None,
        };
        let entry = normalise(raw, Protocol::OpenAi, true);
        assert_eq!(entry.blocked.as_deref(), Some("endpoint"));
        assert_eq!(entry.label, "opencode-go/grok-4.6");

        // A model that needs /messages is runnable: the dispatcher routes
        // through the Anthropic wire shape, so it must not be marked blocked.
        let raw = RawModel {
            id: "opencode-go/minimax-m3".into(),
            display_name: None,
            name: None,
            context_length: None,
            supported_parameters: None,
        };
        let entry = normalise(raw, Protocol::OpenAi, true);
        assert_eq!(entry.blocked, None);
        assert_eq!(entry.label, "opencode-go/minimax-m3");

        // The same id on an Anthropic connection is simply valid.
        let raw = RawModel {
            id: "claude-opus-4-5".into(),
            display_name: Some("Claude Opus 4.5".into()),
            name: None,
            context_length: None,
            supported_parameters: None,
        };
        let entry = normalise(raw, Protocol::Anthropic, false);
        assert_eq!(entry.blocked, None);
        assert_eq!(entry.label, "Claude Opus 4.5");
    }

    #[test]
    fn a_model_that_cannot_call_tools_is_blocked_for_saying_so() {
        let raw = RawModel {
            id: "some/embedding".into(),
            display_name: None,
            name: None,
            context_length: Some(8192),
            supported_parameters: Some(vec!["temperature".into()]),
        };
        let entry = normalise(raw, Protocol::OpenAi, false);
        assert!(!entry.tools);
        assert_eq!(entry.blocked.as_deref(), Some("no-tools"));
        assert_eq!(entry.context, Some(8192));
    }

    #[test]
    fn a_trailing_slash_never_doubles_up_in_a_url() {
        assert_eq!(clean_url("  https://x.dev/v1/  "), "https://x.dev/v1");
    }

    #[test]
    fn an_error_excerpt_never_carries_the_key_back() {
        let config = AssistantConfig {
            api_key: Some("sk-live-2f9d41c0b7a3".into()),
            base_url: "https://api.openai.com/v1".into(),
            model: String::new(),
            protocol: Protocol::OpenAi,
            opencode: false,
            allow_local: false,
        };
        // A gateway that echoes the request it rejected, key and all.
        let body = format!(
            "{{\"error\":\"bad auth\",\"sent\":\"Bearer {}\"}}",
            config.api_key.as_deref().unwrap()
        );
        let shown = excerpt(&body, &config);
        assert!(!shown.contains("sk-live-2f9d41c0b7a3"), "{shown}");
        assert!(shown.contains("bad auth"), "{shown}");

        // Redaction runs before truncation, so a key straddling the 200th
        // character cannot survive as a fragment.
        let long = format!("{}{}", "x".repeat(190), config.api_key.as_deref().unwrap());
        assert!(!excerpt(&long, &config).contains("sk-live"));
    }

    #[tokio::test]
    async fn moving_a_connection_to_a_new_host_needs_the_key_again() {
        let (state, path) = crate::testing::test_state().await;
        let conn = crate::git::repositories::db::get_conn(&state).await;
        sqlx::query(
            "INSERT INTO assistant_connections (id, organization_id, name, provider, protocol, \
             base_url, api_key_encrypted, api_key_hint, is_default, status, models_json, \
             created_at, updated_at) \
             VALUES ('c1', 'organization-a', 'OpenAI', 'openai', 'openai', \
             'https://api.openai.com/v1', ?1, 'sk-liv ···· 41c0', 1, 'ok', '[]', 0, 0)",
        )
        .bind(
            security::encrypt_data("sk-live-2f9d41c0b7a3", &security::get_encryption_key())
                .unwrap(),
        )
        .execute(&conn)
        .await
        .unwrap();

        // Only the URL: this is the request that would have posted the stored
        // key to a host the caller named. It must be refused before any of it
        // leaves the machine, so no network is reached and none is needed here.
        let refused = update(
            &state,
            "organization-a",
            "c1",
            UpdateConnectionRequest {
                name: None,
                base_url: Some("https://collector.example/v1".into()),
                api_key: None,
                is_default: None,
            },
        )
        .await
        .expect_err("a rebase without the key must be refused");
        assert!(
            format!("{refused:?}").contains("Enter the API key again"),
            "{refused:?}"
        );

        // The row is untouched.
        let unchanged = get(&state, "organization-a", "c1").await.unwrap();
        assert_eq!(unchanged.base_url, "https://api.openai.com/v1");

        crate::testing::cleanup_database(&path);
    }

    #[tokio::test]
    async fn cloud_refuses_a_local_model_server_before_dialing_it() {
        // `http://litepod-caddy:2019` is the control plane's own unauthenticated
        // Caddy admin API. On self-hosted an Ollama URL like this is the
        // customer's own network; on cloud it is ours and must never be dialed.
        let (state, path) = crate::testing::test_state_for(true).await;
        for kind in ["ollama", "lmstudio"] {
            let refused = create(
                &state,
                "organization-a",
                CreateConnectionRequest {
                    name: "Local".into(),
                    provider: kind.into(),
                    base_url: Some("http://litepod-caddy:2019".into()),
                    api_key: None,
                    is_default: None,
                },
            )
            .await
            .expect_err("cloud must refuse a keyless local provider");
            assert!(
                format!("{refused:?}").contains("Litepod Cloud"),
                "{refused:?}"
            );
        }
        crate::testing::cleanup_database(&path);
    }

    #[tokio::test]
    async fn a_stored_local_connection_on_cloud_resolves_without_local_access() {
        // A row written before the cloud check existed must still be denied the
        // private network when a turn uses it.
        let (state, path) = crate::testing::test_state_for(true).await;
        let conn = crate::git::repositories::db::get_conn(&state).await;
        sqlx::query(
            "INSERT INTO assistant_connections (id, organization_id, name, provider, protocol, \
             base_url, is_default, status, models_json, created_at, updated_at) \
             VALUES ('c1', 'organization-a', 'Ollama', 'ollama', 'openai', \
             'http://127.0.0.1:11434/v1', 1, 'ok', '[]', 0, 0)",
        )
        .execute(&conn)
        .await
        .unwrap();
        let config = resolve(&state, "organization-a", Some("c1"), Some("llama3"))
            .await
            .unwrap();
        assert!(!config.allow_local);
        assert!(
            endpoint::allowed(&config.base_url, config.allow_local)
                .await
                .is_err()
        );
        crate::testing::cleanup_database(&path);
    }
}
