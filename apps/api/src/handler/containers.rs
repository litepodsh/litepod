use axum::{
    extract::{Path, Query, State},
    response::IntoResponse,
};
use serde_json::{Value, json};
use std::ffi::OsString;

use crate::{
    dtos::response::{AppError, ok},
    git::repositories::audit_log,
    security::authz::extract::{RequireApp, perm},
    state::state::SharedState,
};

/// The `RequireApp` extractor covers the first four segments; these
/// handlers only need `container_name` out of the path.
type ContainerPath = (String, String, String, String, String);

/// Optional replica pin for a group-targeted application. Single-server apps
/// leave it out and keep resolving through `applications.server_id`.
#[derive(Debug, Default, serde::Deserialize)]
pub struct ReplicaQuery {
    pub server_id: Option<String>,
}

/// Runs `future` against the replica named by `server_id`, after checking that
/// server is actually one of the application's targets — otherwise a caller
/// could aim container actions at any server in the organization.
pub(crate) async fn in_replica<T>(
    state: &SharedState,
    application_id: &str,
    server_id: Option<&str>,
    future: impl std::future::Future<Output = Result<T, AppError>>,
) -> Result<T, AppError> {
    let Some(server_id) = server_id.filter(|id| !id.is_empty()) else {
        return future.await;
    };
    let targets = crate::servers::groups::resolve_target_servers(state, application_id).await?;
    if !targets.iter().any(|server| server.id == server_id) {
        return Err(AppError::ResourceNotFound);
    }
    crate::servers::replica::for_replica(server_id, future).await
}

#[derive(Debug, serde::Serialize)]
pub struct ReplicaContainerDto {
    pub container_name: String,
    pub service: Option<String>,
    pub status: String,
    pub ports: Vec<String>,
}

#[derive(Debug, serde::Serialize)]
pub struct ReplicaContainersDto {
    pub server_id: String,
    pub server_name: String,
    pub os: Option<String>,
    pub distribution: Option<String>,
    pub address: Option<String>,
    /// Deployment outcome recorded for this replica (`application_replicas.status`).
    pub replica_status: String,
    pub last_error: Option<String>,
    pub deployed_at: Option<i64>,
    /// `false` when the server could not be asked; `containers` is then empty.
    pub reachable: bool,
    pub containers: Vec<ReplicaContainerDto>,
}

async fn replica_containers(
    state: &SharedState,
    application_id: &str,
    is_compose: bool,
) -> Result<Vec<ReplicaContainerDto>, AppError> {
    if is_compose {
        return Ok(
            crate::deployment::compose_standalone::status(state, application_id)
                .await?
                .into_iter()
                .map(|service| ReplicaContainerDto {
                    container_name: service.container_name,
                    service: Some(service.service),
                    status: service.status,
                    ports: service.ports,
                })
                .collect(),
        );
    }
    let names = crate::handler::deployments::container_names(state, application_id).await?;
    let runtime =
        crate::servers::runtime::ServerRuntime::for_application(state, application_id).await?;
    let mut containers = Vec::with_capacity(names.len());
    for name in names {
        let status = crate::handler::deployments::inspect_container_state(&runtime, &name).await;
        containers.push(ReplicaContainerDto {
            container_name: name,
            service: None,
            status,
            ports: Vec::new(),
        });
    }
    Ok(containers)
}

/// Live containers per target server, for the grouped view of a group-targeted
/// application. A single-server app gets one entry for its own server.
pub async fn replica_containers_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppRead>,
) -> Result<impl IntoResponse, AppError> {
    let application_id = app.application_id;
    let is_compose = crate::deployment::source::get(&state, &application_id)
        .await?
        .deploy_type
        == "compose";
    let servers = crate::servers::groups::resolve_target_servers(&state, &application_id).await?;
    let recorded = crate::deployment::replicas::list(&state, &application_id).await?;

    let lookups = servers.iter().map(|server| {
        let state = state.clone();
        let application_id = application_id.clone();
        let server_id = server.id.clone();
        async move {
            crate::servers::replica::for_replica(
                &server_id,
                replica_containers(&state, &application_id, is_compose),
            )
            .await
        }
    });
    let results = futures_util::future::join_all(lookups).await;

    let replicas = servers
        .into_iter()
        .zip(results)
        .map(|(server, result)| {
            let record = recorded
                .iter()
                .find(|replica| replica.server_id == server.id);
            let (reachable, containers) = match result {
                Ok(containers) => (true, containers),
                Err(_) => (false, Vec::new()),
            };
            let (os, distribution, address) = crate::dtos::server::replica_marks(&server);
            ReplicaContainersDto {
                os,
                distribution,
                address,
                replica_status: record
                    .map(|replica| replica.status.clone())
                    .unwrap_or_else(|| crate::deployment::replicas::STATUS_NOT_DEPLOYED.into()),
                last_error: record.and_then(|replica| replica.last_error.clone()),
                deployed_at: record.and_then(|replica| replica.deployed_at),
                server_id: server.id,
                server_name: server.name,
                reachable,
                containers,
            }
        })
        .collect::<Vec<_>>();
    Ok(ok(replicas))
}

/// Rejects a `container_name` that isn't one of this application's own
/// containers. The extractor proves the *application* belongs to the caller's
/// organization; it says nothing about a container id the caller typed, and
/// without this a member could reach any container on the host by enumerating
/// names.
///
/// Returns the runtime alongside, because every caller needs it next and
/// resolving it separately meant re-running the application lookup.
async fn container_runtime(
    state: &SharedState,
    application_id: &str,
    container_name: &str,
) -> Result<crate::servers::runtime::ServerRuntime, AppError> {
    let names = crate::handler::deployments::container_names(state, application_id).await?;
    if !names.iter().any(|name| name == container_name) {
        return Err(AppError::ResourceNotFound);
    }
    crate::servers::runtime::ServerRuntime::for_application(state, application_id).await
}

pub async fn container_inspect_handler(
    State(state): State<SharedState>,
    Query(replica): Query<ReplicaQuery>,
    RequireApp(app, _): RequireApp<perm::AppRead>,
    Path((_, _, _, _, container_name)): Path<ContainerPath>,
) -> Result<impl IntoResponse, AppError> {
    let output = in_replica(
        &state,
        &app.application_id,
        replica.server_id.as_deref(),
        async {
            let runtime = container_runtime(&state, &app.application_id, &container_name).await?;
            runtime
                .podman(&[OsString::from("inspect"), OsString::from(&container_name)])
                .await
                .map_err(|error| {
                    if crate::handler::deployments::missing_podman_resource(&error) {
                        AppError::ResourceNotFound
                    } else {
                        error
                    }
                })
        },
    )
    .await?;
    let mut value: Value = serde_json::from_str(&output.stdout).map_err(|error| {
        AppError::InternalError(format!("Podman inspect returned invalid JSON: {error}"))
    })?;
    let Some(mut object) = value.as_array_mut().and_then(|items| items.pop()) else {
        return Err(AppError::ResourceNotFound);
    };
    crate::handler::debug::redact_value(&mut object);
    Ok(ok(object))
}

pub async fn container_start_handler(
    State(state): State<SharedState>,
    Query(replica): Query<ReplicaQuery>,
    RequireApp(app, _): RequireApp<perm::AppDeploy>,
    Path((_, _, _, _, container_name)): Path<ContainerPath>,
) -> Result<impl IntoResponse, AppError> {
    in_replica(
        &state,
        &app.application_id,
        replica.server_id.as_deref(),
        async {
            let runtime = container_runtime(&state, &app.application_id, &container_name).await?;
            runtime
                .podman(&[OsString::from("start"), OsString::from(&container_name)])
                .await
        },
    )
    .await?;
    audit_log::record(
        &state,
        &app.organization_id,
        &app.user,
        "container.started",
        Some("container"),
        Some(&container_name),
        Some(&container_name),
        None,
        None,
        None,
        None,
    )
    .await
    .ok();
    Ok(ok(
        json!({ "action": "start", "container": container_name }),
    ))
}

pub async fn container_stop_handler(
    State(state): State<SharedState>,
    Query(replica): Query<ReplicaQuery>,
    RequireApp(app, _): RequireApp<perm::AppDeploy>,
    Path((_, _, _, _, container_name)): Path<ContainerPath>,
) -> Result<impl IntoResponse, AppError> {
    in_replica(
        &state,
        &app.application_id,
        replica.server_id.as_deref(),
        async {
            let runtime = container_runtime(&state, &app.application_id, &container_name).await?;
            runtime
                .podman(&[OsString::from("stop"), OsString::from(&container_name)])
                .await
        },
    )
    .await?;
    audit_log::record(
        &state,
        &app.organization_id,
        &app.user,
        "container.stopped",
        Some("container"),
        Some(&container_name),
        Some(&container_name),
        None,
        None,
        None,
        None,
    )
    .await
    .ok();
    Ok(ok(json!({ "action": "stop", "container": container_name })))
}

pub async fn container_restart_handler(
    State(state): State<SharedState>,
    Query(replica): Query<ReplicaQuery>,
    RequireApp(app, _): RequireApp<perm::AppDeploy>,
    Path((_, _, _, _, container_name)): Path<ContainerPath>,
) -> Result<impl IntoResponse, AppError> {
    in_replica(
        &state,
        &app.application_id,
        replica.server_id.as_deref(),
        async {
            let runtime = container_runtime(&state, &app.application_id, &container_name).await?;
            runtime
                .podman(&[OsString::from("restart"), OsString::from(&container_name)])
                .await
        },
    )
    .await?;
    audit_log::record(
        &state,
        &app.organization_id,
        &app.user,
        "container.restarted",
        Some("container"),
        Some(&container_name),
        Some(&container_name),
        None,
        None,
        None,
        None,
    )
    .await
    .ok();
    Ok(ok(
        json!({ "action": "restart", "container": container_name }),
    ))
}
