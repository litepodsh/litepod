use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket};
use axum::{
    Json, Router,
    extract::{Path, State, WebSocketUpgrade},
    routing::{get, post},
};
use futures_util::io::AsyncWriteExt;
use futures_util::{SinkExt, StreamExt};
use podman_api::{
    Podman,
    api::Exec,
    opts::{ExecCreateOpts, ExecStartOpts},
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::mpsc;

use crate::dtos::response::AppError;
use crate::security::authz::extract::{RequireApp, RequireOrg, perm};

use crate::state::state::SharedState;

pub fn routes() -> Router<SharedState> {
    Router::new()
		.merge(super::host::routes())
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/containers/{container_id}/exec",
            post(create_exec_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/containers/{container_id}/exec/{exec_id}/ws",
            get(exec_ws_handler),
        )
        // Per-server console exec: same machinery, resolved from a server id
        // instead of an application id.
        .route(
            "/api/v1/organizations/{organization_id}/servers/{server_id}/podman/container/{container_id}/exec",
            post(server_create_exec_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/servers/{server_id}/podman/container/{container_id}/exec/{exec_id}/ws",
            get(server_exec_ws_handler),
        )
}

/// Builds a `Podman` client (and optional SSH relay) for a server the caller's
/// organization owns.
async fn server_podman(
    state: &SharedState,
    organization_id: &str,
    server_id: &str,
) -> Result<(Podman, Option<crate::servers::ssh::SshUnixRelay>), AppError> {
    let server =
        crate::servers::repository::get_for_server_route(state, server_id, organization_id)
            .await?
            .ok_or(AppError::ResourceNotFound)?;
    let runtime = crate::servers::runtime::ServerRuntime::for_server(state, &server.id).await?;
    let relay = if runtime.is_remote() {
        Some(runtime.podman_unix_relay().await?)
    } else {
        None
    };
    let socket = relay
        .as_ref()
        .map(|relay| relay.socket_url())
        .unwrap_or_else(|| state.podman_socket.clone());
    let podman =
        Podman::new(&socket).map_err(|error| AppError::InternalError(error.to_string()))?;
    Ok((podman, relay))
}

pub async fn server_create_exec_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerTerminal>,
    Path((_organization_id, server_id, container_id)): Path<(String, String, String)>,
    Json(req): Json<CreateExecRequest>,
) -> Result<Json<CreateExecResponse>, AppError> {
    let (podman, relay) = server_podman(&state, &org.organization_id, &server_id).await?;
    let container = podman.containers().get(&container_id);
    let opts = ExecCreateOpts::builder()
        .command(&req.command)
        .attach_stdin(true)
        .attach_stdout(true)
        .attach_stderr(true)
        .tty(req.tty)
        .env([
            ("TERM", "xterm-256color"),
            ("COLORTERM", "truecolor"),
            ("TERM_PROGRAM", "litepod"),
        ])
        .build();
    let exec = container
        .create_exec(&opts)
        .await
        .map_err(|e| AppError::InternalError(format!("create exec: {e}")))?;
    let exec_id = exec.id().to_string();
    state.pending_execs.lock().unwrap().insert(
        exec_id.clone(),
        crate::state::state::PendingExec {
            exec,
            _remote_relay: relay,
        },
    );
    Ok(Json(CreateExecResponse { exec_id }))
}

pub async fn server_exec_ws_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerTerminal>,
    Path((_organization_id, server_id, container_id, exec_id)): Path<(
        String,
        String,
        String,
        String,
    )>,
    ws: WebSocketUpgrade,
) -> Result<axum::response::Response, AppError> {
    // Session row opens at upgrade (when the shell is about to start), not at
    // exec creation — same lifecycle as the remote-host console in `host.rs`.
    let session_id = crate::podman::terminal::session::start_exec_session(
        &state,
        &org.organization_id,
        None,
        Some(&server_id),
        &container_id,
        &org.user,
    )
    .await?;
    Ok(ws.on_upgrade(move |ws| handle_ws(ws, state, exec_id, session_id)))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct CreateExecRequest {
    pub command: Vec<String>,
    pub tty: bool,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct CreateExecResponse {
    pub exec_id: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum TerminalControl {
    Resize { cols: u16, rows: u16 },
    Error { message: String },
}

#[derive(Debug, Error)]
pub enum DispatchError {
    #[error("podman error: {0}")]
    Podman(#[from] podman_api::Error),
}

/// Send a JSON error frame to the WebSocket before the upgrade completes
/// or after the exec multiplexer fails to start. The frontend uses this to
/// surface a connect-time failure instead of silently dropping the WS.
async fn send_error_frame(ws: &mut WebSocket, message: &str) -> Result<(), axum::Error> {
    let payload = serde_json::to_string(&TerminalControl::Error {
        message: message.to_string(),
    })
    .map_err(axum::Error::new)?;
    ws.send(Message::Text(payload.into())).await
}

/// Inspect one inbound WS message and apply any control action.
///
/// - `Message::Text` containing a parseable `TerminalControl::Resize` triggers
///   `exec.resize(cols, rows)`.
/// - All other `Message::Text` values are dropped (with a `tracing::warn`).
/// - `Message::Binary`, `Message::Ping`, `Message::Pong`, `Message::Close`
///   return `Ok(())` so the caller keeps streaming.
pub async fn dispatch_inbound_message(msg: &Message, exec: &Exec) -> Result<(), DispatchError> {
    let text = match msg {
        Message::Text(text) => text.as_str(),
        _ => return Ok(()),
    };
    let control: TerminalControl = match serde_json::from_str(text) {
        Ok(c) => c,
        Err(_) => {
            tracing::warn!(payload = %text, "dropping malformed terminal control frame");
            return Ok(());
        }
    };
    match control {
        TerminalControl::Resize { cols, rows } => {
            exec.resize(cols as usize, rows as usize).await?;
        }
        TerminalControl::Error { .. } => {
            // Errors are server-to-client only; ignore inbound.
        }
    }
    Ok(())
}

#[utoipa::path(
    post,
    path = "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/containers/{container_id}/exec",
    tag = "terminal",
    params(
        ("organization_id" = String, Path, description = "Organization id"),
        ("project_id" = String, Path, description = "Project id"),
        ("environment_id" = String, Path, description = "Environment id"),
        ("application_id" = String, Path, description = "Application id"),
        ("container_id" = String, Path, description = "Container id or name"),
    ),
    request_body = CreateExecRequest,
    responses(
        (status = 200, description = "Exec instance created", body = CreateExecResponse),
        (status = 403, description = "Not an organization member"),
    ),
)]
pub async fn create_exec_handler(
    State(state): State<SharedState>,
    // An interactive shell is effectively total access to whatever the container can reach —
    // env secrets, mounted volumes, the ability to pivot — so this needs the same floor as any
    // other full-control action, not the read floor an org check would default to.
    RequireApp(app, _): RequireApp<perm::AppExec>,
    // The extractor covers the first four segments; only `container_id` is left.
    // Named rather than `_` because `#[utoipa::path]` needs idents here.
    Path((_organization_id, _project_id, _environment_id, _application_id, container_id)): Path<(
        String,
        String,
        String,
        String,
        String,
    )>,
    Json(req): Json<CreateExecRequest>,
) -> Result<Json<CreateExecResponse>, AppError> {
    let application_id = app.application_id;

    // `RequireApp<perm::AppExec>` proves the application belongs to the caller's organization; it does not prove
    // `container_id` is actually one of that application's containers. Without this check any
    // organization admin could exec into any container on the host by guessing/enumerating ids.
    let valid_containers =
        crate::handler::deployments::container_names(&state, &application_id).await?;
    if !valid_containers.contains(&container_id) {
        return Err(AppError::ResourceNotFound);
    }

    let runtime =
        crate::servers::runtime::ServerRuntime::for_application(&state, &application_id).await?;
    let relay = if runtime.is_remote() {
        Some(runtime.podman_unix_relay().await?)
    } else {
        None
    };
    let podman_socket = relay
        .as_ref()
        .map(|relay| relay.socket_url())
        .unwrap_or_else(|| state.podman_socket.clone());
    let podman = Podman::new(&podman_socket).map_err(|e| AppError::InternalError(e.to_string()))?;
    let container = podman.containers().get(&container_id);

    let opts = ExecCreateOpts::builder()
        .command(&req.command)
        .attach_stdin(true)
        .attach_stdout(true)
        .attach_stderr(true)
        .tty(req.tty)
        .env([
            ("TERM", "xterm-256color"),
            ("COLORTERM", "truecolor"),
            ("TERM_PROGRAM", "litepod"),
        ])
        .build();

    let exec = container
        .create_exec(&opts)
        .await
        .map_err(|e| AppError::InternalError(format!("create exec: {e}")))?;
    let exec_id = exec.id().to_string();
    state.pending_execs.lock().unwrap().insert(
        exec_id.clone(),
        crate::state::state::PendingExec {
            exec,
            _remote_relay: relay,
        },
    );

    Ok(Json(CreateExecResponse { exec_id }))
}

#[utoipa::path(
    get,
    path = "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/containers/{container_id}/exec/{exec_id}/ws",
    tag = "terminal",
    params(
        ("organization_id" = String, Path, description = "Organization id"),
        ("project_id" = String, Path, description = "Project id"),
        ("environment_id" = String, Path, description = "Environment id"),
        ("application_id" = String, Path, description = "Application id"),
        ("container_id" = String, Path, description = "Container id or name"),
        ("exec_id" = String, Path, description = "Exec instance identifier"),
    ),
    responses((status = 101, description = "WebSocket connection upgraded"))
)]
pub async fn exec_ws_handler(
    State(state): State<SharedState>,
    // Same reasoning as `create_exec_handler`: a shell is full control.
    RequireApp(app, _): RequireApp<perm::AppExec>,
    // The extractor covers the first four segments.
    // Named rather than `_` because `#[utoipa::path]` needs idents here.
    Path((
        _organization_id,
        _project_id,
        _environment_id,
        _application_id,
        container_id,
        exec_id,
    )): Path<(String, String, String, String, String, String)>,
    ws: WebSocketUpgrade,
) -> Result<axum::response::Response, AppError> {
    // Same ownership check as exec creation: the exec_id alone doesn't prove this
    // caller's organization owns the container it was created against.
    let valid_containers =
        crate::handler::deployments::container_names(&state, &app.application_id).await?;
    if !valid_containers.contains(&container_id) {
        return Err(AppError::ResourceNotFound);
    }

    // Session row opens at upgrade (when the shell is about to start), not at
    // exec creation — same lifecycle as the remote-host console in `host.rs`.
    let session_id = crate::podman::terminal::session::start_exec_session(
        &state,
        &app.organization_id,
        Some(&app.application_id),
        None,
        &container_id,
        &app.user,
    )
    .await?;
    Ok(ws.on_upgrade(move |ws| handle_ws(ws, state, exec_id, session_id)))
}

async fn handle_ws(mut ws: WebSocket, state: SharedState, exec_id: String, session_id: String) {
    // Remove the pending exec before any await so the temporary `MutexGuard`
    // is dropped (it is not `Send`, and `finish_exec_session` awaits below).
    let pending = state.pending_execs.lock().unwrap().remove(&exec_id);
    let Some(pending) = pending else {
        tracing::error!(exec_id = %exec_id, "no pending exec found for websocket");
        crate::podman::terminal::session::finish_exec_session(
            &state,
            &session_id,
            "failed",
            Some("no pending exec found for websocket"),
        )
        .await;
        return;
    };

    let exec = pending.exec;
    // `pending` remains in this task until the WebSocket closes, retaining the
    // remote Unix tunnel for SSH-backed exec sessions.
    let _relay = pending._remote_relay;
    let opts = ExecStartOpts::builder().detach(false).build();

    match exec.start(&opts).await {
        Ok(Some(multiplexer)) => {
            // Pre-size the PTY immediately so TUIs that query terminal size at
            // process start (btop, htop, vim, nano, less) get a sensible value.
            // The frontend sends a more accurate resize once the WS opens.
            if let Err(e) = exec.resize(120, 40).await {
                tracing::warn!(error = %e, "initial exec resize failed");
            }
            let (mut ws_tx, mut ws_rx) = ws.split();
            let (tx, mut rx) = mpsc::channel::<Bytes>(100);
            let (reader, writer) = multiplexer.split();

            // Bridge stdout/stderr from exec to WebSocket
            let ws_send = tokio::spawn(async move {
                let mut stream = reader.map(|chunk| match chunk {
                    Ok(podman_api::conn::TtyChunk::StdOut(data))
                    | Ok(podman_api::conn::TtyChunk::StdErr(data)) => Some(data),
                    Ok(podman_api::conn::TtyChunk::StdIn(_)) => None,
                    Err(_) => None,
                });

                while let Some(Some(data)) = stream.next().await {
                    if ws_tx
                        .send(Message::Binary(Bytes::from(data)))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            });

            // Bridge WebSocket input to exec stdin
            let stdin_write = tokio::spawn(async move {
                let mut writer = writer;
                while let Some(data) = rx.recv().await {
                    if writer.write_all(&data).await.is_err() {
                        break;
                    }
                }
                let _ = writer.close().await;
            });

            // Read from WebSocket and forward to stdin / dispatch control
            let mut closed_by_client = false;
            while let Some(Ok(msg)) = ws_rx.next().await {
                match &msg {
                    Message::Text(_) | Message::Binary(_) => {
                        if let Err(e) = dispatch_inbound_message(&msg, &exec).await {
                            tracing::error!(error = %e, "dispatch failed");
                            break;
                        }
                        if let Message::Binary(data) = msg {
                            if tx.send(data).await.is_err() {
                                break;
                            }
                        }
                    }
                    Message::Close(_) => {
                        closed_by_client = true;
                        break;
                    }
                    _ => continue,
                }
            }

            let _ = ws_send.await;
            let _ = stdin_write.await;
            crate::podman::terminal::session::finish_exec_session(
                &state,
                &session_id,
                if closed_by_client {
                    "closed"
                } else {
                    "disconnected"
                },
                None,
            )
            .await;
        }
        Ok(None) => {
            tracing::error!("exec start returned None");
            let _ = send_error_frame(&mut ws, "exec start returned no multiplexer").await;
            crate::podman::terminal::session::finish_exec_session(
                &state,
                &session_id,
                "failed",
                Some("exec start returned no multiplexer"),
            )
            .await;
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to start exec");
            let _ = send_error_frame(&mut ws, &format!("exec start failed: {e}")).await;
            crate::podman::terminal::session::finish_exec_session(
                &state,
                &session_id,
                "failed",
                Some(&format!("exec start failed: {e}")),
            )
            .await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::ws::Message;

    /// Returns a `Message` value of the requested kind for unit tests.
    fn msg(kind: &str, payload: &[u8]) -> Message {
        match kind {
            "text" => Message::Text(String::from_utf8(payload.to_vec()).unwrap().into()),
            "binary" => Message::Binary(payload.to_vec().into()),
            _ => panic!("unknown message kind"),
        }
    }

    /// Returns a `podman_api::Exec` handle against a non-existent socket.
    /// The dispatch tests only rely on the public `resize` method existing
    /// (compile-time check) and on the dispatch branch returning `Ok(())`
    /// or `Err(DispatchError)` as designed; they do not require a live podman.
    fn fake_exec() -> podman_api::api::Exec {
        let podman = podman_api::Podman::new("unix:///tmp/litepod-test-nonexistent.sock")
            .expect("podman handle builds even with a missing socket");
        podman
            .execs()
            .get("0000000000000000000000000000000000000000000000000000000000000000")
    }

    #[tokio::test]
    async fn resize_message_parses_and_invokes_podman_resize() {
        // fake_exec() points at a socket that doesn't exist, so a resize that actually
        // reaches podman_api's resize() call fails at the network layer. That failure
        // (rather than the silent Ok(()) a dropped/malformed frame would give) is the
        // proof the "resize" tag parsed and dispatch tried to invoke resize.
        let exec = fake_exec();
        let msg = msg("text", b"{\"type\":\"resize\",\"cols\":120,\"rows\":40}\n");
        let result = dispatch_inbound_message(&msg, &exec).await;
        assert!(
            matches!(result, Err(DispatchError::Podman(_))),
            "expected the resize call to reach (and fail against) the fake socket: {result:?}"
        );
    }

    #[tokio::test]
    async fn malformed_text_message_is_dropped() {
        let exec = fake_exec();
        let msg = msg("text", b"not json");
        let result = dispatch_inbound_message(&msg, &exec).await;
        assert!(matches!(result, Ok(())));
    }

    #[tokio::test]
    async fn binary_message_is_a_noop() {
        let exec = fake_exec();
        let msg = msg("binary", b"ls\n");
        let result = dispatch_inbound_message(&msg, &exec).await;
        assert!(matches!(result, Ok(())));
    }

    #[tokio::test]
    async fn unknown_control_type_is_dropped() {
        let exec = fake_exec();
        let msg = msg("text", b"{\"type\":\"reboot\",\"target\":\"db\"}\n");
        let result = dispatch_inbound_message(&msg, &exec).await;
        assert!(matches!(result, Ok(())));
    }

    #[test]
    fn terminal_control_resize_round_trips() {
        let original = TerminalControl::Resize { cols: 80, rows: 24 };
        let json = serde_json::to_string(&original).unwrap();
        let parsed: TerminalControl = serde_json::from_str(&json).unwrap();
        match parsed {
            TerminalControl::Resize { cols, rows } => {
                assert_eq!(cols, 80);
                assert_eq!(rows, 24);
            }
            TerminalControl::Error { .. } => panic!("round-trip produced Error variant"),
        }
    }

    #[test]
    fn terminal_control_error_round_trips() {
        let original = TerminalControl::Error {
            message: "bash not found".to_string(),
        };
        let json = serde_json::to_string(&original).unwrap();
        let parsed: TerminalControl = serde_json::from_str(&json).unwrap();
        match parsed {
            TerminalControl::Error { message } => {
                assert_eq!(message, "bash not found");
            }
            TerminalControl::Resize { .. } => panic!("round-trip produced Resize variant"),
        }
    }

    /// Integration test stub. Runs only when `LITEPOD_TERMINAL_INTEGRATION=1`
    /// is set; ignored by default so CI is not blocked on a live podman socket.
    #[tokio::test]
    #[ignore]
    async fn integration_resize_frame_round_trip() {
        if std::env::var("LITEPOD_TERMINAL_INTEGRATION").is_err() {
            eprintln!("set LITEPOD_TERMINAL_INTEGRATION=1 to run");
            return;
        }
        // Pseudo-code: this test is left as a manual smoke for the engineer
        // running `cargo test -- --ignored`. A full implementation would
        // spawn `podman run -d --rm alpine sleep 300`, exec into it, and
        // assert that the WS round-trips a resize frame without error.
        // We do not run this in CI because it requires a live podman socket.
    }
}
