use axum::extract::ws::{Message, WebSocket};
use axum::{
    Router,
    extract::{Path, Query, State, WebSocketUpgrade},
    routing::get,
};
use futures_util::{SinkExt, StreamExt};
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use serde::Deserialize;
use std::{
    io::{Read, Write},
    sync::{Arc, Mutex},
};
use tokio::sync::mpsc;

/// Login shell for the browser PTY, as a bare name (`bash`, `sh`, `fish`, `nu`, `zsh`, `pwsh`).
/// `SshExecutor::resolve_shell` maps it to the absolute path we actually run, so
/// the request never carries a path and there is nothing to inject through it.
#[derive(Debug, Deserialize)]
pub struct HostTerminalQuery {
    #[serde(default = "default_shell")]
    shell: String,
}

fn default_shell() -> String {
    "bash".into()
}

use crate::{
    dtos::response::AppError,
    security::authz::extract::{RequireOrg, perm},
    servers::{
        model::{ServerState, SshTarget},
        repository,
        ssh::SshExecutor,
        terminal,
    },
    state::state::SharedState,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminalKind {
    Local,
    Ssh,
}

struct TerminalLaunch {
    argv: Vec<std::ffi::OsString>,
    secret_files: Vec<crate::servers::ssh::SecretFile>,
    /// Extra env applied to the spawned process. Empty for the SSH paths (ssh
    /// carries the remote env); the direct dev PTY uses it to pin `TERM`.
    env: Vec<(String, String)>,
}

#[derive(Debug, PartialEq, Eq)]
struct LocalSshConfig {
    target: SshTarget,
    private_key: String,
    fingerprint: String,
    known_hosts: String,
}

impl LocalSshConfig {
    fn new(
        host: String,
        port: u16,
        user: String,
        private_key: String,
        fingerprint: String,
        known_hosts: String,
    ) -> Result<Self, AppError> {
        if user.trim().is_empty()
            || user == "root"
            || private_key.trim().is_empty()
            || fingerprint.trim().is_empty()
            || known_hosts.trim().is_empty()
        {
            return Err(Self::not_configured());
        }
        Ok(Self {
            target: SshTarget::parse(&host, port, &user)?,
            private_key,
            fingerprint,
            known_hosts,
        })
    }

    fn not_configured() -> AppError {
        AppError::ValidationError("Host console SSH is not configured for this installation; rerun or repair the litepod installer".into())
    }

    fn load() -> Result<Self, AppError> {
        let missing = Self::not_configured;
        let host = std::env::var("LITEPOD_LOCAL_SSH_HOST").map_err(|_| missing())?;
        let port = std::env::var("LITEPOD_LOCAL_SSH_PORT")
            .map_err(|_| missing())?
            .parse::<u16>()
            .map_err(|_| missing())?;
        let user = std::env::var("LITEPOD_LOCAL_SSH_USER").map_err(|_| missing())?;
        let private_key_path =
            std::env::var("LITEPOD_LOCAL_SSH_KEY_FILE").map_err(|_| missing())?;
        let known_hosts_path =
            std::env::var("LITEPOD_LOCAL_SSH_KNOWN_HOSTS_FILE").map_err(|_| missing())?;
        let fingerprint = std::env::var("LITEPOD_LOCAL_SSH_FINGERPRINT").map_err(|_| missing())?;
        let private_key = std::fs::read_to_string(private_key_path).map_err(|_| missing())?;
        let known_hosts = std::fs::read_to_string(known_hosts_path).map_err(|_| missing())?;
        Self::new(host, port, user, private_key, fingerprint, known_hosts)
    }
}

fn authorize_terminal(
    is_sysadmin: bool,
    server_kind: &str,
    server_state: ServerState,
) -> Result<TerminalKind, AppError> {
    if server_state != ServerState::Ready {
        return Err(AppError::ValidationError(
            "Console requires a ready server".into(),
        ));
    }
    match server_kind {
        "local" if is_sysadmin => Ok(TerminalKind::Local),
        "local" => Err(AppError::InsufficientPermissions),
        "ssh" => Ok(TerminalKind::Ssh),
        _ => Err(AppError::ValidationError(
            "Console is unavailable for this server".into(),
        )),
    }
}

pub fn routes() -> Router<SharedState> {
    Router::new().route(
        "/api/v1/organizations/{organization_id}/servers/{server_id}/terminal/ws",
        get(handler),
    )
}

pub async fn handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerTerminal>,
    Path((_organization_id, server_id)): Path<(String, String)>,
    Query(query): Query<HostTerminalQuery>,
    ws: WebSocketUpgrade,
) -> Result<axum::response::Response, AppError> {
    let server = repository::get_for_server_route(&state, &server_id, &org.organization_id)
        .await?
        .ok_or(AppError::ResourceNotFound)?;
    let terminal_kind = authorize_terminal(org.user.is_sysadmin, &server.kind, server.state)?;
    tracing::info!(
        server = %server_id,
        kind = ?terminal_kind,
        shell = %query.shell,
        user = %org.user.email,
        "terminal: opening server console"
    );
    let launch = match build_launch(terminal_kind, &server, &query.shell).await {
        Ok(launch) => launch,
        Err(error) => {
            tracing::error!(
                server = %server_id,
                kind = ?terminal_kind,
                shell = %query.shell,
                error = %error,
                "terminal: could not prepare launch (websocket will not upgrade)"
            );
            return Err(error);
        }
    };
    let session = match terminal::start_session(&state, &org.organization_id, &server.id, &org.user)
        .await
    {
        Ok(session) => session,
        Err(error) => {
            tracing::error!(server = %server_id, error = %error, "terminal: start_session failed");
            return Err(error);
        }
    };
    Ok(ws.on_upgrade(move |ws| bridge(ws, state, session, launch)))
}

/// Resolves the SSH argv for either the host console or a remote server. Kept
/// separate from `handler` so every failure path is logged with the step that
/// produced it instead of vanishing into a bare `?`.
async fn build_launch(
    kind: TerminalKind,
    server: &repository::ServerRecord,
    shell: &str,
) -> Result<TerminalLaunch, AppError> {
    match kind {
        TerminalKind::Local => {
            let config = match LocalSshConfig::load() {
                Ok(config) => config,
                Err(error) => {
                    // Dev fallback: with no installer-provisioned host-SSH identity
                    // (`LITEPOD_LOCAL_SSH_*`), a debug build opens a PTY directly
                    // as the API process user — no SSH hop, no isolation. Release
                    // builds hard-fail and tell the operator to run the installer.
                    if cfg!(debug_assertions) {
                        let shell_path = SshExecutor::resolve_local_shell(shell)?;
                        tracing::warn!(
                            shell = ?shell_path,
                            reason = %error,
                            "terminal: host console SSH not configured — DEBUG fallback to a \
                             direct local PTY as the API user (no SSH isolation; not for production)"
                        );
                        // Login + interactive so the dev gets their real shell:
                        // /etc/profile, ~/.bash_profile, ~/.bashrc (env vars,
                        // aliases, prompt, atuin, brew, …) all load, same as a
                        // normal terminal on this host.
                        let argv: Vec<std::ffi::OsString> =
                            vec![shell_path.into(), "-l".into(), "-i".into()];
                        return Ok(TerminalLaunch {
                            argv,
                            secret_files: vec![],
                            // xterm.js speaks xterm, not whatever TERM the API
                            // inherited from its own launching terminal (e.g. foot).
                            env: vec![("TERM".into(), "xterm-256color".into())],
                        });
                    }
                    tracing::error!(error = %error, "terminal: LocalSshConfig::load failed");
                    return Err(error);
                }
            };
            let encrypted_private_key = crate::utils::security::encrypt_data(
                &config.private_key,
                &crate::utils::security::get_encryption_key(),
            )
            .map_err(|_| {
                AppError::InternalError("Could not prepare host console SSH identity".into())
            })?;
            let (argv, key, hosts) = SshExecutor::interactive_shell_argv(
                &config.target,
                &encrypted_private_key,
                &config.fingerprint,
                Some(config.known_hosts.trim()),
                shell,
            )
            .await
            .inspect_err(|error| {
                tracing::error!(error = %error, "terminal: interactive_shell_argv (local) failed");
            })?;
            Ok(TerminalLaunch {
                argv,
                secret_files: vec![key, hosts],
                env: vec![],
            })
        }
        TerminalKind::Ssh => {
            let target = SshTarget::parse(
                server.ssh_host.as_deref().ok_or_else(|| {
                    AppError::InternalError("Remote server has no SSH host".into())
                })?,
                server.ssh_port.ok_or_else(|| {
                    AppError::InternalError("Remote server has no SSH port".into())
                })?,
                server.ssh_user.as_deref().ok_or_else(|| {
                    AppError::InternalError("Remote server has no SSH user".into())
                })?,
            )?;
            let (argv, key, hosts) = SshExecutor::interactive_shell_argv(
                &target,
                server.private_key_encrypted.as_deref().ok_or_else(|| {
                    AppError::InternalError("Remote server SSH credential is unavailable".into())
                })?,
                server.host_key_fingerprint.as_deref().ok_or_else(|| {
                    AppError::ValidationError("Remote server SSH host key is not confirmed".into())
                })?,
                server.host_key_known_hosts.as_deref(),
                shell,
            )
            .await
            .inspect_err(|error| {
                tracing::error!(error = %error, "terminal: interactive_shell_argv (ssh) failed");
            })?;
            Ok(TerminalLaunch {
                argv,
                secret_files: vec![key, hosts],
                env: vec![],
            })
        }
    }
}

async fn bridge(mut ws: WebSocket, state: SharedState, session: String, launch: TerminalLaunch) {
    let TerminalLaunch {
        argv,
        secret_files,
        env,
    } = launch;
    // SSH reads these paths while the terminal is alive. Local launches carry an empty guard.
    let _secret_files = secret_files;

    // The full argv is only the ssh binary, its flags, the pinned-key/known-hosts
    // temp paths, the target, and the login shell — no secrets — so logging it is
    // what makes "connection closed" debuggable.
    let command_line = argv
        .iter()
        .map(|part| part.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ");
    tracing::info!(session = %session, command = %command_line, "terminal: spawning PTY");

    let result = tokio::task::spawn_blocking(move || {
        let pty = native_pty_system();
        let pair = pty
            .openpty(PtySize {
                rows: 40,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("openpty failed: {e}"))?;
        let mut cmd = CommandBuilder::new(&argv[0]);
        for arg in &argv[1..] {
            cmd.arg(arg);
        }
        for (key, value) in &env {
            cmd.env(key, value);
        }
        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| format!("spawn {:?} failed: {e}", argv[0]))?;
        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("clone reader failed: {e}"))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| format!("take writer failed: {e}"))?;
        Ok::<_, String>((pair.master, child, reader, writer))
    })
    .await;
    let (master, mut child, mut reader, mut writer) = match result {
        Ok(Ok(parts)) => parts,
        Ok(Err(reason)) => {
            tracing::error!(session = %session, reason = %reason, "terminal: could not start PTY");
            let _ = ws
                .send(Message::Binary(
                    format!("\r\n[litepod] terminal failed to start: {reason}\r\n")
                        .into_bytes()
                        .into(),
                ))
                .await;
            let _ = ws.close().await;
            terminal::finish_session(&state, &session, "failed", Some(&reason)).await;
            return;
        }
        Err(join) => {
            let reason = format!("PTY setup task panicked: {join}");
            tracing::error!(session = %session, reason = %reason, "terminal: PTY task join error");
            terminal::finish_session(&state, &session, "failed", Some(&reason)).await;
            return;
        }
    };
    let master = Arc::new(Mutex::new(master));
    let (mut tx, mut rx) = ws.split();
    let (out_tx, mut out_rx) = mpsc::channel::<Vec<u8>>(64);
    let reader_session = session.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => {
                    tracing::debug!(session = %reader_session, "terminal: PTY reached EOF");
                    break;
                }
                Ok(n) => {
                    if out_tx.blocking_send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    tracing::warn!(session = %reader_session, error = %e, "terminal: PTY read error");
                    break;
                }
            }
        }
    });
    let send = tokio::spawn(async move {
        while let Some(bytes) = out_rx.recv().await {
            if tx.send(Message::Binary(bytes.into())).await.is_err() {
                break;
            }
        }
    });
    let mut closed_by_client = false;
    while let Some(Ok(message)) = rx.next().await {
        match message {
            Message::Binary(bytes) => {
                let _ = writer.write_all(&bytes);
                let _ = writer.flush();
            }
            Message::Text(text) => {
                if let Ok(crate::podman::terminal::exec::TerminalControl::Resize { cols, rows }) =
                    serde_json::from_str(&text)
                {
                    let _ = master.lock().unwrap().resize(PtySize {
                        rows,
                        cols,
                        pixel_width: 0,
                        pixel_height: 0,
                    });
                }
            }
            Message::Close(_) => {
                closed_by_client = true;
                break;
            }
            _ => {}
        }
    }
    send.abort();

    // Close the PTY master so ssh gets SIGHUP and winds down, then read its
    // status without blocking. If the child had *already* exited on its own,
    // that exit code is the real reason a session that "just closed" ended — a
    // non-zero code here is usually the remote login shell rejecting the
    // requested shell (e.g. `nu` not on the remote PATH).
    drop(master);
    let exit = tokio::task::spawn_blocking(move || {
        for _ in 0..20 {
            match child.try_wait() {
                Ok(Some(status)) => return Some(status),
                Ok(None) => std::thread::sleep(std::time::Duration::from_millis(50)),
                Err(_) => return None,
            }
        }
        let _ = child.kill();
        child.try_wait().ok().flatten()
    })
    .await
    .ok()
    .flatten();
    match &exit {
        Some(status) if status.success() => {
            tracing::info!(session = %session, "terminal: ssh child exited cleanly");
        }
        Some(status) => {
            tracing::warn!(
                session = %session,
                code = status.exit_code(),
                "terminal: ssh child exited non-zero"
            );
        }
        None => {
            tracing::debug!(session = %session, "terminal: ssh child status unavailable at teardown");
        }
    }

    let note = match &exit {
        Some(status) if !status.success() => {
            Some(format!("ssh exited with code {}", status.exit_code()))
        }
        _ => None,
    };
    terminal::finish_session(
        &state,
        &session,
        if closed_by_client {
            "closed"
        } else {
            "disconnected"
        },
        note.as_deref(),
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::{LocalSshConfig, TerminalKind, authorize_terminal};
    use crate::{dtos::response::AppError, servers::model::ServerState};

    #[test]
    fn local_console_requires_sysadmin_and_ready_local_server() {
        assert_eq!(
            authorize_terminal(true, "local", ServerState::Ready).unwrap(),
            TerminalKind::Local,
        );
        assert!(matches!(
            authorize_terminal(false, "local", ServerState::Ready),
            Err(AppError::InsufficientPermissions)
        ));
        assert!(authorize_terminal(true, "local", ServerState::Offline).is_err());
    }

    #[test]
    fn remote_console_keeps_existing_ready_ssh_policy() {
        assert_eq!(
            authorize_terminal(false, "ssh", ServerState::Ready).unwrap(),
            TerminalKind::Ssh,
        );
        assert_eq!(
            authorize_terminal(false, "ssh", ServerState::Ready).unwrap(),
            TerminalKind::Ssh,
        );
        assert!(authorize_terminal(true, "ssh", ServerState::Offline).is_err());
    }

    #[test]
    fn local_ssh_configuration_rejects_root_and_missing_material() {
        let config = |user: &str, key: &str| {
            LocalSshConfig::new(
                "host.containers.internal".into(),
                22,
                user.into(),
                key.into(),
                "SHA256:pinned".into(),
                "host.containers.internal ssh-ed25519 AAAA".into(),
            )
        };
        assert!(config("root", "private").is_err());
        assert!(config("installer", "").is_err());
        assert_eq!(
            config("installer", "private").unwrap().target.user,
            "installer"
        );
    }
}
