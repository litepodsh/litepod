use std::{
    ffi::OsString,
    path::{Component, Path},
};

use tokio::process::Command;

use crate::{dtos::deployment::SourceConfigDto, state::state::SharedState};

use super::super::{decrypted_environment, domains, managed_volumes};

fn service_exists(content: &str, service: &str) -> Result<bool, String> {
    let value: serde_yaml::Value =
        serde_yaml::from_str(content).map_err(|error| format!("Invalid Compose file: {error}"))?;
    let services = value
        .get("services")
        .and_then(serde_yaml::Value::as_mapping)
        .ok_or_else(|| "Invalid Compose file: expected a services mapping".to_string())?;
    Ok(services.contains_key(serde_yaml::Value::String(service.to_string())))
}

fn override_yaml(service: &str, alias: &str, network: &str, container_name: &str) -> String {
    format!(
        "services:\n  {service}:\n    container_name: {container_name}\n    networks:\n      litepod_proxy:\n        aliases:\n          - {alias}\nnetworks:\n  litepod_proxy:\n    external: true\n    name: {network}\n"
    )
}

fn remote_override_yaml(
    service: &str,
    container_name: &str,
    host_port: u16,
    container_port: u16,
) -> String {
    let host_port = if host_port == 0 {
        String::new()
    } else {
        host_port.to_string()
    };
    let mut document = serde_yaml::Mapping::new();
    let mut services = serde_yaml::Mapping::new();
    let mut config = serde_yaml::Mapping::new();
    config.insert(
        serde_yaml::Value::String("container_name".into()),
        serde_yaml::Value::String(container_name.into()),
    );
    config.insert(
        serde_yaml::Value::String("ports".into()),
        serde_yaml::Value::Sequence(vec![serde_yaml::Value::String(format!(
            "127.0.0.1:{host_port}:{container_port}/tcp"
        ))]),
    );
    services.insert(
        serde_yaml::Value::String(service.into()),
        serde_yaml::Value::Mapping(config),
    );
    document.insert(
        serde_yaml::Value::String("services".into()),
        serde_yaml::Value::Mapping(services),
    );
    serde_yaml::to_string(&document).unwrap_or_default()
}

fn safe_repo_path(repo_root: &str, configured_path: &str) -> Result<String, String> {
    let path = Path::new(configured_path);
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| !matches!(component, Component::CurDir | Component::Normal(_)))
    {
        return Err("Compose file path must stay inside the repository".into());
    }
    Ok(format!("{repo_root}/{configured_path}"))
}

fn remote_generated_path(
    workspace: &str,
    compose_path: &str,
    filename: &str,
) -> Result<String, String> {
    let path = Path::new(compose_path);
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| !matches!(component, Component::CurDir | Component::Normal(_)))
    {
        return Err("Compose file path must stay inside the repository".into());
    }
    let parent = path
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => value.to_str(),
            Component::CurDir => None,
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/");
    if parent.is_empty() {
        Ok(format!("{workspace}/repo/{filename}"))
    } else {
        Ok(format!("{workspace}/repo/{parent}/{filename}"))
    }
}

fn env_file_contents(environment: &[(String, String)]) -> Result<String, String> {
    environment
        .iter()
        .map(|(name, value)| {
            if value.contains(['\n', '\r']) {
                Err(format!(
                    "Runtime variable {name} contains a newline and cannot be used in a Compose env file"
                ))
            } else {
                Ok(format!("{name}={value}\n"))
            }
        })
        .collect()
}

fn tail(output: &[u8]) -> String {
    String::from_utf8_lossy(output)
        .lines()
        .rev()
        .take(40)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n")
}

/// The Compose override that joins this application's services to the networks
/// of the database resources it is linked to, and creates those networks on the
/// server first. Empty when the application has no links.
async fn resource_networks_override(
    state: &SharedState,
    application_id: &str,
    content: &str,
    runtime: &crate::servers::runtime::ServerRuntime,
) -> Result<String, String> {
    super::super::resource_networks::ensure_for_deploy(state, runtime, application_id)
        .await
        .map_err(|error| error.to_string())?;
    let services = super::super::compose::pull::extract_service_images(content)?
        .into_iter()
        .map(|service| service.name)
        .collect::<Vec<_>>();
    super::super::resource_networks::compose_override(state, application_id, &services)
        .await
        .map_err(|error| error.to_string())
}

async fn compose_command(
    state: &SharedState,
    application_id: &str,
    source: &SourceConfigDto,
    repo: &Path,
    action: &[&str],
    project: &str,
) -> Result<(), String> {
    let path = source
        .compose_file_path
        .as_deref()
        .ok_or_else(|| "Compose file path is missing".to_string())?;
    let override_path = repo.join("litepod.compose.override.yml");
    let managed_override_path = repo.join("litepod.managed-volumes.override.yml");
    let networks_override_path = repo.join("litepod.networks.override.yml");
    let environment = decrypted_environment(state, application_id, "runtime")
        .await
        .map_err(|error| error.to_string())?;
    let mut command = Command::new("podman");
    #[cfg(target_os = "linux")]
    command.arg("--url").arg(&state.podman_socket);
    command
        .arg("compose")
        .arg("--project-name")
        .arg(project)
        .arg("--file")
        .arg(path)
        .arg("--file")
        .arg(&override_path);
    if managed_override_path.exists() {
        command.arg("--file").arg(&managed_override_path);
    }
    if networks_override_path.exists() {
        command.arg("--file").arg(&networks_override_path);
    }
    command.args(action).current_dir(repo).envs(environment);
    // Cancelling a queued deployment drops this future; without `kill_on_drop`
    // the provider would keep running unsupervised after the Stop button.
    command.kill_on_drop(true);
    // After `envs`, so an application variable can never shadow it: the compose
    // provider spawns its own `podman` children, and they only find the mounted
    // socket through `CONTAINER_HOST`.
    #[cfg(target_os = "linux")]
    command.env("CONTAINER_HOST", &state.podman_socket);
    let output = command
        .output()
        .await
        .map_err(|error| format!("Could not run podman compose: {error}"))?;
    for line in String::from_utf8_lossy(&output.stdout)
        .lines()
        .chain(String::from_utf8_lossy(&output.stderr).lines())
    {
        tracing::info!(application_id, "[compose] {line}");
    }
    if output.status.success() {
        Ok(())
    } else {
        Err(format!("Podman Compose failed: {}", tail(&output.stderr)))
    }
}

fn local_override_yaml(
    service: &str,
    alias: &str,
    network: &str,
    container_name: &str,
    dev_port: Option<u16>,
) -> Result<String, String> {
    let base = override_yaml(service, alias, network, container_name);
    let Some(port) = dev_port else {
        return Ok(base);
    };
    let mut document: serde_yaml::Value =
        serde_yaml::from_str(&base).map_err(|error| error.to_string())?;
    document["services"][service]["ports"] =
        serde_yaml::Value::Sequence(vec![format!("127.0.0.1::{port}/tcp").into()]);
    serde_yaml::to_string(&document).map_err(|error| error.to_string())
}

async fn project_containers(
    runtime: &crate::servers::runtime::ServerRuntime,
    project: &str,
) -> Result<Vec<String>, String> {
    let output = runtime
        .podman(&[
            "ps".into(),
            "--all".into(),
            "--filter".into(),
            format!("label=io.podman.compose.project={project}").into(),
            "--format".into(),
            "{{.Names}}".into(),
        ])
        .await
        .map_err(|error| error.to_string())?;
    let containers = output
        .stdout
        .lines()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    if containers.is_empty() {
        return Err("Compose created no containers".into());
    }
    Ok(containers)
}

async fn finish_candidate(
    runtime: &crate::servers::runtime::ServerRuntime,
    rollout: &crate::deployment::rollout::Rollout,
    result: Result<(), String>,
) -> Result<(), String> {
    if result
        .as_ref()
        .is_err_and(|error| !error.contains("ingress rollback failed"))
    {
        rollout.cleanup_candidate(runtime).await;
    }
    result
}

async fn run_remote(
    state: &SharedState,
    application_id: &str,
    source: &SourceConfigDto,
    repo: &Path,
    compose_path: &str,
    service: &str,
    content: &str,
    rollout: &crate::deployment::rollout::Rollout,
) -> Result<(), String> {
    if !managed_volumes::runtime_mounts(state, application_id)
        .await
        .map_err(|error| error.to_string())?
        .is_empty()
    {
        return Err("Managed-volume mounts for remote Git Compose deployments are not ready yet; detach them before deploying this application".into());
    }
    let runtime = crate::servers::runtime::ServerRuntime::for_application(state, application_id)
        .await
        .map_err(|error| error.to_string())?;
    let container_port = u16::try_from(
        source
            .container_port
            .ok_or_else(|| "Compose container port is missing".to_string())?,
    )
    .map_err(|_| "Compose container port must be between 1 and 65535".to_string())?;
    let public_container_name = format!("{}-public", rollout.name);
    let workspace = runtime
        .ensure_workspace(application_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Remote Git Compose workspace was not created".to_string())?;

    // Provider credentials are not copied to the slave. The clone metadata is
    // no longer needed once litepod recorded the commit, so exclude .git before
    // transferring the build context.
    let _ = std::fs::remove_dir_all(repo.join(".git"));
    let remote_repo = runtime
        .upload_workspace_tree(application_id, repo, "repo")
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Remote Git Compose workspace was not created".to_string())?;
    let _ = safe_repo_path(&remote_repo, compose_path)?;

    let sanitized = super::super::compose_standalone::prepare_remote_compose(content)?;
    let compose_parent = Path::new(compose_path)
        .parent()
        .unwrap_or_else(|| Path::new(""));
    let sanitized_path = repo
        .join(compose_parent)
        .join("litepod.git.remote.compose.yml");
    tokio::fs::write(&sanitized_path, sanitized)
        .await
        .map_err(|error| format!("Could not write remote Git Compose file: {error}"))?;
    let override_path = repo
        .join(compose_parent)
        .join("litepod.git.remote.override.yml");
    tokio::fs::write(
        &override_path,
        remote_override_yaml(service, &public_container_name, 0, container_port),
    )
    .await
    .map_err(|error| format!("Could not write remote Git ingress override: {error}"))?;
    // Empty when this application is linked to no database resource, in which
    // case no third file is uploaded and the command below is unchanged.
    let networks_override =
        resource_networks_override(state, application_id, content, &runtime).await?;
    let networks_path = repo
        .join(compose_parent)
        .join("litepod.git.remote.networks.yml");
    if !networks_override.is_empty() {
        tokio::fs::write(&networks_path, networks_override)
            .await
            .map_err(|error| {
                format!("Could not write remote resource network override: {error}")
            })?;
    }
    let remote_sanitized_stage = runtime
        .upload_workspace_file(
            application_id,
            &sanitized_path,
            "litepod.git.remote.compose.yml",
        )
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Remote Git Compose file was not uploaded".to_string())?;
    let remote_override_stage = runtime
        .upload_workspace_file(
            application_id,
            &override_path,
            "litepod.git.remote.override.yml",
        )
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Remote Git ingress override was not uploaded".to_string())?;
    let remote_sanitized =
        remote_generated_path(&workspace, compose_path, "litepod.git.remote.compose.yml")?;
    let remote_override =
        remote_generated_path(&workspace, compose_path, "litepod.git.remote.override.yml")?;
    runtime
        .remote_command(&[
            OsString::from("mv"),
            OsString::from(remote_sanitized_stage),
            OsString::from(&remote_sanitized),
        ])
        .await
        .map_err(|error| error.to_string())?;
    runtime
        .remote_command(&[
            OsString::from("mv"),
            OsString::from(remote_override_stage),
            OsString::from(&remote_override),
        ])
        .await
        .map_err(|error| error.to_string())?;
    let remote_networks = if networks_path.exists() {
        let stage = runtime
            .upload_workspace_file(
                application_id,
                &networks_path,
                "litepod.git.remote.networks.yml",
            )
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "Remote resource network override was not uploaded".to_string())?;
        let target =
            remote_generated_path(&workspace, compose_path, "litepod.git.remote.networks.yml")?;
        runtime
            .remote_command(&[
                OsString::from("mv"),
                OsString::from(stage),
                OsString::from(&target),
            ])
            .await
            .map_err(|error| error.to_string())?;
        Some(target)
    } else {
        None
    };
    let environment = decrypted_environment(state, application_id, "runtime")
        .await
        .map_err(|error| error.to_string())?;
    let env_file = crate::servers::ssh::SecretFile::write(&env_file_contents(&environment)?)
        .map_err(|error| error.to_string())?;
    let remote_env = runtime
        .upload_workspace_file(application_id, env_file.path(), "litepod.git.runtime.env")
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Remote Git runtime environment was not uploaded".to_string())?;
    let project = &rollout.project;
    // Keep this vector closed and explicit: no user-provided Compose path can
    // reach SSH argv, and the only variable part is the optional override that
    // this deploy generated itself.
    let mut up = vec![
        OsString::from("compose"),
        OsString::from("--project-name"),
        OsString::from(project),
        OsString::from("--env-file"),
        OsString::from(remote_env),
        OsString::from("--file"),
        OsString::from(remote_sanitized),
        OsString::from("--file"),
        OsString::from(remote_override),
    ];
    if let Some(remote_networks) = remote_networks {
        up.push(OsString::from("--file"));
        up.push(OsString::from(remote_networks));
    }
    up.extend([
        OsString::from("up"),
        OsString::from("--detach"),
        OsString::from("--build"),
        OsString::from("--force-recreate"),
    ]);
    let candidate = async {
        runtime
            .podman(&up)
            .await
            .map_err(|error| format!("Remote Podman Compose failed: {error}"))?;
        let targets = domains::list(state, application_id)
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .filter(|domain| !domain.is_local)
            .map(|domain| {
                (
                    domain.id,
                    public_container_name.clone(),
                    container_port as i64,
                )
            })
            .collect::<Vec<_>>();
        let containers = project_containers(&runtime, project).await?;
        rollout
            .promote(
                state,
                application_id,
                &runtime,
                &public_container_name,
                &targets,
                &containers,
            )
            .await
    }
    .await;
    finish_candidate(&runtime, rollout, candidate).await?;
    Ok(())
}

pub async fn run(
    state: &SharedState,
    application_id: &str,
    source: &SourceConfigDto,
    repo: &Path,
) -> Result<(), String> {
    let compose_path = source
        .compose_file_path
        .as_deref()
        .ok_or_else(|| "Compose file path is missing".to_string())?;
    let service = source
        .compose_public_service
        .as_deref()
        .ok_or_else(|| "Compose public service is missing".to_string())?;
    let content = std::fs::read_to_string(repo.join(compose_path))
        .map_err(|error| format!("Could not read Compose file {compose_path}: {error}"))?;
    if !service_exists(&content, service)? {
        return Err(format!("Compose service '{service}' was not found"));
    }
    for managed_service in managed_volumes::attached_compose_services(state, application_id)
        .await
        .map_err(|error| error.to_string())?
    {
        if !service_exists(&content, &managed_service)? {
            return Err(format!(
                "Managed volume service '{managed_service}' was not found"
            ));
        }
    }
    let runtime = crate::servers::runtime::ServerRuntime::for_application(state, application_id)
        .await
        .map_err(|error| error.to_string())?;
    let rollout =
        crate::deployment::rollout::Rollout::prepare(state, application_id, &runtime, true).await?;
    if rollout.previous_exists {
        crate::deployment::rollout::validate_stateless_compose(&content)?;
    }
    let mounts = managed_volumes::runtime_mounts(state, application_id)
        .await
        .map_err(|error| error.to_string())?;
    rollout
        .check_replaceable(&runtime, !mounts.is_empty(), false)
        .await?;
    let _cleanup = rollout.cleanup_on_drop(&runtime);
    if domains::needs_default_domain(state, application_id)
        .await
        .map_err(|error| error.to_string())?
    {
        domains::generate(state, application_id)
            .await
            .map_err(|error| error.to_string())?;
    }
    if runtime.is_remote() {
        return run_remote(
            state,
            application_id,
            source,
            repo,
            compose_path,
            service,
            &content,
            &rollout,
        )
        .await;
    }
    let alias = &rollout.name;
    let public_container_name = format!("{alias}-public");
    std::fs::write(
        repo.join("litepod.compose.override.yml"),
        local_override_yaml(
            service,
            alias,
            &state.podman_network,
            &public_container_name,
            state
                .dev_mode
                .then_some(source.container_port.unwrap_or(80) as u16),
        )?,
    )
    .map_err(|error| format!("Could not prepare Compose override: {error}"))?;
    let managed_override_path = repo.join("litepod.managed-volumes.override.yml");
    let managed_override = managed_volumes::compose_override(state, application_id)
        .await
        .map_err(|error| error.to_string())?;
    if managed_override.is_empty() {
        let _ = std::fs::remove_file(&managed_override_path);
    } else {
        std::fs::write(&managed_override_path, managed_override)
            .map_err(|error| format!("Could not prepare managed volume override: {error}"))?;
    }
    let networks_override_path = repo.join("litepod.networks.override.yml");
    let networks_override =
        resource_networks_override(state, application_id, &content, &runtime).await?;
    if networks_override.is_empty() {
        let _ = std::fs::remove_file(&networks_override_path);
    } else {
        std::fs::write(&networks_override_path, networks_override)
            .map_err(|error| format!("Could not prepare resource network override: {error}"))?;
    }
    // `podman compose up` re-pulls only when it thinks an image is stale, so a
    // fresh `:latest` push is happily ignored. Pulling the registry images
    // ourselves beforehand makes the redeploy actually swap the container for
    // the new digest; the failure also has to fail the deploy, otherwise the
    // user is left with a stale 1.4.0 process and a green dashboard.
    super::super::compose::pull::pull_compose_images(
        &runtime,
        state,
        application_id,
        None,
        &content,
    )
    .await?;
    let candidate = async {
        compose_command(
            state,
            application_id,
            source,
            repo,
            &["up", "--detach", "--build"],
            &rollout.project,
        )
        .await?;
        let targets = domains::list(state, application_id)
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .filter(|domain| !domain.is_local)
            .map(|domain| {
                (
                    domain.id,
                    public_container_name.clone(),
                    source.container_port.unwrap_or(80),
                )
            })
            .collect::<Vec<_>>();
        let containers = project_containers(&runtime, &rollout.project).await?;
        rollout
            .promote(
                state,
                application_id,
                &runtime,
                &public_container_name,
                &targets,
                &containers,
            )
            .await
    }
    .await;
    finish_candidate(&runtime, &rollout, candidate).await?;
    Ok(())
}

pub async fn down(
    state: &SharedState,
    application_id: &str,
    source: &SourceConfigDto,
) -> Result<(), String> {
    let runtime = crate::servers::runtime::ServerRuntime::for_application(state, application_id)
        .await
        .map_err(|error| error.to_string())?;
    if runtime.is_remote() {
        let workspace = runtime
            .ensure_workspace(application_id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "Remote Git Compose workspace was not created".to_string())?;
        let compose_path = source
            .compose_file_path
            .as_deref()
            .ok_or_else(|| "Compose file path is missing".to_string())?;
        let remote_compose =
            remote_generated_path(&workspace, compose_path, "litepod.git.remote.compose.yml")?;
        let remote_override =
            remote_generated_path(&workspace, compose_path, "litepod.git.remote.override.yml")?;
        let args = vec![
            OsString::from("compose"),
            OsString::from("--project-name"),
            OsString::from(
                crate::deployment::rollout::active_project(state, application_id).await?,
            ),
            OsString::from("--env-file"),
            OsString::from(format!("{workspace}/litepod.git.runtime.env")),
            OsString::from("--file"),
            OsString::from(remote_compose),
            OsString::from("--file"),
            OsString::from(remote_override),
            OsString::from("down"),
            OsString::from("--remove-orphans"),
        ];
        // Keep this vector closed and explicit: no user-provided Compose path
        // or command can reach SSH argv at lifecycle time.
        runtime
            .podman(&args)
            .await
            .map_err(|error| format!("Remote Podman Compose stop failed: {error}"))?;
        return Ok(());
    }
    let repo = super::repo_dir(state, application_id);
    if !repo.exists() {
        return Ok(());
    }
    let result = compose_command(
        state,
        application_id,
        source,
        &repo,
        &["down", "--remove-orphans"],
        &crate::deployment::rollout::active_project(state, application_id).await?,
    )
    .await;
    let _ = std::fs::remove_dir_all(repo);
    result
}

pub async fn remove(
    state: &SharedState,
    application_id: &str,
    source: &SourceConfigDto,
) -> Result<(), String> {
    let runtime = crate::servers::runtime::ServerRuntime::for_application(state, application_id)
        .await
        .map_err(|error| error.to_string())?;
    if runtime.is_remote() {
        let workspace = runtime
            .ensure_workspace(application_id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "Remote Git Compose workspace was not created".to_string())?;
        let compose_path = source
            .compose_file_path
            .as_deref()
            .ok_or_else(|| "Compose file path is missing".to_string())?;
        let remote_compose =
            remote_generated_path(&workspace, compose_path, "litepod.git.remote.compose.yml")?;
        let remote_override =
            remote_generated_path(&workspace, compose_path, "litepod.git.remote.override.yml")?;
        runtime
            .podman(&[
                OsString::from("compose"),
                OsString::from("--project-name"),
                OsString::from(
                    crate::deployment::rollout::active_project(state, application_id).await?,
                ),
                OsString::from("--env-file"),
                OsString::from(format!("{workspace}/litepod.git.runtime.env")),
                OsString::from("--file"),
                OsString::from(remote_compose),
                OsString::from("--file"),
                OsString::from(remote_override),
                OsString::from("down"),
                OsString::from("--remove-orphans"),
                OsString::from("--volumes"),
            ])
            .await
            .map_err(|error| format!("Remote Podman Compose removal failed: {error}"))?;
        return Ok(());
    }
    let repo = super::repo_dir(state, application_id);
    if !repo.exists() {
        return Ok(());
    }
    let result = compose_command(
        state,
        application_id,
        source,
        &repo,
        &["down", "--remove-orphans", "--volumes"],
        &crate::deployment::rollout::active_project(state, application_id).await?,
    )
    .await;
    let _ = std::fs::remove_dir_all(repo);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replacement_overrides_use_isolated_names_and_ephemeral_loopback_ports() {
        let remote = remote_override_yaml("web", "app-release-123456abcdef-public", 0, 3000);
        assert!(remote.contains("127.0.0.1::3000/tcp"));
        assert!(remote.contains("app-release-123456abcdef-public"));
        let local = local_override_yaml(
            "web",
            "app-release-123456abcdef",
            "proxy",
            "app-release-123456abcdef-public",
            Some(3000),
        )
        .unwrap();
        assert!(local.contains("127.0.0.1::3000/tcp"));
        assert!(local.contains("app-release-123456abcdef-public"));
        assert!(local.contains("name: proxy"));
    }

    #[test]
    fn compose_file_must_name_the_public_service() {
        assert!(service_exists("services:\n  web: {}\n", "web").unwrap());
        assert!(!service_exists("services:\n  worker: {}\n", "web").unwrap());
        assert!(service_exists("services: [bad]", "web").is_err());
    }

    #[test]
    fn remote_override_publishes_only_the_selected_loopback_service() {
        let override_file = remote_override_yaml("web", "litepod-app-public", 42_123, 3000);
        assert!(override_file.contains("127.0.0.1:42123:3000/tcp"));
        assert!(override_file.contains("container_name: litepod-app-public"));
        assert!(!override_file.contains("litepod_proxy"));
        assert!(!override_file.contains("0.0.0.0"));
    }

    #[test]
    fn remote_compose_path_rejects_traversal() {
        assert!(safe_repo_path(".local/share/litepod/apps/a/repo", "./compose.yml").is_ok());
        assert!(safe_repo_path(".local/share/litepod/apps/a/repo", "../compose.yml").is_err());
        assert_eq!(
            remote_generated_path(
                ".local/share/litepod/apps/a",
                "infra/compose.yml",
                "litepod.git.remote.compose.yml"
            )
            .unwrap(),
            ".local/share/litepod/apps/a/repo/infra/litepod.git.remote.compose.yml"
        );
    }
}
