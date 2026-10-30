use chrono::Utc;

use crate::{
    dtos::{
        deployment::{SourceConfigDto, UpdateSourceRequest},
        response::AppError,
    },
    state::state::SharedState,
};

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

fn row_to_dto(row: &sqlx::sqlite::SqliteRow) -> Result<SourceConfigDto, AppError> {
    use sqlx::Row;
    let read = |index: usize| -> Result<Option<String>, AppError> {
        row.try_get(index)
            .map_err(|e| AppError::DatabaseError(e.to_string()))
    };
    Ok(SourceConfigDto {
        deploy_type: row
            .try_get::<String, _>(0)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        build_method: row
            .try_get::<String, _>(1)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        git_provider_id: read(2)?,
        git_url: read(3)?,
        git_branch: read(4)?,
        dockerfile_path: read(5)?,
        compose_file_path: read(6)?,
        compose_public_service: read(7)?,
        container_port: row
            .try_get(8)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        railpack_project_path: row
            .try_get::<String, _>(9)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        railpack_start_command: read(17)?,
        image_url: read(18)?,
        has_registry_credentials: read(19)?.is_some(),
        restart_policy: row
            .try_get::<String, _>(21)
            .unwrap_or_else(|_| "unless-stopped".into()),
        auto_deploy: row
            .try_get::<i64, _>(10)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?
            == 1,
        clean_build: row
            .try_get::<i64, _>(13)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?
            == 1,
        build_cpu_cores: row
            .try_get(11)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        build_memory_mib: row
            .try_get(12)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        build_swap_mib: row
            .try_get(14)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        has_git_credentials: row
            .try_get::<Option<String>, _>(15)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?
            .is_some(),
        watch_paths: read(16)?,
        source_upload: row
            .try_get::<i64, _>(22)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?
            == 1,
        source_archive_bytes: None,
    })
}

pub async fn get(state: &SharedState, application_id: &str) -> Result<SourceConfigDto, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(
        "SELECT deploy_type, build_method, git_provider_id, git_url, git_branch, dockerfile_path, compose_file_path, compose_public_service, container_port, railpack_project_path, auto_deploy, build_cpu_cores, build_memory_mib, clean_build, build_swap_mib, git_credentials_password_encrypted, watch_paths, railpack_start_command, image_url, registry_username, registry_password_encrypted, restart_policy, source_upload FROM applications WHERE id = ?1",
    )
    .bind(application_id)
    .fetch_optional(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?
    .ok_or(AppError::ApplicationNotFound)?;
    let mut source = row_to_dto(&row)?;
    if source.source_upload {
        source.source_archive_bytes =
            tokio::fs::metadata(super::source_archive::archive_file(state, application_id))
                .await
                .ok()
                .map(|metadata| metadata.len());
    }
    Ok(source)
}

pub async fn set_clean_build(
    state: &SharedState,
    application_id: &str,
    clean_build: bool,
) -> Result<SourceConfigDto, AppError> {
    let source = get(state, application_id).await?;
    if source.deploy_type != "git" || source.build_method != "railpack" {
        return Err(AppError::ValidationError(
            "Clean build is available only for Git applications using Railpack".into(),
        ));
    }
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("UPDATE applications SET clean_build = ?1, updated_at = ?2 WHERE id = ?3")
        .bind(i64::from(clean_build))
        .bind(now_ms())
        .bind(application_id)
        .execute(&conn)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    get(state, application_id).await
}

/// Applies the optional clean-build choice included in a deploy request.
/// Clean builds are a Railpack-specific cache policy, so other source modes
/// deliberately ignore the field and proceed with their normal deployment.
pub async fn apply_clean_build_for_deploy(
    state: &SharedState,
    application_id: &str,
    clean_build: bool,
) -> Result<(), AppError> {
    let source = get(state, application_id).await?;
    if source.deploy_type == "git" && source.build_method == "railpack" {
        set_clean_build(state, application_id, clean_build).await?;
    }
    Ok(())
}

/// A git deploy needs every source field filled; the error names what's missing
/// so the UI can point at the right input.
fn require_text(value: Option<&str>, name: &str) -> Result<(), AppError> {
    if value.unwrap_or("").trim().is_empty() {
        return Err(AppError::ValidationError(format!(
            "Git deploys require: {name}"
        )));
    }
    Ok(())
}

fn require_relative_path(value: Option<&str>, name: &str) -> Result<(), AppError> {
    require_text(value, name)?;
    let path = std::path::Path::new(value.unwrap().trim());
    if path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(AppError::ValidationError(format!(
            "{name} must be relative to the repository root"
        )));
    }
    Ok(())
}

fn missing_port(port: Option<i64>, missing: &mut Vec<&str>) -> Result<(), AppError> {
    match port {
        Some(port) if (1..=65_535).contains(&port) => Ok(()),
        Some(_) => Err(AppError::ValidationError(
            "Container port must be between 1 and 65535".into(),
        )),
        None => {
            missing.push("container port");
            Ok(())
        }
    }
}

pub(crate) fn validate_runtime_limits(
    runtime_cpu_cores: Option<f64>,
    runtime_memory_mib: Option<i64>,
) -> Result<(), AppError> {
    match (runtime_cpu_cores, runtime_memory_mib) {
        (None, None) => Ok(()),
        (Some(cpu), Some(memory)) if cpu.is_finite() && cpu >= 0.1 && memory > 0 => Ok(()),
        (Some(cpu), Some(_)) if !cpu.is_finite() || cpu < 0.1 => Err(AppError::ValidationError(
            "Runtime CPU limit must be at least 0.1 cores".into(),
        )),
        (Some(_), Some(_)) => Err(AppError::ValidationError(
            "Runtime memory limit must be greater than zero".into(),
        )),
        _ => Err(AppError::ValidationError(
            "Configure both runtime CPU and memory limits, or leave both unlimited".into(),
        )),
    }
}

/// `archive`: the source is an uploaded ZIP, so there is no repository to name.
fn validate(request: &UpdateSourceRequest, archive: bool) -> Result<(), AppError> {
    if let Some(policy) = request.restart_policy.as_deref() {
        if !matches!(policy, "no" | "always" | "unless-stopped" | "on-failure") {
            return Err(AppError::ValidationError(
                "Restart policy must be 'no', 'always', 'unless-stopped', or 'on-failure'".into(),
            ));
        }
    }
    if request
        .railpack_start_command
        .as_deref()
        .is_some_and(|command| command.len() > 2048)
    {
        return Err(AppError::ValidationError(
            "Railpack start command must be 2048 bytes or fewer".into(),
        ));
    }
    match (request.build_cpu_cores, request.build_memory_mib) {
        (None, None) => {}
        (Some(cpu), Some(memory)) if cpu.is_finite() && cpu >= 0.1 && memory > 0 => {}
        (Some(cpu), Some(_)) if !cpu.is_finite() || cpu < 0.1 => {
            return Err(AppError::ValidationError(
                "Build CPU limit must be at least 0.1 cores".into(),
            ));
        }
        (Some(_), Some(_)) => {
            return Err(AppError::ValidationError(
                "Build memory limit must be greater than zero".into(),
            ));
        }
        _ => {
            return Err(AppError::ValidationError(
                "Configure both build CPU and memory limits, or leave both unlimited".into(),
            ));
        }
    }
    if let Some(swap) = request.build_swap_mib {
        if swap < 0 {
            return Err(AppError::ValidationError(
                "Build swap limit must be zero or positive".into(),
            ));
        }
        if request.build_memory_mib.is_none() {
            return Err(AppError::ValidationError(
                "Build swap requires a build memory limit".into(),
            ));
        }
    }
    if request.deploy_type != "static"
        && request.deploy_type != "git"
        && request.deploy_type != "compose"
        && request.deploy_type != "image"
    {
        return Err(AppError::ValidationError(
            "Deploy type must be 'static', 'git', 'compose', or 'image'".into(),
        ));
    }
    if request.deploy_type == "static" || request.deploy_type == "compose" {
        return Ok(());
    }
    if request.deploy_type == "image" {
        let mut missing = Vec::new();
        if request.image_url.as_deref().unwrap_or("").trim().is_empty() {
            missing.push("image reference");
        }
        missing_port(request.container_port, &mut missing)?;
        if !missing.is_empty() {
            return Err(AppError::ValidationError(format!(
                "Image deploys require: {}",
                missing.join(", ")
            )));
        }
        return Ok(());
    }
    let mut missing = Vec::new();
    if !archive && request.git_url.as_deref().unwrap_or("").trim().is_empty() {
        missing.push("repository URL");
    }
    if !archive
        && request
            .git_branch
            .as_deref()
            .unwrap_or("")
            .trim()
            .is_empty()
    {
        missing.push("branch");
    }
    match request.build_method.as_str() {
        "docker" => {
            if request
                .dockerfile_path
                .as_deref()
                .unwrap_or("")
                .trim()
                .is_empty()
            {
                missing.push("Dockerfile path");
            }
            missing_port(request.container_port, &mut missing)?;
        }
        "compose" => {
            if request
                .compose_file_path
                .as_deref()
                .unwrap_or("")
                .trim()
                .is_empty()
            {
                missing.push("Compose file path");
            }
            if request
                .compose_public_service
                .as_deref()
                .unwrap_or("")
                .trim()
                .is_empty()
            {
                missing.push("public service");
            }
            missing_port(request.container_port, &mut missing)?;
        }
        "railpack" => missing_port(request.container_port, &mut missing)?,
        _ => {
            return Err(AppError::ValidationError(
                "Build method must be 'docker', 'compose', or 'railpack'".into(),
            ));
        }
    }
    if !missing.is_empty() {
        return Err(AppError::ValidationError(format!(
            "Git deploys require: {}",
            missing.join(", ")
        )));
    }
    match request.build_method.as_str() {
        "docker" => require_relative_path(request.dockerfile_path.as_deref(), "Dockerfile path"),
        "compose" => {
            require_relative_path(request.compose_file_path.as_deref(), "Compose file path")
        }
        "railpack" => require_relative_path(
            Some(
                if request
                    .railpack_project_path
                    .as_deref()
                    .unwrap_or("")
                    .trim()
                    .is_empty()
                {
                    "."
                } else {
                    request.railpack_project_path.as_deref().unwrap_or(".")
                },
            ),
            "Railpack project path",
        ),
        _ => unreachable!("build method already validated"),
    }?;
    Ok(())
}

/// `owner/repo` from a clone URL, so a push webhook can match on repository.full_name without
/// re-parsing git_url per delivery.
pub(crate) fn repo_full_name(git_url: &str) -> Option<String> {
    let trimmed = git_url
        .trim()
        .trim_end_matches('/')
        .trim_end_matches(".git");
    let mut segments = trimmed.rsplit('/');
    let repo = segments.next()?.to_string();
    let owner = segments.next()?.to_string();
    if owner.is_empty() || repo.is_empty() || owner.contains(':') {
        return None;
    }
    Some(format!("{owner}/{repo}"))
}

/// Raw stored git_url and embedded-URL credentials, used at save time to decide
/// whether an unrelated field-only resave should keep a previously saved token.
async fn current_git_credentials(
    state: &SharedState,
    application_id: &str,
) -> Result<(Option<String>, Option<String>, Option<String>), AppError> {
    use sqlx::Row;
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(
        "SELECT git_url, git_credentials_username, git_credentials_password_encrypted FROM applications WHERE id = ?1",
    )
    .bind(application_id)
    .fetch_optional(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?
    .ok_or(AppError::ApplicationNotFound)?;
    Ok((
        row.try_get(0)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        row.try_get(1)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        row.try_get(2)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
    ))
}

/// Raw stored image reference + registry credentials, used at save time to keep
/// previously saved registry credentials when the image reference is unchanged.
async fn current_registry_credentials(
    state: &SharedState,
    application_id: &str,
) -> Result<(Option<String>, Option<String>, Option<String>), AppError> {
    use sqlx::Row;
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(
        "SELECT image_url, registry_username, registry_password_encrypted FROM applications WHERE id = ?1",
    )
    .bind(application_id)
    .fetch_optional(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?
    .ok_or(AppError::ApplicationNotFound)?;
    Ok((
        row.try_get(0)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        row.try_get(1)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        row.try_get(2)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
    ))
}

pub async fn save(
    state: &SharedState,
    application_id: &str,
    mut request: UpdateSourceRequest,
) -> Result<SourceConfigDto, AppError> {
    // Saving a git source with no repository keeps an uploaded ZIP as the source;
    // naming a repository (or a provider) switches back to cloning.
    let keep_upload = request.deploy_type == "git"
        && request.git_url.as_deref().unwrap_or("").trim().is_empty()
        && request.git_provider_id.is_none()
        && super::source_archive::is_upload(state, application_id).await?;
    validate(&request, keep_upload)?;
    // Only the Compose pipeline resolves `{{url}}` domain markers; any other
    // deploy type would hand them to the container literally.
    if request.deploy_type != "compose" {
        let environment =
            crate::deployment::decrypted_environment(state, application_id, "runtime").await?;
        if crate::deployment::compose::domain_markers::has_markers(&environment) {
            return Err(AppError::ValidationError(
                "Runtime variables use domain markers ({{url}}, {{domain}}), which only Compose applications resolve. Replace them with plain values before changing the deploy type".into(),
            ));
        }
    }
    let auto_deploy = request.auto_deploy.unwrap_or(false) && request.deploy_type == "git";
    // Embedded-URL credentials (Feature B) only apply when there's no saved provider — a
    // provider always wins and clears any previously saved ones (bound below).
    let mut git_credentials_username: Option<String> = None;
    let mut git_credentials_password_encrypted: Option<String> = None;
    if let Some(provider_id) = &request.git_provider_id {
        use sqlx::Row;
        let conn = crate::git::repositories::db::get_conn(state).await;
        let provider: Option<(String, String, String)> =
            sqlx::query("SELECT kind, provider_type, host_url FROM git_providers WHERE id = ?1")
                .bind(provider_id.clone())
                .fetch_optional(&conn)
                .await
                .map_err(|e| AppError::DatabaseError(e.to_string()))?
                .map(|row| {
                    Ok::<_, AppError>((
                        row.try_get(0)
                            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
                        row.try_get(1)
                            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
                        row.try_get(2)
                            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
                    ))
                })
                .transpose()?;
        let Some((kind, provider_type, host_url)) = provider else {
            return Err(AppError::ProviderNotFound);
        };
        let provider_host =
            crate::git::git_providers::effective_host_url(&provider_type, &host_url)?;
        let repository_url = request.git_url.as_deref().unwrap_or_default();
        crate::security::ssrf::parse_url_for_host(repository_url, &provider_host)
            .map_err(AppError::ValidationError)?;
        // GitHub App providers (kind=github_app) always have a webhook. Other providers need
        // a known provider_type with a push-webhook signature scheme (gitea/forgejo/gitlab).
        let has_webhook = kind == "github_app"
            || matches!(
                provider_type.as_str(),
                "gitea" | "forgejo" | "gitlab" | "github"
            );
        if auto_deploy && !has_webhook {
            return Err(AppError::ValidationError(
                "Auto-deploy needs a git provider with push-webhook support — configure one to receive push events"
                    .into(),
            ));
        }
    } else if auto_deploy {
        return Err(AppError::ValidationError(
            "Auto-deploy needs a git provider with push-webhook support — configure one to receive push events".into(),
        ));
    } else if request.deploy_type == "git" && !keep_upload {
        // No saved provider: extract any `user:token@host` credentials the user pasted
        // directly in the URL, validate and store only the stripped URL, and encrypt the
        // extracted credentials (Feature B) — or leave a public URL fully credential-free
        // (Feature A).
        let raw_url = request.git_url.as_deref().unwrap_or_default();
        let (clean_url, embedded) = crate::git::git_providers::split_embedded_credentials(raw_url)
            .map_err(AppError::ValidationError)?;
        crate::security::ssrf::parse_https_url(&clean_url).map_err(AppError::ValidationError)?;
        match embedded {
            Some((username, password)) => {
                let key = crate::utils::security::get_encryption_key();
                let encrypted = crate::utils::security::encrypt_data(&password, &key)
                    .map_err(AppError::InternalError)?;
                git_credentials_username = Some(username);
                git_credentials_password_encrypted = Some(encrypted);
            }
            None => {
                // No new credentials pasted. Keep whatever was saved before only when this
                // save still targets the same repo — an unrelated field change (e.g. the
                // container port) must not silently drop a saved token, and pointing the
                // app at a different repo must not silently reuse an old one.
                let (previous_url, previous_username, previous_encrypted) =
                    current_git_credentials(state, application_id).await?;
                if previous_url.as_deref() == Some(clean_url.as_str()) {
                    git_credentials_username = previous_username;
                    git_credentials_password_encrypted = previous_encrypted;
                }
            }
        }
        request.git_url = Some(clean_url);
    }
    // Registry-image deploys: only save credentials when a private registered
    // image is set. A fresh non-empty password always wins; an empty one keeps
    // the previously saved credentials only while `image_url` is unchanged.
    let mut registry_username = request.registry_username.clone();
    let mut registry_password_encrypted: Option<String> = None;
    let image_url = request
        .image_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty());
    if let Some(url) = image_url {
        let fresh_password = request
            .registry_password
            .as_deref()
            .map(str::trim)
            .filter(|password| !password.is_empty());
        match fresh_password {
            Some(password) => {
                let key = crate::utils::security::get_encryption_key();
                registry_password_encrypted = Some(
                    crate::utils::security::encrypt_data(password, &key)
                        .map_err(AppError::InternalError)?,
                );
                if registry_username
                    .as_deref()
                    .map(str::trim)
                    .unwrap_or("")
                    .is_empty()
                {
                    registry_username = None;
                }
            }
            None => {
                let (previous_url, previous_username, previous_encrypted) =
                    current_registry_credentials(state, application_id).await?;
                if previous_url.as_deref() == Some(url) {
                    registry_username = previous_username;
                    registry_password_encrypted = previous_encrypted;
                }
            }
        }
        request.image_url = Some(url.to_string());
    }
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE applications SET deploy_type = ?1, build_method = ?2, git_provider_id = ?3, git_url = ?4, git_branch = ?5, dockerfile_path = ?6, compose_file_path = ?7, compose_public_service = ?8, container_port = ?9, railpack_project_path = ?10, auto_deploy = ?11, git_repo_full_name = ?12, build_cpu_cores = ?13, build_memory_mib = ?14, build_swap_mib = ?15, git_credentials_username = ?16, git_credentials_password_encrypted = ?17, watch_paths = ?18, railpack_start_command = ?19, image_url = ?20, registry_username = ?21, registry_password_encrypted = ?22, restart_policy = COALESCE(?23, restart_policy), source_upload = ?24, updated_at = ?25 WHERE id = ?26",
    )
    .bind(request.deploy_type)
    .bind(request.build_method)
    .bind(request.git_provider_id)
    .bind(request.git_url.clone())
    .bind(request.git_branch)
    .bind(request.dockerfile_path)
    .bind(request.compose_file_path)
    .bind(request.compose_public_service)
    .bind(request.container_port)
    .bind(
        request
            .railpack_project_path
            .as_deref()
            .filter(|path| !path.trim().is_empty())
            .unwrap_or(".")
            .to_string(),
    )
    .bind(i64::from(auto_deploy))
    .bind(request.git_url.as_deref().and_then(repo_full_name))
    .bind(request.build_cpu_cores)
    .bind(request.build_memory_mib)
    .bind(request.build_swap_mib)
    .bind(git_credentials_username)
    .bind(git_credentials_password_encrypted)
    .bind(
        request
            .watch_paths
            .as_deref()
            .map(str::trim)
            .filter(|paths| !paths.is_empty()),
    )
    .bind(
        request
            .railpack_start_command
            .as_deref()
            .map(str::trim)
            .filter(|command| !command.is_empty()),
    )
    .bind(request.image_url)
    .bind(registry_username)
    .bind(registry_password_encrypted)
    .bind(request.restart_policy.clone())
    .bind(i64::from(keep_upload))
    .bind(now_ms())
    .bind(application_id)
    .execute(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    // A saved source no longer matches the running container.
    sqlx::query(
        "UPDATE application_deployments SET config_stale = 1, updated_at = ?1 WHERE application_id = ?2 AND status = 'running'",
    )
    .bind(now_ms())
    .bind(application_id)
    .execute(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    get(state, application_id).await
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
        let path = std::env::temp_dir().join(format!("litepod-source-{}.db", Uuid::now_v7()));
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
        sqlx::query("INSERT INTO projects (id, organization_id, name, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)").bind("project-1").bind("organization-1").bind("Project").bind(1_i64).bind(1_i64).execute(&conn).await.unwrap();
        sqlx::query("INSERT INTO environments (id, project_id, name, is_production, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)").bind("environment-1").bind("project-1").bind("prod").bind(1_i64).bind(1_i64).bind(1_i64).execute(&conn).await.unwrap();
        sqlx::query("INSERT INTO applications (id, environment_id, name, server_id, created_at, updated_at) VALUES (?1, ?2, ?3, 'local', ?4, ?5)").bind("application-1").bind("environment-1").bind("Application").bind(1_i64).bind(1_i64).execute(&conn).await.unwrap();
        sqlx::query("INSERT INTO git_providers (id, organization_id, name, provider_type, host_url, username, password_encrypted, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)").bind("provider-1").bind("organization-1").bind("GitHub").bind("github").bind("https://github.com").bind("octo").bind("encrypted").bind(1_i64).bind(1_i64).execute(&conn).await.unwrap();
        (state, path)
    }

    fn cleanup(path: std::path::PathBuf) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{}", path.display(), suffix));
        }
    }

    fn git_request() -> UpdateSourceRequest {
        UpdateSourceRequest {
            deploy_type: "git".into(),
            build_method: "docker".into(),
            git_provider_id: Some("provider-1".into()),
            git_url: Some("https://github.com/octo/repo.git".into()),
            git_branch: Some("main".into()),
            dockerfile_path: Some("./Dockerfile".into()),
            container_port: Some(3000),
            auto_deploy: None,
            watch_paths: None,
            compose_file_path: None,
            compose_public_service: None,
            railpack_project_path: Some(".".into()),
            railpack_start_command: None,
            build_cpu_cores: None,
            build_memory_mib: None,
            build_swap_mib: None,
            image_url: None,
            registry_username: None,
            registry_password: None,
            restart_policy: None,
        }
    }

    #[tokio::test]
    async fn deploy_hook_token_is_stable_until_rotated() {
        let (state, path) = test_state().await;
        let first = super::super::deploy_hook::ensure_token(&state, "application-1")
            .await
            .unwrap();
        assert!(
            first.len() >= 40,
            "token must be unguessable, got {} chars",
            first.len()
        );
        assert_eq!(
            super::super::deploy_hook::ensure_token(&state, "application-1")
                .await
                .unwrap(),
            first,
            "reused until rotated"
        );

        let rotated = super::super::deploy_hook::rotate_token(&state, "application-1")
            .await
            .unwrap();
        assert_ne!(rotated, first);
        assert_eq!(
            super::super::deploy_hook::application_for_token(&state, &rotated)
                .await
                .unwrap(),
            "application-1"
        );
        assert!(
            super::super::deploy_hook::application_for_token(&state, &first)
                .await
                .is_err(),
            "rotation revokes the old URL"
        );
        cleanup(path);
    }

    #[test]
    fn repo_full_name_is_derived_from_clone_urls() {
        assert_eq!(
            repo_full_name("https://github.com/octo/repo.git").as_deref(),
            Some("octo/repo")
        );
        assert_eq!(
            repo_full_name("https://github.com/octo/repo").as_deref(),
            Some("octo/repo")
        );
        assert_eq!(
            repo_full_name("https://git.example.test/organization/sub/app.git").as_deref(),
            Some("sub/app")
        );
        assert_eq!(repo_full_name("nonsense"), None);
    }

    #[test]
    fn runtime_limits_must_be_paired_and_positive() {
        assert!(validate_runtime_limits(None, None).is_ok());
        assert!(validate_runtime_limits(Some(0.5), Some(256)).is_ok());
        assert!(validate_runtime_limits(Some(0.5), None).is_err());
        assert!(validate_runtime_limits(None, Some(256)).is_err());
        assert!(validate_runtime_limits(Some(0.0), Some(256)).is_err());
        assert!(validate_runtime_limits(Some(f64::NAN), Some(256)).is_err());
        assert!(validate_runtime_limits(Some(0.5), Some(0)).is_err());
    }

    #[tokio::test]
    async fn auto_deploy_requires_a_push_webhook_provider() {
        let (state, path) = test_state().await;
        // Switch to provider_type = 'generic' — no webhook, should be rejected.
        let conn = crate::git::repositories::db::get_conn(&state).await;
        sqlx::query(
            "UPDATE git_providers SET provider_type = 'generic', kind = 'credentials' WHERE id = 'provider-1'",
        )
        .execute(&conn)
        .await
        .unwrap();
        let mut request = git_request();
        request.auto_deploy = Some(true);
        let result = save(&state, "application-1", request).await;
        assert!(
            result.is_err(),
            "generic provider should reject auto-deploy, got {result:?}"
        );

        // Promote to a webhook-capable type — allowed.
        sqlx::query("UPDATE git_providers SET provider_type = 'forgejo' WHERE id = 'provider-1'")
            .execute(&conn)
            .await
            .unwrap();
        let mut request = git_request();
        request.auto_deploy = Some(true);
        let saved = save(&state, "application-1", request).await.unwrap();
        assert!(saved.auto_deploy);
        cleanup(path);
    }

    #[tokio::test]
    async fn default_source_is_git() {
        let (state, path) = test_state().await;
        let source = get(&state, "application-1").await.unwrap();
        assert_eq!(source.deploy_type, "git");
        assert!(source.git_url.is_none());
        assert!(source.clean_build);
        cleanup(path);
    }

    #[tokio::test]
    async fn git_source_round_trips() {
        let (state, path) = test_state().await;
        let mut request = git_request();
        request.railpack_start_command = Some("node ./dist/server/entry.mjs".into());
        let saved = save(&state, "application-1", request).await.unwrap();
        assert_eq!(saved.deploy_type, "git");
        assert_eq!(saved.git_branch.as_deref(), Some("main"));
        assert_eq!(saved.container_port, Some(3000));
        assert_eq!(
            saved.railpack_start_command.as_deref(),
            Some("node ./dist/server/entry.mjs")
        );
        cleanup(path);
    }

    #[tokio::test]
    async fn blank_railpack_start_command_is_stored_as_none() {
        let (state, path) = test_state().await;
        let mut request = git_request();
        request.railpack_start_command = Some("  \n".into());
        let saved = save(&state, "application-1", request).await.unwrap();
        assert!(saved.railpack_start_command.is_none());
        cleanup(path);
    }

    #[tokio::test]
    async fn railpack_start_command_has_a_bounded_length() {
        let (state, path) = test_state().await;
        let mut request = git_request();
        request.railpack_start_command = Some("x".repeat(2049));
        assert!(save(&state, "application-1", request).await.is_err());
        cleanup(path);
    }

    #[tokio::test]
    async fn public_repo_with_no_provider_deploys_credential_free() {
        let (state, path) = test_state().await;
        let mut request = git_request();
        request.git_provider_id = None;
        request.git_url = Some("https://github.com/octocat/hello-world.git".into());
        let saved = save(&state, "application-1", request).await.unwrap();
        assert!(saved.git_provider_id.is_none());
        assert!(!saved.has_git_credentials);
        assert_eq!(
            saved.git_url.as_deref(),
            Some("https://github.com/octocat/hello-world.git")
        );
        cleanup(path);
    }

    #[tokio::test]
    async fn embedded_url_credentials_are_extracted_encrypted_and_stripped() {
        let (state, path) = test_state().await;
        let mut request = git_request();
        request.git_provider_id = None;
        request.git_url = Some("https://octo:ghp_secret@github.com/octo/private-repo.git".into());
        let saved = save(&state, "application-1", request).await.unwrap();
        assert!(saved.has_git_credentials);
        let clean_url = saved.git_url.clone().unwrap();
        assert_eq!(clean_url, "https://github.com/octo/private-repo.git");
        assert!(
            !clean_url.contains('@'),
            "stored URL must not carry credentials"
        );

        // Re-saving with only an unrelated field changed (same repo, no new
        // credentials pasted) must not drop the previously saved token.
        let mut resave = git_request();
        resave.git_provider_id = None;
        resave.git_url = Some(clean_url.clone());
        resave.container_port = Some(4000);
        let resaved = save(&state, "application-1", resave).await.unwrap();
        assert!(
            resaved.has_git_credentials,
            "unrelated field-only resave must keep the saved token"
        );
        assert_eq!(resaved.container_port, Some(4000));

        // Pointing at a different repo without new credentials clears the old token.
        let mut retarget = git_request();
        retarget.git_provider_id = None;
        retarget.git_url = Some("https://github.com/octo/other-repo.git".into());
        let retargeted = save(&state, "application-1", retarget).await.unwrap();
        assert!(
            !retargeted.has_git_credentials,
            "switching repos must not silently reuse the old token"
        );
        cleanup(path);
    }

    #[tokio::test]
    async fn build_resource_limits_round_trip_and_require_a_pair() {
        let (state, path) = test_state().await;
        let mut request = git_request();
        request.build_cpu_cores = Some(1.5);
        request.build_memory_mib = Some(2048);
        request.build_swap_mib = Some(512);
        let saved = save(&state, "application-1", request).await.unwrap();
        assert_eq!(saved.build_cpu_cores, Some(1.5));
        assert_eq!(saved.build_memory_mib, Some(2048));
        assert_eq!(saved.build_swap_mib, Some(512));

        let mut incomplete = git_request();
        incomplete.build_cpu_cores = Some(1.0);
        let error = save(&state, "application-1", incomplete).await.unwrap_err();
        assert!(error.to_string().contains("both build CPU and memory"));
        cleanup(path);
    }

    #[tokio::test]
    async fn build_swap_rejects_negative_or_orphan_values() {
        let (state, path) = test_state().await;
        let mut request = git_request();
        request.build_swap_mib = Some(-1);
        let error = save(&state, "application-1", request).await.unwrap_err();
        assert!(error.to_string().contains("swap limit"));

        let mut request = git_request();
        request.build_swap_mib = Some(512);
        let error = save(&state, "application-1", request).await.unwrap_err();
        assert!(error.to_string().contains("Build swap requires"));
        cleanup(path);
    }

    #[tokio::test]
    async fn git_source_requires_all_fields() {
        let (state, path) = test_state().await;
        let mut request = git_request();
        request.git_branch = None;
        request.container_port = None;
        let result = save(&state, "application-1", request).await;
        match result {
            Err(AppError::ValidationError(message)) => {
                assert!(
                    message.contains("branch"),
                    "names the missing branch: {message}"
                );
                assert!(
                    message.contains("container port"),
                    "names the missing port: {message}"
                );
            }
            other => panic!("expected ValidationError, got {other:?}"),
        }
        cleanup(path);
    }

    #[tokio::test]
    async fn git_source_must_use_provider_host() {
        let (state, path) = test_state().await;
        let mut request = git_request();
        request.git_url = Some("https://attacker.example/octo/repo.git".into());
        let result = save(&state, "application-1", request).await;
        assert!(
            matches!(result, Err(AppError::ValidationError(ref message)) if message.contains("provider host")),
            "got {result:?}"
        );
        cleanup(path);
    }

    #[tokio::test]
    async fn git_source_rejects_metadata_and_insecure_urls() {
        let (state, path) = test_state().await;
        for git_url in [
            "https://169.254.169.254/octo/repo.git",
            "https://127.0.0.1/octo/repo.git",
            "http://github.com/octo/repo.git",
            "https://user:pw@github.com/octo/repo.git",
            "https://github.com:8443/octo/repo.git",
        ] {
            let mut request = git_request();
            request.git_url = Some(git_url.into());
            let result = save(&state, "application-1", request).await;
            assert!(
                matches!(result, Err(AppError::ValidationError(_))),
                "accepted {git_url}: {result:?}"
            );
        }
        cleanup(path);
    }

    #[tokio::test]
    async fn dockerfile_path_cannot_escape_the_repo() {
        let (state, path) = test_state().await;
        let mut request = git_request();
        request.dockerfile_path = Some("../secrets/Dockerfile".into());
        assert!(matches!(
            save(&state, "application-1", request).await,
            Err(AppError::ValidationError(_))
        ));
        let mut absolute = git_request();
        absolute.dockerfile_path = Some("/etc/Dockerfile".into());
        assert!(matches!(
            save(&state, "application-1", absolute).await,
            Err(AppError::ValidationError(_))
        ));
        cleanup(path);
    }

    #[tokio::test]
    async fn compose_source_requires_file_service_and_port() {
        let (state, path) = test_state().await;
        let mut request = git_request();
        request.build_method = "compose".into();
        request.dockerfile_path = None;
        request.compose_file_path = None;
        request.compose_public_service = None;
        request.container_port = None;
        let error = save(&state, "application-1", request).await.unwrap_err();
        assert!(error.to_string().contains("Compose file path"));
        cleanup(path);
    }

    #[tokio::test]
    async fn railpack_source_accepts_no_file_path() {
        let (state, path) = test_state().await;
        let mut request = git_request();
        request.build_method = "railpack".into();
        request.dockerfile_path = None;
        let saved = save(&state, "application-1", request).await.unwrap();
        assert_eq!(saved.build_method, "railpack");
        cleanup(path);
    }

    #[tokio::test]
    async fn clean_build_is_persisted_only_for_railpack_sources() {
        let (state, path) = test_state().await;
        let mut request = git_request();
        request.build_method = "railpack".into();
        request.dockerfile_path = None;
        save(&state, "application-1", request).await.unwrap();

        let saved = set_clean_build(&state, "application-1", false)
            .await
            .unwrap();
        assert!(!saved.clean_build);
        assert!(!get(&state, "application-1").await.unwrap().clean_build);

        let error = set_clean_build(&state, "missing", false).await.unwrap_err();
        assert!(matches!(error, AppError::ApplicationNotFound));
        cleanup(path);
    }

    #[tokio::test]
    async fn deploy_clean_build_is_applied_only_to_railpack() {
        let (state, path) = test_state().await;

        let mut railpack = git_request();
        railpack.build_method = "railpack".into();
        railpack.dockerfile_path = None;
        save(&state, "application-1", railpack).await.unwrap();
        apply_clean_build_for_deploy(&state, "application-1", false)
            .await
            .unwrap();
        assert!(!get(&state, "application-1").await.unwrap().clean_build);

        cleanup(path);
    }

    #[tokio::test]
    async fn deploy_clean_build_is_ignored_for_static_docker_and_compose() {
        let (state, path) = test_state().await;

        assert!(get(&state, "application-1").await.unwrap().clean_build);
        apply_clean_build_for_deploy(&state, "application-1", false)
            .await
            .unwrap();
        assert!(get(&state, "application-1").await.unwrap().clean_build);

        save(&state, "application-1", git_request()).await.unwrap();
        apply_clean_build_for_deploy(&state, "application-1", false)
            .await
            .unwrap();
        assert!(get(&state, "application-1").await.unwrap().clean_build);

        let mut compose = git_request();
        compose.build_method = "compose".into();
        compose.dockerfile_path = None;
        compose.compose_file_path = Some("compose.yml".into());
        compose.compose_public_service = Some("web".into());
        save(&state, "application-1", compose).await.unwrap();
        apply_clean_build_for_deploy(&state, "application-1", false)
            .await
            .unwrap();
        assert!(get(&state, "application-1").await.unwrap().clean_build);

        cleanup(path);
    }

    #[tokio::test]
    async fn compose_path_cannot_escape_the_repo() {
        let (state, path) = test_state().await;
        let mut request = git_request();
        request.build_method = "compose".into();
        request.compose_file_path = Some("../compose.yml".into());
        request.compose_public_service = Some("web".into());
        assert!(matches!(
            save(&state, "application-1", request).await,
            Err(AppError::ValidationError(_))
        ));
        cleanup(path);
    }

    #[tokio::test]
    async fn unknown_build_method_is_rejected() {
        let (state, path) = test_state().await;
        let mut request = git_request();
        request.build_method = "buildpacks".into();
        assert!(matches!(
            save(&state, "application-1", request).await,
            Err(AppError::ValidationError(_))
        ));
        cleanup(path);
    }

    #[tokio::test]
    async fn unknown_provider_is_rejected() {
        let (state, path) = test_state().await;
        let mut request = git_request();
        request.git_provider_id = Some("missing".into());
        assert!(matches!(
            save(&state, "application-1", request).await,
            Err(AppError::ProviderNotFound)
        ));
        cleanup(path);
    }

    #[tokio::test]
    async fn image_source_requires_reference_and_port() {
        let (state, path) = test_state().await;
        let mut request = git_request();
        request.deploy_type = "image".into();
        request.image_url = None;
        request.container_port = None;
        let error = save(&state, "application-1", request).await.unwrap_err();
        assert!(error.to_string().contains("image reference"));
        assert!(error.to_string().contains("container port"));
        cleanup(path);
    }

    #[tokio::test]
    async fn image_source_saves_and_keeps_registry_credentials() {
        let (state, path) = test_state().await;
        let mut request = git_request();
        request.deploy_type = "image".into();
        request.image_url = Some("ghcr.io/octo/app:v1".into());
        request.registry_username = Some("octo".into());
        request.registry_password = Some("s3cret".into());
        let saved = save(&state, "application-1", request).await.unwrap();
        assert_eq!(saved.deploy_type, "image");
        assert_eq!(saved.image_url.as_deref(), Some("ghcr.io/octo/app:v1"));
        assert!(saved.has_registry_credentials);

        // Unchanged image + empty password keep the previously saved credentials.
        let mut again = git_request();
        again.deploy_type = "image".into();
        again.image_url = Some("ghcr.io/octo/app:v1".into());
        let again = save(&state, "application-1", again).await.unwrap();
        assert_eq!(again.image_url.as_deref(), Some("ghcr.io/octo/app:v1"));
        assert!(again.has_registry_credentials);
        cleanup(path);
    }

    #[tokio::test]
    async fn switching_away_from_image_clears_registry_credentials() {
        let (state, path) = test_state().await;
        let mut request = git_request();
        request.deploy_type = "image".into();
        request.image_url = Some("ghcr.io/octo/app:v1".into());
        request.registry_username = Some("octo".into());
        request.registry_password = Some("s3cret".into());
        save(&state, "application-1", request).await.unwrap();
        assert!(
            get(&state, "application-1")
                .await
                .unwrap()
                .has_registry_credentials
        );

        let mut switched = git_request();
        switched.deploy_type = "git".into();
        let saved = save(&state, "application-1", switched).await.unwrap();
        assert_eq!(saved.deploy_type, "git");
        assert!(!saved.has_registry_credentials);
        assert!(saved.image_url.is_none());
        cleanup(path);
    }

    #[tokio::test]
    async fn restart_policy_defaults_to_unless_stopped_and_persists() {
        let (state, path) = test_state().await;
        let mut selected = git_request();
        selected.restart_policy = Some("on-failure".into());
        let saved = save(&state, "application-1", selected).await.unwrap();
        assert_eq!(saved.restart_policy, "on-failure");

        // A save that omits the policy keeps the stored one.
        let again = save(&state, "application-1", git_request()).await.unwrap();
        assert_eq!(again.restart_policy, "on-failure");

        let mut invalid = git_request();
        invalid.restart_policy = Some("nope".into());
        assert!(matches!(
            save(&state, "application-1", invalid).await,
            Err(AppError::ValidationError(_))
        ));
        cleanup(path);
    }

    #[tokio::test]
    async fn saving_source_flags_a_running_deployment_stale() {
        let (state, path) = test_state().await;
        use sqlx::Row;
        let conn = crate::git::repositories::db::get_conn(&state).await;
        sqlx::query("INSERT INTO application_deployments (application_id, status, container_name, hostname, updated_at) VALUES ('application-1', 'running', 'litepod-git-application-1', '', 1)").execute(&conn).await.unwrap();
        save(&state, "application-1", git_request()).await.unwrap();
        let stale: i64 = sqlx::query("SELECT config_stale FROM application_deployments WHERE application_id = 'application-1'")
            .fetch_one(&conn)
            .await
            .unwrap()
            .try_get(0)
            .unwrap();
        assert_eq!(stale, 1);
        cleanup(path);
    }
}
