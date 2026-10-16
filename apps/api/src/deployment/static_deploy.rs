//! Static (uploaded-artifact) deployments routed through the shared queue.
//!
//! Previously, the static branch in `mod::deploy` ran inline so a deploy
//! request blocked on a Caddy container recreate. The queue worker now owns
//! the lifecycle: this module provides the `enqueue` half (insert a
//! `deployment_jobs` row alongside the `application_deployment_runs` history
//! row) and the `run` half (recreate the Caddy container for a previously
//! uploaded artifact) that the worker calls after claiming the job.

use chrono::Utc;
use futures_util::StreamExt;
use podman_api::{
    Podman, models,
    opts::{ContainerCreateOptsBuilder, ContainerDeleteOpts, PullOpts},
};
use std::{collections::HashMap, ffi::OsString};

use crate::{
    deployment::{domains, history, managed_volumes, queue, source},
    dtos::{deployment::SourceConfigDto, response::AppError},
    git::repositories::db::get_conn,
    state::state::SharedState,
};

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

/// Pulls `image` if Podman doesn't already have it locally. `containers().create`
/// (unlike the `podman run` CLI used on the SSH/remote path below) never pulls
/// implicitly, so a fresh host without the Caddy image would otherwise fail
/// static deploys with a 404 "no such image".
async fn ensure_image_pulled(podman: &Podman, image: &str) -> Result<(), String> {
    let images = podman.images();
    if images.get(image).inspect().await.is_ok() {
        return Ok(());
    }
    let mut pull = images.pull(&PullOpts::builder().reference(image).build());
    while let Some(report) = pull.next().await {
        report.map_err(|error| format!("Could not pull {image}: {error}"))?;
    }
    Ok(())
}

async fn log_stage(
    state: &SharedState,
    application_id: &str,
    run_id: &str,
    level: &str,
    message: &str,
) {
    if level == "error" {
        tracing::error!(application_id, "[static] {message}");
    } else {
        tracing::info!(application_id, "[static] {message}");
    }
    let _ = history::append(state, run_id, level, message).await;
}

/// Inserts the `deployment_jobs` row that the queue worker picks up. Mirrors
/// `compose_standalone::enqueue` so the dashboard sees the job the same way
/// it sees Git and Compose deploys.
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
    if source_config.deploy_type != "static" {
        return Err(AppError::ValidationError(
            "Static enqueue called on a non-static application".into(),
        ));
    }
    super::ensure_deployment(state, application_id).await?;
    let conn = get_conn(state).await;
    let has_artifact =
        sqlx::query("SELECT 1 FROM application_artifacts WHERE application_id = ?1 LIMIT 1")
            .bind(application_id)
            .fetch_optional(&conn)
            .await
            .map_err(|e| AppError::DatabaseError(e.to_string()))?
            .is_some();
    if !has_artifact {
        return Err(AppError::ValidationError(
            "Upload an artifact before deploying".into(),
        ));
    }
    let run_id = history::start(
        state,
        application_id,
        "static",
        "static",
        triggered_by,
        None,
        created_by,
    )
    .await?;
    // Record which artifact this run actually deploys - rollback later needs
    // this to redeploy the artifact that was live *then*, not whatever is
    // current *now*. A plain deploy just records today's current pointer;
    // `rollback` (below) sets that pointer to an old artifact first, so this
    // ends up recording the rolled-back-to artifact too.
    use sqlx::Row;
    if let Some(current_artifact_id) =
        sqlx::query("SELECT artifact_id FROM application_deployments WHERE application_id = ?1")
            .bind(application_id)
            .fetch_optional(&conn)
            .await
            .map_err(|e| AppError::DatabaseError(e.to_string()))?
            .and_then(|row| row.try_get::<Option<String>, _>(0).ok().flatten())
    {
        history::set_artifact(state, &run_id, &current_artifact_id).await?;
    }
    let snapshot = serde_json::to_string(&source_config)
        .map_err(|e| AppError::InternalError(e.to_string()))?;
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
            source_type: "static".into(),
            source_snapshot: snapshot.clone(),
            build_method: "static".into(),
            build_snapshot: snapshot,
            run_id,
        },
    )
    .await
}

/// Redeploys the exact artifact a previous successful run deployed. Points
/// the application at that old artifact first (the same pointer a fresh
/// upload would move), then the normal enqueue path runs unchanged - no
/// rebuild, since the artifact itself is just a static file bundle.
pub async fn enqueue_rollback(
    state: &SharedState,
    organization_id: &str,
    project_id: &str,
    environment_id: &str,
    application_id: &str,
    target_artifact_id: &str,
    created_by: Option<&str>,
) -> Result<queue::DeploymentJobDto, AppError> {
    let conn = get_conn(state).await;
    let exists =
        sqlx::query("SELECT 1 FROM application_artifacts WHERE id = ?1 AND application_id = ?2")
            .bind(target_artifact_id)
            .bind(application_id)
            .fetch_optional(&conn)
            .await
            .map_err(|e| AppError::DatabaseError(e.to_string()))?
            .is_some();
    if !exists {
        return Err(AppError::ValidationError(
            "That deploy's artifact is no longer available".into(),
        ));
    }
    sqlx::query(
        "UPDATE application_deployments SET artifact_id = ?1, updated_at = ?2 WHERE application_id = ?3",
    )
    .bind(target_artifact_id)
    .bind(now_ms())
    .bind(application_id)
    .execute(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    enqueue(
        state,
        organization_id,
        project_id,
        environment_id,
        application_id,
        "manual_rollback",
        created_by,
    )
    .await
}

/// Worker entry point. Recreates the static Caddy container from the
/// previously uploaded artifact. Returns the error message verbatim so the
/// queue worker can surface it in the run log.
pub async fn run(state: &SharedState, application_id: &str, run_id: &str) -> Result<(), String> {
    use sqlx::Row;
    let conn = get_conn(state).await;
    let row = sqlx::query(
        "SELECT d.container_name, a.relative_path FROM application_deployments d LEFT JOIN application_artifacts a ON a.id = d.artifact_id WHERE d.application_id = ?1",
    )
    .bind(application_id)
    .fetch_optional(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))
    .map_err(|e| e.to_string())?
    .ok_or_else(|| "Application not found".to_string())?;
    let container_name: String = row
        .try_get(0)
        .map_err(|e| AppError::DatabaseError(e.to_string()))
        .map_err(|e| e.to_string())?;
    let relative_path: Option<String> = row
        .try_get(1)
        .map_err(|e| AppError::DatabaseError(e.to_string()))
        .map_err(|e| e.to_string())?;
    let Some(relative_path) = relative_path else {
        return Err("Upload an artifact before deploying".into());
    };
    let source_config = source::get(state, application_id)
        .await
        .map_err(|e| e.to_string())?;

    log_stage(
        state,
        application_id,
        run_id,
        "info",
        "Static deploy started",
    )
    .await;

    // A fresh application has no domains yet; seed one so it is reachable.
    domains::seed_if_missing(state, application_id)
        .await
        .map_err(|e| e.to_string())?;
    let application_domains = domains::list(state, application_id)
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
    if runtime.is_remote() {
        let artifact_root = state.data_dir.join(&relative_path);
        log_stage(
            state,
            application_id,
            run_id,
            "info",
            "Uploading static artifact to the selected server",
        )
        .await;
        let remote_root = runtime
            .upload_workspace_tree(application_id, &artifact_root, "static")
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "Remote static workspace was not created".to_string())?;
        if !managed_volumes::runtime_mounts(state, application_id)
            .await
            .map_err(|error| error.to_string())?
            .is_empty()
        {
            return Err("Managed-volume mounts for remote static deployments are not ready yet; detach them before deploying this application".into());
        }
        let endpoint = super::runtime_endpoints::ensure_remote(state, application_id, 80)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| {
                "Remote static deployment did not receive a loopback endpoint".to_string()
            })?;
        let _ = runtime
            .podman(&[
                OsString::from("rm"),
                OsString::from("--force"),
                OsString::from("--volumes"),
                OsString::from(&container_name),
            ])
            .await;
        let environment = super::decrypted_environment(state, application_id, "runtime")
            .await
            .map_err(|error| error.to_string())?;
        let env_contents = environment
            .iter()
            .map(|(name, value)| {
                if value.contains(['\n', '\r']) {
                    Err(format!("Runtime variable {name} contains a newline and cannot be used in an env file"))
                } else {
                    Ok(format!("{name}={value}\n"))
                }
            })
            .collect::<Result<String, String>>()?;
        let env_file = crate::servers::ssh::SecretFile::write(&env_contents)
            .map_err(|error| error.to_string())?;
        let remote_env = runtime
            .upload_workspace_file(application_id, env_file.path(), "static.runtime.env")
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "Remote static environment file was not uploaded".to_string())?;
        history::step(state, "start").await;
        log_stage(
            state,
            application_id,
            run_id,
            "info",
            "Starting the static container on its private loopback endpoint",
        )
        .await;
        runtime
            .podman(&[
                OsString::from("run"),
                OsString::from("--detach"),
                OsString::from("--name"),
                OsString::from(&container_name),
                OsString::from("--restart"),
                OsString::from("unless-stopped"),
                OsString::from("--env-file"),
                OsString::from(remote_env),
                OsString::from("--publish"),
                OsString::from(format!("127.0.0.1:{}:80", endpoint.host_port)),
                OsString::from("--volume"),
                OsString::from(format!("{remote_root}:/srv:ro")),
                OsString::from("docker.io/library/caddy:latest"),
                OsString::from("caddy"),
                OsString::from("file-server"),
                OsString::from("--root"),
                OsString::from("/srv"),
                OsString::from("--listen"),
                OsString::from(":80"),
            ])
            .await
            .map_err(|error| {
                format!("Could not start static container on selected server: {error}")
            })?;
        history::step(state, "route").await;
        if let Err(error) = crate::servers::ingress::sync_application(state, application_id).await {
            let _ = runtime
                .podman(&[
                    OsString::from("rm"),
                    OsString::from("--force"),
                    OsString::from("--volumes"),
                    OsString::from(&container_name),
                ])
                .await;
            return Err(format!("Could not configure remote ingress: {error}"));
        }
        let conn = get_conn(state).await;
        sqlx::query(
            "UPDATE application_deployments SET status = 'running', config_stale = 0, deployed_at = ?1, updated_at = ?1 WHERE application_id = ?2",
        )
        .bind(now_ms())
        .bind(application_id)
        .execute(&conn)
        .await
        .map_err(|error| format!("Database error: {error}"))?;
        log_stage(
            state,
            application_id,
            run_id,
            "info",
            "Static deploy finished",
        )
        .await;
        let _ = history::finish(state, run_id, "succeeded", None).await;
        return Ok(());
    }

    let podman =
        Podman::new(&state.podman_socket).map_err(|e| format!("Invalid Podman socket: {e}"))?;
    let container = podman.containers().get(&container_name);
    if let Ok(info) = container.inspect().await {
        if info.state.and_then(|s| s.running) == Some(true) {
            let _ = container.stop(&Default::default()).await;
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
            while tokio::time::Instant::now() < deadline {
                if let Ok(info) = container.inspect().await {
                    if info.state.as_ref().and_then(|s| s.running) != Some(true) {
                        break;
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        }
    }
    let _ = container
        .delete(
            &ContainerDeleteOpts::builder()
                .force(true)
                .volumes(true)
                .build(),
        )
        .await;

    let environment = super::decrypted_environment(state, application_id, "runtime")
        .await
        .map_err(|e| e.to_string())?;
    let (bind_mounts, mut named_volumes, static_root) = build_static_mounts(state, &relative_path);
    let mut networks = HashMap::new();
    networks.insert(
        state.podman_network.clone(),
        models::PerNetworkOptions {
            aliases: Some(vec![container_name.clone()]),
            interface_name: None,
            static_ips: None,
            static_mac: None,
        },
    );
    // Databases created from the internal templates live on a network of their
    // own; a linked application has to join it to reach them.
    let runtime = crate::servers::runtime::ServerRuntime::for_application(state, application_id)
        .await
        .map_err(|error| error.to_string())?;
    for network in super::resource_networks::ensure_for_deploy(state, &runtime, application_id)
        .await
        .map_err(|error| error.to_string())?
    {
        networks.insert(
            network,
            models::PerNetworkOptions {
                aliases: None,
                interface_name: None,
                static_ips: None,
                static_mac: None,
            },
        );
    }
    named_volumes = managed_volumes::append_runtime_mounts(
        named_volumes,
        managed_volumes::runtime_mounts(state, application_id)
            .await
            .map_err(|e| e.to_string())?,
    );
    let mut builder = super::managed_container_builder(
        &container_name,
        "docker.io/library/caddy:latest".into(),
        "unless-stopped",
    )
    .command([
        "caddy",
        "file-server",
        "--root",
        static_root.as_str(),
        "--listen",
        ":80",
    ])
    .env(environment)
    .mounts(bind_mounts)
    .volumes(named_volumes)
    .net_namespace(models::Namespace {
        nsmode: Some("bridge".into()),
        value: None,
    })
    .networks(networks);
    let mappings = domains::port_mappings(state, application_id)
        .await
        .map_err(|e| e.to_string())?;
    if !mappings.is_empty() {
        builder = builder.portmappings(mappings);
    }
    let options = apply_runtime_limits(builder, &source_config)
        .map_err(|e| e.to_string())?
        .build();
    history::step(state, "start").await;
    let result: Result<(), String> = async {
        ensure_image_pulled(&podman, "docker.io/library/caddy:latest").await?;
        podman
            .containers()
            .create(&options)
            .await
            .map_err(|e| format!("Could not create static container: {e}"))?;
        let mut last_error = String::new();
        for attempt in 0..3 {
            match container.start(None).await {
                Ok(()) => {
                    last_error.clear();
                    break;
                }
                Err(error) => {
                    let err = error.to_string();
                    if err.contains("address already in use") && attempt < 2 {
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                        last_error = err;
                        continue;
                    }
                    return Err(format!("Could not start static container: {err}"));
                }
            }
        }
        if !last_error.is_empty() {
            return Err(format!("Could not start static container: {last_error}"));
        }
        history::step(state, "route").await;
        for domain in application_domains.iter().filter(|domain| !domain.is_local) {
            domains::sync_caddy_route(state, domain, &container_name)
                .await
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    .await;
    if let Err(error) = result {
        let _ = history::finish(state, run_id, "failed", Some(&error)).await;
        let _ = container
            .delete(
                &ContainerDeleteOpts::builder()
                    .force(true)
                    .volumes(true)
                    .build(),
            )
            .await;
        let conn = get_conn(state).await;
        let _ = sqlx::query(
            "UPDATE application_deployments SET status = 'failed', last_error = ?1, updated_at = ?2 WHERE application_id = ?3",
        )
        .bind(error.clone())
        .bind(now_ms())
        .bind(application_id)
        .execute(&conn)
        .await;
        log_stage(state, application_id, run_id, "error", &error).await;
        return Err(error);
    }
    let conn = get_conn(state).await;
    sqlx::query(
        "UPDATE application_deployments SET status = 'running', config_stale = 0, deployed_at = ?1, updated_at = ?1 WHERE application_id = ?2",
    )
    .bind(now_ms())
    .bind(application_id)
    .execute(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))
    .map_err(|e| e.to_string())?;
    log_stage(
        state,
        application_id,
        run_id,
        "info",
        "Static deploy finished",
    )
    .await;
    let _ = history::finish(state, run_id, "succeeded", None).await;
    Ok(())
}

fn build_static_mounts(
    state: &SharedState,
    relative_path: &str,
) -> (
    Vec<models::ContainerMount>,
    Vec<models::NamedVolume>,
    String,
) {
    if let Some(volume) = &state.data_volume {
        (
            vec![],
            vec![models::NamedVolume {
                dest: Some("/data".into()),
                is_anonymous: None,
                name: Some(volume.clone()),
                options: Some(vec!["ro".into()]),
            }],
            format!("/data/{relative_path}"),
        )
    } else {
        (
            vec![models::ContainerMount {
                destination: Some("/srv".into()),
                options: Some(vec!["ro".into()]),
                source: Some(
                    state
                        .host_data_dir
                        .join(relative_path)
                        .to_string_lossy()
                        .to_string(),
                ),
                _type: Some("bind".into()),
                uid_mappings: None,
                gid_mappings: None,
            }],
            vec![],
            "/srv".to_string(),
        )
    }
}

fn apply_runtime_limits(
    builder: ContainerCreateOptsBuilder,
    source_config: &SourceConfigDto,
) -> Result<ContainerCreateOptsBuilder, String> {
    super::with_resource_limits(builder, source_config).map_err(|e| e.to_string())
}
