use std::{ffi::OsString, path::Path, process::Stdio, time::Duration};

use tokio::{
    io::{AsyncBufReadExt, AsyncRead, BufReader},
    process::Command,
};

use crate::{deployment::history, state::state::SharedState};

pub(super) fn compose_launch_description(project: &str, files: &[&str], action: &[&str]) -> String {
    let files = files
        .iter()
        .map(|file| format!("--file {file}"))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "Launching podman compose --project-name {project} {files} {}",
        action.join(" ")
    )
}

pub(super) fn compose_failure(exit_code: Option<i32>, stderr_tail: &[String]) -> String {
    let status = exit_code
        .map(|code| format!("exit code {code}"))
        .unwrap_or_else(|| "termination by signal".to_string());
    let stderr = stderr_tail.join("\n");
    if stderr.is_empty() {
        format!("Podman Compose failed with {status}")
    } else {
        format!("Podman Compose failed with {status}: {stderr}")
    }
}

pub(super) async fn log_stage(
    state: &SharedState,
    application_id: &str,
    run_id: Option<&str>,
    level: &str,
    message: &str,
) {
    if level == "error" {
        tracing::error!(application_id, "[compose-standalone] {message}");
    } else {
        tracing::info!(application_id, "[compose-standalone] {message}");
    }
    if let Some(run_id) = run_id {
        let _ = history::append(state, run_id, level, message).await;
    }
}

pub(super) async fn stream_compose_output<R>(
    reader: R,
    state: SharedState,
    application_id: String,
    run_id: Option<String>,
    stream_name: &'static str,
    level: &'static str,
) -> Result<Vec<String>, String>
where
    R: AsyncRead + Unpin,
{
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    let mut tail = Vec::new();
    loop {
        line.clear();
        let read = reader
            .read_line(&mut line)
            .await
            .map_err(|error| format!("Could not read Podman Compose {stream_name}: {error}"))?;
        if read == 0 {
            break;
        }
        let message = line.trim_end_matches(['\r', '\n']);
        if message.is_empty() {
            continue;
        }
        log_stage(&state, &application_id, run_id.as_deref(), level, message).await;
        tail.push(message.to_string());
        if tail.len() > 40 {
            tail.remove(0);
        }
    }
    Ok(tail)
}

/// Runs one `podman compose` invocation and streams its output into the
/// deployment log. Split out of `compose_cmd` so a failed `up` can be retried
/// after a conflicting container is cleared.
pub(super) async fn spawn_compose(
    state: &SharedState,
    application_id: &str,
    run_id: Option<&str>,
    base_dir: &Path,
    args: &[OsString],
    environment: &[(String, String)],
) -> Result<(), String> {
    let mut cmd = Command::new("podman");
    // `podman compose` delegates to an external provider. On macOS that provider
    // must use Podman's named machine connection; it cannot resolve the temporary
    // unix socket accepted by the main Podman client.
    #[cfg(target_os = "linux")]
    cmd.arg("--url").arg(&state.podman_socket);
    cmd.args(args);
    cmd.current_dir(base_dir);
    cmd.envs(environment.iter().map(|(name, value)| (name, value)));
    // After `envs`, so an application variable can never shadow it: the compose
    // provider spawns its own `podman` children, and they only find the mounted
    // socket through `CONTAINER_HOST`.
    #[cfg(target_os = "linux")]
    cmd.env("CONTAINER_HOST", &state.podman_socket);

    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd
        .spawn()
        .map_err(|error| format!("Could not start Podman Compose: {error}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Could not capture Podman Compose stdout".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "Could not capture Podman Compose stderr".to_string())?;
    let stdout_task = tokio::spawn(stream_compose_output(
        stdout,
        state.clone(),
        application_id.to_string(),
        run_id.map(str::to_string),
        "stdout",
        "info",
    ));
    let stderr_task = tokio::spawn(stream_compose_output(
        stderr,
        state.clone(),
        application_id.to_string(),
        run_id.map(str::to_string),
        "stderr",
        "info",
    ));

    let status = match tokio::time::timeout(Duration::from_secs(30 * 60), child.wait()).await {
        Ok(result) => Some(result.map_err(|error| format!("Podman Compose wait failed: {error}"))?),
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            None
        }
    };
    let _stdout_tail = stdout_task
        .await
        .map_err(|error| format!("Podman Compose stdout task failed: {error}"))??;
    let stderr_tail = stderr_task
        .await
        .map_err(|error| format!("Podman Compose stderr task failed: {error}"))??;

    let Some(status) = status else {
        return Err("Podman Compose timed out after 30 minutes".into());
    };
    if status.success() {
        Ok(())
    } else {
        Err(compose_failure(status.code(), &stderr_tail))
    }
}

/// Same shape as `spawn_compose`, but drives the command over the SSH stream
/// `ServerRuntime::podman_stream` returns instead of a local child, so a
/// remote `podman compose up` gets the same live log lines and the same
/// enforced-timeout-then-kill behavior local deploys already had.
pub(super) async fn spawn_remote_compose(
    state: &SharedState,
    application_id: &str,
    run_id: Option<&str>,
    runtime: &crate::servers::runtime::ServerRuntime,
    args: &[OsString],
) -> Result<(), String> {
    let mut stream = runtime
        .podman_stream(args)
        .await
        .map_err(|error| error.to_string())?;
    let stdout = stream
        .take_stdout()
        .ok_or_else(|| "Could not capture remote Podman Compose stdout".to_string())?;
    let stderr = stream
        .take_stderr()
        .ok_or_else(|| "Could not capture remote Podman Compose stderr".to_string())?;
    let stdout_task = tokio::spawn(stream_compose_output(
        stdout,
        state.clone(),
        application_id.to_string(),
        run_id.map(str::to_string),
        "stdout",
        "info",
    ));
    let stderr_task = tokio::spawn(stream_compose_output(
        stderr,
        state.clone(),
        application_id.to_string(),
        run_id.map(str::to_string),
        "stderr",
        "info",
    ));

    let status = match tokio::time::timeout(Duration::from_secs(30 * 60), stream.wait()).await {
        Ok(result) => Some(result.map_err(|error| error.to_string())?),
        Err(_) => {
            // The stream also carries `kill_on_drop`, but killing explicitly here
            // (rather than waiting for the drop at the end of this function) closes
            // the SSH channel immediately, which is what actually ends the remote
            // command instead of leaving it to keep running detached.
            stream.kill().await;
            let _ = stream.wait().await;
            None
        }
    };
    let _stdout_tail = stdout_task
        .await
        .map_err(|error| format!("Remote Podman Compose stdout task failed: {error}"))??;
    let stderr_tail = stderr_task
        .await
        .map_err(|error| format!("Remote Podman Compose stderr task failed: {error}"))??;

    let Some(status) = status else {
        return Err("Remote Podman Compose timed out after 30 minutes".into());
    };
    if status.success() {
        Ok(())
    } else {
        Err(compose_failure(status.code(), &stderr_tail))
    }
}

/// Best-effort forced teardown of one app's whole Compose project - its pod(s),
/// every container still carrying the project label (including ones an earlier
/// container-naming scheme orphaned inside the pod), and the project's private
/// bridge network. Runs over `runtime`, so it reaches a remote host exactly as
/// it does the local socket.
///
/// Called only for a failed candidate or the old project after promotion.
/// The explicit release project prevents a failed deployment from deleting
/// the application's serving project.
///
/// Named volumes are deliberately never touched: the database data has to
/// survive the recreate. Every step ignores its own failure - a missing pod or
/// network just means there was nothing to clean.
pub(crate) async fn teardown_project(
    runtime: &crate::servers::runtime::ServerRuntime,
    project: &str,
) {
    let label = format!("label=io.podman.compose.project={project}");

    if let Ok(output) = runtime
        .podman(&[
            "pod".into(),
            "ps".into(),
            "--filter".into(),
            label.clone().into(),
            "--format".into(),
            "{{.Name}}".into(),
        ])
        .await
    {
        for pod in output
            .stdout
            .lines()
            .map(str::trim)
            .filter(|p| !p.is_empty())
        {
            let _ = runtime
                .podman(&[
                    "pod".into(),
                    "rm".into(),
                    "--force".into(),
                    "--ignore".into(),
                    pod.into(),
                ])
                .await;
        }
    }

    if let Ok(output) = runtime
        .podman(&[
            "ps".into(),
            "--all".into(),
            "--filter".into(),
            label.into(),
            "--format".into(),
            "{{.ID}}".into(),
        ])
        .await
    {
        for id in output
            .stdout
            .lines()
            .map(str::trim)
            .filter(|c| !c.is_empty())
        {
            let _ = runtime
                .podman(&["rm".into(), "--force".into(), "--ignore".into(), id.into()])
                .await;
        }
    }

    if let Ok(output) = runtime
        .podman(&[
            "network".into(),
            "ls".into(),
            "--filter".into(),
            format!("label=io.podman.compose.project={project}").into(),
            "--format".into(),
            "{{.Name}}".into(),
        ])
        .await
    {
        for network in output
            .stdout
            .lines()
            .map(str::trim)
            .filter(|name| !name.is_empty())
        {
            // No force: a network still serving another container must survive.
            let _ = runtime
                .podman(&["network".into(), "rm".into(), network.into()])
                .await;
        }
    }
}

pub(super) async fn upload_remote_compose_file(
    runtime: &crate::servers::runtime::ServerRuntime,
    application_id: &str,
    path: &Path,
    workspace: &str,
) -> Result<String, String> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "Generated Compose file has an invalid name".to_string())?;
    runtime
        .upload_workspace_file(application_id, path, name)
        .await
        .map_err(|error| error.to_string())?;
    Ok(format!("{workspace}/{name}"))
}
