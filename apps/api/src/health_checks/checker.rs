use std::time::Duration;

use crate::{dtos::response::AppError, servers::runtime::ServerRuntime, state::state::SharedState};

use super::HealthCheckRow;

/// Container IP on whichever network it landed on. Read from the host (local
/// `podman inspect`, or the same command over SSH for a remote server) rather
/// than dialing a published host port: a local deploy is not necessarily
/// published to the host loopback at all, only remote deploys are (see
/// `deployment::compose_standalone::generate_remote_standalone_override`).
/// The container's bridge IP is reachable from its own host either way.
async fn container_ip(runtime: &ServerRuntime, container_name: &str) -> Result<String, AppError> {
    let output = runtime
        .podman(&[
            "inspect".into(),
            "--format".into(),
            "{{range .NetworkSettings.Networks}}{{.IPAddress}} {{end}}".into(),
            container_name.into(),
        ])
        .await?;
    output
        .stdout
        .split_whitespace()
        .find(|ip| !ip.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            AppError::ValidationError(format!(
                "Container '{container_name}' has no network address"
            ))
        })
}

/// Runs one probe and reports whether it passed. Never returns `Err` for a
/// failed check, only for a setup problem (can't find the container, no
/// network address); a failed HTTP/TCP probe is `Ok(false)` with a reason.
pub(super) async fn probe(
    state: &SharedState,
    row: &HealthCheckRow,
) -> Result<(bool, Option<String>), AppError> {
    let runtime = ServerRuntime::for_application(state, &row.application_id).await?;
    let runtime_name: String = sqlx::query_scalar(
        "SELECT container_name FROM application_deployments WHERE application_id = ?1",
    )
    .bind(&row.application_id)
    .fetch_one(&state.db)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    let port = match row.port {
        Some(port) => port,
        None => crate::deployment::app_container_port(state, &row.application_id).await?,
    };
    let ip = container_ip(&runtime, &runtime_name).await?;
    let timeout = Duration::from_secs(row.timeout_seconds.max(1) as u64);

    if runtime.is_remote() {
        remote_probe(
            &runtime,
            &row.protocol,
            &ip,
            port,
            &row.path,
            row.timeout_seconds,
        )
        .await
    } else {
        local_probe(&row.protocol, &ip, port, &row.path, timeout).await
    }
}

/// Local checks are dialed directly from this process: litepod-api runs on
/// the same host as the podman bridge, so the container's bridge IP is
/// routable without going through any host-published port.
async fn local_probe(
    protocol: &str,
    ip: &str,
    port: i64,
    path: &str,
    timeout: Duration,
) -> Result<(bool, Option<String>), AppError> {
    if protocol == "tcp" {
        return Ok(
            match tokio::time::timeout(timeout, tokio::net::TcpStream::connect((ip, port as u16)))
                .await
            {
                Ok(Ok(_)) => (true, None),
                Ok(Err(error)) => (false, Some(error.to_string())),
                Err(_) => (false, Some("Connection timed out".into())),
            },
        );
    }
    let url = format!("http://{ip}:{port}{path}");
    let client = match reqwest::Client::builder().timeout(timeout).build() {
        Ok(client) => client,
        Err(error) => return Ok((false, Some(error.to_string()))),
    };
    Ok(match client.get(&url).send().await {
        Ok(response) if response.status().is_success() || response.status().is_redirection() => {
            (true, None)
        }
        Ok(response) => (false, Some(format!("HTTP {}", response.status().as_u16()))),
        Err(error) => (false, Some(error.to_string())),
    })
}

/// Remote checks run `curl`/`nc` on the remote server's own shell over SSH
/// (same precedent as `ServerRuntime::wait_for_container_ready`), dialing the
/// container's bridge IP, which the remote host itself can reach.
async fn remote_probe(
    runtime: &ServerRuntime,
    protocol: &str,
    ip: &str,
    port: i64,
    path: &str,
    timeout_seconds: i64,
) -> Result<(bool, Option<String>), AppError> {
    let timeout = timeout_seconds.max(1).to_string();
    if protocol == "tcp" {
        return Ok(
            match runtime
                .remote_command(&[
                    "nc".into(),
                    "-z".into(),
                    "-w".into(),
                    timeout.into(),
                    ip.into(),
                    port.to_string().into(),
                ])
                .await
            {
                Ok(_) => (true, None),
                Err(error) => (false, Some(error.to_string())),
            },
        );
    }
    let url = format!("http://{ip}:{port}{path}");
    match runtime
        .remote_command(&[
            "curl".into(),
            "-sS".into(),
            "-o".into(),
            "/dev/null".into(),
            "-w".into(),
            "%{http_code}".into(),
            "--max-time".into(),
            timeout.into(),
            url.into(),
        ])
        .await
    {
        Ok(output) => {
            let code: u16 = output.stdout.trim().parse().unwrap_or(0);
            if (200..400).contains(&code) {
                Ok((true, None))
            } else {
                Ok((false, Some(format!("HTTP {code}"))))
            }
        }
        Err(error) => Ok((false, Some(error.to_string()))),
    }
}
