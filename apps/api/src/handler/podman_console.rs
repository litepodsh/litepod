//! Per-server Podman console. Every route resolves a [`ServerRuntime`] for one
//! server in the caller's organization, so the same handlers reach the local
//! Podman socket (self-hosted) or a remote host over SSH (cloud) without a
//! second code path. Replaces the old sysadmin `/debug/podman` surface.

use std::{convert::Infallible, ffi::OsString, time::Duration};

use axum::{
    body::Body,
    extract::{Multipart, Path, Query, State},
    http::header,
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use futures_util::Stream;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    dtos::deployment::{CreateWorkspaceEntryRequest, UpdateWorkspaceTextRequest},
    dtos::response::{AppError, ok},
    git::repositories::audit_log,
    handler::debug::redact_value,
    podman::containers::volume_workspace::{self, WorkspaceRoot},
    security::authz::extract::{RequireOrg, perm},
    servers::{repository, runtime::ServerRuntime},
    state::state::SharedState,
};

/// The org extractor covers `{organization_id}`; handlers take `{server_id}`
/// (and one more segment where present) out of the path.
type ServerPath = (String, String);
type ServerSubPath = (String, String, String);

const METRICS_STREAM_INTERVAL: Duration = Duration::from_secs(2);
/// Pruning a host with many dangling images/layers routinely runs past the 45s
/// SSH budget `runtime.podman()` uses, so give prune its own maintenance-sized one.
const PRUNE_TIMEOUT: Duration = Duration::from_secs(900);
const LOG_TAIL: &str = "200";

fn a(value: &str) -> OsString {
    OsString::from(value)
}

/// Resolves a runtime for a server the caller's organization owns (or the
/// shared local server). `get_for_organization` returns `None` for a server in
/// another org, which surfaces as "not found" rather than leaking its
/// existence. `ServerRuntime::for_server` then applies the Ready gate; on cloud,
/// `RequireOrg` already limited the `local` server to a SysAdmin.
async fn runtime_for(
    state: &SharedState,
    organization_id: &str,
    server_id: &str,
) -> Result<ServerRuntime, AppError> {
    let server = repository::get_for_server_route(state, server_id, organization_id)
        .await?
        .ok_or(AppError::ResourceNotFound)?;
    ServerRuntime::for_server(state, &server.id).await
}

/// Runs a read-only `podman` invocation and parses its stdout as JSON. An empty
/// stdout (some subcommands print nothing when there is nothing to list) maps
/// to an empty array so the UI never has to special-case it.
async fn podman_json(runtime: &ServerRuntime, args: &[OsString]) -> Result<Value, AppError> {
    let output = runtime.podman(args).await?;
    let trimmed = output.stdout.trim();
    if trimmed.is_empty() {
        return Ok(json!([]));
    }
    serde_json::from_str(trimmed)
        .map_err(|error| AppError::InternalError(format!("Podman returned invalid JSON: {error}")))
}

fn require_confirmation(actual: &str, expected: &str) -> Result<(), AppError> {
    if actual == expected {
        Ok(())
    } else {
        Err(AppError::ValidationError(format!(
            "Type \"{expected}\" exactly to confirm this operation"
        )))
    }
}

/// Guards a name the caller supplies for a `create` call. Podman would reject a
/// bad name too, but this keeps obviously unsafe input off the argv.
fn valid_name(name: &str) -> Result<(), AppError> {
    let ok = !name.is_empty()
        && name.len() <= 128
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'));
    if ok {
        Ok(())
    } else {
        Err(AppError::ValidationError("Invalid resource name".into()))
    }
}

#[derive(Deserialize)]
pub struct ConfirmRequest {
    pub confirmation: String,
}

#[derive(Deserialize)]
pub struct PullImageRequest {
    pub reference: String,
    pub confirmation: String,
}

#[derive(Deserialize)]
pub struct CreateVolumeRequest {
    pub name: String,
    pub driver: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
}

#[derive(Deserialize)]
pub struct CreateNetworkRequest {
    pub name: String,
    pub driver: Option<String>,
    pub subnet: Option<String>,
    pub gateway: Option<String>,
    #[serde(default)]
    pub internal: bool,
}

#[derive(Deserialize)]
pub struct NetworkMemberRequest {
    pub container: String,
}

#[derive(Deserialize, Default)]
pub struct EventsQuery {
    pub since: Option<String>,
}

fn audit(
    state: &SharedState,
    org: &crate::security::middleware::org_scope::OrgScope,
    action: &str,
    target_type: &str,
    target: &str,
) {
    let state = state.clone();
    let organization_id = org.organization_id.clone();
    let user = org.user.clone();
    let action = action.to_string();
    let target_type = target_type.to_string();
    let target = target.to_string();
    tokio::spawn(async move {
        audit_log::record(
            &state,
            &organization_id,
            &user,
            &action,
            Some(&target_type),
            Some(&target),
            Some(&target),
            None,
            None,
            None,
            None,
        )
        .await
        .ok();
    });
}

// ---------------------------------------------------------------------------
// Overview / health / disk usage
// ---------------------------------------------------------------------------

pub async fn overview_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerRead>,
    Path((_, server_id)): Path<ServerPath>,
) -> Result<impl IntoResponse, AppError> {
    let runtime = runtime_for(&state, &org.organization_id, &server_id).await?;
    let info = podman_json(&runtime, &[a("info"), a("--format"), a("json")]).await?;
    let version = podman_json(&runtime, &[a("version"), a("--format"), a("json")])
        .await
        .unwrap_or(Value::Null);
    Ok(ok(json!({
        "remote": runtime.is_remote(),
        "info": info,
        "version": version,
    })))
}

pub async fn system_df_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerRead>,
    Path((_, server_id)): Path<ServerPath>,
) -> Result<impl IntoResponse, AppError> {
    let runtime = runtime_for(&state, &org.organization_id, &server_id).await?;
    Ok(ok(podman_json(
        &runtime,
        &[a("system"), a("df"), a("--format"), a("json")],
    )
    .await?))
}

pub async fn prune_build_cache_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerManage>,
    Path((_, server_id)): Path<ServerPath>,
    axum::Json(request): axum::Json<ConfirmRequest>,
) -> Result<impl IntoResponse, AppError> {
    require_confirmation(&request.confirmation, "prune")?;
    let runtime = runtime_for(&state, &org.organization_id, &server_id).await?;
    let output = runtime
        .podman_with_timeout(&[a("builder"), a("prune"), a("--force")], PRUNE_TIMEOUT)
        .await?;
    audit(
        &state,
        &org,
        "podman.build_cache_pruned",
        "server",
        &server_id,
    );
    Ok(ok(json!({ "output": output.stdout })))
}

// ---------------------------------------------------------------------------
// Generic resource list / inspect / delete / prune
// ---------------------------------------------------------------------------

/// `containers | pods | images | volumes | networks` → the argv that lists it.
fn list_argv(resource: &str) -> Option<Vec<OsString>> {
    Some(match resource {
        "containers" => vec![a("ps"), a("--all"), a("--format"), a("json")],
        "pods" => vec![a("pod"), a("ps"), a("--format"), a("json")],
        "images" => vec![a("images"), a("--format"), a("json")],
        "volumes" => vec![a("volume"), a("ls"), a("--format"), a("json")],
        "networks" => vec![a("network"), a("ls"), a("--format"), a("json")],
        _ => return None,
    })
}

fn inspect_argv(resource: &str, id: &str) -> Option<Vec<OsString>> {
    Some(match resource {
        "containers" => vec![a("inspect"), a(id)],
        "pods" => vec![a("pod"), a("inspect"), a(id)],
        "images" => vec![a("image"), a("inspect"), a(id)],
        "volumes" => vec![a("volume"), a("inspect"), a(id)],
        "networks" => vec![a("network"), a("inspect"), a(id)],
        _ => return None,
    })
}

fn delete_argv(resource: &str, id: &str) -> Option<Vec<OsString>> {
    Some(match resource {
        "containers" => vec![a("rm"), a("--force"), a(id)],
        "pods" => vec![a("pod"), a("rm"), a("--force"), a(id)],
        "images" => vec![a("rmi"), a("--force"), a(id)],
        "volumes" => vec![a("volume"), a("rm"), a(id)],
        "networks" => vec![a("network"), a("rm"), a(id)],
        _ => return None,
    })
}

fn prune_argv(resource: &str) -> Option<Vec<OsString>> {
    Some(match resource {
        "images" => vec![a("image"), a("prune"), a("--all"), a("--force")],
        "volumes" => vec![a("volume"), a("prune"), a("--force")],
        "networks" => vec![a("network"), a("prune"), a("--force")],
        _ => return None,
    })
}

pub async fn list_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerRead>,
    Path((_, server_id, resource)): Path<ServerSubPath>,
) -> Result<impl IntoResponse, AppError> {
    let args = list_argv(&resource).ok_or(AppError::ResourceNotFound)?;
    let runtime = runtime_for(&state, &org.organization_id, &server_id).await?;
    Ok(ok(podman_json(&runtime, &args).await?))
}

pub async fn inspect_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerRead>,
    Path((_, server_id, resource, id)): Path<(String, String, String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let args = inspect_argv(&resource, &id).ok_or(AppError::ResourceNotFound)?;
    let runtime = runtime_for(&state, &org.organization_id, &server_id).await?;
    let output = runtime.podman(&args).await.map_err(|error| {
        if error.to_string().to_lowercase().contains("no such") {
            AppError::ResourceNotFound
        } else {
            error
        }
    })?;
    let value: Value = serde_json::from_str(output.stdout.trim()).map_err(|error| {
        AppError::InternalError(format!("Podman inspect returned invalid JSON: {error}"))
    })?;
    // `podman inspect` returns a single-element array.
    let mut object = match value {
        Value::Array(mut items) => items.pop().unwrap_or(Value::Null),
        other => other,
    };
    redact_value(&mut object);
    Ok(ok(object))
}

pub async fn delete_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerManage>,
    Path((_, server_id, resource, id)): Path<(String, String, String, String)>,
    axum::Json(request): axum::Json<ConfirmRequest>,
) -> Result<impl IntoResponse, AppError> {
    require_confirmation(&request.confirmation, &id)?;
    let args = delete_argv(&resource, &id).ok_or(AppError::ResourceNotFound)?;
    let runtime = runtime_for(&state, &org.organization_id, &server_id).await?;
    runtime.podman(&args).await?;
    audit(&state, &org, "podman.resource_deleted", &resource, &id);
    Ok(ok(json!({ "deleted": id })))
}

pub async fn prune_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerManage>,
    Path((_, server_id, resource)): Path<ServerSubPath>,
    axum::Json(request): axum::Json<ConfirmRequest>,
) -> Result<impl IntoResponse, AppError> {
    require_confirmation(&request.confirmation, "prune")?;
    let args = prune_argv(&resource).ok_or(AppError::ResourceNotFound)?;
    let runtime = runtime_for(&state, &org.organization_id, &server_id).await?;
    let output = runtime.podman_with_timeout(&args, PRUNE_TIMEOUT).await?;
    audit(
        &state,
        &org,
        "podman.resources_pruned",
        &resource,
        &server_id,
    );
    Ok(ok(json!({ "output": output.stdout })))
}

// ---------------------------------------------------------------------------
// Container / pod lifecycle
// ---------------------------------------------------------------------------

const CONTAINER_ACTIONS: [&str; 6] = ["start", "stop", "restart", "kill", "pause", "unpause"];
const POD_ACTIONS: [&str; 4] = ["start", "stop", "restart", "kill"];

pub async fn container_action_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerManage>,
    Path((_, server_id, id, action)): Path<(String, String, String, String)>,
    axum::Json(request): axum::Json<ConfirmRequest>,
) -> Result<impl IntoResponse, AppError> {
    if !CONTAINER_ACTIONS.contains(&action.as_str()) {
        return Err(AppError::ResourceNotFound);
    }
    require_confirmation(&request.confirmation, &id)?;
    let runtime = runtime_for(&state, &org.organization_id, &server_id).await?;
    runtime.podman(&[a(&action), a(&id)]).await?;
    audit(
        &state,
        &org,
        &format!("podman.container_{action}"),
        "container",
        &id,
    );
    Ok(ok(json!({ "action": action, "id": id })))
}

pub async fn pod_action_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerManage>,
    Path((_, server_id, id, action)): Path<(String, String, String, String)>,
    axum::Json(request): axum::Json<ConfirmRequest>,
) -> Result<impl IntoResponse, AppError> {
    if !POD_ACTIONS.contains(&action.as_str()) {
        return Err(AppError::ResourceNotFound);
    }
    require_confirmation(&request.confirmation, &id)?;
    let runtime = runtime_for(&state, &org.organization_id, &server_id).await?;
    runtime.podman(&[a("pod"), a(&action), a(&id)]).await?;
    audit(&state, &org, &format!("podman.pod_{action}"), "pod", &id);
    Ok(ok(json!({ "action": action, "id": id })))
}

// ---------------------------------------------------------------------------
// Container logs / top / metrics
// ---------------------------------------------------------------------------

pub async fn container_logs_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerRead>,
    Path((_, server_id, id)): Path<ServerSubPath>,
) -> Result<impl IntoResponse, AppError> {
    let runtime = runtime_for(&state, &org.organization_id, &server_id).await?;
    let output = runtime
        .podman(&[a("logs"), a("--tail"), a(LOG_TAIL), a(&id)])
        .await?;
    let text = output
        .stdout
        .lines()
        .chain(output.stderr.lines())
        .filter(|line| !is_transport_noise(line))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(ok(json!({ "logs": text })))
}

/// SSH-client chatter that leaks onto a remote command's stderr — control-socket
/// multiplexing warnings and host-key notices. It is never container output, so
/// it is dropped from the log/event stream.
fn is_transport_noise(line: &str) -> bool {
    let line = line.trim();
    line.starts_with("mux_client_request_session")
        || line.starts_with("ControlSocket")
        || line.contains("disabling multiplexing")
        || line.starts_with("Warning: Permanently added")
        || line.starts_with("kex_exchange_identification")
        || line.starts_with("Shared connection to ")
        || line.starts_with("Connection to ") && line.contains("closed")
}

/// `podman top` prints a plain table, not JSON. Returned verbatim for the UI to
/// render in a monospace block.
pub async fn container_top_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerRead>,
    Path((_, server_id, id)): Path<ServerSubPath>,
) -> Result<impl IntoResponse, AppError> {
    let runtime = runtime_for(&state, &org.organization_id, &server_id).await?;
    let output = runtime.podman(&[a("top"), a(&id)]).await?;
    Ok(ok(json!({ "processes": output.stdout })))
}

fn forward_lines<R>(reader: R, sender: tokio::sync::mpsc::Sender<String>)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        use tokio::io::AsyncBufReadExt;
        let mut lines = tokio::io::BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if sender.send(line).await.is_err() {
                break;
            }
        }
    });
}

/// Shared SSE body for the follow-mode commands (`logs -f`, `events`).
fn stream_command(
    mut child: crate::servers::ssh::SshStream,
    event_name: &'static str,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let (sender, mut receiver) = tokio::sync::mpsc::channel::<String>(256);
    if let Some(stdout) = child.take_stdout() {
        forward_lines(stdout, sender.clone());
    }
    if let Some(stderr) = child.take_stderr() {
        forward_lines(stderr, sender.clone());
    }
    drop(sender);
    let stream = async_stream::stream! {
        // The stream owns the child: a client disconnect drops the response,
        // which kills the process and ends the remote `podman` command.
        let _child = child;
        while let Some(line) = receiver.recv().await {
            if is_transport_noise(&line) {
                continue;
            }
            yield Ok(Event::default().event(event_name).data(line));
        }
    };
    Sse::new(stream).keep_alive(KeepAlive::default())
}

pub async fn container_logs_stream_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerRead>,
    Path((_, server_id, id)): Path<ServerSubPath>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let runtime = runtime_for(&state, &org.organization_id, &server_id).await?;
    let child = runtime
        .podman_stream(&[a("logs"), a("--follow"), a("--tail"), a(LOG_TAIL), a(&id)])
        .await?;
    Ok(stream_command(child, "log"))
}

pub async fn metrics_stream_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerRead>,
    Path((_, server_id)): Path<ServerPath>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let organization_id = org.organization_id;
    let stream = async_stream::stream! {
        let mut interval = tokio::time::interval(METRICS_STREAM_INTERVAL);
        loop {
            interval.tick().await;
            let snapshot = match runtime_for(&state, &organization_id, &server_id).await {
                Ok(runtime) => {
                    crate::podman::podman_metrics::collect_runtime_container_metrics(&runtime, &[]).await
                }
                Err(error) => Err(error),
            };
            match snapshot.and_then(|snapshot| {
                serde_json::to_string(&snapshot)
                    .map_err(|error| AppError::InternalError(error.to_string()))
            }) {
                Ok(payload) => yield Ok(Event::default().event("metrics").data(payload)),
                Err(error) => yield Ok(Event::default()
                    .event("metrics-error")
                    .data(json!({ "message": error.to_string() }).to_string())),
            }
        }
    };
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

// ---------------------------------------------------------------------------
// Images
// ---------------------------------------------------------------------------

pub async fn image_pull_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerManage>,
    Path((_, server_id)): Path<ServerPath>,
    axum::Json(request): axum::Json<PullImageRequest>,
) -> Result<impl IntoResponse, AppError> {
    require_confirmation(&request.confirmation, &request.reference)?;
    let runtime = runtime_for(&state, &org.organization_id, &server_id).await?;
    // Stream to completion so a slow pull does not hit the plain command budget.
    let mut child = runtime
        .podman_stream(&[a("pull"), a(&request.reference)])
        .await?;
    let (sender, mut receiver) = tokio::sync::mpsc::channel::<String>(64);
    if let Some(stdout) = child.take_stdout() {
        forward_lines(stdout, sender.clone());
    }
    if let Some(stderr) = child.take_stderr() {
        forward_lines(stderr, sender.clone());
    }
    drop(sender);
    let mut tail = String::new();
    while let Some(line) = receiver.recv().await {
        tail.push_str(&line);
        tail.push('\n');
    }
    let status = child.wait().await;
    if !status.map(|status| status.success()).unwrap_or(false) {
        return Err(AppError::ValidationError(format!(
            "Image pull failed: {}",
            tail.lines().last().unwrap_or("unknown error")
        )));
    }
    audit(
        &state,
        &org,
        "podman.image_pulled",
        "image",
        &request.reference,
    );
    Ok(ok(json!({ "pulled": request.reference })))
}

// ---------------------------------------------------------------------------
// Volumes / networks create + membership
// ---------------------------------------------------------------------------

pub async fn volume_create_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerManage>,
    Path((_, server_id)): Path<ServerPath>,
    axum::Json(request): axum::Json<CreateVolumeRequest>,
) -> Result<impl IntoResponse, AppError> {
    valid_name(&request.name)?;
    let mut args = vec![a("volume"), a("create")];
    if let Some(driver) = request.driver.as_deref().filter(|d| !d.is_empty()) {
        args.push(a("--driver"));
        args.push(a(driver));
    }
    for label in &request.labels {
        if !label.is_empty() {
            args.push(a("--label"));
            args.push(a(label));
        }
    }
    args.push(a(&request.name));
    let runtime = runtime_for(&state, &org.organization_id, &server_id).await?;
    runtime.podman(&args).await?;
    audit(
        &state,
        &org,
        "podman.volume_created",
        "volume",
        &request.name,
    );
    Ok(ok(json!({ "created": request.name })))
}

pub async fn network_create_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerManage>,
    Path((_, server_id)): Path<ServerPath>,
    axum::Json(request): axum::Json<CreateNetworkRequest>,
) -> Result<impl IntoResponse, AppError> {
    valid_name(&request.name)?;
    let mut args = vec![a("network"), a("create")];
    if let Some(driver) = request.driver.as_deref().filter(|d| !d.is_empty()) {
        args.push(a("--driver"));
        args.push(a(driver));
    }
    if let Some(subnet) = request.subnet.as_deref().filter(|s| !s.is_empty()) {
        args.push(a("--subnet"));
        args.push(a(subnet));
    }
    if let Some(gateway) = request.gateway.as_deref().filter(|g| !g.is_empty()) {
        args.push(a("--gateway"));
        args.push(a(gateway));
    }
    if request.internal {
        args.push(a("--internal"));
    }
    args.push(a(&request.name));
    let runtime = runtime_for(&state, &org.organization_id, &server_id).await?;
    runtime.podman(&args).await?;
    audit(
        &state,
        &org,
        "podman.network_created",
        "network",
        &request.name,
    );
    Ok(ok(json!({ "created": request.name })))
}

pub async fn network_member_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerManage>,
    Path((_, server_id, id, action)): Path<(String, String, String, String)>,
    axum::Json(request): axum::Json<NetworkMemberRequest>,
) -> Result<impl IntoResponse, AppError> {
    if !matches!(action.as_str(), "connect" | "disconnect") {
        return Err(AppError::ResourceNotFound);
    }
    let runtime = runtime_for(&state, &org.organization_id, &server_id).await?;
    runtime
        .podman(&[a("network"), a(&action), a(&id), a(&request.container)])
        .await?;
    audit(
        &state,
        &org,
        &format!("podman.network_{action}"),
        "network",
        &id,
    );
    Ok(ok(
        json!({ "network": id, "container": request.container, "action": action }),
    ))
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

pub async fn events_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerRead>,
    Path((_, server_id)): Path<ServerPath>,
    Query(query): Query<EventsQuery>,
) -> Result<impl IntoResponse, AppError> {
    let runtime = runtime_for(&state, &org.organization_id, &server_id).await?;
    let mut args = vec![a("events"), a("--format"), a("json"), a("--stream=false")];
    let since = query.since.unwrap_or_else(|| "1h".into());
    args.push(a("--since"));
    args.push(a(&since));
    let output = runtime.podman(&args).await?;
    // `podman events` prints one JSON object per line.
    let events: Vec<Value> = output
        .stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    Ok(ok(json!(events)))
}

pub async fn events_stream_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerRead>,
    Path((_, server_id)): Path<ServerPath>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let runtime = runtime_for(&state, &org.organization_id, &server_id).await?;
    let child = runtime
        .podman_stream(&[a("events"), a("--format"), a("json")])
        .await?;
    Ok(stream_command(child, "event"))
}

// ---------------------------------------------------------------------------
// Volume file workspace — local socket or remote host over SSH
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct WorkspacePathQuery {
    pub path: String,
}

#[derive(Deserialize, Default)]
pub struct WorkspaceListQuery {
    pub path: Option<String>,
}

type VolumeWorkspacePath = (String, String, String);

async fn volume_root(
    state: &SharedState,
    organization_id: &str,
    server_id: &str,
    volume: String,
) -> Result<WorkspaceRoot, AppError> {
    let server = repository::get_for_server_route(state, server_id, organization_id)
        .await?
        .ok_or(AppError::ResourceNotFound)?;
    // Fail fast if the server is unusable, matching the rest of the console.
    ServerRuntime::for_server(state, &server.id).await?;
    Ok(if server.kind == "local" {
        WorkspaceRoot::NamedVolume(volume)
    } else {
        WorkspaceRoot::RemoteServerVolume {
            server_id: server.id,
            volume,
        }
    })
}

pub async fn volume_workspace_list_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerRead>,
    Path((_, server_id, volume)): Path<VolumeWorkspacePath>,
    Query(query): Query<WorkspaceListQuery>,
) -> Result<impl IntoResponse, AppError> {
    let root = volume_root(&state, &org.organization_id, &server_id, volume).await?;
    let depth = if query.path.is_some() { 1 } else { 2 };
    Ok(ok(volume_workspace::list_entries(
        &state,
        root,
        query.path.as_deref(),
        depth,
    )
    .await?))
}

pub async fn volume_workspace_file_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerRead>,
    Path((_, server_id, volume)): Path<VolumeWorkspacePath>,
    Query(query): Query<WorkspacePathQuery>,
) -> Result<impl IntoResponse, AppError> {
    let root = volume_root(&state, &org.organization_id, &server_id, volume).await?;
    Ok(ok(
        volume_workspace::read_file(&state, root, &query.path).await?
    ))
}

pub async fn volume_workspace_save_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerManage>,
    Path((_, server_id, volume)): Path<VolumeWorkspacePath>,
    Query(query): Query<WorkspacePathQuery>,
    axum::Json(request): axum::Json<UpdateWorkspaceTextRequest>,
) -> Result<impl IntoResponse, AppError> {
    let root = volume_root(&state, &org.organization_id, &server_id, volume).await?;
    Ok(ok(volume_workspace::write_text_file(
        &state,
        root,
        &query.path,
        request.content,
    )
    .await?))
}

pub async fn volume_workspace_create_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerManage>,
    Path((_, server_id, volume)): Path<VolumeWorkspacePath>,
    axum::Json(request): axum::Json<CreateWorkspaceEntryRequest>,
) -> Result<impl IntoResponse, AppError> {
    let root = volume_root(&state, &org.organization_id, &server_id, volume).await?;
    Ok(ok(volume_workspace::create_entry(
        &state,
        root,
        &request.path,
        &request.kind,
        request.content,
    )
    .await?))
}

pub async fn volume_workspace_upload_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerManage>,
    Path((_, server_id, volume)): Path<VolumeWorkspacePath>,
    Query(query): Query<WorkspacePathQuery>,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, AppError> {
    let root = volume_root(&state, &org.organization_id, &server_id, volume).await?;
    let field = multipart
        .next_field()
        .await
        .map_err(|error| AppError::ValidationError(error.to_string()))?
        .ok_or_else(|| AppError::ValidationError("Choose one file to upload".into()))?;
    let bytes = field
        .bytes()
        .await
        .map_err(|error| AppError::InternalError(error.to_string()))?;
    Ok(ok(volume_workspace::upload_new_file(
        &state,
        root,
        &query.path,
        bytes.to_vec(),
    )
    .await?))
}

pub async fn volume_workspace_download_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerRead>,
    Path((_, server_id, volume)): Path<VolumeWorkspacePath>,
    Query(query): Query<WorkspacePathQuery>,
) -> Result<Response, AppError> {
    let root = volume_root(&state, &org.organization_id, &server_id, volume).await?;
    let (bytes, mime) = volume_workspace::read_file_bytes(&state, root, &query.path).await?;
    let filename = std::path::Path::new(&query.path)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let mut response = Response::new(Body::from(bytes));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        axum::http::HeaderValue::from_str(
            &mime.unwrap_or_else(|| "application/octet-stream".into()),
        )
        .map_err(|error| AppError::InternalError(error.to_string()))?,
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        axum::http::HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
            .map_err(|error| AppError::InternalError(error.to_string()))?,
    );
    Ok(response)
}
