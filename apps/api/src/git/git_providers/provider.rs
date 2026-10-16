use chrono::Utc;
use uuid::Uuid;

use super::{bitbucket_oauth, gitea_oauth, github_app, gitlab_oauth};

use crate::{
    dtos::{
        git_provider::{
            CreateGitProviderRequest, GitProviderDto, GitRepositoryDto, PROVIDER_TYPES,
            TestGitProviderRequest, UpdateGitProviderRequest,
        },
        response::AppError,
    },
    state::state::SharedState,
    utils::security,
};

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

/// `oauth2` sends the token as basic auth `oauth2:<token>` (Gitea/Forgejo OAuth2 apps);
/// `token` sends it in the provider's token header;
/// `gitlab_oauth` reads a short-lived access token minted via OAuth2 authorization-code;
/// `gitea_oauth` reads a Gitea/Forgejo access token minted via authorization-code.
pub const AUTH_METHODS: [&str; 4] = ["oauth2", "token", "gitlab_oauth", "gitea_oauth"];

/// Gitea/Forgejo OAuth2 tokens authenticate as the literal user `oauth2` over HTTPS.
fn effective_username(auth_method: &str, username: &str) -> String {
    if auth_method == "oauth2" {
        "oauth2".to_string()
    } else {
        username.to_string()
    }
}

const SELECT_COLUMNS: &str = "id, organization_id, name, provider_type, host_url, username, password_encrypted, auth_method, kind, created_at, updated_at";

fn row_to_dto(row: &sqlx::sqlite::SqliteRow) -> Result<GitProviderDto, AppError> {
    use sqlx::Row;
    let read = |index: usize| -> Result<String, AppError> {
        row.try_get(index)
            .map_err(|e| AppError::DatabaseError(e.to_string()))
    };
    let read_int = |index: usize| -> Result<i64, AppError> {
        row.try_get(index)
            .map_err(|e| AppError::DatabaseError(e.to_string()))
    };
    Ok(GitProviderDto {
        id: read(0)?,
        organization_id: read(1)?,
        name: read(2)?,
        provider_type: read(3)?,
        host_url: read(4)?,
        username: read(5)?,
        has_password: !read(6)?.is_empty(),
        auth_method: read(7)?,
        kind: read(8)?,
        created_at: read_int(9)?,
        updated_at: read_int(10)?,
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

fn validate(
    provider_type: &str,
    name: &str,
    host_url: &str,
    username: &str,
    auth_method: &str,
) -> Result<(), AppError> {
    if !AUTH_METHODS.contains(&auth_method) {
        return Err(AppError::ValidationError(format!(
            "Auth method must be one of: {}",
            AUTH_METHODS.join(", ")
        )));
    }
    if name.trim().is_empty() {
        return Err(AppError::ValidationError(
            "Provider name is required".into(),
        ));
    }
    if !PROVIDER_TYPES.contains(&provider_type) {
        return Err(AppError::ValidationError(format!(
            "Provider type must be one of: {}",
            PROVIDER_TYPES.join(", ")
        )));
    }
    if username.trim().is_empty() {
        return Err(AppError::ValidationError("Username is required".into()));
    }
    if host_url.trim().is_empty() && !matches!(provider_type, "github" | "gitlab" | "bitbucket") {
        return Err(AppError::ValidationError(
            "Host URL is required for this provider".into(),
        ));
    }
    if !host_url.is_empty() {
        crate::security::ssrf::parse_self_hosted_url(host_url)
            .map_err(AppError::ValidationError)?;
    }
    Ok(())
}

/// Resolve the web host used by repository URLs and provider API calls.
pub fn effective_host_url(provider_type: &str, host_url: &str) -> Result<String, AppError> {
    if !host_url.trim().is_empty() {
        return Ok(host_url.trim().trim_end_matches('/').to_string());
    }
    match provider_type {
        "github" => Ok("https://github.com".into()),
        "gitlab" => Ok("https://gitlab.com".into()),
        "bitbucket" => Ok("https://bitbucket.org".into()),
        _ => Err(AppError::ValidationError(
            "Host URL is required for this provider".into(),
        )),
    }
}

pub fn allows_private_addresses(is_cloud: bool, provider_type: &str, host_url: &str) -> bool {
    if is_cloud {
        return false;
    }
    let host_url = host_url.trim().trim_end_matches('/');
    if host_url.is_empty() {
        return false;
    }
    match provider_type {
        "github" => url::Url::parse(host_url)
            .ok()
            .and_then(|url| {
                url.host_str()
                    .map(|host| !host.eq_ignore_ascii_case("github.com"))
            })
            .unwrap_or(false),
        "gitlab" => url::Url::parse(host_url)
            .ok()
            .and_then(|url| {
                url.host_str()
                    .map(|host| !host.eq_ignore_ascii_case("gitlab.com"))
            })
            .unwrap_or(false),
        // Cloud only - never a self-hosted target.
        "bitbucket" => false,
        _ => true,
    }
}

pub async fn provider_host_url(
    state: &SharedState,
    provider_id: &str,
) -> Result<(String, bool), AppError> {
    use sqlx::Row;
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query("SELECT provider_type, host_url FROM git_providers WHERE id = ?1")
        .bind(provider_id)
        .fetch_optional(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?
        .ok_or(AppError::ProviderNotFound)?;
    let provider_type: String = row
        .try_get(0)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let host_url: String = row
        .try_get(1)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok((
        effective_host_url(&provider_type, &host_url)?,
        allows_private_addresses(state.is_cloud, &provider_type, &host_url),
    ))
}

fn validate_test_request(
    request: &TestGitProviderRequest,
) -> Result<(String, String, String, String), AppError> {
    let host_url = request
        .host_url
        .as_deref()
        .unwrap_or_default()
        .trim()
        .trim_end_matches('/')
        .to_string();
    let username = request.username.trim().to_string();
    let auth_method = request
        .auth_method
        .clone()
        .unwrap_or_else(|| "token".into());
    validate(
        &request.provider_type,
        "test",
        &host_url,
        &username,
        &auth_method,
    )?;
    if request.password.is_empty() {
        return Err(AppError::ValidationError(
            "Password or token is required to test the connection".into(),
        ));
    }
    Ok((
        request.provider_type.clone(),
        host_url,
        username,
        auth_method,
    ))
}

pub async fn list(
    state: &SharedState,
    organization_id: &str,
) -> Result<Vec<GitProviderDto>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(&format!(
        "SELECT {SELECT_COLUMNS} FROM git_providers WHERE organization_id = ?1 ORDER BY name"
    ))
    .bind(organization_id)
    .fetch_all(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let mut providers = Vec::new();
    for row in &rows {
        providers.push(row_to_dto(row)?);
    }
    Ok(providers)
}

pub async fn create(
    state: &SharedState,
    organization_id: &str,
    request: CreateGitProviderRequest,
) -> Result<GitProviderDto, AppError> {
    let name = request.name.trim().to_string();
    let host_url = request
        .host_url
        .unwrap_or_default()
        .trim()
        .trim_end_matches('/')
        .to_string();
    let username = request.username.trim().to_string();
    // When the user pastes OAuth client credentials for a GitLab provider, the auth method is
    // forced to `gitlab_oauth` regardless of what the form also sent. The PAT in `password`
    // becomes unused, but the column is NOT NULL so a placeholder is stored.
    let has_gitlab_oauth = request.provider_type == "gitlab"
        && request.gitlab_oauth_client_id.is_some()
        && request.gitlab_oauth_client_secret.is_some();
    // Bitbucket has no token auth at all (App Passwords are gone), so every Bitbucket
    // provider is OAuth-only from the moment it is created, credentials or not yet.
    let is_bitbucket = request.provider_type == "bitbucket";
    // Gitea/Forgejo: when the user pastes OAuth client credentials, the auth method is
    // forced to `gitea_oauth` (same trick as the GitLab branch below).
    let has_gitea_oauth = (request.provider_type == "gitea" || request.provider_type == "forgejo")
        && request.gitea_oauth_client_id.is_some()
        && request.gitea_oauth_client_secret.is_some();
    let mut auth_method = request
        .auth_method
        .clone()
        .unwrap_or_else(|| "token".into());
    if has_gitlab_oauth {
        auth_method = "gitlab_oauth".to_string();
    } else if is_bitbucket {
        auth_method = "bitbucket_oauth".to_string();
    } else if has_gitea_oauth {
        auth_method = "gitea_oauth".to_string();
    }
    validate(
        &request.provider_type,
        &name,
        &host_url,
        &username,
        &auth_method,
    )?;
    if request.password.is_empty() {
        return Err(AppError::ValidationError(
            "Password or token is required".into(),
        ));
    }
    let key = security::get_encryption_key();
    let encrypted =
        security::encrypt_data(&request.password, &key).map_err(AppError::InternalError)?;
    let id = Uuid::now_v7().to_string();
    let timestamp = now_ms();
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "INSERT INTO git_providers (id, organization_id, name, provider_type, host_url, username, password_encrypted, auth_method, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
    )
    .bind(id.clone())
    .bind(organization_id)
    .bind(&name)
    .bind(&request.provider_type)
    .bind(&host_url)
    .bind(&username)
    .bind(&encrypted)
    .bind(&auth_method)
    .bind(timestamp)
    .bind(timestamp)
    .execute(&conn)
    .await
    .map_err(map_write_error)?;
    if has_gitlab_oauth {
        // The user already opted in: write the client credentials and a webhook secret now so
        // the UI can show the GitLab App setup instructions immediately after creation.
        let client_id = request.gitlab_oauth_client_id.unwrap_or_default();
        let client_secret = request.gitlab_oauth_client_secret.unwrap_or_default();
        gitlab_oauth::save_client_credentials(
            state,
            &id,
            organization_id,
            &client_id,
            &client_secret,
        )
        .await?;
    }
    if is_bitbucket
        && request.bitbucket_oauth_client_id.is_some()
        && request.bitbucket_oauth_client_secret.is_some()
    {
        let client_id = request.bitbucket_oauth_client_id.unwrap_or_default();
        let client_secret = request.bitbucket_oauth_client_secret.unwrap_or_default();
        bitbucket_oauth::save_client_credentials(
            state,
            &id,
            organization_id,
            &client_id,
            &client_secret,
        )
        .await?;
    }
    if has_gitea_oauth {
        let client_id = request.gitea_oauth_client_id.unwrap_or_default();
        let client_secret = request.gitea_oauth_client_secret.unwrap_or_default();
        gitea_oauth::save_client_credentials(
            state,
            &id,
            organization_id,
            &client_id,
            &client_secret,
        )
        .await?;
    }
    get(state, organization_id, &id).await
}

async fn get(
    state: &SharedState,
    organization_id: &str,
    provider_id: &str,
) -> Result<GitProviderDto, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(&format!(
        "SELECT {SELECT_COLUMNS} FROM git_providers WHERE id = ?1 AND organization_id = ?2"
    ))
    .bind(provider_id)
    .bind(organization_id)
    .fetch_optional(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?
    .ok_or(AppError::ProviderNotFound)?;
    row_to_dto(&row)
}

/// Stored credentials for git operations; never leaves the backend.
pub async fn credentials(
    state: &SharedState,
    provider_id: &str,
) -> Result<(String, String), AppError> {
    use sqlx::Row;
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(
        "SELECT username, password_encrypted, auth_method FROM git_providers WHERE id = ?1",
    )
    .bind(provider_id)
    .fetch_optional(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?
    .ok_or(AppError::ProviderNotFound)?;
    let username: String = row
        .try_get(0)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let encrypted: String = row
        .try_get(1)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let auth_method: String = row
        .try_get(2)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    // gitlab_oauth tokens live in a separate column and are refreshed lazily on each use.
    if auth_method == "gitlab_oauth" {
        let token = gitlab_oauth::access_token(state, provider_id).await?;
        // GitLab accepts OAuth2 tokens as basic auth `oauth2:<token>` over HTTPS, same as
        // Gitea/Forgejo, so the existing git2 basic-auth callbacks work without changes.
        return Ok(("oauth2".to_string(), token));
    }
    if auth_method == "bitbucket_oauth" {
        let token = bitbucket_oauth::access_token(state, provider_id).await?;
        // Bitbucket's documented git-over-HTTPS convention for an OAuth access token: the
        // literal username `x-token-auth` with the token as the password.
        return Ok(("x-token-auth".to_string(), token));
    }
    if auth_method == "gitea_oauth" {
        let token = gitea_oauth::access_token(state, provider_id).await?;
        // Gitea/Forgejo accept OAuth2 tokens as basic auth `oauth2:<token>` over HTTPS.
        return Ok(("oauth2".to_string(), token));
    }
    let key = security::get_encryption_key();
    let password = security::decrypt_data(&encrypted, &key).map_err(AppError::InternalError)?;
    Ok((effective_username(&auth_method, &username), password))
}

/// Git secrets for one clone or ls-remote. GitHub App tokens expire in an hour, so these are
/// resolved immediately before use and never persisted by the caller.
pub struct GitCredentials {
    pub username: String,
    pub secret: String,
}

/// Credentials for any provider kind: stored secrets for `credentials`, a freshly minted
/// installation token for `github_app`.
pub async fn resolve_credentials(
    state: &SharedState,
    provider_id: &str,
) -> Result<GitCredentials, AppError> {
    use sqlx::Row;
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query("SELECT kind FROM git_providers WHERE id = ?1")
        .bind(provider_id)
        .fetch_optional(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?
        .ok_or(AppError::ProviderNotFound)?;
    let kind: String = row
        .try_get(0)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    if kind == "github_app" {
        return Ok(GitCredentials {
            username: "x-access-token".into(),
            secret: github_app::installation_token(state, provider_id).await?,
        });
    }
    let (username, secret) = credentials(state, provider_id).await?;
    Ok(GitCredentials { username, secret })
}

pub async fn update(
    state: &SharedState,
    organization_id: &str,
    provider_id: &str,
    request: UpdateGitProviderRequest,
) -> Result<GitProviderDto, AppError> {
    let current = get(state, organization_id, provider_id).await?;
    let name = request
        .name
        .map(|v| v.trim().to_string())
        .unwrap_or(current.name);
    let provider_type = request.provider_type.unwrap_or(current.provider_type);
    let host_url = request
        .host_url
        .map(|v| v.trim().trim_end_matches('/').to_string())
        .unwrap_or(current.host_url);
    let username = request
        .username
        .map(|v| v.trim().to_string())
        .unwrap_or(current.username);
    let auth_method = request.auth_method.unwrap_or(current.auth_method);
    validate(&provider_type, &name, &host_url, &username, &auth_method)?;
    let is_gitlab = provider_type == "gitlab";
    let conn = crate::git::repositories::db::get_conn(state).await;
    // Blank password keeps the stored secret, matching masked-env-var UX.
    if let Some(password) = request.password.filter(|p| !p.is_empty()) {
        let key = security::get_encryption_key();
        let encrypted = security::encrypt_data(&password, &key).map_err(AppError::InternalError)?;
        sqlx::query(
            "UPDATE git_providers SET name = ?1, provider_type = ?2, host_url = ?3, username = ?4, password_encrypted = ?5, auth_method = ?6, updated_at = ?7 WHERE id = ?8 AND organization_id = ?9",
        )
        .bind(&name)
        .bind(&provider_type)
        .bind(&host_url)
        .bind(&username)
        .bind(&encrypted)
        .bind(&auth_method)
        .bind(now_ms())
        .bind(provider_id)
        .bind(organization_id)
        .execute(&conn)
        .await
        .map_err(map_write_error)?;
    } else {
        sqlx::query(
            "UPDATE git_providers SET name = ?1, provider_type = ?2, host_url = ?3, username = ?4, auth_method = ?5, updated_at = ?6 WHERE id = ?7 AND organization_id = ?8",
        )
        .bind(&name)
        .bind(&provider_type)
        .bind(&host_url)
        .bind(&username)
        .bind(&auth_method)
        .bind(now_ms())
        .bind(provider_id)
        .bind(organization_id)
        .execute(&conn)
        .await
        .map_err(map_write_error)?;
    }
    // GitLab OAuth client credentials are stored in separate columns; `update()` can replace
    // them even when password is left blank. Both fields must be supplied together so we
    // never persist a half-rotation.
    if is_gitlab {
        match (
            request.gitlab_oauth_client_id,
            request.gitlab_oauth_client_secret,
        ) {
            (Some(client_id), Some(client_secret))
                if !client_id.is_empty() && !client_secret.is_empty() =>
            {
                gitlab_oauth::save_client_credentials(
                    state,
                    provider_id,
                    organization_id,
                    &client_id,
                    &client_secret,
                )
                .await?;
            }
            (None, None) => {}
            _ => {
                return Err(AppError::ValidationError("Both gitlab_oauth_client_id and gitlab_oauth_client_secret must be supplied together".into()));
            }
        }
    }
    if provider_type == "bitbucket" {
        match (
            request.bitbucket_oauth_client_id,
            request.bitbucket_oauth_client_secret,
        ) {
            (Some(client_id), Some(client_secret))
                if !client_id.is_empty() && !client_secret.is_empty() =>
            {
                bitbucket_oauth::save_client_credentials(
                    state,
                    provider_id,
                    organization_id,
                    &client_id,
                    &client_secret,
                )
                .await?;
            }
            (None, None) => {}
            _ => {
                return Err(AppError::ValidationError("Both bitbucket_oauth_client_id and bitbucket_oauth_client_secret must be supplied together".into()));
            }
        }
    }
    if provider_type == "gitea" || provider_type == "forgejo" {
        match (
            request.gitea_oauth_client_id,
            request.gitea_oauth_client_secret,
        ) {
            (Some(client_id), Some(client_secret))
                if !client_id.is_empty() && !client_secret.is_empty() =>
            {
                gitea_oauth::save_client_credentials(
                    state,
                    provider_id,
                    organization_id,
                    &client_id,
                    &client_secret,
                )
                .await?;
            }
            (None, None) => {}
            _ => {
                return Err(AppError::ValidationError("Both gitea_oauth_client_id and gitea_oauth_client_secret must be supplied together".into()));
            }
        }
    }
    get(state, organization_id, provider_id).await
}

pub async fn delete(
    state: &SharedState,
    organization_id: &str,
    provider_id: &str,
) -> Result<(), AppError> {
    use sqlx::Row;
    get(state, organization_id, provider_id).await?;
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query("SELECT COUNT(*) FROM applications WHERE git_provider_id = ?1")
        .bind(provider_id)
        .fetch_one(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let count: i64 = row
        .try_get(0)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    if count > 0 {
        return Err(AppError::ResourceInUse(format!(
            "Provider is used by {count} application{} — reconfigure them first",
            if count == 1 { "" } else { "s" }
        )));
    }
    sqlx::query("DELETE FROM git_providers WHERE id = ?1 AND organization_id = ?2")
        .bind(provider_id)
        .bind(organization_id)
        .execute(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok(())
}

/// Strips credentials from git error messages before they reach logs or clients.
pub fn sanitize_git_error(message: &str, password: &str) -> String {
    if password.is_empty() {
        return message.to_string();
    }
    message.replace(password, "••••••")
}

/// Splits `user:pass@host` credentials out of a pasted git URL. Returns the
/// URL with credentials removed (always what gets stored/displayed) and the
/// extracted (username, password) when any were present. Never logs the raw
/// input.
pub fn split_embedded_credentials(raw: &str) -> Result<(String, Option<(String, String)>), String> {
    let mut url = url::Url::parse(raw.trim()).map_err(|_| "That is not a valid URL".to_string())?;
    let username = url.username().to_string();
    let password = url.password().map(str::to_string);
    if username.is_empty() && password.is_none() {
        return Ok((raw.trim().to_string(), None));
    }
    let creds = (username, password.unwrap_or_default());
    let _ = url.set_username("");
    let _ = url.set_password(None);
    Ok((url.to_string(), Some(creds)))
}

/// Decrypted embedded-URL credentials saved on the application itself (used
/// when no `GitProvider` is configured). Backend-only — never surfaced in a DTO.
pub async fn application_git_credentials(
    state: &SharedState,
    application_id: &str,
) -> Result<Option<(String, String)>, AppError> {
    use sqlx::Row;
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(
        "SELECT git_credentials_username, git_credentials_password_encrypted FROM applications WHERE id = ?1",
    )
    .bind(application_id)
    .fetch_optional(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?
    .ok_or(AppError::ApplicationNotFound)?;
    let username: Option<String> = row
        .try_get(0)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let encrypted: Option<String> = row
        .try_get(1)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    match (username, encrypted) {
        (Some(username), Some(encrypted)) => {
            let key = security::get_encryption_key();
            let password =
                security::decrypt_data(&encrypted, &key).map_err(AppError::InternalError)?;
            Ok(Some((username, password)))
        }
        _ => Ok(None),
    }
}

/// Blocking ls-remote; call inside `tokio::task::spawn_blocking`. `credentials`
/// is `None` for an anonymous/public clone — the callback is only registered
/// when there is something to authenticate with.
pub fn remote_branches(
    repo_url: &str,
    credentials: Option<(&str, &str)>,
) -> Result<Vec<String>, String> {
    let password = credentials.map(|(_, p)| p).unwrap_or("");
    let mut callbacks = git2::RemoteCallbacks::new();
    if let Some((username, password)) = credentials {
        let username = username.to_string();
        let password_owned = password.to_string();
        callbacks.credentials(move |_url, username_from_url, _allowed| {
            git2::Cred::userpass_plaintext(username_from_url.unwrap_or(&username), &password_owned)
        });
    }
    let mut remote = git2::Remote::create_detached(repo_url)
        .map_err(|e| sanitize_git_error(&e.to_string(), password))?;
    let connection = remote
        .connect_auth(git2::Direction::Fetch, Some(callbacks), None)
        .map_err(|e| sanitize_git_error(&e.to_string(), password))?;
    let heads = connection
        .list()
        .map_err(|e| sanitize_git_error(&e.to_string(), password))?;
    let mut branches: Vec<String> = heads
        .iter()
        .filter_map(|head| head.name().strip_prefix("refs/heads/").map(String::from))
        .collect();
    branches.sort();
    Ok(branches)
}

/// REST base for a provider, deriving GitHub's split api host from the web host.
fn api_base(provider_type: &str, host_url: &str) -> String {
    let host = host_url.trim_end_matches('/');
    match provider_type {
        "github" => {
            if host.is_empty()
                || host == "https://github.com"
                || host == "http://github.com"
                || host == "https://api.github.com"
            {
                "https://api.github.com".to_string()
            } else {
                format!("{host}/api/v3")
            }
        }
        "gitlab" => format!(
            "{}/api/v4",
            if host.is_empty() {
                "https://gitlab.com"
            } else {
                host
            }
        ),
        "bitbucket" => "https://api.bitbucket.org/2.0".to_string(),
        _ => format!("{host}/api/v1"),
    }
}

fn json_str(value: &serde_json::Value, key: &str) -> Option<String> {
    value.get(key).and_then(|v| v.as_str()).map(String::from)
}

/// The `https` entry of Bitbucket's `links.clone` array - the only one of the
/// two (the other is `ssh`) that works with the stored OAuth token.
fn bitbucket_https_clone_url(item: &serde_json::Value) -> Option<String> {
    item.get("links")?
        .get("clone")?
        .as_array()?
        .iter()
        .find(|entry| entry.get("name").and_then(|v| v.as_str()) == Some("https"))
        .and_then(|entry| json_str(entry, "href"))
}

pub(crate) fn map_repositories(
    provider_type: &str,
    items: &[serde_json::Value],
) -> Vec<GitRepositoryDto> {
    items
        .iter()
        .filter_map(|item| {
            let (full_name, clone_url) = match provider_type {
                "gitlab" => (
                    json_str(item, "path_with_namespace")?,
                    json_str(item, "http_url_to_repo")?,
                ),
                "bitbucket" => (
                    json_str(item, "full_name")?,
                    bitbucket_https_clone_url(item)?,
                ),
                _ => (json_str(item, "full_name")?, json_str(item, "clone_url")?),
            };
            let private = match provider_type {
                "gitlab" => item
                    .get("visibility")
                    .and_then(|v| v.as_str())
                    .is_some_and(|v| v != "public"),
                "bitbucket" => item
                    .get("is_private")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true),
                _ => item
                    .get("private")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
            };
            let default_branch = match provider_type {
                "bitbucket" => item
                    .get("mainbranch")
                    .and_then(|b| b.get("name"))
                    .and_then(|v| v.as_str())
                    .map(String::from),
                _ => json_str(item, "default_branch"),
            };
            Some(GitRepositoryDto {
                full_name,
                clone_url,
                default_branch,
                private,
            })
        })
        .collect()
}

/// Lists every repository the stored credentials can see, paging until exhausted.
pub async fn repositories(
    state: &SharedState,
    organization_id: &str,
    provider_id: &str,
) -> Result<Vec<GitRepositoryDto>, AppError> {
    let provider = get(state, organization_id, provider_id).await?;
    if provider.kind == "github_app" {
        return github_app::installation_repositories(state, provider_id, organization_id).await;
    }
    if provider.provider_type == "generic" {
        return Err(AppError::ValidationError(
            "Generic providers have no repository API — enter the repository URL manually".into(),
        ));
    }
    let (_, password) = credentials(state, provider_id).await?;
    repositories_for_credentials(
        &provider.provider_type,
        &provider.host_url,
        &password,
        &provider.auth_method,
        allows_private_addresses(state.is_cloud, &provider.provider_type, &provider.host_url),
    )
    .await
}

/// Lists repositories visible to credentials provided by the form without persisting them.
pub async fn test_repositories(
    state: &SharedState,
    request: TestGitProviderRequest,
) -> Result<Vec<GitRepositoryDto>, AppError> {
    let (provider_type, host_url, username, auth_method) = validate_test_request(&request)?;
    // Generic servers expose no repository API, so authentication is proven with ls-remote on one repo.
    if provider_type == "generic" {
        let repo_url = request.repo_url.unwrap_or_default().trim().to_string();
        if repo_url.is_empty() {
            return Err(AppError::ValidationError(
                "Enter a repository URL to test generic git credentials".into(),
            ));
        }
        let provider_host = effective_host_url(&provider_type, &host_url)?;
        let allow_private = allows_private_addresses(state.is_cloud, &provider_type, &host_url);
        crate::security::ssrf::validate_for_host(&repo_url, &provider_host, allow_private)
            .await
            .map_err(AppError::ValidationError)?;
        let password = request.password.clone();
        let url = repo_url.clone();
        let username = effective_username(&auth_method, &username);
        let branches = tokio::task::spawn_blocking(move || {
            remote_branches(&url, Some((&username, &password)))
        })
        .await
        .map_err(|e| AppError::InternalError(format!("Branch listing task failed: {e}")))?
        .map_err(AppError::ValidationError)?;
        let full_name = repo_url
            .trim_end_matches(".git")
            .rsplit('/')
            .take(2)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("/");
        return Ok(vec![GitRepositoryDto {
            full_name: if full_name.is_empty() {
                repo_url.clone()
            } else {
                full_name
            },
            clone_url: repo_url,
            default_branch: branches
                .iter()
                .find(|b| *b == "main" || *b == "master")
                .cloned()
                .or_else(|| branches.first().cloned()),
            private: true,
        }]);
    }
    repositories_for_credentials(
        &provider_type,
        &host_url,
        &request.password,
        &auth_method,
        allows_private_addresses(state.is_cloud, &provider_type, &host_url),
    )
    .await
}

async fn repositories_for_credentials(
    provider_type: &str,
    host_url: &str,
    password: &str,
    auth_method: &str,
    allow_private: bool,
) -> Result<Vec<GitRepositoryDto>, AppError> {
    let base = api_base(provider_type, host_url);
    let base_url = crate::security::ssrf::parse_https_url_with_port(&base)
        .map_err(AppError::ValidationError)?;
    crate::security::ssrf::assert_host(&base_url, allow_private)
        .await
        .map_err(AppError::ValidationError)?;
    let client = reqwest::Client::builder()
        .user_agent("litepod")
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| AppError::InternalError(format!("HTTP client build failed: {e}")))?;

    const PER_PAGE: usize = 100;
    const MAX_PAGES: usize = 10;
    let mut repositories = Vec::new();
    // Bitbucket pages by opaque cursor (a full `next` URL in the response body),
    // not by page number - every other provider here pages by `page=N`.
    let mut bitbucket_next_url: Option<String> = None;

    for page in 1..=MAX_PAGES {
        if provider_type == "bitbucket" && page > 1 && bitbucket_next_url.is_none() {
            break;
        }
        let url = match provider_type {
            "github" => format!(
                "{base}/user/repos?per_page={PER_PAGE}&page={page}&sort=full_name&affiliation=owner,collaborator,organization_member"
            ),
            "gitlab" => format!(
                "{base}/projects?membership=true&simple=true&order_by=path&per_page={PER_PAGE}&page={page}"
            ),
            "bitbucket" => bitbucket_next_url
                .take()
                .unwrap_or_else(|| format!("{base}/repositories?role=member&pagelen={PER_PAGE}")),
            _ => format!("{base}/user/repos?limit={PER_PAGE}&page={page}"),
        };
        let request = match (provider_type, auth_method) {
            // OAuth2 tokens are accepted as basic auth `oauth2:<token>` by Gitea and Forgejo.
            (_, "oauth2") => client.get(&url).basic_auth("oauth2", Some(password)),
            // GitLab OAuth (gitlab_oauth) and PATs both work with PRIVATE-TOKEN, but Bearer is
            // the documented modern way for OAuth-minted tokens and matches what /oauth/token
            // returns. PATs are handled by the `gitlab`+token branch below.
            ("gitlab", "gitlab_oauth") => client.get(&url).bearer_auth(password),
            ("gitlab", _) => client.get(&url).header("PRIVATE-TOKEN", password),
            ("github", _) => client
                .get(&url)
                .header("Accept", "application/vnd.github+json")
                .bearer_auth(password),
            ("bitbucket", _) => client.get(&url).bearer_auth(password),
            _ => client
                .get(&url)
                .header("Authorization", format!("token {password}")),
        };
        let response = request
            .send()
            .await
            .map_err(|e| AppError::ValidationError(sanitize_git_error(&e.to_string(), password)))?;
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            let detail = if status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN
            {
                "credentials rejected — check the token and its scopes".to_string()
            } else {
                sanitize_git_error(body.chars().take(200).collect::<String>().trim(), password)
            };
            return Err(AppError::ValidationError(format!(
                "Provider returned {status}: {detail}"
            )));
        }
        let items: Vec<serde_json::Value> = if provider_type == "bitbucket" {
            let envelope: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
                AppError::InternalError(format!("Unexpected provider response: {e}"))
            })?;
            bitbucket_next_url = envelope
                .get("next")
                .and_then(|v| v.as_str())
                .map(String::from);
            envelope
                .get("values")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default()
        } else {
            serde_json::from_str(&body).map_err(|e| {
                AppError::InternalError(format!("Unexpected provider response: {e}"))
            })?
        };
        let count = items.len();
        repositories.extend(map_repositories(provider_type, &items));
        if count < PER_PAGE {
            break;
        }
    }

    repositories.sort_by(|a, b| a.full_name.to_lowercase().cmp(&b.full_name.to_lowercase()));
    repositories.dedup_by(|a, b| a.clone_url == b.clone_url);
    Ok(repositories)
}

pub async fn branches(
    state: &SharedState,
    organization_id: &str,
    provider_id: &str,
    repo_url: &str,
) -> Result<Vec<String>, AppError> {
    let provider = get(state, organization_id, provider_id).await?;
    let provider_host = effective_host_url(&provider.provider_type, &provider.host_url)?;
    crate::security::ssrf::validate_for_host(
        repo_url,
        &provider_host,
        allows_private_addresses(state.is_cloud, &provider.provider_type, &provider.host_url),
    )
    .await
    .map_err(AppError::ValidationError)?;
    let GitCredentials {
        username,
        secret: password,
    } = resolve_credentials(state, provider_id).await?;
    let url = repo_url.to_string();
    tokio::task::spawn_blocking(move || remote_branches(&url, Some((&username, &password))))
        .await
        .map_err(|e| AppError::InternalError(format!("Branch listing task failed: {e}")))?
        .map_err(AppError::ValidationError)
}

/// Lists branches for a repo with no saved `GitProvider` — either a bare
/// public URL or one carrying `user:token@host` credentials. Serves both the
/// zero-credential and embedded-URL-credential flows with the same helper.
pub async fn public_branches(repo_url: &str) -> Result<Vec<String>, AppError> {
    let (clean_url, embedded) =
        split_embedded_credentials(repo_url).map_err(AppError::ValidationError)?;
    crate::security::ssrf::validate_public_url(&clean_url)
        .await
        .map_err(AppError::ValidationError)?;
    let url = clean_url;
    tokio::task::spawn_blocking(move || {
        remote_branches(
            &url,
            embedded.as_ref().map(|(u, p)| (u.as_str(), p.as_str())),
        )
    })
    .await
    .map_err(|e| AppError::InternalError(format!("Branch listing task failed: {e}")))?
    .map_err(AppError::ValidationError)
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
        let path =
            std::env::temp_dir().join(format!("litepod-git-providers-crud-{}.db", Uuid::now_v7()));
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
            public_url: String::new(),
            oauth_states: Mutex::new(HashMap::new()),
            public_ip: "203.0.113.7".into(),
            is_cloud: false,
            polar: None,
            dev_mode: false,
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
        let conn = crate::git::repositories::db::get_conn(&state).await;
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
        sqlx::query("INSERT INTO projects (id, organization_id, name, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)")
            .bind("project-1")
            .bind("organization-1")
            .bind("Project")
            .bind(1_i64)
            .bind(1_i64)
            .execute(&conn)
            .await
            .unwrap();
        sqlx::query("INSERT INTO environments (id, project_id, name, is_production, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)")
            .bind("environment-1")
            .bind("project-1")
            .bind("prod")
            .bind(1_i64)
            .bind(1_i64)
            .bind(1_i64)
            .execute(&conn)
            .await
            .unwrap();
        sqlx::query("INSERT INTO applications (id, environment_id, name, server_id, created_at, updated_at) VALUES (?1, ?2, ?3, 'local', ?4, ?5)")
            .bind("application-1")
            .bind("environment-1")
            .bind("Application")
            .bind(1_i64)
            .bind(1_i64)
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

    fn request(name: &str, provider_type: &str) -> CreateGitProviderRequest {
        CreateGitProviderRequest {
            name: name.into(),
            provider_type: provider_type.into(),
            host_url: Some("https://github.com".into()),
            username: "octo".into(),
            password: "secret-token".into(),
            auth_method: None,
            gitlab_oauth_client_id: None,
            gitlab_oauth_client_secret: None,
            bitbucket_oauth_client_id: None,
            bitbucket_oauth_client_secret: None,
            gitea_oauth_client_id: None,
            gitea_oauth_client_secret: None,
        }
    }

    #[tokio::test]
    async fn create_encrypts_password_and_never_returns_it() {
        let (state, path) = test_state().await;
        let provider = create(&state, "organization-1", request("Work GitHub", "github"))
            .await
            .unwrap();
        assert!(provider.has_password);
        let serialized = serde_json::to_string(&provider).unwrap();
        assert!(
            !serialized.contains("secret-token"),
            "DTO must not leak the password: {serialized}"
        );

        let (username, password) = credentials(&state, &provider.id).await.unwrap();
        assert_eq!(username, "octo");
        assert_eq!(password, "secret-token", "decrypt round-trips");
        cleanup(path);
    }

    #[tokio::test]
    async fn update_with_blank_password_keeps_the_stored_one() {
        let (state, path) = test_state().await;
        let provider = create(&state, "organization-1", request("Work GitHub", "github"))
            .await
            .unwrap();
        let updated = update(
            &state,
            "organization-1",
            &provider.id,
            UpdateGitProviderRequest {
                name: Some("Renamed".into()),
                provider_type: None,
                host_url: None,
                username: None,
                password: None,
                auth_method: None,
                gitlab_oauth_client_id: None,
                gitlab_oauth_client_secret: None,
                bitbucket_oauth_client_id: None,
                bitbucket_oauth_client_secret: None,
                gitea_oauth_client_id: None,
                gitea_oauth_client_secret: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(updated.name, "Renamed");
        let (_, password) = credentials(&state, &provider.id).await.unwrap();
        assert_eq!(password, "secret-token", "password unchanged");
        cleanup(path);
    }

    #[tokio::test]
    async fn duplicate_name_per_organization_is_a_conflict() {
        let (state, path) = test_state().await;
        create(&state, "organization-1", request("Work GitHub", "github"))
            .await
            .unwrap();
        let duplicate = create(&state, "organization-1", request("Work GitHub", "gitlab")).await;
        assert!(
            matches!(duplicate, Err(AppError::AlreadyExists)),
            "got {duplicate:?}"
        );
        cleanup(path);
    }

    #[tokio::test]
    async fn invalid_provider_type_is_rejected() {
        let (state, path) = test_state().await;
        let result = create(&state, "organization-1", request("Other", "bitbucket")).await;
        assert!(
            matches!(result, Err(AppError::ValidationError(_))),
            "got {result:?}"
        );
        cleanup(path);
    }

    #[tokio::test]
    async fn delete_blocked_while_applications_reference_the_provider() {
        let (state, path) = test_state().await;
        let provider = create(&state, "organization-1", request("Work GitHub", "github"))
            .await
            .unwrap();
        let conn = crate::git::repositories::db::get_conn(&state).await;
        sqlx::query("UPDATE applications SET git_provider_id = ?1 WHERE id = 'application-1'")
            .bind(provider.id.clone())
            .execute(&conn)
            .await
            .unwrap();
        let result = delete(&state, "organization-1", &provider.id).await;
        assert!(
            matches!(result, Err(AppError::ResourceInUse(_))),
            "got {result:?}"
        );

        sqlx::query("UPDATE applications SET git_provider_id = NULL WHERE id = 'application-1'")
            .execute(&conn)
            .await
            .unwrap();
        delete(&state, "organization-1", &provider.id)
            .await
            .unwrap();
        assert!(list(&state, "organization-1").await.unwrap().is_empty());
        cleanup(path);
    }

    #[tokio::test]
    async fn cloud_stored_provider_policy_rejects_internal_destinations() {
        for cloud in [true, false] {
            let (state, _path) = crate::testing::test_state_for(cloud).await;
            let mut draft = request("Internal Git", "gitea");
            draft.host_url = Some("https://git.internal.example".into());
            let provider = create(&state, "organization-a", draft).await.unwrap();
            let (_, allow_private) = provider_host_url(&state, &provider.id).await.unwrap();
            assert_eq!(allow_private, !cloud);
            // Exercise the network policy on deterministic resolved addresses,
            // without opening a connection or relying on external DNS services.
            for host in ["127.0.0.1", "10.0.0.1", "169.254.169.254"] {
                let url = url::Url::parse(&format!("https://{host}")).unwrap();
                let result = crate::security::ssrf::assert_host(&url, allow_private).await;
                assert_eq!(result.is_err(), cloud);
            }
        }
    }

    #[test]
    fn cloud_never_allows_private_git_addresses() {
        for provider in ["github", "gitlab", "gitea", "forgejo", "generic"] {
            assert!(!allows_private_addresses(
                true,
                provider,
                "https://git.internal.example"
            ));
        }
    }

    #[tokio::test]
    async fn cloud_draft_credentials_reject_loopback_before_network_request() {
        let (state, _path) = crate::testing::test_state_for(true).await;
        for provider in ["generic", "gitea"] {
            let request = TestGitProviderRequest {
                provider_type: provider.into(),
                host_url: Some("https://127.0.0.1".into()),
                username: "user".into(),
                password: "secret".into(),
                auth_method: None,
                repo_url: Some("https://127.0.0.1/repo.git".into()),
            };
            assert!(matches!(
                test_repositories(&state, request).await,
                Err(AppError::ValidationError(_))
            ));
        }
    }

    #[test]
    fn sanitize_strips_passwords_from_error_messages() {
        assert_eq!(
            sanitize_git_error(
                "auth failed for https://octo:hunter2@github.com/o/r.git",
                "hunter2"
            ),
            "auth failed for https://octo:••••••@github.com/o/r.git"
        );
        assert_eq!(sanitize_git_error("no secret here", ""), "no secret here");
    }

    #[test]
    fn private_addresses_are_only_allowed_for_explicit_self_hosted_hosts() {
        assert!(!allows_private_addresses(false, "github", ""));
        assert!(!allows_private_addresses(
            false,
            "github",
            "https://github.com"
        ));
        assert!(allows_private_addresses(
            false,
            "gitlab",
            "https://git.sebasgc.xyz"
        ));
        assert!(allows_private_addresses(
            false,
            "gitea",
            "https://git.internal.example"
        ));
    }

    #[test]
    fn test_request_requires_credentials() {
        let missing_password = TestGitProviderRequest {
            provider_type: "github".into(),
            host_url: Some("https://github.com".into()),
            username: "octo".into(),
            password: "".into(),
            auth_method: None,
            repo_url: None,
        };
        assert!(
            matches!(validate_test_request(&missing_password), Err(AppError::ValidationError(message)) if message.contains("required to test"))
        );

        let bad_method = TestGitProviderRequest {
            provider_type: "gitea".into(),
            host_url: Some("https://git.example.test".into()),
            username: "octo".into(),
            password: "secret".into(),
            auth_method: Some("basic".into()),
            repo_url: None,
        };
        assert!(
            matches!(validate_test_request(&bad_method), Err(AppError::ValidationError(message)) if message.contains("Auth method"))
        );
    }

    #[tokio::test]
    async fn oauth2_providers_authenticate_as_the_oauth2_user() {
        let (state, path) = test_state().await;
        let mut input = request("Forgejo", "forgejo");
        input.host_url = Some("https://code.example.test".into());
        input.auth_method = Some("oauth2".into());
        let provider = create(&state, "organization-1", input).await.unwrap();
        assert_eq!(provider.auth_method, "oauth2");
        let (username, password) = credentials(&state, &provider.id).await.unwrap();
        assert_eq!(
            username, "oauth2",
            "OAuth2 tokens clone as the literal oauth2 user"
        );
        assert_eq!(password, "secret-token");
        cleanup(path);
    }

    #[test]
    fn split_embedded_credentials_extracts_and_strips() {
        assert_eq!(
            split_embedded_credentials("https://github.com/octo/repo.git").unwrap(),
            ("https://github.com/octo/repo.git".to_string(), None)
        );
        let (url, creds) =
            split_embedded_credentials("https://octo:ghp_token@github.com/octo/repo.git").unwrap();
        assert_eq!(url, "https://github.com/octo/repo.git");
        assert_eq!(creds, Some(("octo".to_string(), "ghp_token".to_string())));
        let (url, creds) =
            split_embedded_credentials("https://octo@github.com/octo/repo.git").unwrap();
        assert_eq!(url, "https://github.com/octo/repo.git");
        assert_eq!(creds, Some(("octo".to_string(), String::new())));
        assert!(split_embedded_credentials("not a url").is_err());
    }

    #[tokio::test]
    async fn generic_test_without_repository_url_is_rejected() {
        let generic = TestGitProviderRequest {
            provider_type: "generic".into(),
            host_url: Some("https://git.example.test".into()),
            username: "octo".into(),
            password: "secret".into(),
            auth_method: None,
            repo_url: None,
        };
        let (state, _path) = crate::testing::test_state().await;
        let result = test_repositories(&state, generic).await;
        assert!(
            matches!(result, Err(AppError::ValidationError(ref message)) if message.contains("repository URL")),
            "got {result:?}"
        );
    }
}
