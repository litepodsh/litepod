use super::super::{compose_standalone, domains, git, source};

use std::{collections::HashMap, ffi::OsString};

use podman_api::{Podman, opts::ContainerDeleteOpts};
use sqlx::Row;

use crate::{
    deployment::compose::store::ComposeFileStore,
    dtos::{
        deployment::{DeploymentDto, VolumeDto},
        response::AppError,
    },
    state::state::SharedState,
};

use super::identity::*;
use super::queue_ops::*;
use super::resources::*;

pub async fn stop(
    state: &SharedState,
    application_id: &str,
    created_by: Option<&str>,
) -> Result<DeploymentDto, AppError> {
    let servers = crate::servers::groups::resolve_target_servers(state, application_id).await?;
    // A group (even of one member) only resolves its runtime inside a replica scope.
    let is_group = crate::servers::replica::target_server_id(state, application_id)
        .await?
        .is_none();
    if !is_group {
        return stop_single(state, application_id, created_by).await;
    }
    let mut first_error: Option<AppError> = None;
    for server in &servers {
        let result = crate::servers::replica::for_replica(
            &server.id,
            stop_single(state, application_id, created_by),
        )
        .await;
        match result {
            Ok(_) => {
                let _ = crate::deployment::replicas::set_status(
                    state,
                    application_id,
                    &server.id,
                    "stopped",
                    None,
                )
                .await;
            }
            Err(error) => {
                let _ = crate::deployment::replicas::set_status(
                    state,
                    application_id,
                    &server.id,
                    "failed",
                    Some(&error.to_string()),
                )
                .await;
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => deployment(state, application_id).await,
    }
}

async fn stop_single(
    state: &SharedState,
    application_id: &str,
    created_by: Option<&str>,
) -> Result<DeploymentDto, AppError> {
    ensure_deployment(state, application_id).await?;
    crate::deployment::boot::mark_stopped(state, application_id).await?;
    let source_config = source::get(state, application_id).await?;
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row =
        sqlx::query("SELECT container_name FROM application_deployments WHERE application_id = ?1")
            .bind(application_id)
            .fetch_optional(&conn)
            .await
            .map_err(|e| AppError::DatabaseError(e.to_string()))?
            .ok_or(AppError::ApplicationNotFound)?;
    let container_name: String = row
        .try_get(0)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    if source_config.deploy_type == "compose" {
        return compose_standalone::stop(state, application_id, created_by).await;
    }
    let runtime =
        crate::servers::runtime::ServerRuntime::for_application(state, application_id).await?;
    if runtime.is_remote()
        && source_config.deploy_type == "git"
        && source_config.build_method == "compose"
    {
        git::compose::down(state, application_id, &source_config)
            .await
            .map_err(AppError::InternalError)?;
        drop(conn);
        for domain in domains::list(state, application_id).await? {
            domains::remove_caddy_route(state, application_id, &domain.id, &domain.host).await;
        }
        let conn = crate::git::repositories::db::get_conn(state).await;
        sqlx::query("UPDATE application_deployments SET status = 'stopped', updated_at = ?1 WHERE application_id = ?2")
            .bind(now_ms())
            .bind(application_id)
            .execute(&conn)
            .await
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        return deployment(state, application_id).await;
    }
    if runtime.is_remote() {
        // Remote lifecycle must never fall through to litepod's Podman socket.
        // `rm --force` is idempotent enough for a stopped/missing container and
        // keeps anonymous volumes tied to the assigned server.
        let _ = runtime
            .podman(&[
                OsString::from("rm"),
                OsString::from("--force"),
                OsString::from("--volumes"),
                OsString::from(&container_name),
            ])
            .await;
        drop(conn);
        for domain in domains::list(state, application_id).await? {
            domains::remove_caddy_route(state, application_id, &domain.id, &domain.host).await;
        }
        let conn = crate::git::repositories::db::get_conn(state).await;
        sqlx::query("UPDATE application_deployments SET status = 'stopped', updated_at = ?1 WHERE application_id = ?2")
            .bind(now_ms())
            .bind(application_id)
            .execute(&conn)
            .await
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        return deployment(state, application_id).await;
    }
    drop(conn);
    if source_config.deploy_type == "git" && source_config.build_method == "compose" {
        git::compose::down(state, application_id, &source_config)
            .await
            .map_err(AppError::InternalError)?;
    } else {
        let podman = Podman::new(&state.podman_socket)
            .map_err(|e| AppError::InternalError(format!("Invalid Podman socket: {e}")))?;
        // volumes(true) drops anonymous volumes attached to the container; the shared
        // data volume is named and stays untouched.
        let _ = podman
            .containers()
            .get(&container_name)
            .delete(
                &ContainerDeleteOpts::builder()
                    .force(true)
                    .volumes(true)
                    .build(),
            )
            .await;
    }
    for domain in domains::list(state, application_id).await? {
        domains::remove_caddy_route(state, application_id, &domain.id, &domain.host).await;
    }
    // Routes created before domains existed used a per-application id.
    if let Ok((client, base)) = crate::deployment::caddy_admin::client(&state.caddy_admin_url, None)
    {
        let _ = client
            .delete(format!("{base}/id/litepod-route-{application_id}"))
            .send()
            .await;
    }
    let conn = crate::git::repositories::db::get_conn(state).await;
    crate::git::repositories::db::retry_on_stale_snapshot(|| async {
        sqlx::query("UPDATE application_deployments SET status = 'stopped', updated_at = ?1 WHERE application_id = ?2")
            .bind(now_ms())
            .bind(application_id)
            .execute(&conn)
            .await
            .map_err(|e| AppError::DatabaseError(e.to_string()))
    })
    .await?;
    deployment(state, application_id).await
}

/// Removes runtime containers and their managed volumes but retains the
/// application record and source configuration for a future deploy.
pub async fn remove_runtime(
    state: &SharedState,
    application_id: &str,
    created_by: Option<&str>,
) -> Result<DeploymentDto, AppError> {
    ensure_deployment(state, application_id).await?;
    crate::deployment::boot::mark_stopped(state, application_id).await?;
    let source_config = source::get(state, application_id).await?;
    if source_config.deploy_type == "compose" {
        return compose_standalone::remove(state, application_id, created_by).await;
    }
    if source_config.deploy_type == "git" && source_config.build_method == "compose" {
        git::compose::remove(state, application_id, &source_config)
            .await
            .map_err(AppError::InternalError)?;
        let conn = crate::git::repositories::db::get_conn(state).await;
        sqlx::query("UPDATE application_deployments SET status = 'stopped', updated_at = ?1 WHERE application_id = ?2")
            .bind(now_ms())
            .bind(application_id)
            .execute(&conn)
            .await
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        return deployment(state, application_id).await;
    }
    stop(state, application_id, created_by).await
}

/// Podman reports a named-volume mount's `Source` as its backing storage path.
/// That path lives inside the Podman VM on macOS and is not a valid volume name
/// when the workspace helper container mounts it. Prefer `Name` for named
/// volumes while retaining `Source` for bind mounts.
pub(super) fn mount_source(mount: &serde_json::Value) -> String {
    let source = mount
        .get("Source")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    if mount.get("Type").and_then(|value| value.as_str()) == Some("volume") {
        return mount
            .get("Name")
            .and_then(|value| value.as_str())
            .filter(|name| !name.is_empty())
            .unwrap_or(source)
            .to_owned();
    }
    source.to_owned()
}

pub async fn volumes(
    state: &SharedState,
    application_id: &str,
) -> Result<Vec<VolumeDto>, AppError> {
    ensure_deployment(state, application_id).await?;
    let source_config = source::get(state, application_id).await?;
    let runtime =
        crate::servers::runtime::ServerRuntime::for_application(state, application_id).await?;
    if runtime.is_remote() {
        let container_names = if source_config.deploy_type == "compose" {
            compose_standalone::status(state, application_id)
                .await?
                .into_iter()
                .map(|service| service.container_name)
                .collect::<Vec<_>>()
        } else {
            let conn = crate::git::repositories::db::get_conn(state).await;
            let row = sqlx::query("SELECT status, container_name FROM application_deployments WHERE application_id = ?1")
                .bind(application_id)
                .fetch_optional(&conn)
                .await
                .map_err(|e| AppError::DatabaseError(e.to_string()))?
                .ok_or(AppError::ApplicationNotFound)?;
            let status: String = row
                .try_get(0)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?;
            if status == "running" {
                vec![
                    row.try_get(1)
                        .map_err(|e| AppError::DatabaseError(e.to_string()))?,
                ]
            } else {
                Vec::new()
            }
        };
        let mut mounts: HashMap<(String, String), VolumeDto> = HashMap::new();
        for container_name in container_names {
            let output = runtime
                .podman(&[
                    OsString::from("container"),
                    OsString::from("inspect"),
                    OsString::from(&container_name),
                    OsString::from("--format"),
                    OsString::from("json"),
                ])
                .await?;
            let parsed: serde_json::Value =
                serde_json::from_str(&output.stdout).map_err(|error| {
                    AppError::InternalError(format!(
                        "Remote Podman returned invalid inspect JSON: {error}"
                    ))
                })?;
            // `podman container inspect --format json` always wraps its result in an
            // array, even for one container - unlike the local branch below, which
            // gets an already-unwrapped object from the podman-api crate. Reading
            // `Mounts` straight off `parsed` silently returns nothing here (`.get`
            // on an array ignores string keys), so every remote volume looked empty
            // regardless of what was actually mounted.
            let json = parsed.as_array().and_then(|values| values.first());
            for mount in json
                .and_then(|value| value.get("Mounts"))
                .and_then(|value| value.as_array())
                .into_iter()
                .flatten()
            {
                let source = mount_source(mount);
                let destination = mount
                    .get("Destination")
                    .and_then(|value| value.as_str())
                    .unwrap_or("")
                    .to_owned();
                mounts
                    .entry((source.clone(), destination.clone()))
                    .or_insert_with(|| VolumeDto {
                        mount_type: mount
                            .get("Type")
                            .and_then(|value| value.as_str())
                            .unwrap_or("unknown")
                            .to_owned(),
                        source,
                        destination,
                        read_only: mount
                            .get("RW")
                            .and_then(|value| value.as_bool())
                            .map(|value| !value)
                            .unwrap_or(false),
                        services: Vec::new(),
                    });
            }
        }
        let mut mounts: Vec<_> = mounts.into_values().collect();
        mounts.sort_by(|left, right| {
            left.source
                .cmp(&right.source)
                .then(left.destination.cmp(&right.destination))
        });
        return Ok(mounts);
    }
    // Compose stores a project name in `application_deployments`, so inspect every
    // service container instead of treating that project name as a container ID.
    if source_config.deploy_type == "compose" {
        let podman = Podman::new(&state.podman_socket)
            .map_err(|e| AppError::InternalError(format!("Invalid Podman socket: {e}")))?;
        let mut mounts: HashMap<(String, String), VolumeDto> = HashMap::new();
        for service in compose_standalone::status(state, application_id).await? {
            let inspect = match podman
                .containers()
                .get(&service.container_name)
                .inspect()
                .await
            {
                Ok(inspect) => inspect,
                Err(error) => {
                    tracing::warn!(application_id, service = %service.service, %error, "Could not inspect Compose service mounts");
                    continue;
                }
            };
            let json = serde_json::to_value(inspect)
                .map_err(|e| AppError::InternalError(e.to_string()))?;
            for mount in json
                .get("Mounts")
                .and_then(|value| value.as_array())
                .into_iter()
                .flatten()
            {
                let source = mount_source(mount);
                let destination = mount
                    .get("Destination")
                    .and_then(|value| value.as_str())
                    .unwrap_or("")
                    .to_owned();
                let entry = mounts
                    .entry((source.clone(), destination.clone()))
                    .or_insert_with(|| VolumeDto {
                        mount_type: mount
                            .get("Type")
                            .and_then(|value| value.as_str())
                            .unwrap_or("unknown")
                            .to_owned(),
                        source,
                        destination,
                        read_only: mount
                            .get("RW")
                            .and_then(|value| value.as_bool())
                            .map(|value| !value)
                            .unwrap_or(false),
                        services: Vec::new(),
                    });
                if !entry.services.contains(&service.service) {
                    entry.services.push(service.service.clone());
                }
            }
        }
        let mut mounts: Vec<_> = mounts.into_values().collect();
        mounts.sort_by(|left, right| {
            left.source
                .cmp(&right.source)
                .then(left.destination.cmp(&right.destination))
        });
        return Ok(mounts);
    }
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(
        "SELECT status, container_name FROM application_deployments WHERE application_id = ?1",
    )
    .bind(application_id)
    .fetch_optional(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?
    .ok_or(AppError::ApplicationNotFound)?;
    let status: String = row
        .try_get(0)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    if status != "running" {
        return Ok(Vec::new());
    }
    let container_name: String = row
        .try_get(1)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let podman = Podman::new(&state.podman_socket)
        .map_err(|e| AppError::InternalError(format!("Invalid Podman socket: {e}")))?;
    let inspect = podman.containers().get(&container_name).inspect().await.map_err(|_| AppError::ValidationError("The deployment container is no longer available; redeploy it to restore volume inspection".into()))?;
    let json = serde_json::to_value(inspect).map_err(|e| AppError::InternalError(e.to_string()))?;
    Ok(json
        .get("Mounts")
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
        .map(|mount| VolumeDto {
            mount_type: mount
                .get("Type")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown")
                .to_owned(),
            source: mount_source(mount),
            destination: mount
                .get("Destination")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .to_owned(),
            read_only: mount
                .get("RW")
                .and_then(|value| value.as_bool())
                .map(|value| !value)
                .unwrap_or(false),
            services: Vec::new(),
        })
        .collect())
}

pub async fn start(
    state: &SharedState,
    application_id: &str,
    created_by: Option<&str>,
) -> Result<DeploymentDto, AppError> {
    ensure_deployment(state, application_id).await?;
    let source_config = source::get(state, application_id).await?;
    if source_config.deploy_type == "compose" {
        return compose_standalone::start(state, application_id, created_by).await;
    }
    // Non-compose apps rebuild on start so they pick up current environment
    // variables. podman start would reuse the image from the last build with
    // stale vars baked in.
    enqueue_for_application(state, application_id, "manual_deploy", created_by).await?;
    deployment(state, application_id).await
}

pub async fn cleanup(state: &SharedState, application_id: &str) {
    // Caddy routes (local or a remote server's native config) outlive the SQL
    // cascade on `applications`, so tear them down while the domain rows that
    // name them still exist - otherwise the fragment is orphaned on disk
    // forever and can collide with an unrelated domain that later reuses the
    // same host ("ambiguous site definition" from Caddy).
    if let Ok(app_domains) = domains::list(state, application_id).await {
        for domain in app_domains {
            domains::remove_caddy_route(state, application_id, &domain.id, &domain.host).await;
        }
    }
    let _ = remove_runtime(state, application_id, None).await;
    // An internal database resource owns a network of its own. Drop it here and
    // not in `force_project_teardown`, which also runs on every redeploy.
    if crate::deployment::resource_networks::is_resource(state, application_id)
        .await
        .unwrap_or(false)
        && let Ok(runtime) =
            crate::servers::runtime::ServerRuntime::for_application(state, application_id).await
    {
        crate::deployment::resource_networks::remove_network(&runtime, application_id).await;
    }
    let _ = std::fs::remove_dir_all(app_root(state, application_id));
    // Internal DB templates (SQL Server, Postgres, ...) write their own
    // compose files + port-overrides.yml under `data_dir/compose/<id>`,
    // separate from `app_root`. Without this, a deleted application's host
    // port stays "reserved" forever in `ComposeFileStore::list_host_ports`.
    let _ = ComposeFileStore::new(&state.data_dir, application_id)
        .remove_all()
        .await;
}
