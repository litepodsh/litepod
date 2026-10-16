use std::ffi::OsString;

use sqlx::Row;

use crate::{
    deployment::{
        compose::{self, store::ComposeFileStore},
        decrypted_environment, domains, history, managed_volumes,
        queue::DeploymentJobDto,
        resource_networks, source,
    },
    dtos::{deployment::DeploymentDto, response::AppError},
    state::state::SharedState,
};

use super::overrides::*;
use super::process::*;
use super::shared::*;

pub(super) async fn compose_cmd(
    state: &SharedState,
    application_id: &str,
    store: &ComposeFileStore,
    action: &[&str],
    run_id: Option<&str>,
    candidate_name: Option<&str>,
) -> Result<(), String> {
    let stored_content_path = store.content_path();
    let relative_base_dir = stored_content_path.parent().unwrap();
    let base_dir = if relative_base_dir.is_absolute() {
        relative_base_dir.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| format!("Could not resolve the current directory: {e}"))?
            .join(relative_base_dir)
    };
    let content_path = base_dir.join("docker-compose.yml");
    let override_path = base_dir.join("port-overrides.yml");
    // Generated files are per replica: parallel group deploys each write their
    // own server-specific overrides before uploading them.
    let generated_dir = crate::servers::replica::scoped_path(base_dir.clone());
    if generated_dir != base_dir {
        tokio::fs::create_dir_all(&generated_dir)
            .await
            .map_err(|e| format!("Could not create the replica Compose directory: {e}"))?;
    }
    let normalized_content_path = generated_dir.join("litepod.compose.normalized.yml");
    let standalone_override_path = generated_dir.join("litepod.standalone.override.yml");
    let managed_override_path = generated_dir.join("litepod.managed-volumes.override.yml");
    let networks_override_path = generated_dir.join("litepod.networks.override.yml");

    log_stage(
        state,
        application_id,
        run_id,
        "info",
        "Reading and validating the Compose file",
    )
    .await;
    let mut content = tokio::fs::read_to_string(&content_path)
        .await
        .map_err(|error| format!("Could not read Compose file: {error}"))?;
    // Database resources generated before resource networks still name
    // `litepod_proxy`. Move them onto their own network here, once, so an
    // existing resource ends up isolated like a freshly created one. Compose
    // overrides can only add, never remove a network, so this has to rewrite
    // the stored file.
    if resource_networks::is_resource(state, application_id)
        .await
        .map_err(|error| error.to_string())?
        && let Some(migrated) =
            resource_networks::migrate_resource_compose(&content, application_id)?
    {
        store.write_content(&migrated).await?;
        log_stage(
            state,
            application_id,
            run_id,
            "info",
            "Moved this resource off the shared network and onto its own",
        )
        .await;
        content = migrated;
    }
    let services = compose_services(&content)?;
    // Compose would register these names as DNS aliases on the shared network,
    // shadowing the control-plane containers that answer to them today.
    let conflicts =
        compose::reserved_service_conflicts(services.iter().map(|service| service.name.as_str()));
    if !conflicts.is_empty() {
        return Err(compose::reserved_service_message(&conflicts));
    }
    let runtime_name = match candidate_name {
        Some(name) => name.to_string(),
        None => crate::deployment::rollout::active_runtime_name(state, application_id).await?,
    };
    // The user's file stays as written; the provider gets the rewritten copy so
    // its `networks` shape matches the override generated below.
    let base_file = match normalize_compose_networks(&content, &runtime_name)? {
        Some(normalized) => {
            tokio::fs::write(&normalized_content_path, normalized)
                .await
                .map_err(|error| format!("Failed to write normalized Compose file: {error}"))?;
            normalized_content_path.clone()
        }
        None => {
            let _ = tokio::fs::remove_file(&normalized_content_path).await;
            content_path.clone()
        }
    };
    log_stage(
        state,
        application_id,
        run_id,
        "info",
        &format!(
            "Resolved Compose services: {}",
            services
                .iter()
                .map(|service| service.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    )
    .await;
    log_stage(
        state,
        application_id,
        run_id,
        "info",
        "Resolving proxy routes and generating Compose overrides",
    )
    .await;
    let proxy_routes = crate::deployment::compose::proxy_routes::list(state, application_id)
        .await
        .map_err(|error| error.to_string())?;
    let proxy_services = proxy_routes
        .iter()
        .map(|route| route.service.clone())
        .collect::<Vec<_>>();
    let standalone = generate_standalone_override(
        &runtime_name,
        &state.podman_network,
        &services,
        &proxy_services,
    );
    let standalone = if state.dev_mode && candidate_name.is_some() {
        let endpoints = proxy_routes
            .iter()
            .map(|route| (route.service.as_str(), 0, route.container_port))
            .collect::<Vec<_>>();
        let ports = generate_remote_standalone_override(&runtime_name, &services, &endpoints)?;
        let mut document: serde_yaml::Value =
            serde_yaml::from_str(&standalone).map_err(|error| error.to_string())?;
        let ports: serde_yaml::Value =
            serde_yaml::from_str(&ports).map_err(|error| error.to_string())?;
        for service in &services {
            if let Some(mapping) = ports["services"][&service.name].get("ports") {
                document["services"][&service.name]["ports"] = mapping.clone();
            }
        }
        serde_yaml::to_string(&document).map_err(|error| error.to_string())?
    } else {
        standalone
    };
    tokio::fs::write(&standalone_override_path, &standalone)
        .await
        .map_err(|e| format!("Failed to write standalone override: {e}"))?;
    log_stage(
        state,
        application_id,
        run_id,
        "info",
        "Loading managed-volume configuration",
    )
    .await;
    let managed_override = managed_volumes::compose_override(state, application_id)
        .await
        .map_err(|error| error.to_string())?;
    if managed_override.is_empty() {
        let _ = tokio::fs::remove_file(&managed_override_path).await;
    } else {
        tokio::fs::write(&managed_override_path, managed_override)
            .await
            .map_err(|error| format!("Failed to write managed volume override: {error}"))?;
    }

    // Resource networks: this app's own when it is an internal database
    // resource, plus one per database it is linked to. Generated here rather
    // than written into the stored Compose file, so resources that already
    // exist gain their network without their file being rewritten.
    let networks_override = resource_networks::compose_override(
        state,
        application_id,
        &services
            .iter()
            .map(|service| service.name.clone())
            .collect::<Vec<_>>(),
    )
    .await
    .map_err(|error| error.to_string())?;
    if networks_override.is_empty() {
        let _ = tokio::fs::remove_file(&networks_override_path).await;
    } else {
        tokio::fs::write(&networks_override_path, networks_override)
            .await
            .map_err(|error| format!("Failed to write resource network override: {error}"))?;
    }

    // Load source config to read resource limits
    let source = source::get(state, application_id)
        .await
        .map_err(|e| e.to_string())?;
    let runtime_override_path = generated_dir.join("litepod.runtime-resources.override.yml");
    if let Some(limits) = super::super::resource_limits(&source)? {
        let runtime_override = generate_runtime_resources_override(&services, &limits);
        tokio::fs::write(&runtime_override_path, runtime_override)
            .await
            .map_err(|e| format!("Failed to write runtime resources override: {e}"))?;
    } else {
        let _ = tokio::fs::remove_file(&runtime_override_path).await;
    }

    log_stage(
        state,
        application_id,
        run_id,
        "info",
        "Loading runtime environment variables",
    )
    .await;
    let environment = decrypted_environment(state, application_id, "runtime")
        .await
        .map_err(|e| e.to_string())?;
    // `{{url}}`-style domain markers are stored as written and resolved here,
    // against the routes as they are now. Only `up` needs real addresses:
    // `down`/`stop` must keep working even when a marker's route was deleted,
    // or a broken app could not be torn down.
    let environment = if action.first() == Some(&"up") {
        crate::deployment::compose::marker_routes::resolve_runtime_env(
            state,
            application_id,
            &content,
            environment,
        )
        .await
        .map_err(|e| e.to_string())?
    } else {
        environment
    };

    let project = crate::deployment::rollout::compose_project(application_id, &runtime_name);
    let mut args: Vec<OsString> = vec![
        "compose".into(),
        "--project-name".into(),
        project.clone().into(),
        "--file".into(),
        base_file.clone().into(),
    ];
    if override_path.exists() {
        args.push("--file".into());
        args.push(override_path.clone().into());
    }
    args.push("--file".into());
    args.push(standalone_override_path.clone().into());
    if managed_override_path.exists() {
        args.push("--file".into());
        args.push(managed_override_path.clone().into());
    }
    if networks_override_path.exists() {
        args.push("--file".into());
        args.push(networks_override_path.clone().into());
    }
    if runtime_override_path.exists() {
        args.push("--file".into());
        args.push(runtime_override_path.clone().into());
    }
    args.extend(action.iter().map(|argument| OsString::from(*argument)));

    let mut file_names = vec![if base_file == normalized_content_path {
        "litepod.compose.normalized.yml"
    } else {
        "docker-compose.yml"
    }];
    if override_path.exists() {
        file_names.push("port-overrides.yml");
    }
    file_names.push("litepod.standalone.override.yml");
    if managed_override_path.exists() {
        file_names.push("litepod.managed-volumes.override.yml");
    }
    if networks_override_path.exists() {
        file_names.push("litepod.networks.override.yml");
    }
    if runtime_override_path.exists() {
        file_names.push("litepod.runtime-resources.override.yml");
    }
    log_stage(
        state,
        application_id,
        run_id,
        "info",
        &compose_launch_description(&project, &file_names, action),
    )
    .await;

    // Remote applications never use the litepod host's working directory or Podman socket. Copy
    // the generated, reviewed inputs into the remote rootless user's private workspace and run
    // the same Compose command there. Runtime secrets travel in a 0600 temporary env file and
    // are not included in logs or process arguments.
    let runtime = crate::servers::runtime::ServerRuntime::for_application(state, application_id)
        .await
        .map_err(|error| error.to_string())?;
    // Both sides declare these `external: true`, so they have to exist on the
    // server that runs the project before Compose is asked to attach anything.
    let resource_network_names = resource_networks::networks_for(state, application_id)
        .await
        .map_err(|error| error.to_string())?;
    resource_networks::ensure_networks(&runtime, &resource_network_names)
        .await
        .map_err(|error| error.to_string())?;
    if runtime.is_remote() {
        log_stage(
            state,
            application_id,
            run_id,
            "info",
            "Uploading Compose inputs to the selected server",
        )
        .await;
        let workspace = runtime
            .ensure_workspace(application_id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "Remote server workspace was not created".to_string())?;
        // A remote application gets one loopback tunnel per distinct
        // (service, container_port) target — any number of domains is fine,
        // routed or not, each domain's own proxy route picks which tunnel it
        // dials.
        let mut remote_targets: Vec<(&str, u16)> = Vec::new();
        for route in &proxy_routes {
            let key = (route.service.as_str(), route.container_port);
            if !remote_targets.contains(&key) {
                remote_targets.push(key);
            }
        }
        let mut remote_endpoints: Vec<(&str, u16, u16)> = Vec::new();
        for (service, container_port) in &remote_targets {
            let host_port = if candidate_name.is_some() {
                0 // Podman assigns a free loopback port; promote persists it.
            } else {
                let server_id = crate::servers::replica::target_server_id(state, application_id)
                    .await
                    .map_err(|error| error.to_string())?
                    .ok_or_else(|| "Remote compose deployment has no target server".to_string())?;
                crate::deployment::runtime_endpoints::get_for_service(
                    state,
                    application_id,
                    &server_id,
                    *container_port,
                )
                .await
                .map_err(|error| error.to_string())?
                .map(|endpoint| endpoint.host_port)
                .unwrap_or(0)
            };
            remote_endpoints.push((*service, host_port, *container_port));
        }
        let remote_content =
            prepare_remote_compose(&tokio::fs::read_to_string(&base_file).await.map_err(
                |error| format!("Could not read Compose file for remote deployment: {error}"),
            )?)?;
        let remote_base_path = generated_dir.join("litepod.remote.compose.yml");
        tokio::fs::write(&remote_base_path, remote_content)
            .await
            .map_err(|error| format!("Could not write remote Compose file: {error}"))?;
        let remote_override_path = generated_dir.join("litepod.remote.standalone.override.yml");
        let remote_override =
            generate_remote_standalone_override(&runtime_name, &services, &remote_endpoints)?;
        tokio::fs::write(&remote_override_path, remote_override)
            .await
            .map_err(|error| format!("Could not write remote ingress override: {error}"))?;
        let remote_base =
            upload_remote_compose_file(&runtime, application_id, &remote_base_path, &workspace)
                .await?;
        let remote_standalone =
            upload_remote_compose_file(&runtime, application_id, &remote_override_path, &workspace)
                .await?;
        let remote_managed = if managed_override_path.exists() {
            Some(
                upload_remote_compose_file(
                    &runtime,
                    application_id,
                    &managed_override_path,
                    &workspace,
                )
                .await?,
            )
        } else {
            None
        };
        let remote_networks = if networks_override_path.exists() {
            Some(
                upload_remote_compose_file(
                    &runtime,
                    application_id,
                    &networks_override_path,
                    &workspace,
                )
                .await?,
            )
        } else {
            None
        };
        let remote_resources = if runtime_override_path.exists() {
            Some(
                upload_remote_compose_file(
                    &runtime,
                    application_id,
                    &runtime_override_path,
                    &workspace,
                )
                .await?,
            )
        } else {
            None
        };
        // The Ports section of the Compose editor (`port-overrides.yml`) is the one
        // sanctioned way to publish a raw TCP/UDP port straight to a remote host's
        // public interface, bypassing the loopback Caddy endpoint above entirely -
        // Compose concatenates the `ports:` list across `-f` files for the same
        // service, so this and the loopback endpoint both take effect.
        let remote_port_overrides = if override_path.exists() {
            let port_override_content =
                tokio::fs::read_to_string(&override_path)
                    .await
                    .map_err(|error| {
                        format!("Could not read port overrides for remote deployment: {error}")
                    })?;
            let declared_services = compose_services(&port_override_content)?;
            for declared in &declared_services {
                if !services.iter().any(|service| service.name == declared.name) {
                    return Err(format!(
                        "Port override for service '{}' does not match any service in the Compose file",
                        declared.name
                    ));
                }
            }
            Some(
                upload_remote_compose_file(&runtime, application_id, &override_path, &workspace)
                    .await?,
            )
        } else {
            None
        };
        let env_contents = environment.iter().map(|(name, value)| {
            if value.contains('\n') || value.contains('\r') { Err(format!("Runtime variable {name} contains a newline and cannot be used in a Compose env file")) } else { Ok(format!("{name}={value}\n")) }
        }).collect::<Result<String, String>>()?;
        let env_file = crate::servers::ssh::SecretFile::write(&env_contents)
            .map_err(|error| error.to_string())?;
        let remote_env = runtime
            .upload_workspace_file(application_id, env_file.path(), "litepod.runtime.env")
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "Remote environment file was not uploaded".to_string())?;
        let mut remote_args: Vec<OsString> = vec![
            "compose".into(),
            "--project-name".into(),
            project.clone().into(),
            "--env-file".into(),
            remote_env.into(),
            "--file".into(),
            remote_base.into(),
        ];
        for file in [
            Some(remote_standalone),
            remote_managed,
            remote_networks,
            remote_resources,
            remote_port_overrides,
        ]
        .into_iter()
        .flatten()
        {
            remote_args.push("--file".into());
            remote_args.push(file.into());
        }
        remote_args.extend(action.iter().map(|argument| OsString::from(*argument)));
        log_stage(
            state,
            application_id,
            run_id,
            "info",
            &format!(
                "Running {} Podman Compose on {}",
                runtime
                    .remote_podman_mode()
                    .expect("remote branch always has an SSH runtime")
                    .as_str(),
                runtime
                    .remote_server_name()
                    .expect("remote branch always has an SSH runtime"),
            ),
        )
        .await;
        let first_attempt = spawn_remote_compose(state, application_id, run_id, &runtime, &remote_args)
            .await
            .map_err(|detail| {
                if detail.contains("looking up compose provider failed") {
                    format!(
                        "Podman Compose provider is unavailable on {}. Run Repair for that server so litepod installs podman-compose, then retry. Original error: {detail}",
                        runtime
                            .remote_server_name()
                            .expect("remote branch always has an SSH runtime"),
                    )
                } else {
                    detail
                }
            });
        first_attempt?;
        if action.first() == Some(&"up") {
            chown_rootless_volumes(&runtime, &project, &content).await;
        }
        return Ok(());
    }

    spawn_compose(
        state,
        application_id,
        run_id,
        &base_dir,
        &args,
        &environment,
    )
    .await?;
    if action.first() == Some(&"up") {
        chown_rootless_volumes(&runtime, &project, &content).await;
    }
    Ok(())
}

/// Redeploys the exact Compose YAML a previous successful run deployed. The
/// store is one mutable file with no history, so this overwrites it with the
/// old content first - the same edit-then-deploy path a manual save takes,
/// just with saved text instead of user-typed text. Whatever the file said a
/// moment ago is gone unless it was itself a deployed run's content.
pub async fn enqueue_rollback(
    state: &SharedState,
    application_id: &str,
    target_content: &str,
    created_by: Option<&str>,
) -> Result<DeploymentJobDto, AppError> {
    let store = ComposeFileStore::new(&state.data_dir, application_id);
    store
        .write_content(target_content)
        .await
        .map_err(AppError::InternalError)?;
    enqueue(state, application_id, "manual_rollback", created_by).await
}

/// Resolves the queue ownership fields from an application ID. Compose deploys
/// enter the same queue as Git deploys, so these IDs must come from the server
/// rather than the client request.
pub(super) async fn owner_ids(
    state: &SharedState,
    application_id: &str,
) -> Result<(String, String, String), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(
        "SELECT p.organization_id, e.project_id, a.environment_id FROM applications a INNER JOIN environments e ON e.id = a.environment_id INNER JOIN projects p ON p.id = e.project_id WHERE a.id = ?1",
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

/// Queues a Compose deployment so it observes the shared concurrency limit and
/// can be canceled from the dashboard. The worker below retains the detailed
/// Compose lifecycle logging implemented for direct deployments.
///
/// Returns the queued job so the dashboard can attach to its live log stream the
/// same way it does for Git deployments.
pub async fn enqueue(
    state: &SharedState,
    application_id: &str,
    triggered_by: &str,
    created_by: Option<&str>,
) -> Result<DeploymentJobDto, AppError> {
    let source_config = source::get(state, application_id).await?;
    let store = ComposeFileStore::new(&state.data_dir, application_id);
    let content = store
        .read_content()
        .await
        .map_err(AppError::InternalError)?;
    if content.trim().is_empty() {
        return Err(AppError::ValidationError(
            "Write a compose file before deploying".into(),
        ));
    }
    super::super::ensure_deployment(state, application_id).await?;
    let (organization_id, project_id, environment_id) = owner_ids(state, application_id).await?;
    let run_id = history::start(
        state,
        application_id,
        "compose",
        "compose",
        triggered_by,
        None,
        created_by,
    )
    .await?;
    // The store is one mutable file with no history - a rollback later needs
    // the exact text this run deployed, or it is gone the next time someone
    // edits and redeploys. A rollback deploy (see `rollback`) overwrites the
    // store with the old content first, so this ends up recording it again.
    history::set_compose_content(state, &run_id, &content).await?;
    log_stage(
        state,
        application_id,
        Some(&run_id),
        "info",
        "Standalone Compose deployment queued",
    )
    .await;
    let snapshot = serde_json::to_string(&source_config)
        .map_err(|e| AppError::InternalError(e.to_string()))?;
    let conn = crate::git::repositories::db::get_conn(state).await;
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
    super::super::queue::enqueue(
        state,
        super::super::queue::EnqueueJobInput {
            application_id: application_id.into(),
            project_id,
            environment_id,
            organization_id,
            source_type: "compose".into(),
            source_snapshot: snapshot.clone(),
            build_method: "compose".into(),
            build_snapshot: snapshot,
            run_id,
        },
    )
    .await
}

/// Runs inside the deployment queue worker. On cancellation the future is
/// dropped, which also terminates the child Compose process via `kill_on_drop`.
pub(crate) async fn run(
    state: SharedState,
    application_id: String,
    run_id: String,
) -> Result<(), String> {
    let store = ComposeFileStore::new(&state.data_dir, &application_id);
    log_stage(
        &state,
        &application_id,
        Some(&run_id),
        "info",
        "Preparing standalone Compose deployment",
    )
    .await;
    match deploy_started(&state, &application_id, &store, &run_id).await {
        Ok(_) => {
            log_stage(
                &state,
                &application_id,
                Some(&run_id),
                "info",
                "Compose deployment completed",
            )
            .await;
            let _ = history::finish(&state, &run_id, "succeeded", None).await;
            Ok(())
        }
        Err(error) => {
            let message = error.to_string();
            tracing::error!(
                application_id,
                error = %message,
                "[compose-standalone] Compose deployment failed"
            );
            let _ = history::finish(&state, &run_id, "failed", Some(&message)).await;
            crate::deployment::rollout::record_failure(&state, &application_id, &message).await;
            Err(message)
        }
    }
}

pub(super) async fn deploy_started(
    state: &SharedState,
    application_id: &str,
    store: &ComposeFileStore,
    run_id: &str,
) -> Result<DeploymentDto, AppError> {
    let content = store
        .read_content()
        .await
        .map_err(AppError::InternalError)?;
    if content.trim().is_empty() {
        return Err(AppError::ValidationError(
            "Write a compose file before deploying".into(),
        ));
    }

    let runtime =
        crate::servers::runtime::ServerRuntime::for_application(state, application_id).await?;
    let mut rollout =
        crate::deployment::rollout::Rollout::prepare(state, application_id, &runtime, true)
            .await
            .map_err(AppError::ValidationError)?;
    if rollout.previous_exists {
        let mut needs_in_place_recreate =
            crate::deployment::rollout::requires_in_place_recreate(&content)
                .map_err(AppError::ValidationError)?;
        let ports = store
            .content_path()
            .parent()
            .unwrap()
            .join("port-overrides.yml");
        if ports.exists() {
            let overrides = tokio::fs::read_to_string(ports)
                .await
                .map_err(|error| AppError::InternalError(error.to_string()))?;
            needs_in_place_recreate |=
                crate::deployment::rollout::requires_in_place_recreate(&overrides)
                    .map_err(AppError::ValidationError)?;
        }
        let has_managed_volumes = !managed_volumes::runtime_mounts(state, application_id)
            .await?
            .is_empty();
        needs_in_place_recreate |= rollout
            .check_replaceable(&runtime, has_managed_volumes, false)
            .await
            .is_err();
        if needs_in_place_recreate {
            log_stage(
                state,
                application_id,
                Some(run_id),
                "info",
                "Persistent storage detected; recreating the current Compose runtime while preserving volumes",
            )
            .await;
            compose_cmd(
                state,
                application_id,
                store,
                &["down", "--remove-orphans"],
                Some(run_id),
                None,
            )
            .await
            .map_err(AppError::InternalError)?;
            rollout =
                crate::deployment::rollout::Rollout::prepare(state, application_id, &runtime, true)
                    .await
                    .map_err(AppError::ValidationError)?;
            if rollout.previous_exists {
                return Err(AppError::InternalError(
                    "Compose runtime could not be stopped before recreation".into(),
                ));
            }
        }
    }

    if domains::needs_default_domain(state, application_id).await? {
        log_stage(
            state,
            application_id,
            Some(run_id),
            "info",
            "Creating the default application domain",
        )
        .await;
        domains::generate(state, application_id).await?;
    }

    if let Err(error) = wire_catalog_route(state, application_id, run_id, &content).await {
        log_stage(
            state,
            application_id,
            Some(run_id),
            "warn",
            &format!("Skipped catalog proxy-route auto-wire: {error}"),
        )
        .await;
    }

    // Runs after `wire_catalog_route`, so a `{{url}}` marker on the primary port
    // reuses the primary domain instead of getting a second one. In dev mode the
    // secondary routes get a `localhost:<port>` domain, but `port_mappings`
    // publishes every local domain to the app's single container port, so those
    // URLs are not reachable locally.
    for host in
        crate::deployment::compose::marker_routes::ensure_routes(state, application_id, &content)
            .await?
    {
        log_stage(
            state,
            application_id,
            Some(run_id),
            "info",
            &format!("Routed domain {host} for an env domain marker"),
        )
        .await;
    }

    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE application_deployments SET status = 'deploying', last_error = NULL, updated_at = ?1 WHERE application_id = ?2",
    )
    .bind(now_ms())
    .bind(application_id)
    .execute(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let _ = history::set_status(state, &run_id, "deploying").await;

    // Pre-pull every registry image declared in the Compose file. We bypass
    // `podman compose pull` on purpose: it does not honour `--policy always`
    // (it rejects the flag) and its own caching is happy to skip the digest a
    // fresh `:latest` push just left behind. Calling `podman pull` directly
    // makes the redeploy actually pick up new image bytes, and surfacing the
    // error fails the deploy instead of quietly recycling a stale container.
    history::step(state, "pull").await;
    log_stage(
        state,
        application_id,
        Some(run_id),
        "info",
        "Pulling every image declared in the Compose file",
    )
    .await;
    let _ = crate::deployment::compose::pull::pull_compose_images(
        &runtime,
        state,
        application_id,
        Some(run_id),
        &content,
    )
    .await
    .map_err(|error| {
        tracing::error!(application_id, "[compose-standalone] {error}");
        AppError::InternalError(error)
    })?;

    history::step(state, "start").await;
    let _cleanup = rollout.cleanup_on_drop(&runtime);
    let candidate = async {
        compose_cmd(
            state,
            application_id,
            store,
            &["up", "--detach", "--build"],
            Some(run_id),
            Some(&rollout.name),
        )
        .await?;
        let routes = crate::deployment::compose::proxy_routes::list(state, application_id)
            .await
            .map_err(|error| error.to_string())?;
        let targets = routes
            .into_iter()
            .map(|route| {
                (
                    route.domain_id,
                    format!("{}-{}", rollout.name, route.service),
                    route.container_port as i64,
                )
            })
            .collect::<Vec<_>>();
        let containers = compose_services(&content)?
            .into_iter()
            .map(|service| format!("{}-{}", rollout.name, service.name))
            .collect::<Vec<_>>();
        rollout
            .promote(
                state,
                application_id,
                &runtime,
                &rollout.name,
                &targets,
                &containers,
            )
            .await
    }
    .await;
    if let Err(error) = candidate {
        if !error.contains("ingress rollback failed") {
            rollout.cleanup_candidate(&runtime).await;
        }
        return Err(AppError::InternalError(error));
    }
    crate::deployment::deployment(state, application_id).await
}

/// Wires the primary domain to a Compose service the first time a catalog app is
/// deployed.
///
/// Two rules keep this from breaking working routes:
///
/// - The service is the one that actually exposes the catalog's primary port.
///   The catalog only records port numbers (`CatalogPort` has no service field),
///   so pairing that port with whichever service happens to come first in the
///   file sends Caddy to a port nothing listens on as soon as the primary port
///   belongs to a later service.
/// - An existing route is never rewritten. This runs on every deploy, so an
///   upsert would undo whatever service the user picked in the UI on the next
///   redeploy - including the redeploy that follows a litepod update.
pub(super) async fn wire_catalog_route(
    state: &SharedState,
    application_id: &str,
    run_id: &str,
    compose_content: &str,
) -> Result<(), String> {
    let Some(port) =
        crate::git::repositories::project::get_catalog_primary_port(state, application_id)
            .await
            .map_err(|e| e.to_string())?
    else {
        return Ok(());
    };
    let domain = domains::list(state, application_id)
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|domain| domain.is_primary);
    let Some(domain) = domain else {
        return Err("no primary domain is available yet".into());
    };
    let already_wired = crate::deployment::compose::proxy_routes::list(state, application_id)
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .any(|route| route.domain_id == domain.id);
    if already_wired {
        return Ok(());
    }
    let service_name = match service_for_port(compose_content, port) {
        Some(name) => name,
        None => {
            let fallback = first_service_name(compose_content).ok_or_else(|| {
                "compose has no services to route the default domain to".to_string()
            })?;
            log_stage(
                state,
                application_id,
                Some(run_id),
                "warn",
                &format!(
                    "No Compose service publishes port {port}; routing {} to '{fallback}' as a guess",
                    domain.host
                ),
            )
            .await;
            fallback
        }
    };
    crate::deployment::compose::proxy_routes::upsert(
        state,
        application_id,
        &domain.id,
        &service_name,
        port,
    )
    .await
    .map(|_| ())
    .map_err(|e| e.to_string())
}
