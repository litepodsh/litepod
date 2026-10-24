//! The web search an organization lets its assistant and MCP agents use.
//!
//! There is no search built in. An organization brings its own: a SearXNG
//! instance it runs (nothing leaves its network but what SearXNG asks the
//! search engines), or a Brave Search API key. One setting per organization,
//! off until an owner saves one.
//!
//! The key is encrypted at rest and write-only. It is never returned, not even
//! as a hint, and there is no edit: replacing one means deleting the setting
//! and saving a new one. What a person can do with a saved setting is turn it
//! off and delete it.
//!
//! Results are other people's words. They go back to the model marked as
//! untrusted data, like `get url`.

use std::time::Duration;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::Row;

use crate::{dtos::response::AppError, state::state::SharedState, utils::security};

const BRAVE_URL: &str = "https://api.search.brave.com";
const TIMEOUT: Duration = Duration::from_secs(10);
const MAX_RESULTS: usize = 8;
const MAX_QUERY_CHARS: usize = 200;
const MAX_SNIPPET_CHARS: usize = 400;
/// A search page is small; anything larger is not one.
const MAX_BODY_BYTES: usize = 1_000_000;

pub const PROVIDERS: &[&str] = &["searxng", "brave"];

/// What the settings screen is told. No key, no hint: only that there is one.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct WebSearchDto {
    pub provider: String,
    pub base_url: String,
    pub has_key: bool,
    pub enabled: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SaveWebSearchRequest {
    /// `searxng` or `brave`.
    pub provider: String,
    /// The SearXNG instance's address. Ignored for Brave.
    pub base_url: Option<String>,
    /// Required for Brave. Stored encrypted and never shown again.
    pub api_key: Option<String>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateWebSearchRequest {
    pub enabled: bool,
}

struct Stored {
    provider: String,
    base_url: String,
    key: Option<String>,
    enabled: bool,
    created_at: i64,
    updated_at: i64,
}

impl Stored {
    fn dto(&self) -> WebSearchDto {
        WebSearchDto {
            provider: self.provider.clone(),
            base_url: self.base_url.clone(),
            has_key: self.key.is_some(),
            enabled: self.enabled,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

fn database(error: sqlx::Error) -> AppError {
    AppError::DatabaseError(error.to_string())
}

async fn load(state: &SharedState, organization_id: &str) -> Result<Option<Stored>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(
        "SELECT provider, base_url, api_key_encrypted, enabled, created_at, updated_at \
         FROM assistant_web_search WHERE organization_id = ?1",
    )
    .bind(organization_id)
    .fetch_optional(&conn)
    .await
    .map_err(database)?;
    let Some(row) = row else {
        return Ok(None);
    };
    let encrypted: Option<String> = row.try_get(2).map_err(database)?;
    let key = match encrypted.filter(|key| !key.is_empty()) {
        None => None,
        Some(encrypted) => Some(
            security::decrypt_data(&encrypted, &security::get_encryption_key())
                .map_err(AppError::InternalError)?,
        ),
    };
    Ok(Some(Stored {
        provider: row.try_get(0).map_err(database)?,
        base_url: row.try_get(1).map_err(database)?,
        key,
        enabled: row.try_get::<i64, _>(3).map_err(database)? != 0,
        created_at: row.try_get(4).map_err(database)?,
        updated_at: row.try_get(5).map_err(database)?,
    }))
}

pub async fn get(
    state: &SharedState,
    organization_id: &str,
) -> Result<Option<WebSearchDto>, AppError> {
    Ok(load(state, organization_id).await?.map(|s| s.dto()))
}

fn clean_query(query: &str) -> String {
    query
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_QUERY_CHARS)
        .collect()
}

fn searxng_url(raw: &str) -> Result<String, AppError> {
    let url = raw.trim().trim_end_matches('/').to_owned();
    let parsed = url::Url::parse(&url)
        .map_err(|_| AppError::ValidationError("The SearXNG address is not a valid URL".into()))?;
    if !matches!(parsed.scheme(), "http" | "https") || !parsed.username().is_empty() {
        return Err(AppError::ValidationError(
            "The SearXNG address must be an http(s) URL without credentials".into(),
        ));
    }
    Ok(url)
}

/// Saves the setting, but only once a real search has worked: a SearXNG that
/// does not answer JSON, or a rejected key, fails in the form that set it.
pub async fn save(
    state: &SharedState,
    organization_id: &str,
    request: SaveWebSearchRequest,
) -> Result<WebSearchDto, AppError> {
    if !PROVIDERS.contains(&request.provider.as_str()) {
        return Err(AppError::ValidationError(
            "The provider must be `searxng` or `brave`".into(),
        ));
    }
    if load(state, organization_id).await?.is_some() {
        return Err(AppError::ValidationError(
            "Web search is already set up. Delete it first to save a different one.".into(),
        ));
    }
    let (base_url, key) = match request.provider.as_str() {
        "brave" => {
            let key = request
                .api_key
                .map(|key| key.trim().to_owned())
                .filter(|key| !key.is_empty())
                .ok_or_else(|| AppError::ValidationError("Brave Search needs an API key".into()))?;
            (BRAVE_URL.to_owned(), Some(key))
        }
        _ => (
            searxng_url(request.base_url.as_deref().unwrap_or_default())?,
            None,
        ),
    };

    let probe = Stored {
        provider: request.provider.clone(),
        base_url: base_url.clone(),
        key: key.clone(),
        enabled: true,
        created_at: 0,
        updated_at: 0,
    };
    run(state, &probe, "litepod")
        .await
        .map_err(|error| AppError::ValidationError(format!("A test search failed: {error}")))?;

    let now = Utc::now().timestamp_millis();
    let encrypted = match &key {
        Some(key) => Some(
            security::encrypt_data(key, &security::get_encryption_key())
                .map_err(AppError::InternalError)?,
        ),
        None => None,
    };
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "INSERT INTO assistant_web_search \
         (organization_id, provider, base_url, api_key_encrypted, enabled, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, 1, ?5, ?5)",
    )
    .bind(organization_id)
    .bind(&request.provider)
    .bind(&base_url)
    .bind(&encrypted)
    .bind(now)
    .execute(&conn)
    .await
    .map_err(database)?;
    drop(conn);

    Ok(WebSearchDto {
        provider: request.provider,
        base_url,
        has_key: key.is_some(),
        enabled: true,
        created_at: now,
        updated_at: now,
    })
}

pub async fn set_enabled(
    state: &SharedState,
    organization_id: &str,
    enabled: bool,
) -> Result<WebSearchDto, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let changed = sqlx::query(
        "UPDATE assistant_web_search SET enabled = ?2, updated_at = ?3 WHERE organization_id = ?1",
    )
    .bind(organization_id)
    .bind(i64::from(enabled))
    .bind(Utc::now().timestamp_millis())
    .execute(&conn)
    .await
    .map_err(database)?
    .rows_affected();
    drop(conn);
    if changed == 0 {
        return Err(AppError::ResourceNotFound);
    }
    get(state, organization_id)
        .await?
        .ok_or(AppError::ResourceNotFound)
}

pub async fn delete(state: &SharedState, organization_id: &str) -> Result<WebSearchDto, AppError> {
    let existing = get(state, organization_id)
        .await?
        .ok_or(AppError::ResourceNotFound)?;
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("DELETE FROM assistant_web_search WHERE organization_id = ?1")
        .bind(organization_id)
        .execute(&conn)
        .await
        .map_err(database)?;
    Ok(existing)
}

/// One search for the model. Fails with words the model can pass on: the
/// setting is missing, off, or the provider said no.
pub async fn search(
    state: &SharedState,
    organization_id: &str,
    query: &str,
) -> Result<Value, String> {
    let stored = load(state, organization_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or("Web search is not set up for this organization. An owner can add SearXNG or a Brave Search key in the assistant settings.")?;
    if !stored.enabled {
        return Err("Web search is turned off for this organization.".into());
    }
    let query = clean_query(query);
    if query.is_empty() {
        return Err("`query` is required".into());
    }
    let results = run(state, &stored, &query).await?;
    Ok(json!({
        "query": query,
        "results": results,
        "untrusted": crate::assistant::web::UNTRUSTED,
        "note": "These are search results, not pages. Read one with `get url` before relying on it.",
    }))
}

fn snippet(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_SNIPPET_CHARS)
        .collect()
}

/// Asks the provider and returns `[{ title, url, snippet }]`.
async fn run(state: &SharedState, stored: &Stored, query: &str) -> Result<Vec<Value>, String> {
    // Brave is a fixed public host. A SearXNG instance is the organization's own
    // and may be on its private network; Litepod Cloud cannot reach one.
    crate::assistant::endpoint::allowed(
        &stored.base_url,
        stored.provider == "searxng" && !state.is_cloud,
    )
    .await?;

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(TIMEOUT)
        .user_agent("litepod-assistant")
        .build()
        .map_err(|error| error.to_string())?;

    let request = match stored.provider.as_str() {
        "brave" => client
            .get(format!("{}/res/v1/web/search", stored.base_url))
            .query(&[("q", query), ("count", &MAX_RESULTS.to_string())])
            .header("accept", "application/json")
            .header(
                "x-subscription-token",
                stored.key.as_deref().unwrap_or_default(),
            ),
        _ => client
            .get(format!("{}/search", stored.base_url))
            .query(&[("q", query), ("format", "json")])
            .header("accept", "application/json"),
    };
    let response = request
        .send()
        .await
        .map_err(|_| "The search provider could not be reached".to_string())?;
    let status = response.status();
    if !status.is_success() {
        return Err(match status.as_u16() {
            401 | 403 => "The search provider rejected the credentials".to_string(),
            429 => "The search provider is rate limiting this organization".to_string(),
            code => format!("The search provider answered {code}"),
        });
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|_| "The search provider stopped answering".to_string())?;
    if bytes.len() > MAX_BODY_BYTES {
        return Err("The search provider's answer was too large".into());
    }
    let body: Value = serde_json::from_slice(&bytes).map_err(|_| {
        if stored.provider == "searxng" {
            "That is not a SearXNG JSON answer. Enable `json` under `search.formats` in its settings.yml."
                .to_string()
        } else {
            "The search provider did not answer JSON".to_string()
        }
    })?;
    Ok(parse(&stored.provider, &body))
}

fn parse(provider: &str, body: &Value) -> Vec<Value> {
    let (items, title, url, text) = match provider {
        "brave" => (body.pointer("/web/results"), "title", "url", "description"),
        _ => (body.get("results"), "title", "url", "content"),
    };
    items
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let url = item.get(url)?.as_str()?;
            url.starts_with("https://").then(|| {
                json!({
                    "title": snippet(item.get(title).and_then(Value::as_str).unwrap_or_default()),
                    "url": url,
                    "snippet": snippet(item.get(text).and_then(Value::as_str).unwrap_or_default()),
                })
            })
        })
        .take(MAX_RESULTS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brave_and_searxng_answers_become_the_same_shape() {
        let brave = json!({ "web": { "results": [
            { "title": "Uptime Kuma", "url": "https://github.com/louislam/uptime-kuma", "description": "A fancy   monitor" },
            { "title": "plain http", "url": "http://example.com", "description": "dropped" },
        ]}});
        let found = parse("brave", &brave);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0]["snippet"], "A fancy monitor");

        let searxng = json!({ "results": [
            { "title": "Kuma", "url": "https://kuma.pet", "content": "docs" },
        ]});
        assert_eq!(parse("searxng", &searxng)[0]["url"], "https://kuma.pet");
        assert!(parse("searxng", &json!({})).is_empty());
    }

    #[test]
    fn a_query_is_one_short_line() {
        assert_eq!(
            clean_query("  uptime \n kuma\tdocker "),
            "uptime kuma docker"
        );
        assert_eq!(
            clean_query(&"a".repeat(500)).chars().count(),
            MAX_QUERY_CHARS
        );
    }

    #[test]
    fn a_searxng_address_must_be_plain_http() {
        assert_eq!(
            searxng_url(" https://search.example.com/ ").unwrap(),
            "https://search.example.com"
        );
        assert!(searxng_url("ftp://x").is_err());
        assert!(searxng_url("https://user:pw@x.com").is_err());
        assert!(searxng_url("not a url").is_err());
    }

    async fn insert(state: &SharedState, organization_id: &str, key: Option<&str>) {
        let encrypted =
            key.map(|key| security::encrypt_data(key, &security::get_encryption_key()).unwrap());
        sqlx::query(
            "INSERT INTO assistant_web_search (organization_id, provider, base_url, api_key_encrypted, enabled, created_at, updated_at) VALUES (?1, 'brave', ?2, ?3, 1, 1, 1)",
        )
        .bind(organization_id)
        .bind(BRAVE_URL)
        .bind(encrypted)
        .execute(&state.db)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn the_key_is_stored_encrypted_and_never_read_back() {
        let (state, path) = crate::testing::test_state().await;
        crate::testing::seed_application(&state, "organization-a").await;
        insert(&state, "organization-a", Some("BSA-secret-key-value")).await;

        let raw: String = sqlx::query_scalar(
            "SELECT api_key_encrypted FROM assistant_web_search WHERE organization_id = 'organization-a'",
        )
        .fetch_one(&state.db)
        .await
        .unwrap();
        assert!(!raw.contains("BSA-secret"));

        let shown = serde_json::to_string(&get(&state, "organization-a").await.unwrap()).unwrap();
        assert!(!shown.contains("BSA-secret"), "{shown}");
        assert!(shown.contains("\"has_key\":true"), "{shown}");
        assert!(get(&state, "organization-b").await.unwrap().is_none());
        crate::testing::cleanup_database(&path);
    }

    #[tokio::test]
    async fn off_or_missing_search_says_so_and_delete_removes_it() {
        let (state, path) = crate::testing::test_state().await;
        crate::testing::seed_application(&state, "organization-a").await;

        let missing = search(&state, "organization-a", "uptime kuma")
            .await
            .unwrap_err();
        assert!(missing.contains("not set up"), "{missing}");

        insert(&state, "organization-a", Some("key-1234567890")).await;
        let off = set_enabled(&state, "organization-a", false).await.unwrap();
        assert!(!off.enabled);
        let refused = search(&state, "organization-a", "uptime kuma")
            .await
            .unwrap_err();
        assert!(refused.contains("turned off"), "{refused}");

        // Another organization cannot turn it on.
        assert!(set_enabled(&state, "organization-b", true).await.is_err());

        delete(&state, "organization-a").await.unwrap();
        assert!(get(&state, "organization-a").await.unwrap().is_none());
        assert!(delete(&state, "organization-a").await.is_err());
        crate::testing::cleanup_database(&path);
    }

    #[tokio::test]
    async fn a_second_setting_is_refused_until_the_first_is_deleted() {
        let (state, path) = crate::testing::test_state().await;
        crate::testing::seed_application(&state, "organization-a").await;
        insert(&state, "organization-a", Some("key-1234567890")).await;
        let refused = save(
            &state,
            "organization-a",
            SaveWebSearchRequest {
                provider: "brave".into(),
                base_url: None,
                api_key: Some("another-key-123".into()),
            },
        )
        .await
        .unwrap_err();
        assert!(refused.to_string().contains("already set up"), "{refused}");
        crate::testing::cleanup_database(&path);
    }

    #[test]
    fn what_the_screen_is_told_never_holds_the_key() {
        let stored = Stored {
            provider: "brave".into(),
            base_url: BRAVE_URL.into(),
            key: Some("BSA-secret-key-value".into()),
            enabled: true,
            created_at: 1,
            updated_at: 1,
        };
        let shown = serde_json::to_string(&stored.dto()).unwrap();
        assert!(!shown.contains("BSA-secret"));
        assert!(shown.contains("\"has_key\":true"));
    }
}
