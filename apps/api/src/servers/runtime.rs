use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    time::Duration,
};

use tokio::process::Command;

use crate::{
    dtos::response::AppError,
    servers::{
        model::{PodmanMode, ServerState, SshTarget},
        repository::{self, ServerRecord},
        ssh::{SshBytesOutput, SshExecutor, SshOutput, SshStream, SshUnixRelay, clamp_tail},
    },
    state::state::SharedState,
};

/// The only entry point deployment/runtime code should use to reach Podman. It deliberately
/// resolves placement from the application row so an unavailable remote can never fall through
/// to the local Podman socket.
#[derive(Clone)]
pub enum ServerRuntime {
    Local {
        podman_socket: String,
    },
    Ssh {
        server_name: String,
        target: SshTarget,
        private_key_encrypted: String,
        host_key_fingerprint: String,
        host_key_known_hosts: String,
        podman_mode: PodmanMode,
    },
}

impl ServerRuntime {
    pub const fn is_remote(&self) -> bool {
        matches!(self, Self::Ssh { .. })
    }

    /// The deployment log must reflect the command privilege actually used;
    /// it is also useful when diagnosing provider paths such as `/root`.
    pub const fn remote_podman_mode(&self) -> Option<PodmanMode> {
        match self {
            Self::Local { .. } => None,
            Self::Ssh { podman_mode, .. } => Some(*podman_mode),
        }
    }

    /// A safe display name only; connection material stays inside the runtime.
    pub fn remote_server_name(&self) -> Option<&str> {
        match self {
            Self::Local { .. } => None,
            Self::Ssh { server_name, .. } => Some(server_name),
        }
    }

    /// `podman` plus whatever escalation the server's mode requires.
    fn remote_podman_argv(
        podman_mode: PodmanMode,
        target: &SshTarget,
        args: &[OsString],
    ) -> Vec<OsString> {
        let mut command = podman_mode
            .podman_prefix(&target.user)
            .iter()
            .map(OsString::from)
            .collect::<Vec<_>>();
        command.push(OsString::from("podman"));
        command.extend(args.iter().cloned());
        command
    }

    pub async fn for_application(
        state: &SharedState,
        application_id: &str,
    ) -> Result<Self, AppError> {
        let runtime = Self::application_runtime(state, application_id).await?;
        // On cloud the local server is the control plane host: watchable by a
        // SysAdmin, never a place to run an application.
        if state.is_cloud && matches!(runtime, Self::Local { .. }) {
            return Err(AppError::ValidationError(
                "Cloud mode requires a ready remote server. Connect and set up a remote server first."
                    .into(),
            ));
        }
        Ok(runtime)
    }

    async fn application_runtime(
        state: &SharedState,
        application_id: &str,
    ) -> Result<Self, AppError> {
        if let Some(server_id) = crate::servers::replica::current() {
            return Self::for_server(state, &server_id).await;
        }
        use sqlx::Row;
        let conn = crate::git::repositories::db::get_conn(state).await;
        let application = sqlx::query("SELECT server_id FROM applications WHERE id = ?1")
            .bind(application_id)
            .fetch_optional(&conn)
            .await
            .map_err(|error| AppError::DatabaseError(error.to_string()))?
            .ok_or(AppError::ApplicationNotFound)?;
        let server_id: Option<String> = application
            .try_get(0)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?;
        let server_id = server_id.ok_or_else(|| {
            AppError::ValidationError(
                "This application targets a server group; deploy through a replica".into(),
            )
        })?;
        let server = repository::get(state, &server_id).await?.ok_or_else(|| {
            AppError::DatabaseError("Application references a missing server".into())
        })?;
        Self::from_server(state, server).await
    }

    /// Build a runtime for a server looked up by id. Lets callers that pin a
    /// resource (scheduled task, ad-hoc job) to a server reach the same
    /// Local/SSH dispatch the application path uses.
    pub async fn for_server(state: &SharedState, server_id: &str) -> Result<Self, AppError> {
        let server = repository::get(state, server_id).await?.ok_or_else(|| {
            AppError::ValidationError(format!("Server '{server_id}' was not found"))
        })?;
        Self::from_server(state, server).await
    }

    async fn from_server(state: &SharedState, server: ServerRecord) -> Result<Self, AppError> {
        if server.state != ServerState::Ready {
            return Err(AppError::ResourceInUse(format!(
                "Server '{}' is unavailable",
                server.name
            )));
        }
        Self::from_server_unchecked(state, server).await
    }

    /// Builds a runtime for a server whose `state` column says it is unusable. Only
    /// `servers::monitor` may call this: re-checking that claim is the whole point of
    /// the health monitor. Everything else goes through `from_server`, which keeps the
    /// Ready gate that stops a deploy from touching a broken host.
    pub(crate) async fn from_server_unchecked(
        state: &SharedState,
        server: ServerRecord,
    ) -> Result<Self, AppError> {
        let server = repository::ensure_host_key_known_hosts(state, server).await?;
        if server.kind == "local" {
            return Ok(Self::Local {
                podman_socket: state.podman_socket.clone(),
            });
        }
        let server_name = server.name.clone();
        let target =
            SshTarget::parse(
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
        let host_key_fingerprint = server.host_key_fingerprint.ok_or_else(|| {
            AppError::ValidationError("Remote server SSH host key is not confirmed".into())
        })?;
        let host_key_known_hosts = server.host_key_known_hosts.ok_or_else(|| {
            AppError::InternalError("Remote server SSH known-hosts entry is unavailable".into())
        })?;
        Ok(Self::Ssh {
            server_name,
            target,
            private_key_encrypted: server.private_key_encrypted.ok_or_else(|| {
                AppError::InternalError("Remote server SSH credential is unavailable".into())
            })?,
            host_key_fingerprint,
            host_key_known_hosts,
            podman_mode: server.podman_mode,
        })
    }

    pub async fn podman(&self, args: &[OsString]) -> Result<SshOutput, AppError> {
        // The two placements kept different budgets before they shared a body, and
        // widening the remote one would let a hung SSH session sit on a request.
        let budget = match self {
            Self::Local { .. } => Duration::from_secs(90),
            Self::Ssh { .. } => Duration::from_secs(45),
        };
        self.podman_with_timeout(args, budget).await
    }

    /// `podman` with an explicit budget. A liveness probe must not hold a slot for the
    /// whole span a deploy step is allowed.
    pub async fn podman_with_timeout(
        &self,
        args: &[OsString],
        budget: Duration,
    ) -> Result<SshOutput, AppError> {
        match self {
            Self::Local { podman_socket } => {
                let output = tokio::time::timeout(
                    budget,
                    Command::new("podman")
                        .arg("--url")
                        .arg(podman_socket)
                        .args(args)
                        .output(),
                )
                .await
                .map_err(|_| AppError::ValidationError("Podman command timed out".into()))?
                .map_err(|error| {
                    AppError::InternalError(format!("Could not start Podman: {error}"))
                })?;
                let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
                let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
                if !output.status.success() {
                    return Err(AppError::ValidationError(format!(
                        "Podman command failed: {}",
                        clamp_tail(&stderr, 500)
                    )));
                }
                Ok(SshOutput { stdout, stderr })
            }
            Self::Ssh {
                target,
                private_key_encrypted,
                host_key_fingerprint,
                host_key_known_hosts,
                podman_mode,
                ..
            } => {
                let command = Self::remote_podman_argv(*podman_mode, target, args);
                SshExecutor::run_with_timeout(
                    target,
                    private_key_encrypted,
                    host_key_fingerprint,
                    Some(host_key_known_hosts),
                    &command,
                    budget,
                )
                .await
            }
        }
    }

    /// Reads a file out of a container as the tar stream produced by
    /// `podman cp <container>:<path> -`. Byte-preserving on both placements —
    /// local captures raw stdout, ssh routes through [`Self::remote_podman_bytes`]
    /// — so binary database dumps survive intact. The caller unwraps the
    /// single member.
    pub async fn podman_cp_from(
        &self,
        container: &str,
        path: &str,
        budget: Duration,
    ) -> Result<Vec<u8>, AppError> {
        let source = format!("{container}:{path}");
        let args = [OsString::from("cp"), source.into(), OsString::from("-")];
        match self {
            Self::Local { podman_socket } => {
                let output = tokio::time::timeout(
                    budget,
                    Command::new("podman")
                        .arg("--url")
                        .arg(podman_socket)
                        .args(&args)
                        .output(),
                )
                .await
                .map_err(|_| AppError::ValidationError("Podman cp timed out".into()))?
                .map_err(|error| {
                    AppError::InternalError(format!("Could not start Podman: {error}"))
                })?;
                if !output.status.success() {
                    return Err(AppError::ValidationError(format!(
                        "Podman cp failed: {}",
                        clamp_tail(&String::from_utf8_lossy(&output.stderr), 500)
                    )));
                }
                Ok(output.stdout)
            }
            Self::Ssh { .. } => {
                let bytes = self.remote_podman_bytes(&args, budget).await?.stdout;
                Ok(bytes)
            }
        }
    }

    /// Writes a tar stream *into* a container path (`podman cp - <container>:<path>`).
    /// Local pipes the tar through stdin; ssh stages the tar on the remote host
    /// (the application workspace) and `podman cp`s it in, then removes the
    /// staging file. Used by database restore only.
    #[allow(clippy::too_many_arguments)]
    pub async fn podman_cp_to(
        &self,
        application_id: &str,
        container: &str,
        path: &str,
        tar_bytes: Vec<u8>,
        budget: Duration,
    ) -> Result<(), AppError> {
        let destination = format!("{container}:{path}");
        match self {
            Self::Local { podman_socket } => {
                let mut child = Command::new("podman")
                    .arg("--url")
                    .arg(podman_socket)
                    .args(["cp", "-"])
                    .arg(&destination)
                    .stdin(std::process::Stdio::piped())
                    .kill_on_drop(true)
                    .spawn()
                    .map_err(|error| {
                        AppError::InternalError(format!("Could not start Podman: {error}"))
                    })?;
                if let Some(mut stdin) = child.stdin.take() {
                    use tokio::io::AsyncWriteExt;
                    stdin.write_all(&tar_bytes).await.map_err(|error| {
                        AppError::InternalError(format!("Could not pipe restore tar: {error}"))
                    })?;
                }
                let output = tokio::time::timeout(budget, child.wait_with_output())
                    .await
                    .map_err(|_| AppError::ValidationError("Podman cp timed out".into()))?
                    .map_err(|error| {
                        AppError::InternalError(format!("Could not start Podman: {error}"))
                    })?;
                if !output.status.success() {
                    return Err(AppError::ValidationError(format!(
                        "Podman cp failed: {}",
                        clamp_tail(&String::from_utf8_lossy(&output.stderr), 500)
                    )));
                }
                Ok(())
            }
            Self::Ssh {
                target,
                private_key_encrypted,
                host_key_fingerprint,
                host_key_known_hosts,
                ..
            } => {
                let temp =
                    std::env::temp_dir().join(format!("litepod-cp-{}.tar", uuid::Uuid::now_v7()));
                std::fs::write(&temp, tar_bytes).map_err(|error| {
                    AppError::InternalError(format!("Could not stage restore tar: {error}"))
                })?;
                let result = async {
                    let workspace =
                        self.ensure_workspace(application_id)
                            .await?
                            .ok_or_else(|| {
                                AppError::InternalError(
                                    "Remote restore needs an application workspace".into(),
                                )
                            })?;
                    let staged = format!("{workspace}/litepod-db-restore.tar");
                    SshExecutor::copy_file(
                        target,
                        private_key_encrypted,
                        host_key_fingerprint,
                        Some(host_key_known_hosts),
                        &temp,
                        &staged,
                    )
                    .await?;
                    self.podman_with_timeout(
                        &[
                            OsString::from("cp"),
                            staged.clone().into(),
                            destination.into(),
                        ],
                        budget,
                    )
                    .await?;
                    let cleanup = [OsString::from("rm"), OsString::from("-f"), staged.into()];
                    let _ = SshExecutor::run(
                        target,
                        private_key_encrypted,
                        host_key_fingerprint,
                        Some(host_key_known_hosts),
                        &cleanup,
                    )
                    .await;
                    Ok::<(), AppError>(())
                }
                .await;
                let _ = std::fs::remove_file(&temp);
                result
            }
        }
    }

    /// Podman output consumed incrementally instead of collected, for follow-mode
    /// reads. Both placements return a child with a piped stdout so callers keep a
    /// single code path; dropping the returned value stops the command.
    pub async fn podman_stream(&self, args: &[OsString]) -> Result<SshStream, AppError> {
        match self {
            Self::Local { podman_socket } => {
                let child = Command::new("podman")
                    .arg("--url")
                    .arg(podman_socket)
                    .args(args)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .kill_on_drop(true)
                    .spawn()
                    .map_err(|error| {
                        AppError::InternalError(format!("Could not start Podman: {error}"))
                    })?;
                Ok(SshStream::local(child))
            }
            Self::Ssh {
                target,
                private_key_encrypted,
                host_key_fingerprint,
                host_key_known_hosts,
                podman_mode,
                ..
            } => {
                let command = Self::remote_podman_argv(*podman_mode, target, args);
                SshExecutor::stream_command(
                    target,
                    private_key_encrypted,
                    host_key_fingerprint,
                    Some(host_key_known_hosts),
                    &command,
                )
                .await
            }
        }
    }

    /// Establishes a short-lived Unix-socket tunnel to a remote Podman API.
    /// Terminal execs use this rather than a raw `ssh podman exec` command so
    /// the existing Podman multiplexer can carry PTY resize events.
    pub async fn podman_unix_relay(&self) -> Result<SshUnixRelay, AppError> {
        let Self::Ssh {
            target,
            private_key_encrypted,
            host_key_fingerprint,
            host_key_known_hosts,
            podman_mode,
            ..
        } = self
        else {
            return Err(AppError::InternalError(
                "Remote Podman tunnel requested for the Local server".into(),
            ));
        };
        let remote_socket = match podman_mode {
            PodmanMode::Rootful => {
                let socket = PathBuf::from("/run/podman/podman.sock");
                SshExecutor::run(
                    target,
                    private_key_encrypted,
                    host_key_fingerprint,
                    Some(host_key_known_hosts),
                    &[
                        OsString::from("test"),
                        OsString::from("-r"),
                        socket.as_os_str().to_owned(),
                        OsString::from("-a"),
                        OsString::from("-w"),
                        socket.as_os_str().to_owned(),
                    ],
                )
                .await
                .map_err(|_| AppError::ValidationError(
                    "Rootful remote terminal access requires Server Repair so Litepod can grant the SSH user access to Podman's socket".into(),
                ))?;
                socket
            }
            PodmanMode::Rootless => {
                let output = SshExecutor::run(
                    target,
                    private_key_encrypted,
                    host_key_fingerprint,
                    Some(host_key_known_hosts),
                    &[OsString::from("id"), OsString::from("-u")],
                )
                .await?;
                let uid = output.stdout.trim();
                if uid.is_empty() || !uid.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err(AppError::InternalError(
                        "Remote SSH user returned an invalid uid".into(),
                    ));
                }
                PathBuf::from(format!("/run/user/{uid}/podman/podman.sock"))
            }
        };
        SshExecutor::start_unix_socket_forward(
            target,
            private_key_encrypted,
            host_key_fingerprint,
            Some(host_key_known_hosts),
            &remote_socket,
        )
        .await
    }

    /// Byte-preserving Podman output for remote archive reads. Local volume
    /// workspace access keeps its existing Podman API implementation.
    pub async fn remote_podman_bytes(
        &self,
        args: &[OsString],
        budget: Duration,
    ) -> Result<SshBytesOutput, AppError> {
        match self {
            Self::Ssh {
                target,
                private_key_encrypted,
                host_key_fingerprint,
                host_key_known_hosts,
                podman_mode,
                ..
            } => {
                let command = Self::remote_podman_argv(*podman_mode, target, args);
                SshExecutor::run_bytes(
                    target,
                    private_key_encrypted,
                    host_key_fingerprint,
                    Some(host_key_known_hosts),
                    &command,
                    budget,
                )
                .await
            }
            Self::Local { .. } => Err(AppError::InternalError(
                "Remote volume archive requested for the Local server".into(),
            )),
        }
    }

    /// Runs a fixed non-Podman command in a remote application's isolated
    /// workspace. Callers must supply their own closed argv; this boundary
    /// deliberately refuses Local execution so a remote file operation can
    /// never mutate litepod's filesystem by accident.
    pub async fn remote_command(&self, args: &[OsString]) -> Result<SshOutput, AppError> {
        match self {
            Self::Ssh {
                target,
                private_key_encrypted,
                host_key_fingerprint,
                host_key_known_hosts,
                ..
            } => {
                SshExecutor::run(
                    target,
                    private_key_encrypted,
                    host_key_fingerprint,
                    Some(host_key_known_hosts),
                    args,
                )
                .await
            }
            Self::Local { .. } => Err(AppError::InternalError(
                "Remote workspace command requested for the Local server".into(),
            )),
        }
    }

    /// How long [`Self::wait_for_container_ready`] waits for the port to answer.
    pub const REMOTE_READY_BUDGET: Duration = Duration::from_secs(120);

    /// Wait until a remote container is running and accepting HTTP traffic on its
    /// loopback endpoint before ingress/status are updated. `Ok(false)` means the
    /// container kept running but nothing answered on the port (e.g. the routed
    /// port does not match the one the app listens on); only an exited container
    /// is an error.
    pub async fn wait_for_container_ready(
        &self,
        container_name: &str,
        host_port: u16,
    ) -> Result<bool, AppError> {
        if !self.is_remote() {
            return Ok(true);
        }
        let deadline = tokio::time::Instant::now() + Self::REMOTE_READY_BUDGET;
        while tokio::time::Instant::now() < deadline {
            let status = self
                .podman(&[
                    OsString::from("inspect"),
                    OsString::from("--format"),
                    OsString::from("{{.State.Status}} {{.State.ExitCode}}"),
                    OsString::from(container_name),
                ])
                .await;
            if let Ok(output) = status {
                let state = output.stdout.split_whitespace().next().unwrap_or_default();
                if state == "exited" || state == "dead" {
                    return Err(AppError::ValidationError(format!(
                        "Container '{container_name}' stopped before becoming ready"
                    )));
                }
                if state == "running"
                    && self
                        .remote_command(&[
                            OsString::from("curl"),
                            OsString::from("-sS"),
                            OsString::from("-o"),
                            OsString::from("/dev/null"),
                            OsString::from("--max-time"),
                            OsString::from("2"),
                            OsString::from(format!("http://127.0.0.1:{host_port}/")),
                        ])
                        .await
                        .is_ok()
                {
                    return Ok(true);
                }
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        Ok(false)
    }

    /// Streams a fixed non-Podman remote command whose output is consumed
    /// incrementally (e.g. a scheduled task running directly on the host).
    /// Refuses Local execution; dropping the returned [`SshStream`] stops the
    /// remote command with the SSH session.
    pub async fn remote_stream(&self, args: &[OsString]) -> Result<SshStream, AppError> {
        match self {
            Self::Ssh {
                target,
                private_key_encrypted,
                host_key_fingerprint,
                host_key_known_hosts,
                ..
            } => {
                SshExecutor::stream_command(
                    target,
                    private_key_encrypted,
                    host_key_fingerprint,
                    Some(host_key_known_hosts),
                    args,
                )
                .await
            }
            Self::Local { .. } => Err(AppError::InternalError(
                "Remote stream requested for the Local server".into(),
            )),
        }
    }

    /// Resolves a staging directory key to a remote path. A bare UUID is an
    /// application workspace; a `srv-<id>` key is a per-server console scratch
    /// area used by the container console's volume file browser.
    fn workspace(key: &str) -> Result<String, AppError> {
        if let Some(server) = key.strip_prefix("srv-") {
            if !server.is_empty()
                && server
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
            {
                return Ok(format!(".local/share/litepod/console/{key}"));
            }
            return Err(AppError::ValidationError(
                "Invalid console workspace key".into(),
            ));
        }
        uuid::Uuid::parse_str(key)
            .map_err(|_| AppError::ValidationError("Application ID must be a UUID".into()))?;
        Ok(format!(".local/share/litepod/apps/{key}"))
    }

    /// Creates the remote application's isolated working directory under the configured
    /// rootless user's home. Local callers keep using litepod's existing data directory.
    ///
    /// No shared Podman network is ensured here anymore: a remote app's containers never
    /// join one (see `compose_standalone::generate_remote_standalone_override`) - every
    /// ingress mode reaches a remote app through its loopback `endpoint` binding instead,
    /// so each app gets nothing but Compose's own private per-app network.
    pub async fn ensure_workspace(&self, application_id: &str) -> Result<Option<String>, AppError> {
        let workspace = Self::workspace(application_id)?;
        match self {
            Self::Local { .. } => Ok(None),
            Self::Ssh {
                target,
                private_key_encrypted,
                host_key_fingerprint,
                host_key_known_hosts,
                ..
            } => {
                SshExecutor::run(
                    target,
                    private_key_encrypted,
                    host_key_fingerprint,
                    Some(host_key_known_hosts),
                    &[
                        OsString::from("mkdir"),
                        OsString::from("-p"),
                        OsString::from(&workspace),
                    ],
                )
                .await?;
                Ok(Some(workspace))
            }
        }
    }

    /// Uploads one generated deployment input after the workspace exists. `filename` is kept
    /// deliberately closed to a single path component to prevent a deployment from escaping its
    /// assigned workspace.
    pub async fn upload_workspace_file(
        &self,
        application_id: &str,
        local: &Path,
        filename: &str,
    ) -> Result<Option<String>, AppError> {
        if filename.is_empty()
            || filename.contains(|c: char| matches!(c, '/' | '\\' | '\0') || c.is_whitespace())
        {
            return Err(AppError::InternalError(
                "Refusing an unsafe deployment filename".into(),
            ));
        }
        let workspace = self.ensure_workspace(application_id).await?;
        match (self, workspace) {
            (Self::Local { .. }, None) => Ok(None),
            (
                Self::Ssh {
                    target,
                    private_key_encrypted,
                    host_key_fingerprint,
                    host_key_known_hosts,
                    ..
                },
                Some(workspace),
            ) => {
                let destination = format!("{workspace}/{filename}");
                SshExecutor::copy_file(
                    target,
                    private_key_encrypted,
                    host_key_fingerprint,
                    Some(host_key_known_hosts),
                    local,
                    &destination,
                )
                .await?;
                Ok(Some(destination))
            }
            _ => Err(AppError::InternalError(
                "Invalid server runtime workspace state".into(),
            )),
        }
    }

    /// Archives a validated local directory, uploads it as one SSH file, then
    /// extracts it under the application's remote workspace. The temporary
    /// archive and the previous remote directory are both scoped to the UUID
    /// application workspace; no caller can choose a remote destination.
    pub async fn upload_workspace_tree(
        &self,
        application_id: &str,
        local: &Path,
        directory_name: &str,
    ) -> Result<Option<String>, AppError> {
        if directory_name.is_empty()
            || directory_name
                .contains(|c: char| matches!(c, '/' | '\\' | '\0') || c.is_whitespace())
            || !local.is_dir()
        {
            return Err(AppError::ValidationError(
                "Refusing an invalid local deployment directory".into(),
            ));
        }
        let Some(workspace) = self.ensure_workspace(application_id).await? else {
            return Ok(None);
        };
        let archive_path =
            std::env::temp_dir().join(format!("litepod-workspace-{}.tar", uuid::Uuid::now_v7()));
        let source = local.to_path_buf();
        let archive = archive_path.clone();
        let archive_directory = directory_name.to_string();
        tokio::task::spawn_blocking(move || -> Result<(), AppError> {
            let file = std::fs::File::create(&archive).map_err(|error| {
                AppError::InternalError(format!("Could not create deployment archive: {error}"))
            })?;
            let mut builder = tar::Builder::new(file);
            builder
                .append_dir_all(&archive_directory, &source)
                .map_err(|error| {
                    AppError::InternalError(format!("Could not archive deployment files: {error}"))
                })?;
            builder.finish().map_err(|error| {
                AppError::InternalError(format!("Could not finalize deployment archive: {error}"))
            })
        })
        .await
        .map_err(|error| {
            AppError::InternalError(format!("Deployment archive task failed: {error}"))
        })??;
        let archive_name = format!("{directory_name}.tar");
        let upload = self
            .upload_workspace_file(application_id, &archive_path, &archive_name)
            .await;
        let _ = std::fs::remove_file(&archive_path);
        upload?;
        self.remote_command(&[
            OsString::from("rm"),
            OsString::from("-rf"),
            OsString::from(format!("{workspace}/{directory_name}")),
        ])
        .await?;
        self.remote_command(&[
            OsString::from("tar"),
            OsString::from("-xf"),
            OsString::from(format!("{workspace}/{archive_name}")),
            OsString::from("-C"),
            OsString::from(&workspace),
        ])
        .await?;
        Ok(Some(format!("{workspace}/{directory_name}")))
    }
}
