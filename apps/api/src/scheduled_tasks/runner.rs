use futures_util::StreamExt;
use podman_api::{
    Podman,
    conn::TtyChunk,
    models,
    opts::{ContainerCreateOpts, ContainerLogsOpts, PullOpts},
};
use sqlx::Row;
use std::ffi::OsString;
use std::process::Stdio;
use tokio::process::Command;

use crate::{dtos::response::AppError, state::state::SharedState};

const LOG_DRAIN_BUDGET_SECS: u64 = 5;

fn podman_client(state: &SharedState) -> Result<Podman, AppError> {
    Podman::new(&state.podman_socket)
        .map_err(|error| AppError::InternalError(format!("Podman is unavailable: {error}")))
}

pub async fn execute(state: SharedState, execution_id: String) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(&state).await;
    let Some(row) = sqlx::query(
        "SELECT runner, image, network_access, server_id, shell, script, timeout_seconds, application_id, compose_service
         FROM scheduled_task_executions WHERE id = ?1 AND status = 'queued'",
    )
    .bind(execution_id.clone())
    .fetch_optional(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?
    else {
        return Ok(());
    };
    let runner: String = row
        .try_get(0)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let image: String = row
        .try_get::<Option<String>, _>(1)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?
        .unwrap_or_default();
    let network_access = row
        .try_get::<i64, _>(2)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?
        != 0;
    let server_id: String = row
        .try_get(3)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let shell: String = row
        .try_get(4)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let script: String = row
        .try_get(5)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let timeout_seconds: i64 = row
        .try_get(6)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let application_id: Option<String> = row
        .try_get(7)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let compose_service: Option<String> = row
        .try_get(8)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    if runner == "application_exec" {
        let Some(application_id) = application_id else {
            finish(
                &state,
                &execution_id,
                "failed",
                Some("The application_exec runner requires an application".into()),
            )
            .await?;
            return Ok(());
        };
        return execute_application_exec(
            state,
            execution_id,
            application_id,
            compose_service,
            shell,
            script,
            timeout_seconds,
        )
        .await;
    }
    if runner == "litepod_api" {
        if state.is_cloud {
            finish(
                &state,
                &execution_id,
                "failed",
                Some("The litepod_api runner is not available in cloud mode".into()),
            )
            .await?;
            return Ok(());
        }
        return execute_litepod_api(state, execution_id, shell, script, timeout_seconds).await;
    }
    if runner == "remote_shell" {
        return execute_remote_shell(
            state,
            execution_id,
            server_id,
            shell,
            script,
            timeout_seconds,
        )
        .await;
    }
    if runner != "ephemeral" {
        finish(
            &state,
            &execution_id,
            "failed",
            Some("Unknown scheduled task runner".into()),
        )
        .await?;
        return Ok(());
    }
    let image = normalize_image(&image);
    // The full id: a UUIDv7 prefix is only a timestamp, so runs started in the same
    // minute (clones, a burst of manual runs) used to collide on the name.
    let name = format!("litepod-scheduled-task-{execution_id}");
    let is_remote = server_id != crate::scheduled_tasks::LOCAL_SERVER_ID;
    // belt-and-suspenders: create/update/boot already refuse local placement in
    // cloud mode, but the control plane must never run a container on itself
    if !is_remote && state.is_cloud {
        finish(
            &state,
            &execution_id,
            "failed",
            Some("Cloud mode requires a ready remote server for scheduled tasks".into()),
        )
        .await?;
        return Ok(());
    }
    if is_remote {
        execute_remote_ephemeral(
            state,
            execution_id,
            server_id,
            image,
            network_access,
            shell,
            script,
            timeout_seconds,
            name,
        )
        .await
    } else {
        execute_local_ephemeral(
            state,
            execution_id,
            image,
            network_access,
            shell,
            script,
            timeout_seconds,
            name,
        )
        .await
    }
}

#[allow(clippy::too_many_arguments)]
async fn execute_local_ephemeral(
    state: SharedState,
    execution_id: String,
    image: String,
    network_access: bool,
    shell: String,
    script: String,
    timeout_seconds: i64,
    name: String,
) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(&state).await;
    let podman = podman_client(&state)?;
    let images = podman.images();
    if images.get(&image).inspect().await.is_err() {
        let mut pull = images.pull(&PullOpts::builder().reference(&image).build());
        while let Some(report) = pull.next().await {
            match report {
                Ok(report)
                    if report
                        .error
                        .as_deref()
                        .is_none_or(|error| error.trim().is_empty()) => {}
                Ok(report) => {
                    finish(
                        &state,
                        &execution_id,
                        "failed",
                        Some(format!(
                            "Could not pull image {image}: {}",
                            report.error.unwrap_or_default()
                        )),
                    )
                    .await?;
                    return Ok(());
                }
                Err(error) => {
                    finish(
                        &state,
                        &execution_id,
                        "failed",
                        Some(format!("Could not pull image {image}: {error}")),
                    )
                    .await?;
                    return Ok(());
                }
            }
        }
    }
    let container = podman.containers().get(&name);
    let now = chrono::Utc::now().timestamp_millis();
    sqlx::query("UPDATE scheduled_task_executions SET status = 'running', started_at = ?1, container_name = ?2 WHERE id = ?3 AND status = 'queued'").bind(now).bind(name.clone()).bind(execution_id.clone()).execute(&conn).await.map_err(|e| AppError::DatabaseError(e.to_string()))?;
    sqlx::query("UPDATE scheduled_task_executions SET image = ?1 WHERE id = ?2")
        .bind(image.clone())
        .bind(execution_id.clone())
        .execute(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let options = ContainerCreateOpts::builder()
        .name(&name)
        .image(&image)
        .command([shell.as_str(), "-c", script.as_str()])
        .net_namespace(models::Namespace {
            nsmode: Some(if network_access { "bridge" } else { "none" }.into()),
            value: None,
        })
        .no_new_privilages(true)
        .build();
    if let Err(error) = podman.containers().create(&options).await {
        let error = start_error(error.to_string(), &shell, &image);
        finish(&state, &execution_id, "failed", Some(error)).await?;
        return Ok(());
    }
    if let Err(error) = container.start(None).await {
        let error = start_error(error.to_string(), &shell, &image);
        finish(&state, &execution_id, "failed", Some(error)).await?;
        let _ = container.remove().await;
        return Ok(());
    }
    // stream logs in the background: the follow stream can outlive the container, so the
    // completion of the run is detected by polling the container state instead.
    let log_state = state.clone();
    let log_execution_id = execution_id.clone();
    let log_container_name = name.clone();
    let mut logs_handle = tokio::spawn(async move {
        let Ok(podman) = podman_client(&log_state) else {
            return;
        };
        let conn = crate::git::repositories::db::get_conn(&log_state).await;
        let options = ContainerLogsOpts::builder()
            .stdout(true)
            .stderr(true)
            .follow(true)
            .build();
        let log_container = podman.containers().get(&log_container_name);
        let mut logs = log_container.logs(&options);
        let mut log = LiveLog::new(conn.clone(), log_execution_id.clone());
        let mut flush = tokio::time::interval(LIVE_FLUSH_INTERVAL);
        loop {
            tokio::select! {
                chunk = logs.next() => match chunk {
                    Some(Ok(TtyChunk::StdOut(data))) => log.feed(0, &data).await,
                    Some(Ok(TtyChunk::StdErr(data))) => log.feed(1, &data).await,
                    Some(_) => {}
                    None => break,
                },
                _ = flush.tick() => log.flush().await,
            }
        }
        log.flush().await;
    });
    let timed_out = !wait_until_stopped(&container, timeout_seconds).await;
    if timed_out {
        let _ = container.kill().await;
    }
    // give the follow stream a moment to drain the tail, then stop it
    if tokio::time::timeout(
        std::time::Duration::from_secs(LOG_DRAIN_BUDGET_SECS),
        &mut logs_handle,
    )
    .await
    .is_err()
    {
        logs_handle.abort();
    }
    if timed_out {
        finish(
            &state,
            &execution_id,
            "failed",
            Some(format!("Execution timed out after {timeout_seconds}s")),
        )
        .await?;
        let _ = container.remove().await;
        return Ok(());
    }
    let exit_code = match container.inspect().await {
        Ok(inspect) => inspect
            .state
            .and_then(|state| state.exit_code)
            .map(i64::from)
            .unwrap_or(1),
        Err(error) => {
            finish(
                &state,
                &execution_id,
                "failed",
                Some(format!("Could not inspect execution result: {error}")),
            )
            .await?;
            let _ = container.remove().await;
            return Ok(());
        }
    };
    let conn = crate::git::repositories::db::get_conn(&state).await;
    sqlx::query("UPDATE scheduled_task_executions SET exit_code = ?1 WHERE id = ?2")
        .bind(exit_code)
        .bind(execution_id.clone())
        .execute(&conn)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    if exit_code == 0 {
        finish(&state, &execution_id, "succeeded", None).await?;
    } else {
        finish(
            &state,
            &execution_id,
            "failed",
            Some(format!("Process exited with code {exit_code}")),
        )
        .await?;
    }
    let _ = container.remove().await;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn execute_remote_ephemeral(
    state: SharedState,
    execution_id: String,
    server_id: String,
    image: String,
    network_access: bool,
    shell: String,
    script: String,
    timeout_seconds: i64,
    name: String,
) -> Result<(), AppError> {
    let runtime = match crate::servers::runtime::ServerRuntime::for_server(&state, &server_id).await
    {
        Ok(runtime) => runtime,
        Err(error) => {
            finish(&state, &execution_id, "failed", Some(error.to_string())).await?;
            return Ok(());
        }
    };
    if !runtime.is_remote() {
        // belt-and-suspenders: the runtime layer must not hand us a Local
        // handle in cloud mode, but if it ever does we still abort cleanly.
        finish(
            &state,
            &execution_id,
            "failed",
            Some(
                "Scheduled task is pinned to a remote server but the runtime resolved to Local"
                    .into(),
            ),
        )
        .await?;
        return Ok(());
    }
    let conn = crate::git::repositories::db::get_conn(&state).await;
    if let Err(error) = runtime.podman(&["pull".into(), image.clone().into()]).await {
        finish(
            &state,
            &execution_id,
            "failed",
            Some(format!(
                "Could not pull image {image} on {server_id}: {error}"
            )),
        )
        .await?;
        return Ok(());
    }
    let now = chrono::Utc::now().timestamp_millis();
    sqlx::query("UPDATE scheduled_task_executions SET status = 'running', started_at = ?1, container_name = ?2 WHERE id = ?3 AND status = 'queued'").bind(now).bind(name.clone()).bind(execution_id.clone()).execute(&conn).await.map_err(|e| AppError::DatabaseError(e.to_string()))?;
    sqlx::query("UPDATE scheduled_task_executions SET image = ?1 WHERE id = ?2")
        .bind(image.clone())
        .bind(execution_id.clone())
        .execute(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let network = remote_network_arg(&runtime, network_access);
    let create_args: Vec<OsString> = vec![
        "create".into(),
        "--name".into(),
        name.clone().into(),
        "--network".into(),
        network.into(),
        "--security-opt".into(),
        "no-new-privileges".into(),
        image.clone().into(),
        shell.clone().into(),
        "-c".into(),
        script.clone().into(),
    ];
    if let Err(error) = runtime.podman(&create_args).await {
        finish(&state, &execution_id, "failed", Some(error.to_string())).await?;
        let _ = runtime
            .podman(&["rm".into(), "-f".into(), name.clone().into()])
            .await;
        return Ok(());
    }
    if let Err(error) = runtime.podman(&["start".into(), name.clone().into()]).await {
        let error = start_error(error.to_string(), &shell, &image);
        finish(&state, &execution_id, "failed", Some(error)).await?;
        let _ = runtime
            .podman(&["rm".into(), "-f".into(), name.clone().into()])
            .await;
        return Ok(());
    }
    // `podman logs -f` over SSH streams the container's output live and returns
    // when the container exits, which is the completion signal. The stream has no
    // SSH timeout, so a task running longer than the usual 45s command budget is
    // not cut off; the deadline below bounds the whole run instead.
    let mut stream = match runtime
        .podman_stream(&["logs".into(), "-f".into(), name.clone().into()])
        .await
    {
        Ok(stream) => stream,
        Err(error) => {
            finish(&state, &execution_id, "failed", Some(error.to_string())).await?;
            let _ = runtime
                .podman(&["rm".into(), "-f".into(), name.clone().into()])
                .await;
            return Ok(());
        }
    };
    let conn = crate::git::repositories::db::get_conn(&state).await;
    let deadline =
        tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_seconds.max(1) as u64);
    // stream both pipes into the log table so the UI can follow the run live
    let streamed = tokio::time::timeout_at(
        deadline,
        pump_live_logs(
            &conn,
            &execution_id,
            stream.take_stdout(),
            stream.take_stderr(),
            false,
        ),
    )
    .await;
    if streamed.is_err() {
        let _ = runtime.podman(&["kill".into(), name.clone().into()]).await;
        // give the stream a moment to drain the tail, then drop it (which kills the ssh)
        tokio::time::sleep(std::time::Duration::from_secs(LOG_DRAIN_BUDGET_SECS)).await;
        finish(
            &state,
            &execution_id,
            "failed",
            Some(format!("Execution timed out after {timeout_seconds}s")),
        )
        .await?;
        let _ = runtime
            .podman(&["rm".into(), "-f".into(), name.clone().into()])
            .await;
        return Ok(());
    }
    drop(stream);
    let exit_code = match runtime
        .podman(&[
            "inspect".into(),
            name.clone().into(),
            "--format".into(),
            "{{.State.ExitCode}}".into(),
        ])
        .await
    {
        Ok(output) => output.stdout.trim().parse::<i64>().unwrap_or(1),
        Err(error) => {
            finish(
                &state,
                &execution_id,
                "failed",
                Some(format!(
                    "Could not inspect execution result on {server_id}: {error}"
                )),
            )
            .await?;
            let _ = runtime
                .podman(&["rm".into(), "-f".into(), name.clone().into()])
                .await;
            return Ok(());
        }
    };
    let conn = crate::git::repositories::db::get_conn(&state).await;
    sqlx::query("UPDATE scheduled_task_executions SET exit_code = ?1 WHERE id = ?2")
        .bind(exit_code)
        .bind(execution_id.clone())
        .execute(&conn)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    if exit_code == 0 {
        finish(&state, &execution_id, "succeeded", None).await?;
    } else {
        finish(
            &state,
            &execution_id,
            "failed",
            Some(format!("Process exited with code {exit_code}")),
        )
        .await?;
    }
    let _ = runtime
        .podman(&["rm".into(), "-f".into(), name.clone().into()])
        .await;
    Ok(())
}

fn remote_network_arg(
    runtime: &crate::servers::runtime::ServerRuntime,
    network_access: bool,
) -> String {
    if !network_access {
        return "none".to_string();
    }
    if let Some(podman_mode) = runtime.remote_podman_mode() {
        // ServerRuntime exposes the resolved bridge network for SSH; for the
        // rootless and rootful modes the network value is the same name and
        // podman handles the mode internally via the `podman` CLI.
        let _ = podman_mode;
    }
    // Fall back to the control-plane bridge when no remote-specific value is
    // known; podman resolves it relative to the remote host's namespaces.
    "bridge".to_string()
}

/// Runs the script inside an already-running application container via
/// `podman exec`, instead of a throwaway container from an image. `runtime`
/// already knows whether the application's server is local or remote, so
/// this is the one runner that works unmodified in both placements.
///
/// Output is streamed line by line into the log table as it happens, so the
/// execution can be watched live. For sh/bash/zsh the script's pid is written
/// inside the container so `kill_application_exec` can stop it; other shells
/// can only stop being followed, and the script runs to completion.
async fn execute_application_exec(
    state: SharedState,
    execution_id: String,
    application_id: String,
    compose_service: Option<String>,
    shell: String,
    script: String,
    timeout_seconds: i64,
) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(&state).await;
    let now = chrono::Utc::now().timestamp_millis();
    sqlx::query("UPDATE scheduled_task_executions SET status = 'running', started_at = ?1 WHERE id = ?2 AND status = 'queued'")
        .bind(now)
        .bind(execution_id.clone())
        .execute(&conn)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;

    let runtime = match crate::servers::runtime::ServerRuntime::for_application(
        &state,
        &application_id,
    )
    .await
    {
        Ok(runtime) => runtime,
        Err(error) => {
            finish(&state, &execution_id, "failed", Some(error.to_string())).await?;
            return Ok(());
        }
    };
    // Resolve against the containers the application really owns: a compose app has no
    // container named after the bare runtime name, only `{runtime_name}-{service}` ones.
    let targets = match crate::handler::deployments::log_targets(&state, &application_id).await {
        Ok(targets) => targets,
        Err(error) => {
            finish(&state, &execution_id, "failed", Some(error.to_string())).await?;
            return Ok(());
        }
    };
    let container_name = match pick_exec_container(&targets, compose_service.as_deref()) {
        Ok(name) => name,
        Err(message) => {
            finish(&state, &execution_id, "failed", Some(message)).await?;
            return Ok(());
        }
    };

    // POSIX-like shells record their pid inside the container first, so a kill request
    // can stop the script itself rather than only the exec client following it
    let killable = matches!(shell.as_str(), "sh" | "bash" | "zsh");
    let mut command: Vec<OsString> = vec!["exec".into(), container_name.clone().into()];
    if killable {
        command.extend([
            shell.clone().into(),
            "-c".into(),
            format!(
                "echo $$ > {}; exec \"$0\" -c \"$1\"",
                exec_pid_path(&execution_id)
            )
            .into(),
            shell.clone().into(),
            script.into(),
        ]);
    } else {
        command.extend([shell.clone().into(), "-c".into(), script.into()]);
    }
    if killable {
        let _ =
            sqlx::query("UPDATE scheduled_task_executions SET container_name = ?1 WHERE id = ?2")
                .bind(&container_name)
                .bind(&execution_id)
                .execute(&conn)
                .await;
    }
    let mut stream = match runtime.podman_stream(&command).await {
        Ok(stream) => stream,
        Err(error) => {
            finish(
                &state,
                &execution_id,
                "failed",
                Some(format!("Container '{container_name}': {error}")),
            )
            .await?;
            return Ok(());
        }
    };
    let deadline =
        tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_seconds.max(1) as u64);
    // stream both pipes into the log table so the UI can follow the run live; stop early
    // once the execution was canceled or replaced, since nobody owns it any more
    let streamed = tokio::time::timeout_at(
        deadline,
        pump_live_logs(
            &conn,
            &execution_id,
            stream.take_stdout(),
            stream.take_stderr(),
            true,
        ),
    )
    .await;
    if !matches!(streamed, Ok(true)) && killable {
        // canceled, replaced or timed out: stop the script itself, never the container
        kill_application_exec(&state, &application_id, &container_name, &execution_id).await;
    }
    match streamed {
        // canceled or replaced: that path already recorded the terminal state
        Ok(false) => return Ok(()),
        Err(_) => {
            finish(
                &state,
                &execution_id,
                "failed",
                Some(format!("Execution timed out after {timeout_seconds}s")),
            )
            .await?;
            return Ok(());
        }
        Ok(true) => {}
    }
    let exit_code = match stream.wait().await {
        Ok(status) => status.code().unwrap_or(1) as i64,
        Err(error) => {
            finish(
                &state,
                &execution_id,
                "failed",
                Some(format!("Container '{container_name}': {error}")),
            )
            .await?;
            return Ok(());
        }
    };
    if killable {
        let _ = runtime
            .podman(&[
                "exec".into(),
                container_name.clone().into(),
                "rm".into(),
                "-f".into(),
                exec_pid_path(&execution_id).into(),
            ])
            .await;
    }
    sqlx::query("UPDATE scheduled_task_executions SET exit_code = ?1 WHERE id = ?2")
        .bind(exit_code)
        .bind(execution_id.clone())
        .execute(&conn)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    if exit_code == 0 {
        finish(&state, &execution_id, "succeeded", None).await?;
    } else if exit_code == 127 && !shell_exists(&runtime, Some(&container_name), &shell).await {
        let error = missing_shell(&shell, &format!("container '{container_name}'"));
        finish(&state, &execution_id, "failed", Some(error)).await?;
    } else {
        finish(
            &state,
            &execution_id,
            "failed",
            Some(format!("Process exited with code {exit_code}")),
        )
        .await?;
    }
    Ok(())
}

/// Pid file an `application_exec` run writes inside its container.
fn exec_pid_path(execution_id: &str) -> String {
    format!("/tmp/litepod-task-{execution_id}.pid")
}

/// Stops an `application_exec` script inside its container: its direct children
/// first (e.g. a running `sleep`), then the shell itself. Best effort: a container
/// without `sh` or `/proc` children support keeps the script running.
pub async fn kill_application_exec(
    state: &SharedState,
    application_id: &str,
    container: &str,
    execution_id: &str,
) {
    let Ok(runtime) =
        crate::servers::runtime::ServerRuntime::for_application(state, application_id).await
    else {
        return;
    };
    let pid_path = exec_pid_path(execution_id);
    let kill = format!(
        "pid=$(cat {pid_path} 2>/dev/null) || exit 0; \
         kill -TERM $(cat /proc/$pid/task/*/children 2>/dev/null) $pid 2>/dev/null; \
         rm -f {pid_path}"
    );
    let _ = runtime
        .podman(&[
            "exec".into(),
            container.into(),
            "sh".into(),
            "-c".into(),
            kill.into(),
        ])
        .await;
}

/// Longest a form test may run: it holds the HTTP request open, so it is capped well
/// below the scheduled timeout.
const TEST_TIMEOUT_SECS: i64 = 120;

/// What a form test runs: the (possibly unsaved) task settings, already validated.
pub struct TestParams {
    pub runner: String,
    pub server_id: String,
    pub image: Option<String>,
    pub network_access: bool,
    pub application_id: Option<String>,
    pub compose_service: Option<String>,
    pub shell: String,
    pub script: String,
    pub timeout_seconds: i64,
}

/// One server-sent event of a form test. Output is forwarded as raw chunks, not
/// lines, so `\r`-redrawn progress bars move live in the UI.
#[derive(Debug, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TestEvent {
    Started {
        target: Option<String>,
    },
    Output {
        stream: &'static str,
        data: String,
    },
    Done {
        success: bool,
        exit_code: Option<i64>,
        error: Option<String>,
        duration_ms: i64,
    },
}

/// Runs the form's script once with the same placement rules as a scheduled
/// execution, streaming its output as it happens. Nothing is persisted. Dropping the
/// returned stream (the client went away) stops the command.
pub fn test_stream(
    state: SharedState,
    params: TestParams,
) -> tokio_stream::wrappers::UnboundedReceiverStream<TestEvent> {
    // unbounded: a slow browser must never stall reading the script's pipes
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        let started = std::time::Instant::now();
        let done = |success, exit_code, error| TestEvent::Done {
            success,
            exit_code,
            error,
            duration_ms: started.elapsed().as_millis() as i64,
        };
        let budget = params.timeout_seconds.clamp(1, TEST_TIMEOUT_SECS) as u64;
        let (mut stream, target, cleanup) = match spawn_test(&state, &params, budget).await {
            Ok(spawned) => spawned,
            Err(error) => {
                let _ = tx.send(done(false, None, Some(error)));
                return;
            }
        };
        if tx.send(TestEvent::Started { target }).is_err() {
            cleanup_test(cleanup).await;
            return;
        }
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(budget);
        let outcome = tokio::time::timeout_at(deadline, pump_output(&mut stream, &tx)).await;
        let finished = match outcome {
            Err(_) => Err(format!("Test timed out after {budget}s")),
            // the client disconnected: nobody is left to tell
            Ok(false) => {
                drop(stream);
                cleanup_test(cleanup).await;
                return;
            }
            Ok(true) => stream
                .wait()
                .await
                .map(|status| status.code().unwrap_or(1) as i64)
                .map_err(|error| error.to_string()),
        };
        drop(stream);
        let event = match finished {
            Ok(0) => done(true, Some(0), None),
            // a test is not worth a second probe: name the likely cause instead
            Ok(127) => done(
                false,
                Some(127),
                Some(format!(
                    "Process exited with code 127 (command not found; is shell '{}' installed on the target?)",
                    params.shell
                )),
            ),
            Ok(code) => done(
                false,
                Some(code),
                Some(format!("Process exited with code {code}")),
            ),
            Err(error) => {
                cleanup_test(cleanup).await;
                done(false, None, Some(error))
            }
        };
        let _ = tx.send(event);
    });
    tokio_stream::wrappers::UnboundedReceiverStream::new(rx)
}

/// Forwards stdout/stderr chunks until both pipes close. Returns false when the
/// receiver is gone, so the caller can stop the command early.
async fn pump_output(
    stream: &mut crate::servers::ssh::SshStream,
    tx: &tokio::sync::mpsc::UnboundedSender<TestEvent>,
) -> bool {
    use tokio::io::AsyncReadExt;
    let mut stdout = stream.take_stdout();
    let mut stderr = stream.take_stderr();
    let (mut out_buf, mut err_buf) = ([0_u8; 4096], [0_u8; 4096]);
    loop {
        let (stdout_open, stderr_open) = (stdout.is_some(), stderr.is_some());
        if !stdout_open && !stderr_open {
            return true;
        }
        let (name, data) = tokio::select! {
            read = async { stdout.as_mut().unwrap().read(&mut out_buf).await }, if stdout_open => match read {
                Ok(n) if n > 0 => ("stdout", String::from_utf8_lossy(&out_buf[..n]).into_owned()),
                _ => { stdout = None; continue; }
            },
            read = async { stderr.as_mut().unwrap().read(&mut err_buf).await }, if stderr_open => match read {
                Ok(n) if n > 0 => ("stderr", String::from_utf8_lossy(&err_buf[..n]).into_owned()),
                _ => { stderr = None; continue; }
            },
        };
        if tx.send(TestEvent::Output { stream: name, data }).is_err() {
            return false;
        }
    }
}

/// An ephemeral test container that must be force-removed if the run is cut short:
/// `--rm` only fires when the container exits on its own.
type TestCleanup = Option<(crate::servers::runtime::ServerRuntime, String)>;

async fn cleanup_test(cleanup: TestCleanup) {
    if let Some((runtime, name)) = cleanup {
        let _ = runtime
            .podman(&["rm".into(), "-f".into(), name.into()])
            .await;
    }
}

async fn spawn_test(
    state: &SharedState,
    params: &TestParams,
    budget: u64,
) -> Result<(crate::servers::ssh::SshStream, Option<String>, TestCleanup), String> {
    let shell = params.shell.as_str();
    let script = params.script.as_str();
    match params.runner.as_str() {
        "litepod_api" => {
            if state.is_cloud {
                return Err("The litepod_api runner is not available in cloud mode".into());
            }
            let child = Command::new(shell)
                .arg("-c")
                .arg(script)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .map_err(|error| spawn_error(&error, shell))?;
            Ok((crate::servers::ssh::SshStream::local(child), None, None))
        }
        "application_exec" => {
            let application_id = params.application_id.as_deref().unwrap_or_default();
            let runtime =
                crate::servers::runtime::ServerRuntime::for_application(state, application_id)
                    .await
                    .map_err(|error| error.to_string())?;
            let targets = crate::handler::deployments::log_targets(state, application_id)
                .await
                .map_err(|error| error.to_string())?;
            let container = pick_exec_container(&targets, params.compose_service.as_deref())?;
            let stream = runtime
                .podman_stream(&[
                    "exec".into(),
                    container.clone().into(),
                    shell.into(),
                    "-c".into(),
                    script.into(),
                ])
                .await
                .map_err(|error| format!("Container '{container}': {error}"))?;
            Ok((stream, Some(container), None))
        }
        "remote_shell" => {
            let runtime =
                crate::servers::runtime::ServerRuntime::for_server(state, &params.server_id)
                    .await
                    .map_err(|error| error.to_string())?;
            if !runtime.is_remote() {
                return Err("The remote_shell runner requires a remote server".into());
            }
            // `timeout` stops the script on the host even if the SSH session lingers
            let stream = runtime
                .remote_stream(&[
                    "timeout".into(),
                    budget.to_string().into(),
                    shell.into(),
                    "-c".into(),
                    script.into(),
                ])
                .await
                .map_err(|error| error.to_string())?;
            Ok((stream, None, None))
        }
        "ephemeral" => {
            let runtime =
                crate::servers::runtime::ServerRuntime::for_server(state, &params.server_id)
                    .await
                    .map_err(|error| error.to_string())?;
            let image = normalize_image(params.image.as_deref().unwrap_or_default());
            let name = format!(
                "litepod-scheduled-task-test-{}",
                &uuid::Uuid::now_v7().simple().to_string()[..12]
            );
            let stream = runtime
                .podman_stream(&[
                    "run".into(),
                    "--rm".into(),
                    "--pull=missing".into(),
                    "--name".into(),
                    name.clone().into(),
                    "--network".into(),
                    remote_network_arg(&runtime, params.network_access).into(),
                    image.into(),
                    shell.into(),
                    "-c".into(),
                    script.into(),
                ])
                .await
                .map_err(|error| error.to_string())?;
            Ok((stream, None, Some((runtime, name))))
        }
        _ => Err("Unknown scheduled task runner".into()),
    }
}

/// Picks the container an `application_exec` run targets. `compose_service` may be
/// the service name or the full container name; when it is empty, a single-container
/// application is unambiguous, while a multi-service compose app must name one.
fn pick_exec_container(
    targets: &[crate::handler::deployments::RuntimeLogTarget],
    compose_service: Option<&str>,
) -> Result<String, String> {
    if targets.is_empty() {
        return Err("The application has no deployed container to run the task in".into());
    }
    let services = || {
        targets
            .iter()
            .filter_map(|target| target.service.as_deref())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let target = match compose_service {
        Some(wanted) => targets
            .iter()
            .find(|target| {
                target.service.as_deref() == Some(wanted) || target.container_name == wanted
            })
            .ok_or_else(|| {
                format!(
                    "Compose service '{wanted}' not found; available services: {}",
                    services()
                )
            })?,
        None if targets.len() == 1 => &targets[0],
        None => {
            return Err(format!(
                "This Compose application has several services ({}); set the Compose service field of the task",
                services()
            ));
        }
    };
    // Only checked, never started: a stopped container is the operator's call.
    if !crate::handler::deployments::is_running_container(&target.status) {
        return Err(format!(
            "Container '{}' is not running (status: {}); start it before running the task",
            target.container_name, target.status
        ));
    }
    Ok(target.container_name.clone())
}

/// Runs the script directly on a remote server host via SSH (no container),
/// as the SSH user configured on the server. The command is backgrounded so
/// its remote pid can be captured for cancellation; a streamed SSH session
/// reports no exit status, so the exit code is persisted to a temp file that
/// is read back over a second (control-master multiplexed) SSH session.
#[allow(clippy::too_many_arguments)]
async fn execute_remote_shell(
    state: SharedState,
    execution_id: String,
    server_id: String,
    shell: String,
    script: String,
    timeout_seconds: i64,
) -> Result<(), AppError> {
    let runtime = match crate::servers::runtime::ServerRuntime::for_server(&state, &server_id).await
    {
        Ok(runtime) => runtime,
        Err(error) => {
            finish(&state, &execution_id, "failed", Some(error.to_string())).await?;
            return Ok(());
        }
    };
    if !runtime.is_remote() {
        finish(
            &state,
            &execution_id,
            "failed",
            Some("The remote_shell runner requires a remote server".into()),
        )
        .await?;
        return Ok(());
    }
    let conn = crate::git::repositories::db::get_conn(&state).await;
    let now = chrono::Utc::now().timestamp_millis();
    sqlx::query("UPDATE scheduled_task_executions SET status = 'running', started_at = ?1 WHERE id = ?2 AND status = 'queued'").bind(now).bind(execution_id.clone()).execute(&conn).await.map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let pid_path = format!("/tmp/litepod-{execution_id}.pid");
    let code_path = format!("/tmp/litepod-{execution_id}.code");
    let script_quoted = script.replace('\'', "'\\''");
    // The wrapper needs POSIX job control (`&`, `$!`, `wait`, `$?`) to capture the
    // pid and exit code, so it always runs under `/bin/sh`. Only the inner
    // `{shell} -c '<script>'` runs the user's script under their chosen shell,
    // which is what lets `fish` / `nu` work here without breaking the wrapper.
    let wrapper = format!(
        "{shell} -c '{script_quoted}' & pid=$!; echo $pid > {pid_path}; wait $pid; code=$?; echo $code > {code_path}"
    );
    let command: Vec<OsString> = vec!["/bin/sh".into(), "-c".into(), wrapper.into()];
    let mut stream = match runtime.remote_stream(&command).await {
        Ok(stream) => stream,
        Err(error) => {
            finish(&state, &execution_id, "failed", Some(error.to_string())).await?;
            return Ok(());
        }
    };
    // Persist the remote pid so a cancel request can kill the script. The pid
    // file is written a moment after the stream starts, so poll briefly.
    for _ in 0..15 {
        if let Ok(output) = runtime
            .remote_command(&[OsString::from("cat"), pid_path.clone().into()])
            .await
        {
            if let Ok(pid) = output.stdout.trim().parse::<i64>() {
                let _ = sqlx::query(
                    "UPDATE scheduled_task_executions SET process_group_id = ?1 WHERE id = ?2",
                )
                .bind(pid)
                .bind(execution_id.clone())
                .execute(&conn)
                .await;
                break;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    let deadline =
        tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_seconds.max(1) as u64);
    let streamed = tokio::time::timeout_at(
        deadline,
        pump_live_logs(
            &conn,
            &execution_id,
            stream.take_stdout(),
            stream.take_stderr(),
            false,
        ),
    )
    .await;
    if streamed.is_err() {
        let _ = remote_kill_by_pid_file(&runtime, &pid_path).await;
        // give the stream a moment to drain the tail, then drop it (which kills the ssh)
        tokio::time::sleep(std::time::Duration::from_secs(LOG_DRAIN_BUDGET_SECS)).await;
        finish(
            &state,
            &execution_id,
            "failed",
            Some(format!("Execution timed out after {timeout_seconds}s")),
        )
        .await?;
        let _ = cleanup_remote_files(&runtime, &pid_path, &code_path).await;
        return Ok(());
    }
    drop(stream);
    let exit_code = match runtime
        .remote_command(&[OsString::from("cat"), code_path.clone().into()])
        .await
    {
        Ok(output) => output.stdout.trim().parse::<i64>().unwrap_or(1),
        Err(error) => {
            finish(
                &state,
                &execution_id,
                "failed",
                Some(format!(
                    "Could not read execution result on {server_id}: {error}"
                )),
            )
            .await?;
            let _ = cleanup_remote_files(&runtime, &pid_path, &code_path).await;
            return Ok(());
        }
    };
    sqlx::query("UPDATE scheduled_task_executions SET exit_code = ?1 WHERE id = ?2")
        .bind(exit_code)
        .bind(execution_id.clone())
        .execute(&conn)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    if exit_code == 0 {
        finish(&state, &execution_id, "succeeded", None).await?;
    } else if exit_code == 127 && !shell_exists(&runtime, None, &shell).await {
        let error = missing_shell(&shell, &format!("host of server {server_id}"));
        finish(&state, &execution_id, "failed", Some(error)).await?;
    } else {
        finish(
            &state,
            &execution_id,
            "failed",
            Some(format!("Process exited with code {exit_code}")),
        )
        .await?;
    }
    let _ = cleanup_remote_files(&runtime, &pid_path, &code_path).await;
    Ok(())
}

async fn remote_kill_by_pid_file(
    runtime: &crate::servers::runtime::ServerRuntime,
    pid_path: &str,
) -> Result<(), AppError> {
    let Ok(output) = runtime
        .remote_command(&[OsString::from("cat"), pid_path.to_string().into()])
        .await
    else {
        // The pid capture may have raced the timeout; fall back to killing any
        // wrapper whose command line still references this execution's pid file.
        let tag = pid_path_tag(pid_path);
        let _ = runtime
            .remote_command(&[
                OsString::from("pkill"),
                OsString::from("-9"),
                OsString::from("-f"),
                OsString::from(format!("litepod-{tag}.pid")),
            ])
            .await;
        return Ok(());
    };
    let Ok(pid) = output.stdout.trim().parse::<i64>() else {
        return Ok(());
    };
    let _ = runtime
        .remote_command(&[
            OsString::from("kill"),
            OsString::from("-9"),
            OsString::from(pid.to_string()),
        ])
        .await;
    Ok(())
}

/// `/tmp/litepod-<execution_id>.pid` -> the execution_id used in `pkill -f`.
fn pid_path_tag(pid_path: &str) -> &str {
    pid_path
        .trim_start_matches("/tmp/litepod-")
        .trim_end_matches(".pid")
}

async fn cleanup_remote_files(
    runtime: &crate::servers::runtime::ServerRuntime,
    pid_path: &str,
    code_path: &str,
) -> Result<(), AppError> {
    let _ = runtime
        .remote_command(&[
            OsString::from("rm"),
            OsString::from("-f"),
            pid_path.to_string().into(),
            code_path.to_string().into(),
        ])
        .await;
    Ok(())
}

/// Best-effort kill of a remote host process (the `remote_shell` runner).
pub async fn kill_remote_process(
    state: &SharedState,
    server_id: &str,
    pid: i64,
) -> Result<(), AppError> {
    let runtime = crate::servers::runtime::ServerRuntime::for_server(state, server_id).await?;
    let _ = runtime
        .remote_command(&[
            OsString::from("kill"),
            OsString::from("-9"),
            OsString::from(pid.to_string()),
        ])
        .await;
    Ok(())
}

async fn wait_until_stopped(container: &podman_api::api::Container, timeout_seconds: i64) -> bool {
    let deadline =
        tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_seconds.max(1) as u64);
    loop {
        match container.inspect().await {
            Ok(inspect) => {
                let running = inspect
                    .state
                    .and_then(|state| state.running)
                    .unwrap_or(false);
                if !running {
                    return true;
                }
            }
            // the container disappeared (removed or never started): nothing left to wait for
            Err(_) => return true,
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(std::time::Duration::from_millis(750)).await;
    }
}

/// Runs at boot: in-process runners do not survive a restart, so anything queued or running
/// outside a container is orphaned.
pub async fn reconcile_on_startup(state: &SharedState) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let now = chrono::Utc::now().timestamp_millis();
    sqlx::query("UPDATE scheduled_task_executions SET status = 'interrupted', finished_at = ?1, error = ?2 WHERE status = 'queued' OR (status = 'running' AND runner IN ('litepod_api', 'remote_shell'))").bind(now).bind("Execution was interrupted by an API restart").execute(&conn).await.map_err(|e| AppError::DatabaseError(e.to_string()))?;
    sqlx::query("UPDATE scheduled_tasks SET active_execution_id = NULL, updated_at = ?1 WHERE active_execution_id IN (SELECT id FROM scheduled_task_executions WHERE status NOT IN ('queued', 'running'))").bind(now).execute(&conn).await.map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok(())
}

/// Closes executions still marked as running whose container is already gone: an API restart or a
/// stalled log stream would otherwise leave them running forever.
pub async fn reconcile_running(state: &SharedState) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let now = chrono::Utc::now().timestamp_millis();
    let rows = sqlx::query("SELECT id, runner, server_id, container_name, started_at, timeout_seconds FROM scheduled_task_executions WHERE status = 'running' AND started_at IS NOT NULL AND started_at < ?1")
        .bind(now - 120_000)
        .fetch_all(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let mut stale = Vec::new();
    for row in &rows {
        stale.push((
            row.try_get::<String, _>(0)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            row.try_get::<String, _>(1)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            row.try_get::<String, _>(2)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            row.try_get::<Option<String>, _>(3)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            row.try_get::<i64, _>(4)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            row.try_get::<i64, _>(5)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        ));
    }
    for (execution_id, runner, server_id, container_name, started_at, timeout_seconds) in stale {
        let expired = now - started_at > (timeout_seconds.max(1) * 1000) + 60_000;
        if runner != "ephemeral" {
            if expired {
                finish(
                    state,
                    &execution_id,
                    "failed",
                    Some(format!("Execution timed out after {timeout_seconds}s")),
                )
                .await?;
            }
            continue;
        }
        let Some(container_name) = container_name else {
            finish(
                state,
                &execution_id,
                "interrupted",
                Some("Execution was interrupted".into()),
            )
            .await?;
            continue;
        };
        if server_id == crate::scheduled_tasks::LOCAL_SERVER_ID {
            let Ok(podman) = podman_client(state) else {
                continue;
            };
            let container = podman.containers().get(&container_name);
            let still_running = match container.inspect().await {
                Ok(inspect) => inspect
                    .state
                    .as_ref()
                    .and_then(|value| value.running)
                    .unwrap_or(false),
                Err(_) => false,
            };
            if still_running && !expired {
                continue;
            }
            if still_running {
                let _ = container.kill().await;
                finish(
                    state,
                    &execution_id,
                    "failed",
                    Some(format!("Execution timed out after {timeout_seconds}s")),
                )
                .await?;
                let _ = container.remove().await;
                continue;
            }
            let exit_code = match container.inspect().await {
                Ok(inspect) => inspect
                    .state
                    .and_then(|value| value.exit_code)
                    .map(i64::from)
                    .unwrap_or(1),
                Err(_) => 1,
            };
            let _ =
                sqlx::query("UPDATE scheduled_task_executions SET exit_code = ?1 WHERE id = ?2")
                    .bind(exit_code)
                    .bind(execution_id.clone())
                    .execute(&conn)
                    .await;
            if exit_code == 0 {
                finish(state, &execution_id, "succeeded", None).await?;
            } else {
                finish(
                    state,
                    &execution_id,
                    "failed",
                    Some(format!("Process exited with code {exit_code}")),
                )
                .await?;
            }
            let _ = container.remove().await;
            continue;
        }
        let Ok(runtime) =
            crate::servers::runtime::ServerRuntime::for_server(state, &server_id).await
        else {
            continue;
        };
        let running = match runtime
            .podman(&[
                "inspect".into(),
                container_name.clone().into(),
                "--format".into(),
                "{{.State.Running}}".into(),
            ])
            .await
        {
            Ok(output) => output.stdout.trim().eq_ignore_ascii_case("true"),
            Err(_) => false,
        };
        if running && !expired {
            continue;
        }
        if running {
            let _ = runtime
                .podman(&["kill".into(), container_name.clone().into()])
                .await;
            finish(
                state,
                &execution_id,
                "failed",
                Some(format!("Execution timed out after {timeout_seconds}s")),
            )
            .await?;
            let _ = runtime
                .podman(&["rm".into(), "-f".into(), container_name.clone().into()])
                .await;
            continue;
        }
        let exit_code = match runtime
            .podman(&[
                "inspect".into(),
                container_name.clone().into(),
                "--format".into(),
                "{{.State.ExitCode}}".into(),
            ])
            .await
        {
            Ok(output) => output.stdout.trim().parse::<i64>().unwrap_or(1),
            Err(_) => 1,
        };
        let _ = sqlx::query("UPDATE scheduled_task_executions SET exit_code = ?1 WHERE id = ?2")
            .bind(exit_code)
            .bind(execution_id.clone())
            .execute(&conn)
            .await;
        if exit_code == 0 {
            finish(state, &execution_id, "succeeded", None).await?;
        } else {
            finish(
                state,
                &execution_id,
                "failed",
                Some(format!("Process exited with code {exit_code}")),
            )
            .await?;
        }
        let _ = runtime
            .podman(&["rm".into(), "-f".into(), container_name.clone().into()])
            .await;
    }
    Ok(())
}

pub async fn kill_container(
    state: &SharedState,
    server_id: &str,
    container_name: &str,
) -> Result<(), AppError> {
    if server_id == crate::scheduled_tasks::LOCAL_SERVER_ID {
        let podman = podman_client(state)?;
        let container = podman.containers().get(container_name);
        let _ = container.kill().await;
        let _ = container.remove().await;
        return Ok(());
    }
    let runtime = crate::servers::runtime::ServerRuntime::for_server(state, server_id).await?;
    let _ = runtime
        .podman(&["kill".into(), container_name.to_string().into()])
        .await;
    let _ = runtime
        .podman(&["rm".into(), "-f".into(), container_name.to_string().into()])
        .await;
    Ok(())
}

pub async fn kill_process_group(process_group_id: i64) {
    // negative pid targets the whole process group spawned for the script
    let _ = Command::new("kill")
        .arg("-TERM")
        .arg(format!("-{process_group_id}"))
        .status()
        .await;
}

/// The error a run reports when its target lacks the chosen shell.
fn missing_shell(shell: &str, place: &str) -> String {
    format!("Shell '{shell}' not found in {place}")
}

/// crun (`executable file `x` not found`) and runc (`executable file not found`)
/// both word a missing container entrypoint this way.
fn is_missing_executable(error: &str) -> bool {
    error.contains("executable file") && error.contains("not found")
}

/// Turns a container start error into `missing_shell` when the shell was the
/// missing executable; any other error is passed through.
fn start_error(error: String, shell: &str, image: &str) -> String {
    if is_missing_executable(&error) {
        missing_shell(shell, &format!("image {image}"))
    } else {
        error
    }
}

/// A `litepod_api` spawn failure, naming the shell when it is not installed.
fn spawn_error(error: &std::io::Error, shell: &str) -> String {
    if error.kind() == std::io::ErrorKind::NotFound {
        missing_shell(shell, "the Litepod API host")
    } else {
        format!("Could not start litepod API task: {error}")
    }
}

/// Exit 127 is "command not found" for both a missing shell and a missing command
/// inside the script, so one `<shell> -c true` probe tells them apart. Probe failures
/// for any other reason also read as missing, which only costs the wording.
async fn shell_exists(
    runtime: &crate::servers::runtime::ServerRuntime,
    container: Option<&str>,
    shell: &str,
) -> bool {
    let probe: [OsString; 3] = [shell.into(), "-c".into(), "true".into()];
    match container {
        Some(container) => {
            let mut args: Vec<OsString> = vec!["exec".into(), container.into()];
            args.extend(probe);
            runtime.podman(&args).await.is_ok()
        }
        None => runtime.remote_command(&probe).await.is_ok(),
    }
}

fn normalize_image(reference: &str) -> String {
    let reference = reference.trim();
    let first = reference.split('/').next().unwrap_or_default();
    if reference.contains('/') {
        if first == "localhost" || first.contains('.') || first.contains(':') {
            reference.to_string()
        } else {
            format!("docker.io/{reference}")
        }
    } else {
        format!("docker.io/library/{reference}")
    }
}

async fn execute_litepod_api(
    state: SharedState,
    execution_id: String,
    shell: String,
    script: String,
    timeout_seconds: i64,
) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(&state).await;
    let now = chrono::Utc::now().timestamp_millis();
    sqlx::query("UPDATE scheduled_task_executions SET status = 'running', started_at = ?1 WHERE id = ?2 AND status = 'queued'").bind(now).bind(execution_id.clone()).execute(&conn).await.map_err(|error| AppError::DatabaseError(error.to_string()))?;
    let mut child = match Command::new(&shell)
        .arg("-c")
        .arg(script)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // own process group so a kill request can take down the whole script
        .process_group(0)
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            finish(
                &state,
                &execution_id,
                "failed",
                Some(spawn_error(&error, &shell)),
            )
            .await?;
            return Ok(());
        }
    };
    if let Some(pid) = child.id() {
        let _ =
            sqlx::query("UPDATE scheduled_task_executions SET process_group_id = ?1 WHERE id = ?2")
                .bind(pid as i64)
                .bind(execution_id.clone())
                .execute(&conn)
                .await;
    }
    let deadline =
        tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_seconds.max(1) as u64);
    // stream both pipes into the log table so the UI can follow the run live
    let streamed = tokio::time::timeout_at(
        deadline,
        pump_live_logs(
            &conn,
            &execution_id,
            child.stdout.take(),
            child.stderr.take(),
            false,
        ),
    )
    .await;
    if streamed.is_err() {
        let _ = child.start_kill();
        finish(
            &state,
            &execution_id,
            "failed",
            Some(format!("Execution timed out after {timeout_seconds}s")),
        )
        .await?;
        return Ok(());
    }
    let exit_code = match child.wait().await {
        Ok(status) => status.code().unwrap_or(1) as i64,
        Err(error) => {
            finish(
                &state,
                &execution_id,
                "failed",
                Some(format!("Could not wait for litepod API task: {error}")),
            )
            .await?;
            return Ok(());
        }
    };
    sqlx::query("UPDATE scheduled_task_executions SET exit_code = ?1 WHERE id = ?2")
        .bind(exit_code)
        .bind(execution_id.clone())
        .execute(&conn)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    if exit_code == 0 {
        finish(&state, &execution_id, "succeeded", None).await?;
    } else {
        finish(
            &state,
            &execution_id,
            "failed",
            Some(format!("Process exited with code {exit_code}")),
        )
        .await?;
    }
    Ok(())
}

/// How often a line still being written (no trailing newline yet) is saved, so a
/// `\r`-redrawn progress bar can be watched while it moves.
const LIVE_FLUSH_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);
/// A single line longer than this is cut, so one runaway line cannot bloat the table.
const MAX_LIVE_LINE_CHARS: usize = 16_384;

#[derive(Default)]
struct OpenLine {
    /// Row this line was saved under, once its partial text was flushed.
    sequence: Option<i64>,
    text: String,
    /// A trailing `\r` was seen: the next text overwrites from the line start.
    at_line_start: bool,
    /// Bytes of a UTF-8 character split across two chunks.
    pending: Vec<u8>,
    dirty: bool,
}

/// Writes a run's stdout/stderr into the log table as it arrives. Complete lines are
/// stored as before; the line still being written is upserted on each flush with `\r`
/// already applied, so the table holds what a terminal would show, not every frame.
struct LiveLog {
    conn: sqlx::SqlitePool,
    execution_id: String,
    next_sequence: i64,
    open: [OpenLine; 2],
}

impl LiveLog {
    const STREAMS: [&'static str; 2] = ["stdout", "stderr"];

    fn new(conn: sqlx::SqlitePool, execution_id: String) -> Self {
        Self {
            conn,
            execution_id,
            next_sequence: 0,
            open: Default::default(),
        }
    }

    async fn feed(&mut self, stream: usize, bytes: &[u8]) {
        let line = &mut self.open[stream];
        line.pending.extend_from_slice(bytes);
        let valid = match std::str::from_utf8(&line.pending) {
            Ok(text) => text.len(),
            // an incomplete character at the end waits for the next chunk
            Err(error) if error.error_len().is_none() => error.valid_up_to(),
            Err(_) => line.pending.len(),
        };
        let decoded = String::from_utf8_lossy(&line.pending[..valid]).into_owned();
        line.pending.drain(..valid);
        let mut segments = decoded.split('\n').peekable();
        while let Some(segment) = segments.next() {
            let line = &mut self.open[stream];
            apply_carriage_returns(&mut line.text, &mut line.at_line_start, segment);
            if segments.peek().is_some() {
                self.write(stream).await;
                self.open[stream] = OpenLine {
                    pending: std::mem::take(&mut self.open[stream].pending),
                    ..Default::default()
                };
            } else if !segment.is_empty() {
                self.open[stream].dirty = true;
                // viewers get the open line right away; the table catches up on the next flush
                if super::live::is_watched(&self.execution_id) {
                    self.publish(stream);
                }
            }
        }
    }

    /// Saves every line still being written that changed since the last flush.
    async fn flush(&mut self) {
        for stream in 0..2 {
            if self.open[stream].dirty {
                self.write(stream).await;
            }
        }
    }

    /// Row number of the stream's open line, claimed the first time it is needed.
    fn sequence(&mut self, stream: usize) -> i64 {
        let next = &mut self.next_sequence;
        *self.open[stream].sequence.get_or_insert_with(|| {
            *next += 1;
            *next - 1
        })
    }

    fn publish(&mut self, stream: usize) {
        let sequence = self.sequence(stream);
        super::live::publish(
            &self.execution_id,
            super::live::LiveEvent::Line {
                sequence,
                stream: Self::STREAMS[stream],
                message: self.open[stream].text.clone(),
            },
        );
    }

    async fn write(&mut self, stream: usize) {
        let sequence = self.sequence(stream);
        self.publish(stream);
        let line = &mut self.open[stream];
        line.dirty = false;
        let _ = sqlx::query(
            "INSERT INTO scheduled_task_execution_logs (execution_id, sequence, stream, message, created_at) VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT(execution_id, sequence) DO UPDATE SET message = excluded.message",
        )
        .bind(&self.execution_id)
        .bind(sequence)
        .bind(Self::STREAMS[stream])
        .bind(&line.text)
        .bind(chrono::Utc::now().timestamp_millis())
        .execute(&self.conn)
        .await;
    }
}

/// Appends `segment` to `text` the way a terminal would: after a `\r`, new text
/// overwrites the line from its start instead of being appended.
fn apply_carriage_returns(text: &mut String, at_line_start: &mut bool, segment: &str) {
    for (index, part) in segment.split('\r').enumerate() {
        if index > 0 {
            *at_line_start = true;
        }
        if part.is_empty() {
            continue;
        }
        if *at_line_start {
            let kept: String = text.chars().skip(part.chars().count()).collect();
            *text = format!("{part}{kept}");
            *at_line_start = false;
        } else {
            text.push_str(part);
        }
    }
    if text.chars().count() > MAX_LIVE_LINE_CHARS {
        *text = text.chars().take(MAX_LIVE_LINE_CHARS).collect();
    }
}

/// Output a slow log writer may hold in memory before further chunks are dropped.
const MAX_LOG_BACKLOG_BYTES: usize = 8 * 1024 * 1024;

/// Pumps both pipes of a run into the log table until they close.
///
/// Keeping the process healthy always wins over showing its output: the pipes are
/// drained by this loop without ever waiting on the database, which a separate writer
/// task handles. If that writer falls behind, chunks are dropped (and a note is
/// logged) instead of letting a full pipe block the script.
///
/// With `watch_status`, stops early (returning false) only once the execution is
/// positively known to be canceled or replaced; a failed status read keeps going.
async fn pump_live_logs(
    conn: &sqlx::SqlitePool,
    execution_id: &str,
    stdout: Option<impl tokio::io::AsyncRead + Unpin>,
    stderr: Option<impl tokio::io::AsyncRead + Unpin>,
    watch_status: bool,
) -> bool {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use tokio::io::AsyncReadExt;

    let backlog = Arc::new(AtomicUsize::new(0));
    let dropped = Arc::new(AtomicBool::new(false));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(usize, Vec<u8>)>();
    let writer = {
        let (backlog, dropped) = (backlog.clone(), dropped.clone());
        let mut log = LiveLog::new(conn.clone(), execution_id.to_owned());
        tokio::spawn(async move {
            let mut flush = tokio::time::interval(LIVE_FLUSH_INTERVAL);
            loop {
                tokio::select! {
                    chunk = rx.recv() => match chunk {
                        Some((stream, bytes)) => {
                            backlog.fetch_sub(bytes.len(), Ordering::Relaxed);
                            log.feed(stream, &bytes).await;
                        }
                        None => break,
                    },
                    _ = flush.tick() => log.flush().await,
                }
            }
            if dropped.load(Ordering::Relaxed) {
                log.feed(
                    1,
                    b"\n[litepod] some output was not saved because the log writer fell behind; the script kept running\n",
                )
                .await;
            }
            log.flush().await;
        })
    };

    let (mut stdout, mut stderr) = (stdout, stderr);
    let (mut out_buf, mut err_buf) = ([0_u8; 8192], [0_u8; 8192]);
    let mut status_check = tokio::time::interval(std::time::Duration::from_secs(2));
    let forward = |stream: usize, bytes: &[u8]| {
        if backlog.load(Ordering::Relaxed) + bytes.len() > MAX_LOG_BACKLOG_BYTES {
            dropped.store(true, Ordering::Relaxed);
            return;
        }
        backlog.fetch_add(bytes.len(), Ordering::Relaxed);
        // the writer only goes away when this function returns
        let _ = tx.send((stream, bytes.to_vec()));
    };
    let finished = loop {
        let (stdout_open, stderr_open) = (stdout.is_some(), stderr.is_some());
        if !stdout_open && !stderr_open {
            break true;
        }
        tokio::select! {
            read = async { stdout.as_mut().unwrap().read(&mut out_buf).await }, if stdout_open => match read {
                Ok(n) if n > 0 => forward(0, &out_buf[..n]),
                _ => stdout = None,
            },
            read = async { stderr.as_mut().unwrap().read(&mut err_buf).await }, if stderr_open => match read {
                Ok(n) if n > 0 => forward(1, &err_buf[..n]),
                _ => stderr = None,
            },
            _ = status_check.tick(), if watch_status => {
                let status: Option<String> = sqlx::query_scalar("SELECT status FROM scheduled_task_executions WHERE id = ?1")
                    .bind(execution_id)
                    .fetch_optional(conn)
                    .await
                    .ok()
                    .flatten();
                // only a definite terminal state stops following; an unreadable status does not
                if status.as_deref().is_some_and(|status| !matches!(status, "queued" | "running")) {
                    break false;
                }
            }
        }
    };
    drop(tx);
    // let the writer save the tail, but never hold the run hostage to a stuck database
    let _ = tokio::time::timeout(
        std::time::Duration::from_secs(LOG_DRAIN_BUDGET_SECS),
        writer,
    )
    .await;
    finished
}

async fn finish(
    state: &SharedState,
    execution_id: &str,
    status: &str,
    error: Option<String>,
) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let now = chrono::Utc::now().timestamp_millis();
    // a canceled or replaced execution already reached a terminal state: keep it
    let changed = sqlx::query("UPDATE scheduled_task_executions SET status = ?1, finished_at = ?2, error = ?3 WHERE id = ?4 AND status IN ('queued', 'running')")
        .bind(status)
        .bind(now)
        .bind(error.clone())
        .bind(execution_id)
        .execute(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?
        .rows_affected();
    if changed == 0 {
        return Ok(());
    }
    super::live::finished(execution_id, status);
    if let Some(message) = error.as_deref() {
        let _ = sqlx::query("INSERT INTO scheduled_task_execution_logs (execution_id, sequence, stream, message, created_at) SELECT ?1, COALESCE(MAX(sequence) + 1, 0), 'system', ?2, ?3 FROM scheduled_task_execution_logs WHERE execution_id = ?1")
            .bind(execution_id)
            .bind(message)
            .bind(now)
            .execute(&conn)
            .await;
    }
    sqlx::query("UPDATE scheduled_tasks SET active_execution_id = NULL, updated_at = ?1 WHERE active_execution_id = ?2")
        .bind(now)
        .bind(execution_id)
        .execute(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::pick_exec_container;
    use crate::handler::deployments::RuntimeLogTarget;

    fn target(container_name: &str, service: Option<&str>) -> RuntimeLogTarget {
        RuntimeLogTarget {
            container_name: container_name.into(),
            service: service.map(Into::into),
            status: "running".into(),
        }
    }

    #[test]
    fn single_container_app_needs_no_service() {
        let targets = [target("static-b81e40", None)];
        assert_eq!(
            pick_exec_container(&targets, None).unwrap(),
            "static-b81e40"
        );
    }

    #[test]
    fn compose_service_resolves_by_service_or_full_name() {
        let targets = [
            target("umami-x-umami-db", Some("umami-db")),
            target("umami-x-umami", Some("umami")),
        ];
        assert_eq!(
            pick_exec_container(&targets, Some("umami")).unwrap(),
            "umami-x-umami"
        );
        assert_eq!(
            pick_exec_container(&targets, Some("umami-x-umami-db")).unwrap(),
            "umami-x-umami-db"
        );
        assert!(
            pick_exec_container(&targets, Some("web"))
                .unwrap_err()
                .contains("umami-db")
        );
    }

    #[test]
    fn stopped_container_is_refused_not_started() {
        let mut stopped = target("a-web", Some("web"));
        stopped.status = "exited".into();
        assert!(
            pick_exec_container(&[stopped], Some("web"))
                .unwrap_err()
                .contains("is not running")
        );
    }

    #[test]
    fn multi_service_compose_app_requires_a_service() {
        let targets = [target("a-db", Some("db")), target("a-web", Some("web"))];
        assert!(
            pick_exec_container(&targets, None)
                .unwrap_err()
                .contains("db, web")
        );
    }

    fn api_params(script: &str, timeout_seconds: i64) -> super::TestParams {
        super::TestParams {
            runner: "litepod_api".into(),
            server_id: "local".into(),
            image: None,
            network_access: false,
            application_id: None,
            compose_service: None,
            shell: "sh".into(),
            script: script.into(),
            timeout_seconds,
        }
    }

    #[tokio::test]
    async fn test_stream_forwards_raw_chunks_before_finishing() {
        use futures_util::StreamExt;
        let (state, _dir) = crate::testing::test_state().await;
        let events: Vec<_> = super::test_stream(
            state,
            api_params(
                "printf 'a\\r'; sleep 0.2; printf 'b'; echo oops >&2; exit 3",
                10,
            ),
        )
        .collect()
        .await;
        assert!(matches!(
            events.first(),
            Some(super::TestEvent::Started { .. })
        ));
        let stdout: String = events
            .iter()
            .filter_map(|event| match event {
                super::TestEvent::Output {
                    stream: "stdout",
                    data,
                } => Some(data.as_str()),
                _ => None,
            })
            .collect();
        // the carriage return survives and arrives in its own chunk, ahead of the rest
        assert_eq!(stdout, "a\rb");
        assert!(matches!(&events[1], super::TestEvent::Output { data, .. } if data == "a\r"));
        assert!(matches!(
            events.last(),
            Some(super::TestEvent::Done {
                success: false,
                exit_code: Some(3),
                ..
            })
        ));
    }

    #[tokio::test]
    async fn test_stream_refuses_litepod_api_in_cloud_mode() {
        use futures_util::StreamExt;
        let (state, _dir) = crate::testing::test_state_for(true).await;
        let events: Vec<_> = super::test_stream(state, api_params("echo ran", 10))
            .collect()
            .await;
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            super::TestEvent::Done { success: false, error: Some(error), .. }
                if error.contains("not available in cloud mode")
        ));
    }

    /// Seeds a queued execution pinned to the control plane's own `local` server,
    /// as if it had slipped past the create/update/boot guards.
    async fn seed_local_execution(
        state: &crate::state::state::SharedState,
        runner: &str,
    ) -> String {
        let image = (runner == "ephemeral").then_some("alpine:3.20");
        sqlx::query("INSERT INTO scheduled_tasks (id, organization_id, server_id, name, cron_expression, runner, image, shell, script, timeout_seconds, created_by, created_at, updated_at) VALUES ('task-1', 'organization-a', 'local', 'task', '* * * * *', ?1, ?2, 'sh', 'echo ran', 10, 'editor', 1, 1)")
            .bind(runner)
            .bind(image)
            .execute(&state.db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO scheduled_task_executions (id, task_id, organization_id, server_id, trigger, runner, image, shell, script, timeout_seconds, status, queued_at) VALUES ('execution-00000001', 'task-1', 'organization-a', 'local', 'manual', ?1, ?2, 'sh', 'echo ran', 10, 'queued', 1)")
            .bind(runner)
            .bind(image)
            .execute(&state.db)
            .await
            .unwrap();
        "execution-00000001".into()
    }

    async fn execution_outcome(
        state: &crate::state::state::SharedState,
        id: &str,
    ) -> (String, Option<String>) {
        sqlx::query_as("SELECT status, error FROM scheduled_task_executions WHERE id = ?1")
            .bind(id)
            .fetch_one(&state.db)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn execute_refuses_local_runners_in_cloud_mode() {
        for (runner, needle) in [
            ("litepod_api", "not available in cloud mode"),
            ("ephemeral", "Cloud mode requires a ready remote server"),
        ] {
            let (state, _dir) = crate::testing::test_state_for(true).await;
            let id = seed_local_execution(&state, runner).await;
            super::execute(state.clone(), id.clone()).await.unwrap();
            let (status, error) = execution_outcome(&state, &id).await;
            assert_eq!(status, "failed", "{runner}");
            assert!(error.unwrap_or_default().contains(needle), "{runner}");
        }
    }

    #[tokio::test]
    async fn test_stream_times_out() {
        use futures_util::StreamExt;
        let (state, _dir) = crate::testing::test_state().await;
        let events: Vec<_> = super::test_stream(state, api_params("sleep 5", 1))
            .collect()
            .await;
        assert!(matches!(
            events.last(),
            Some(super::TestEvent::Done { success: false, exit_code: None, error: Some(error), .. })
                if error.contains("timed out")
        ));
    }

    #[test]
    fn carriage_returns_overwrite_the_line_like_a_terminal() {
        let (mut text, mut at_start) = (String::new(), false);
        super::apply_carriage_returns(&mut text, &mut at_start, "[#   ] 10%\r[##  ] 50%\r");
        assert_eq!(text, "[##  ] 50%");
        super::apply_carriage_returns(&mut text, &mut at_start, "[####] 100%");
        assert_eq!(text, "[####] 100%");
        super::apply_carriage_returns(&mut text, &mut at_start, "\rab");
        assert_eq!(text, "ab###] 100%");
    }

    async fn rows(conn: &sqlx::SqlitePool) -> Vec<(i64, String, String)> {
        sqlx::query_as(
            "SELECT sequence, stream, message FROM scheduled_task_execution_logs ORDER BY sequence",
        )
        .fetch_all(conn)
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn live_log_updates_the_open_line_in_place() {
        // the log table alone, without the execution row its foreign key points at
        // one connection: every in-memory connection would otherwise be its own database
        let conn = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("CREATE TABLE scheduled_task_execution_logs (execution_id TEXT NOT NULL, sequence INTEGER NOT NULL, stream TEXT NOT NULL, message TEXT NOT NULL, created_at INTEGER NOT NULL, PRIMARY KEY (execution_id, sequence))")
            .execute(&conn)
            .await
            .unwrap();
        let mut log = super::LiveLog::new(conn.clone(), "live".into());
        log.feed(0, b"first\n[#  ] 1").await;
        log.flush().await;
        assert_eq!(
            rows(&conn).await,
            vec![
                (0, "stdout".into(), "first".into()),
                (1, "stdout".into(), "[#  ] 1".into())
            ]
        );
        // a split UTF-8 character waits for its second byte
        log.feed(0, "%\r[###] 100% \u{a1}".as_bytes().split_last().unwrap().1)
            .await;
        log.feed(1, b"warn\n").await;
        log.feed(0, &[0xa1, b'\n']).await;
        log.flush().await;
        assert_eq!(
            rows(&conn).await,
            vec![
                (0, "stdout".into(), "first".into()),
                (1, "stdout".into(), "[###] 100% \u{a1}".into()),
                (2, "stderr".into(), "warn".into()),
            ]
        );
    }

    async fn memory_pool() -> sqlx::SqlitePool {
        sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap()
    }

    fn spawn_sh(script: &str) -> tokio::process::Child {
        tokio::process::Command::new("sh")
            .arg("-c")
            .arg(script)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap()
    }

    #[tokio::test]
    async fn pump_keeps_running_when_the_database_is_unusable() {
        // no tables at all: every status read and every log write fails
        let conn = memory_pool().await;
        let mut child = spawn_sh("echo a; sleep 2.5; echo b; exit 7");
        let finished = super::pump_live_logs(
            &conn,
            "exec",
            child.stdout.take(),
            child.stderr.take(),
            true,
        )
        .await;
        assert!(
            finished,
            "an unreadable status must not stop following the run"
        );
        assert_eq!(child.wait().await.unwrap().code(), Some(7));
    }

    #[tokio::test]
    async fn pump_drains_large_output_without_blocking_the_process() {
        let conn = memory_pool().await;
        // ~4 MiB, far beyond a pipe buffer, with the database failing every write
        let mut child = spawn_sh("head -c 4194304 /dev/zero | tr '\\0' 'x'; echo; echo done");
        let finished = tokio::time::timeout(
            std::time::Duration::from_secs(20),
            super::pump_live_logs(
                &conn,
                "exec",
                child.stdout.take(),
                child.stderr.take(),
                false,
            ),
        )
        .await
        .expect("the pump must keep draining the pipes");
        assert!(finished);
        assert!(child.wait().await.unwrap().success());
    }

    #[test]
    fn missing_shell_is_named_on_container_start() {
        // crun and runc wordings of a missing entrypoint
        for raw in [
            "crun: executable file `fish` not found in $PATH: No such file or directory",
            "exec: \"fish\": executable file not found in $PATH",
        ] {
            assert_eq!(
                super::start_error(raw.into(), "fish", "docker.io/library/bash:5"),
                "Shell 'fish' not found in image docker.io/library/bash:5"
            );
        }
        assert_eq!(
            super::start_error("port already allocated".into(), "fish", "x"),
            "port already allocated"
        );
    }

    #[tokio::test]
    async fn test_stream_names_a_missing_litepod_api_shell() {
        use futures_util::StreamExt;
        let (state, _dir) = crate::testing::test_state().await;
        let mut params = api_params("echo ran", 10);
        params.shell = "litepod-no-such-shell".into();
        let events: Vec<_> = super::test_stream(state, params).collect().await;
        assert!(matches!(
            events.last(),
            Some(super::TestEvent::Done { success: false, error: Some(error), .. })
                if error == "Shell 'litepod-no-such-shell' not found in the Litepod API host"
        ));
    }

    #[test]
    fn single_service_compose_app_is_unambiguous() {
        let targets = [target("gh-runner", Some("runner"))];
        assert_eq!(pick_exec_container(&targets, None).unwrap(), "gh-runner");
    }
}
