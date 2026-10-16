//! Registry-image deployments routed through the shared queue.
//!
//! `deploy_type = 'image'` pulls a prebuilt image (public or private registry)
//! and hands it to the same rollout path as built Docker/Railpack images, so
//! env vars, volumes, domains, resource limits and the ingress swap behave
//! identically to every other source. Local deployments pull through the
//! Podman API; remote servers pull over SSH with credentials passed inline
//! (`podman pull --creds user:pass`), so private registries work on any host.

use std::ffi::OsString;

use chrono::Utc;
use futures_util::StreamExt;
use podman_api::{
    Podman,
    opts::{PullOptsBuilder, RegistryAuth},
};

use crate::{
    deployment::{domains, history, queue, source},
    dtos::{deployment::SourceConfigDto, response::AppError},
    git::repositories::db::get_conn,
    state::state::SharedState,
};

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

async fn log_stage(
    state: &SharedState,
    application_id: &str,
    run_id: &str,
    level: &str,
    message: &str,
) {
    if level == "error" {
        tracing::error!(application_id, "[image] {message}");
    } else {
        tracing::info!(application_id, "[image] {message}");
    }
    let _ = history::append(state, run_id, level, message).await;
}

async fn image_reference(source: &SourceConfigDto) -> Result<String, AppError> {
    match source
        .image_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
    {
        Some(url) => Ok(url.to_string()),
        None => Err(AppError::ValidationError(
            "Set an image reference before deploying".into(),
        )),
    }
}

/// Resolves (and decrypts) the credentials saved for a private registry image.
/// `None` means the image is public or no credentials are on file.
async fn registry_credentials(
    state: &SharedState,
    application_id: &str,
) -> Result<Option<(String, String)>, AppError> {
    use sqlx::Row;
    let conn = get_conn(state).await;
    let row = sqlx::query(
        "SELECT registry_username, registry_password_encrypted FROM applications WHERE id = ?1",
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
    let Some(username) = username else {
        return Ok(None);
    };
    let Some(encrypted) = encrypted else {
        return Ok(None);
    };
    let key = crate::utils::security::get_encryption_key();
    let password =
        crate::utils::security::decrypt_data(&encrypted, &key).map_err(AppError::InternalError)?;
    Ok(Some((username, password)))
}

/// Inserts the `deployment_jobs` row the queue worker picks up, mirroring the
/// other sources so the dashboard sees the job, snapshot and run the same way.
pub async fn enqueue(
    state: &SharedState,
    organization_id: &str,
    project_id: &str,
    environment_id: &str,
    application_id: &str,
    triggered_by: &str,
    created_by: Option<&str>,
) -> Result<queue::DeploymentJobDto, AppError> {
    let source_config = source::get(state, application_id).await?;
    if source_config.deploy_type != "image" {
        return Err(AppError::ValidationError(
            "Image enqueue called on a non-image application".into(),
        ));
    }
    image_reference(&source_config).await?;
    super::ensure_deployment(state, application_id).await?;
    let run_id = history::start(
        state,
        application_id,
        "image",
        "image",
        triggered_by,
        None,
        created_by,
    )
    .await?;
    let snapshot = serde_json::to_string(&source_config)
        .map_err(|e| AppError::InternalError(e.to_string()))?;
    let conn = get_conn(state).await;
    crate::git::repositories::db::retry_on_stale_snapshot(|| async {
        sqlx::query(
            "UPDATE application_deployments SET status = 'deploying', last_error = NULL, updated_at = ?1 WHERE application_id = ?2",
        )
        .bind(now_ms())
        .bind(application_id)
        .execute(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))
    })
    .await?;
    queue::enqueue(
        state,
        queue::EnqueueJobInput {
            application_id: application_id.into(),
            project_id: project_id.into(),
            environment_id: environment_id.into(),
            organization_id: organization_id.into(),
            source_type: "image".into(),
            source_snapshot: snapshot.clone(),
            build_method: "image".into(),
            build_snapshot: snapshot,
            run_id,
        },
    )
    .await
}

/// `podman pull` argv for the remote host. Credentials travel inline via
/// `--creds` (USERNAME:PASSWORD) so no `podman login` handshake is needed on
/// the target; `None` means the image is public.
fn remote_pull_args(image: &str, creds: Option<&(String, String)>) -> Vec<OsString> {
    let mut args = vec![OsString::from("pull")];
    if let Some((username, password)) = creds {
        args.push(OsString::from("--creds"));
        args.push(OsString::from(format!("{username}:{password}")));
    }
    args.push(OsString::from(image));
    args
}

/// Remote-server half of `run`: pulls the image over SSH, then hands the rollout
/// to the shared `deploy_image` path (which already runs `podman run` remotely).
async fn run_remote(
    state: &SharedState,
    application_id: &str,
    run_id: &str,
    source_config: &SourceConfigDto,
    image: &str,
    runtime: &crate::servers::runtime::ServerRuntime,
) -> Result<(), String> {
    let creds = match registry_credentials(state, application_id).await {
        Ok(Some((username, password))) => {
            log_stage(
                state,
                application_id,
                run_id,
                "info",
                "Using saved registry credentials for a private image",
            )
            .await;
            Some((username, password))
        }
        Ok(None) => None,
        Err(error) => return Err(error.to_string()),
    };
    let server = runtime.remote_server_name().unwrap_or("selected server");
    log_stage(
        state,
        application_id,
        run_id,
        "info",
        &format!("Pulling {image} on {server}"),
    )
    .await;
    runtime
        .podman(&remote_pull_args(image, creds.as_ref()))
        .await
        .map_err(|e| format!("Could not pull {image} on {server}: {e}"))?;
    let _ = history::set_status(state, run_id, "deploying").await;
    crate::deployment::rollout::deploy_image(state, application_id, source_config, image, runtime)
        .await
}

/// Worker entry point. Pulls the configured image (with credentials when a
/// private registry is set), then rolls it out exactly like a built image.
/// Returns the error message verbatim so the queue worker surfaces it.
pub async fn run(state: &SharedState, application_id: &str, run_id: &str) -> Result<(), String> {
    let source_config = source::get(state, application_id)
        .await
        .map_err(|e| e.to_string())?;
    if source_config.deploy_type != "image" {
        return Err("Image run called on a non-image application".into());
    }
    let image = image_reference(&source_config)
        .await
        .map_err(|e| e.to_string())?;

    log_stage(
        state,
        application_id,
        run_id,
        "info",
        "Registry image deploy started",
    )
    .await;

    // A fresh application has no domains yet; seed one so it is reachable.
    domains::seed_if_missing(state, application_id)
        .await
        .map_err(|e| e.to_string())?;
    let conn = get_conn(state).await;
    sqlx::query(
        "UPDATE application_deployments SET status = 'deploying', last_error = NULL, updated_at = ?1 WHERE application_id = ?2",
    )
    .bind(now_ms())
    .bind(application_id)
    .execute(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))
    .map_err(|e| e.to_string())?;
    let _ = history::set_status(state, run_id, "deploying").await;

    let runtime = crate::servers::runtime::ServerRuntime::for_application(state, application_id)
        .await
        .map_err(|error| error.to_string())?;
    let _ = history::append(
        state,
        run_id,
        "info",
        &super::resource_log_line(&source_config),
    )
    .await;

    if runtime.is_remote() {
        return run_remote(
            state,
            application_id,
            run_id,
            &source_config,
            &image,
            &runtime,
        )
        .await;
    }

    let podman =
        Podman::new(&state.podman_socket).map_err(|e| format!("Invalid Podman socket: {e}"))?;
    if let Ok(info) = podman.images().get(&image).inspect().await {
        // Already present; skip the network pull (matches `podman run`).
        if info.id.is_some() {
            log_stage(
                state,
                application_id,
                run_id,
                "info",
                &format!("Image {image} already present locally"),
            )
            .await;
        }
    }

    let auth = match registry_credentials(state, application_id).await {
        Ok(Some((username, password))) => {
            let mut registry_builder = RegistryAuth::builder();
            registry_builder.username(username).password(password);
            log_stage(
                state,
                application_id,
                run_id,
                "info",
                "Using saved registry credentials for a private image",
            )
            .await;
            Some(registry_builder.build())
        }
        Ok(None) => None,
        Err(error) => return Err(error.to_string()),
    };
    let mut pull_builder = PullOptsBuilder::default();
    pull_builder = pull_builder.reference(&image);
    if let Some(auth) = auth {
        let _ = pull_builder.auth(auth);
    }
    let images = podman.images();
    let _ = history::append(state, run_id, "info", &format!("Pulling {image}")).await;
    let pull_opts = pull_builder.build();
    let mut pull = images.pull(&pull_opts);
    while let Some(report) = pull.next().await {
        report.map_err(|error| format!("Could not pull {image}: {error}"))?;
    }
    podman
        .images()
        .get(&image)
        .inspect()
        .await
        .map_err(|e| format!("Image pull finished but {image} is not available locally: {e}"))?;

    let _ = history::set_status(state, run_id, "deploying").await;
    crate::deployment::rollout::deploy_image(
        state,
        application_id,
        &source_config,
        &image,
        &runtime,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flatten(args: &[OsString]) -> Vec<String> {
        args.iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn remote_pull_without_credentials_is_a_plain_pull() {
        assert_eq!(
            flatten(&remote_pull_args("ghcr.io/acme/app:latest", None)),
            vec!["pull".to_string(), "ghcr.io/acme/app:latest".to_string()]
        );
    }

    #[test]
    fn remote_pull_inlines_registry_credentials_as_creds_flag() {
        // The password itself may contain ':' and is passed as one argv pair,
        // so the remote side never parses it through a shell.
        let creds = ("robot".to_string(), "s3cr3t:2026".to_string());
        assert_eq!(
            flatten(&remote_pull_args("docker.io/acme/app:1.2.3", Some(&creds))),
            vec![
                "pull".to_string(),
                "--creds".to_string(),
                "robot:s3cr3t:2026".to_string(),
                "docker.io/acme/app:1.2.3".to_string(),
            ]
        );
    }
}
