use serde::{Deserialize, Serialize};
use tokio::process::Command;

use crate::{
    deployment::{compose::store::ComposeFileStore, history, source},
    dtos::{deployment::DeploymentDto, response::AppError},
    state::state::SharedState,
};

use super::pipeline::*;
use super::process::*;
use super::shared::*;

#[derive(Debug, Deserialize, Serialize)]
pub struct ComposeServiceStatus {
    pub service: String,
    pub status: String,
    pub ports: Vec<String>,
    pub container_name: String,
}

pub(super) async fn run_lifecycle_command(
    state: &SharedState,
    application_id: &str,
    store: &ComposeFileStore,
    triggered_by: &str,
    created_by: Option<&str>,
    requested_message: &str,
    success_message: &str,
    success_status: &str,
    warning: Option<&str>,
    action: &[&str],
) -> Result<(), AppError> {
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
    log_stage(
        state,
        application_id,
        Some(&run_id),
        "info",
        requested_message,
    )
    .await;
    if let Some(warning) = warning {
        log_stage(state, application_id, Some(&run_id), "info", warning).await;
    }
    let _ = history::set_status(state, &run_id, "deploying").await;

    let result = match compose_cmd(state, application_id, store, action, Some(&run_id), None).await
    {
        Ok(()) => {
            let conn = crate::git::repositories::db::get_conn(state).await;
            sqlx::query(
                "UPDATE application_deployments SET status = ?1, last_error = NULL, updated_at = ?2 WHERE application_id = ?3",
            )
            .bind(success_status)
            .bind(now_ms())
            .bind(application_id)
            .execute(&conn)
            .await
            .map(|_| ())
            .map_err(|error| AppError::DatabaseError(error.to_string()))
        }
        Err(error) => Err(AppError::InternalError(error)),
    };

    match result {
        Ok(()) => {
            log_stage(
                state,
                application_id,
                Some(&run_id),
                "info",
                success_message,
            )
            .await;
            let _ = history::finish(state, &run_id, "succeeded", None).await;
            Ok(())
        }
        Err(error) => {
            let message = error.to_string();
            tracing::error!(
                application_id,
                triggered_by,
                error = %message,
                "[compose-standalone] Compose lifecycle command failed"
            );
            let _ = history::finish(state, &run_id, "failed", Some(&message)).await;
            let conn = crate::git::repositories::db::get_conn(state).await;
            let _ = sqlx::query(
                "UPDATE application_deployments SET last_error = ?1, updated_at = ?2 WHERE application_id = ?3",
            )
            .bind(message)
            .bind(now_ms())
            .bind(application_id)
            .execute(&conn)
            .await;
            Err(error)
        }
    }
}

pub async fn stop(
    state: &SharedState,
    application_id: &str,
    created_by: Option<&str>,
) -> Result<DeploymentDto, AppError> {
    source::get(state, application_id).await?;
    crate::deployment::boot::mark_stopped(state, application_id).await?;
    let store = ComposeFileStore::new(&state.data_dir, application_id);
    run_lifecycle_command(
        state,
        application_id,
        &store,
        "manual_stop",
        created_by,
        "Compose stop requested",
        "Compose stop completed successfully",
        "stopped",
        None,
        &["down", "--remove-orphans"],
    )
    .await?;
    crate::deployment::deployment(state, application_id).await
}

/// Removes the Compose runtime and its Compose-managed volumes while retaining
/// the saved Compose configuration so the application can be deployed again.
pub async fn remove(
    state: &SharedState,
    application_id: &str,
    created_by: Option<&str>,
) -> Result<DeploymentDto, AppError> {
    source::get(state, application_id).await?;
    crate::deployment::boot::mark_stopped(state, application_id).await?;
    let store = ComposeFileStore::new(&state.data_dir, application_id);
    run_lifecycle_command(
        state,
        application_id,
        &store,
        "manual_remove",
        created_by,
        "Compose remove requested",
        "Compose remove completed successfully",
        "stopped",
        Some(REMOVE_VOLUMES_WARNING),
        &["down", "--remove-orphans", "--volumes"],
    )
    .await?;
    crate::deployment::deployment(state, application_id).await
}

pub async fn start(
    state: &SharedState,
    application_id: &str,
    created_by: Option<&str>,
) -> Result<DeploymentDto, AppError> {
    source::get(state, application_id).await?;
    super::pipeline::enqueue(state, application_id, "manual_start", created_by).await?;
    crate::deployment::deployment(state, application_id).await
}

pub async fn status(
    state: &SharedState,
    application_id: &str,
) -> Result<Vec<ComposeServiceStatus>, AppError> {
    let runtime_name = crate::deployment::rollout::active_runtime_name(state, application_id)
        .await
        .map_err(AppError::InternalError)?;
    let runtime =
        crate::servers::runtime::ServerRuntime::for_application(state, application_id).await?;
    if runtime.is_remote() {
        // The remote files live exclusively in the SSH user's workspace, so
        // compose-provider reconstruction on litepod would inspect the wrong
        // Podman host. Direct JSON listing is provider-independent and shares
        // the same deterministic name prefix as the remote override.
        let output = runtime
            .podman(&[
                "ps".into(),
                "--all".into(),
                "--format".into(),
                "json".into(),
            ])
            .await?;
        return Ok(
            parse_compose_status(application_id, &runtime_name, output.stdout.as_bytes())
                .into_iter()
                .filter(|service| service.container_name.contains(&runtime_name))
                .collect(),
        );
    }
    let store = ComposeFileStore::new(&state.data_dir, application_id);
    let stored_content_path = store.content_path();
    let relative_base_dir = stored_content_path.parent().unwrap();
    let base_dir = if relative_base_dir.is_absolute() {
        relative_base_dir.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| {
                AppError::InternalError(format!("Could not resolve the current directory: {e}"))
            })?
            .join(relative_base_dir)
    };
    let normalized_content_path = base_dir.join("litepod.compose.normalized.yml");
    // Must match the file the deployment used, or the provider reconstructs a
    // different project and reports no services.
    let content_path = if normalized_content_path.exists() {
        normalized_content_path
    } else {
        base_dir.join("docker-compose.yml")
    };
    let override_path = base_dir.join("port-overrides.yml");
    let standalone_override_path = base_dir.join("litepod.standalone.override.yml");
    let managed_override_path = base_dir.join("litepod.managed-volumes.override.yml");
    let networks_override_path = base_dir.join("litepod.networks.override.yml");
    let runtime_override_path = base_dir.join("litepod.runtime-resources.override.yml");
    let mut command = Command::new("podman");
    #[cfg(target_os = "linux")]
    command.arg("--url").arg(&state.podman_socket);
    command
        .arg("compose")
        .arg("--project-name")
        .arg(crate::deployment::rollout::compose_project(
            application_id,
            &runtime_name,
        ))
        .arg("--file")
        .arg(&content_path);
    if override_path.exists() {
        command.arg("--file").arg(&override_path);
    }
    if standalone_override_path.exists() {
        command.arg("--file").arg(&standalone_override_path);
    }
    if managed_override_path.exists() {
        command.arg("--file").arg(&managed_override_path);
    }
    if networks_override_path.exists() {
        command.arg("--file").arg(&networks_override_path);
    }
    if runtime_override_path.exists() {
        command.arg("--file").arg(&runtime_override_path);
    }
    // The provider spawns its own `podman` children; they only find the mounted
    // socket through `CONTAINER_HOST`.
    #[cfg(target_os = "linux")]
    command.env("CONTAINER_HOST", &state.podman_socket);
    let output = command
        .arg("ps")
        .arg("--format")
        .arg("json")
        .current_dir(&base_dir)
        .output()
        .await
        .map_err(|e| AppError::InternalError(format!("Could not run podman compose ps: {e}")))?;
    if output.status.success() {
        let services = parse_compose_status(application_id, &runtime_name, &output.stdout);
        if !services.is_empty() {
            return Ok(services);
        }
    } else {
        tracing::warn!(application_id, error = %tail(&output.stderr), "podman compose ps failed; falling back to container-name discovery");
    }

    // The external Compose provider is not always able to reconstruct a project
    // after an upgrade. Inspect Podman directly as a reliable fallback. Keep
    // the previous UUID-based prefix here so already-deployed apps remain
    // discoverable until their next deployment.
    let mut fallback = Command::new("podman");
    #[cfg(target_os = "linux")]
    fallback.arg("--url").arg(&state.podman_socket);
    let output = fallback
        .arg("ps")
        .arg("--all")
        .arg("--format")
        .arg("json")
        .output()
        .await
        .map_err(|e| {
            AppError::InternalError(format!("Could not inspect Compose containers: {e}"))
        })?;
    if !output.status.success() {
        tracing::warn!(application_id, error = %tail(&output.stderr), "podman container fallback failed");
        return Ok(Vec::new());
    }
    Ok(
        parse_compose_status(application_id, &runtime_name, &output.stdout)
            .into_iter()
            .filter(|service| {
                service.container_name.contains(&runtime_name)
                    || service.container_name.contains(application_id)
            })
            .collect(),
    )
}

/// Podman writes some `ps --format json` fields as a single string and others as a
/// one-element array depending on version, so every reader of that output goes
/// through this.
pub(crate) fn json_string(value: Option<&serde_json::Value>) -> Option<&str> {
    match value? {
        serde_json::Value::String(value) => Some(value),
        serde_json::Value::Array(values) => values.first()?.as_str(),
        _ => None,
    }
}

pub(super) fn parse_compose_status(
    application_id: &str,
    runtime_name: &str,
    output: &[u8],
) -> Vec<ComposeServiceStatus> {
    if output.iter().all(u8::is_ascii_whitespace) {
        return Vec::new();
    }
    let value = match serde_json::from_slice::<serde_json::Value>(output) {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(%error, output = %String::from_utf8_lossy(output), "Could not parse container status JSON");
            return Vec::new();
        }
    };
    let entries = value.as_array().cloned().unwrap_or_else(|| vec![value]);
    entries
        .into_iter()
        .filter_map(|entry| {
            let name = json_string(
                entry
                    .get("Name")
                    .or_else(|| entry.get("name"))
                    .or_else(|| entry.get("Names"))
                    .or_else(|| entry.get("names")),
            )?
            .to_string();
            let status = json_string(
                entry
                    .get("State")
                    .or_else(|| entry.get("state"))
                    .or_else(|| entry.get("Status"))
                    .or_else(|| entry.get("status")),
            )
            .unwrap_or("unknown")
            .to_string();
            let ports = json_string(entry.get("Ports").or_else(|| entry.get("ports")))
                .map(|ports| ports.split(", ").map(String::from).collect())
                .unwrap_or_default();
            let service = json_string(entry.get("Service").or_else(|| entry.get("service")))
                .map(String::from)
                .or_else(|| {
                    name.strip_prefix(&format!("{runtime_name}-"))
                        .map(String::from)
                })
                .or_else(|| {
                    name.strip_prefix(&format!("litepod-{application_id}-"))
                        .map(String::from)
                })
                .unwrap_or_else(|| name.rsplit('_').next().unwrap_or(&name).to_string());
            Some(ComposeServiceStatus {
                service,
                status,
                ports,
                container_name: name,
            })
        })
        .collect()
}
