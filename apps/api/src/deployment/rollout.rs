//! Keep the serving release until its isolated replacement is ready.
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use serde_json::Value;
use sqlx::Row;

use crate::{servers::runtime::ServerRuntime, state::state::SharedState};

use super::{domains, runtime_endpoints::RuntimeEndpoint};

#[derive(Clone)]
pub struct Rollout {
    pub name: String,
    pub previous_name: String,
    pub previous_project: String,
    pub project: String,
    pub previous_exists: bool,
    previous_containers: Vec<Value>,
    previous_endpoints: Vec<RuntimeEndpoint>,
    compose: bool,
    keep_candidate: Arc<AtomicBool>,
}

pub struct CandidateCleanup {
    runtime: ServerRuntime,
    name: String,
    project: String,
    compose: bool,
    keep: Arc<AtomicBool>,
}

impl Drop for CandidateCleanup {
    fn drop(&mut self) {
        if self.keep.swap(true, Ordering::SeqCst) {
            return;
        }
        let runtime = self.runtime.clone();
        let name = self.name.clone();
        let project = self.project.clone();
        let compose = self.compose;
        tokio::spawn(async move {
            if compose {
                super::compose_standalone::teardown_project(&runtime, &project).await;
            } else {
                let _ = runtime
                    .podman(&[
                        "rm".into(),
                        "--force".into(),
                        "--ignore".into(),
                        name.into(),
                    ])
                    .await;
            }
        });
    }
}

pub fn compose_project(application_id: &str, runtime_name: &str) -> String {
    let suffix = application_id.rsplit('-').next().unwrap_or(application_id);
    let base = format!("app-{suffix}-compose");
    match runtime_name.rsplit_once("-release-") {
        Some((_, release))
            if release.len() == 12 && release.bytes().all(|c| c.is_ascii_hexdigit()) =>
        {
            format!("{base}-release-{release}")
        }
        _ => base,
    }
}

pub async fn active_runtime_name(state: &SharedState, app: &str) -> Result<String, String> {
    let stored: Option<(String, String)> = sqlx::query_as(
        "SELECT d.container_name, a.deploy_type FROM application_deployments d JOIN applications a ON a.id=d.application_id WHERE d.application_id = ?1",
    ).bind(app).fetch_optional(&state.db).await.map_err(|e| e.to_string())?;
    match stored.filter(|(name, kind)| {
        !name.is_empty() && !(kind == "compose" && name.ends_with("-compose"))
    }) {
        Some((name, _)) => Ok(name.strip_suffix("-public").unwrap_or(&name).to_owned()),
        None => super::identity(state, app)
            .await
            .map(|(name, _)| name)
            .map_err(|e| e.to_string()),
    }
}

pub async fn active_project(state: &SharedState, app: &str) -> Result<String, String> {
    Ok(compose_project(
        app,
        &active_runtime_name(state, app).await?,
    ))
}

async fn inspect(runtime: &ServerRuntime, names: &[String]) -> Result<Vec<Value>, String> {
    if names.is_empty() {
        return Ok(Vec::new());
    }
    let mut args = vec!["inspect".into()];
    args.extend(names.iter().map(Into::into));
    let output = runtime.podman(&args).await.map_err(|e| e.to_string())?;
    serde_json::from_str(&output.stdout).map_err(|e| format!("Invalid container inspection: {e}"))
}

impl Rollout {
    pub async fn prepare(
        state: &SharedState,
        app: &str,
        runtime: &ServerRuntime,
        compose: bool,
    ) -> Result<Self, String> {
        let previous_name = active_runtime_name(state, app).await?;
        let previous_project = compose_project(app, &previous_name);
        let (base, _) = super::identity(state, app)
            .await
            .map_err(|e| e.to_string())?;
        // Listing first distinguishes an absent container from a broken Podman/SSH connection.
        let mut args = vec![
            "ps".into(),
            "--all".into(),
            "--format".into(),
            "{{.Names}}".into(),
        ];
        if compose {
            args.extend([
                "--filter".into(),
                format!("label=io.podman.compose.project={previous_project}").into(),
            ]);
        }
        let output = runtime.podman(&args).await.map_err(|e| e.to_string())?;
        let names = output
            .stdout
            .lines()
            .map(str::trim)
            .filter(|name| !name.is_empty() && (compose || *name == previous_name))
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let previous_containers = inspect(runtime, &names).await?;
        let previous_exists = !previous_containers.is_empty();
        let name = if previous_exists {
            format!(
                "{base}-release-{}",
                &uuid::Uuid::now_v7().simple().to_string()[20..]
            )
        } else {
            previous_name.clone()
        };
        let project = compose_project(app, &name);
        let rows = sqlx::query("SELECT server_id, host_port, container_port FROM application_runtime_endpoints WHERE application_id = ?1")
            .bind(app).fetch_all(&state.db).await.map_err(|e| e.to_string())?;
        let mut previous_endpoints = Vec::new();
        for row in rows {
            previous_endpoints.push(RuntimeEndpoint {
                application_id: app.into(),
                server_id: row.try_get(0).map_err(|e| e.to_string())?,
                host_port: row.try_get::<i64, _>(1).map_err(|e| e.to_string())? as u16,
                container_port: row.try_get::<i64, _>(2).map_err(|e| e.to_string())? as u16,
            });
        }
        Ok(Self {
            name,
            previous_name,
            previous_project,
            project,
            previous_exists,
            previous_containers,
            previous_endpoints,
            compose,
            keep_candidate: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn cleanup_on_drop(&self, runtime: &ServerRuntime) -> CandidateCleanup {
        CandidateCleanup {
            runtime: runtime.clone(),
            name: self.name.clone(),
            project: self.project.clone(),
            compose: self.compose,
            keep: self.keep_candidate.clone(),
        }
    }

    pub async fn cleanup_candidate(&self, runtime: &ServerRuntime) {
        if self.keep_candidate.swap(true, Ordering::SeqCst) {
            return;
        }
        if self.compose {
            super::compose_standalone::teardown_project(runtime, &self.project).await;
        } else {
            let _ = runtime
                .podman(&[
                    "rm".into(),
                    "--force".into(),
                    "--ignore".into(),
                    self.name.clone().into(),
                ])
                .await;
        }
    }

    pub async fn check_replaceable(
        &self,
        _runtime: &ServerRuntime,
        managed_volumes: bool,
        fixed_ports: bool,
    ) -> Result<(), String> {
        if self.previous_exists
            && (managed_volumes
                || fixed_ports
                || self.previous_containers.iter().any(has_persistent_mounts))
        {
            return Err("Replacement refused: this application uses persistent mounts or fixed host ports. The existing release has been preserved. Remove the previous runtime explicitly before deploying this configuration.".into());
        }
        Ok(())
    }

    /// Wait first, then switch ingress, then commit the serving identity. Callers
    /// may only retire the previous runtime after this returns successfully.
    pub async fn promote(
        &self,
        state: &SharedState,
        app: &str,
        runtime: &ServerRuntime,
        deployment_name: &str,
        targets: &[(String, String, i64)],
        containers: &[String],
    ) -> Result<(), String> {
        super::history::step(state, "health_check").await;
        wait_running(runtime, containers).await?;
        let mut ports = HashMap::new();
        for (_, container, port) in targets {
            let port = u16::try_from(*port).map_err(|_| "Invalid application port".to_string())?;
            if runtime.is_remote() || state.dev_mode {
                let published = published_port(runtime, container, port).await?;
                if runtime.is_remote() {
                    let listening = with_wait_progress(
                        state,
                        port,
                        ServerRuntime::REMOTE_READY_BUDGET,
                        runtime.wait_for_container_ready(container, published),
                    )
                    .await
                    .map_err(|e| e.to_string())?;
                    if !listening {
                        warn_not_listening(state, port).await;
                    }
                    if ports
                        .insert(port, published)
                        .is_some_and(|previous| previous != published)
                    {
                        return Err("Two routed services use the same container port; the current endpoint model cannot safely promote them".into());
                    }
                } else if !with_wait_progress(
                    state,
                    port,
                    LISTEN_BUDGET,
                    wait_listening("127.0.0.1", published),
                )
                .await?
                {
                    warn_not_listening(state, port).await;
                }
            } else if !with_wait_progress(
                state,
                port,
                LISTEN_BUDGET,
                wait_listening(container, port),
            )
            .await?
            {
                warn_not_listening(state, port).await;
            }
        }
        // Cancellation may stop a build/readiness wait, but must not drop a
        // half-applied ingress switch. The short commit/rollback task completes
        // even when the queue worker is canceled.
        super::history::step(state, "route").await;
        self.keep_candidate.store(true, Ordering::SeqCst);
        let rollout = self.clone();
        let state = state.clone();
        let app = app.to_owned();
        let runtime = runtime.clone();
        let deployment_name = deployment_name.to_owned();
        let targets = targets.to_vec();
        tokio::spawn(async move {
            let result = rollout.switch(&state, &app, &runtime, &deployment_name, &targets, ports).await;
            if result.is_ok() && rollout.previous_exists {
                if rollout.compose {
                    super::compose_standalone::teardown_project(&runtime, &rollout.previous_project).await;
                } else if let Err(error) = runtime.podman(&["stop".into(), "--time".into(), "30".into(), rollout.previous_name.clone().into()]).await {
                    tracing::warn!(application_id = app, %error, "previous release could not be stopped after promotion");
                } else if let Err(error) = runtime.podman(&["rm".into(), "--ignore".into(), rollout.previous_name.clone().into()]).await {
                    tracing::warn!(application_id = app, %error, "previous release cleanup failed after promotion");
                }
            } else if result.as_ref().is_err_and(|error| !error.contains("ingress rollback failed")) {
                // The caller may have been canceled while the switch ran. A
                // guard owned here also cleans up a rolled-back candidate.
                rollout.keep_candidate.store(false, Ordering::SeqCst);
                rollout.cleanup_candidate(&runtime).await;
            }
            result
        }).await.map_err(|e| format!("Ingress promotion task failed: {e}"))?
    }

    async fn switch(
        &self,
        state: &SharedState,
        app: &str,
        runtime: &ServerRuntime,
        deployment_name: &str,
        targets: &[(String, String, i64)],
        ports: HashMap<u16, u16>,
    ) -> Result<(), String> {
        let targets = targets
            .iter()
            .filter(|(id, _, _)| !id.is_empty())
            .cloned()
            .collect::<Vec<_>>();
        let targets = targets.as_slice();
        // Prevent the relay supervisor from rebuilding an obsolete endpoint
        // halfway through a switch or its rollback.
        let _guard = crate::servers::ingress::lock_reconciliation().await;
        let old_routes = if runtime.is_remote() || targets.is_empty() {
            None
        } else {
            Some(snapshot_routes(state, targets).await?)
        };
        let result = async {
            if runtime.is_remote() {
                let server_id: String = sqlx::query_scalar("SELECT server_id FROM applications WHERE id = ?1")
                    .bind(app).fetch_one(&state.db).await.map_err(|e| e.to_string())?;
                let mut tx = state.db.begin().await.map_err(|e| e.to_string())?;
                for (container_port, host_port) in &ports {
                    sqlx::query("INSERT INTO application_runtime_endpoints (application_id, server_id, host_port, container_port, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?5) ON CONFLICT(application_id, server_id, container_port) DO UPDATE SET host_port = excluded.host_port, updated_at = excluded.updated_at")
                        .bind(app).bind(&server_id).bind(i64::from(*host_port)).bind(i64::from(*container_port)).bind(chrono::Utc::now().timestamp_millis())
                        .execute(&mut *tx).await.map_err(|e| e.to_string())?;
                }
                tx.commit().await.map_err(|e| e.to_string())?;
                crate::servers::ingress::sync_application(state, app).await.map_err(|e| e.to_string())?;
            } else if !targets.is_empty() {
                super::caddy_application_routes(state, app, targets).await.map_err(|e| e.to_string())?;
            }
            sqlx::query("UPDATE application_deployments SET container_name = ?1, status = 'running', config_stale = 0, last_error = NULL, deployed_at = ?2, updated_at = ?2 WHERE application_id = ?3")
                .bind(deployment_name).bind(chrono::Utc::now().timestamp_millis()).bind(app).execute(&state.db).await.map_err(|e| e.to_string())?;
            Ok::<(), String>(())
        }.await;
        if let Err(error) = result {
            let rollback = if runtime.is_remote() {
                async {
                    let mut tx = state.db.begin().await.map_err(|e| e.to_string())?;
                    sqlx::query("DELETE FROM application_runtime_endpoints WHERE application_id = ?1").bind(app).execute(&mut *tx).await.map_err(|e| e.to_string())?;
                    for endpoint in &self.previous_endpoints {
                        sqlx::query("INSERT INTO application_runtime_endpoints (application_id, server_id, host_port, container_port, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?5)")
                            .bind(app).bind(&endpoint.server_id).bind(i64::from(endpoint.host_port)).bind(i64::from(endpoint.container_port)).bind(chrono::Utc::now().timestamp_millis())
                            .execute(&mut *tx).await.map_err(|e| e.to_string())?;
                    }
                    tx.commit().await.map_err(|e| e.to_string())?;
                    crate::servers::ingress::sync_application(state, app).await.map_err(|e| e.to_string())
                }.await
            } else if targets.is_empty() {
                Ok(())
            } else {
                restore_routes(state, targets, old_routes.unwrap_or_default()).await
            };
            if let Err(rollback) = rollback {
                return Err(format!(
                    "{error}; ingress rollback failed: {rollback}. Previous release retained; repair ingress before retrying."
                ));
            }
            return Err(error);
        }
        Ok(())
    }
}

fn has_persistent_mounts(container: &Value) -> bool {
    container["Mounts"].as_array().is_some_and(|mounts| {
        mounts
            .iter()
            .any(|mount| matches!(mount["Type"].as_str(), Some("volume" | "bind")))
    })
}

pub fn validate_stateless_compose(content: &str) -> Result<(), String> {
    let document: serde_yaml::Value = serde_yaml::from_str(content).map_err(|e| e.to_string())?;
    let services = document["services"]
        .as_mapping()
        .ok_or("Compose services must be a mapping")?;
    for (name, config) in services {
        for key in [
            "volumes",
            "volumes_from",
            "ports",
            "container_name",
            "network_mode",
            "pid",
            "ipc",
            "devices",
        ] {
            if config.get(key).is_some_and(|value| {
                !value.is_null() && value.as_sequence().is_none_or(|values| !values.is_empty())
            }) {
                return Err(format!(
                    "Replacement refused: Compose service '{}' declares {key}; the existing release is preserved. Remove the previous runtime explicitly before deploying.",
                    name.as_str().unwrap_or("unknown")
                ));
            }
        }
    }
    if document["networks"].as_mapping().is_some_and(|networks| {
        networks.values().any(|config| {
            config.get("name").is_some()
                || config
                    .get("external")
                    .is_some_and(|v| v.as_bool() != Some(false))
        })
    }) {
        return Err("Replacement refused: Compose uses shared named/external networks. The existing release is preserved.".into());
    }
    Ok(())
}

/// Whether a running Compose app must be recreated in place instead of being
/// started beside its current release. Syntax errors still reach the caller.
pub fn requires_in_place_recreate(content: &str) -> Result<bool, String> {
    match validate_stateless_compose(content) {
        Ok(()) => Ok(false),
        Err(error) if error.starts_with("Replacement refused:") => Ok(true),
        Err(error) => Err(error),
    }
}

pub async fn published_port(runtime: &ServerRuntime, name: &str, port: u16) -> Result<u16, String> {
    let output = runtime
        .podman(&["port".into(), name.into(), format!("{port}/tcp").into()])
        .await
        .map_err(|e| e.to_string())?;
    output
        .stdout
        .lines()
        .find_map(|line| {
            line.trim()
                .rsplit_once(':')
                .and_then(|(_, port)| port.parse::<u16>().ok())
                .filter(|port| *port != 0)
        })
        .ok_or_else(|| format!("Candidate '{name}' has no published endpoint for port {port}"))
}

/// One-shot Compose services (init/migration jobs gated by
/// `service_completed_successfully`) finish with exit 0 and no restart policy.
/// They are done, not crashed, so readiness skips them.
fn completed_job(fact: &Value) -> bool {
    fact.pointer("/State/Status").and_then(Value::as_str) == Some("exited")
        && fact.pointer("/State/ExitCode").and_then(Value::as_i64) == Some(0)
        && matches!(
            fact.pointer("/HostConfig/RestartPolicy/Name")
                .and_then(Value::as_str),
            None | Some("" | "no")
        )
}

async fn wait_running(runtime: &ServerRuntime, names: &[String]) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    let mut consecutive = 0;
    loop {
        let facts = inspect(runtime, names).await?;
        let mut ready = facts.len() == names.len() && facts.iter().any(|fact| !completed_job(fact));
        for fact in &facts {
            let state = &fact["State"];
            if completed_job(fact) {
                continue;
            }
            if matches!(state["Status"].as_str(), Some("exited" | "dead")) {
                return Err(
                    "Candidate exited before it became ready; existing release retained".into(),
                );
            }
            let health = state
                .pointer("/Health/Status")
                .or_else(|| state.pointer("/Healthcheck/Status"))
                .and_then(Value::as_str);
            ready &= state["Running"].as_bool() == Some(true)
                && health.is_none_or(|status| status == "healthy" || status.is_empty());
        }
        consecutive = if ready { consecutive + 1 } else { 0 };
        if consecutive >= 2 {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("Candidate readiness timed out; existing release retained".into());
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// A running container that does not answer on its routed port is still
/// promoted: the port may simply not match what the app listens on, and the
/// user fixes that afterwards. The deploy log says so instead of failing.
async fn warn_not_listening(state: &SharedState, port: u16) {
    super::history::note(
        state,
        "info",
        &format!(
            "Warning: nothing answered on container port {port}; the release was routed anyway. Check that the application's port matches the one the app listens on."
        ),
    )
    .await;
}

/// Runs a readiness wait, logging how long it has waited every 15 seconds so a
/// slow-booting or mis-ported app does not look like a hung deploy.
async fn with_wait_progress<T>(
    state: &SharedState,
    port: u16,
    budget: Duration,
    wait: impl std::future::Future<Output = T>,
) -> T {
    const EVERY: Duration = Duration::from_secs(15);
    let mut ticker = tokio::time::interval_at(tokio::time::Instant::now() + EVERY, EVERY);
    let mut elapsed = 0;
    tokio::pin!(wait);
    loop {
        tokio::select! {
            result = &mut wait => return result,
            _ = ticker.tick() => {
                elapsed += EVERY.as_secs();
                let budget = budget.as_secs();
                super::history::note(
                    state,
                    "info",
                    &format!("Waiting for container port {port} to respond... {elapsed}s / {budget}s"),
                )
                .await;
            }
        }
    }
}

const LISTEN_BUDGET: Duration = Duration::from_secs(180);

async fn wait_listening(host: &str, port: u16) -> Result<bool, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .map_err(|e| e.to_string())?;
    let url = format!("http://{host}:{port}/");
    // First boots seed databases/run migrations before listening (Logto,
    // Keycloak, Chatwoot take 30-90s), so allow minutes, not seconds.
    // ponytail: fixed budget, make per-template if some app needs longer.
    let deadline = tokio::time::Instant::now() + LISTEN_BUDGET;
    while tokio::time::Instant::now() < deadline {
        // Match existing remote readiness: an HTTP response proves the listener
        // is accepting requests. Configurable readiness policy is separate work.
        if client.get(&url).send().await.is_ok() || tcp_listening(host, port).await {
            return Ok(true);
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    Ok(false)
}

/// Readiness for a port that does not speak HTTP (any-sync, databases, game
/// servers). A bare connect is not enough: a published-port forwarder accepts
/// and then closes when nothing listens behind it. A real service either sends
/// a banner or stays silent waiting for its own handshake.
async fn tcp_listening(host: &str, port: u16) -> bool {
    use tokio::io::AsyncReadExt;
    let connect = tokio::net::TcpStream::connect((host, port));
    let Ok(Ok(mut stream)) = tokio::time::timeout(Duration::from_secs(2), connect).await else {
        return false;
    };
    let mut byte = [0u8; 1];
    match tokio::time::timeout(Duration::from_secs(1), stream.read(&mut byte)).await {
        Err(_) => true,
        Ok(Ok(read)) => read > 0,
        Ok(Err(_)) => false,
    }
}

async fn snapshot_routes(
    state: &SharedState,
    targets: &[(String, String, i64)],
) -> Result<Vec<Value>, String> {
    if targets.is_empty() {
        return Ok(Vec::new());
    }
    let (client, base) = crate::deployment::caddy_admin::client(
        &state.caddy_admin_url,
        Some(Duration::from_secs(10)),
    )?;
    let response = client
        .get(format!("{base}/config/"))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?;
    let routes: Value = response.json().await.map_err(|e| e.to_string())?;
    Ok(routes
        .pointer("/apps/http/servers/srv0/routes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|route| {
            targets
                .iter()
                .any(|(id, _, _)| route["@id"] == domains::route_id(id))
        })
        .cloned()
        .collect())
}

async fn restore_routes(
    state: &SharedState,
    targets: &[(String, String, i64)],
    previous: Vec<Value>,
) -> Result<(), String> {
    if targets.is_empty() {
        return Ok(());
    }
    super::edit_caddy_config(state, |config| {
        if let Some(routes) = config
            .pointer_mut("/apps/http/servers/srv0/routes")
            .and_then(Value::as_array_mut)
        {
            routes.retain(|route| {
                !targets
                    .iter()
                    .any(|(id, _, _)| route["@id"] == domains::route_id(id))
            });
            // Managed routes must precede the panel's catch-all.
            for route in previous.into_iter().rev() {
                routes.insert(0, route);
            }
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())
}

pub async fn record_failure(state: &SharedState, app: &str, error: &str) {
    // A failed build/candidate must not erase the live release's identity.
    let alive = async {
        let runtime = ServerRuntime::for_application(state, app).await.ok()?;
        let name: String = sqlx::query_scalar(
            "SELECT container_name FROM application_deployments WHERE application_id = ?1",
        )
        .bind(app)
        .fetch_optional(&state.db)
        .await
        .ok()??;
        let output = runtime
            .podman(&["ps".into(), "--format".into(), "{{.Names}}".into()])
            .await
            .ok()?;
        Some(
            output
                .stdout
                .lines()
                .any(|candidate| candidate == name || candidate.starts_with(&format!("{name}-"))),
        )
    }
    .await
    .unwrap_or(false);
    let _ = sqlx::query("UPDATE application_deployments SET status = ?1, last_error = ?2, updated_at = ?3 WHERE application_id = ?4")
        .bind(if alive { "running" } else { "failed" }).bind(error).bind(chrono::Utc::now().timestamp_millis()).bind(app).execute(&state.db).await;
}

/// Shared by Dockerfile and Railpack after their image has been built/loaded.
pub async fn deploy_image(
    state: &SharedState,
    app: &str,
    source: &crate::dtos::deployment::SourceConfigDto,
    image: &str,
    runtime: &ServerRuntime,
) -> Result<(), String> {
    use podman_api::{Podman, models};
    super::history::step(state, "start").await;
    let rollout = Rollout::prepare(state, app, runtime, false).await?;
    let mounts = super::managed_volumes::runtime_mounts(state, app)
        .await
        .map_err(|e| e.to_string())?;
    domains::seed_if_missing(state, app)
        .await
        .map_err(|e| e.to_string())?;
    let mappings = domains::port_mappings(state, app)
        .await
        .map_err(|e| e.to_string())?;
    rollout
        .check_replaceable(
            runtime,
            !mounts.is_empty(),
            !runtime.is_remote() && !mappings.is_empty(),
        )
        .await?;
    let _cleanup = rollout.cleanup_on_drop(runtime);
    let port = u16::try_from(
        source
            .container_port
            .ok_or("Application container port is missing")?,
    )
    .map_err(|_| "Invalid application container port".to_string())?;
    if port == 0 {
        return Err("Application port must be nonzero".into());
    }
    let environment = super::decrypted_environment(state, app, "runtime")
        .await
        .map_err(|e| e.to_string())?;
    let networks = super::resource_networks::ensure_for_deploy(state, runtime, app)
        .await
        .map_err(|e| e.to_string())?;
    let mut targets = domains::list(state, app)
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|domain| !domain.is_local)
        .map(|domain| (domain.id, rollout.name.clone(), i64::from(port)))
        .collect::<Vec<_>>();
    if targets.is_empty() {
        targets.push((String::new(), rollout.name.clone(), i64::from(port)));
    }
    let result = async {
        if runtime.is_remote() {
            if !mounts.is_empty() {
                return Err(
                    "Managed volumes are not supported by remote Git image deployments".into(),
                );
            }
            let contents = environment
                .iter()
                .map(|(key, value)| {
                    if value.contains(['\n', '\r']) {
                        Err(format!("Runtime variable {key} contains a newline"))
                    } else {
                        Ok(format!("{key}={value}\n"))
                    }
                })
                .collect::<Result<String, String>>()?;
            let env =
                crate::servers::ssh::SecretFile::write(&contents).map_err(|e| e.to_string())?;
            let path = runtime
                .upload_workspace_file(app, env.path(), "release.runtime.env")
                .await
                .map_err(|e| e.to_string())?
                .ok_or("Remote runtime environment was not uploaded")?;
            let mut args = vec![
                "run".into(),
                "--detach".into(),
                "--name".into(),
                rollout.name.clone().into(),
                "--restart".into(),
                source.restart_policy.clone().into(),
                "--env-file".into(),
                path.into(),
                "--network".into(),
                state.podman_network.clone().into(),
                "--publish".into(),
                format!("127.0.0.1::{port}").into(),
            ];
            if let Some(limits) = super::resource_limits(source)? {
                args.extend([
                    "--cpu-period".into(),
                    super::BUILD_CPU_PERIOD_US.to_string().into(),
                    "--cpu-quota".into(),
                    limits.cpu_quota_us.to_string().into(),
                    "--memory".into(),
                    limits.memory_bytes.to_string().into(),
                    "--memory-swap".into(),
                    limits.memswap_bytes.to_string().into(),
                ]);
            }
            for network in networks {
                args.extend(["--network".into(), network.into()]);
            }
            args.push(image.into());
            runtime.podman(&args).await.map_err(|e| e.to_string())?;
        } else {
            let podman = Podman::new(&state.podman_socket).map_err(|e| e.to_string())?;
            let mut network_options = HashMap::new();
            for network in std::iter::once(state.podman_network.clone()).chain(networks) {
                network_options.insert(
                    network,
                    models::PerNetworkOptions {
                        aliases: Some(vec![rollout.name.clone()]),
                        interface_name: None,
                        static_ips: None,
                        static_mac: None,
                    },
                );
            }
            let mut builder = super::managed_container_builder(
                &rollout.name,
                image.to_owned(),
                &source.restart_policy,
            )
            .env(environment)
            .net_namespace(models::Namespace {
                nsmode: Some("bridge".into()),
                value: None,
            })
            .networks(network_options);
            if !mounts.is_empty() {
                builder = builder.volumes(mounts);
            }
            let mut mappings = mappings;
            if state.dev_mode
                && !mappings
                    .iter()
                    .any(|mapping| mapping.container_port == Some(port))
            {
                mappings.push(models::PortMapping {
                    container_port: Some(port),
                    host_ip: Some("127.0.0.1".into()),
                    host_port: Some(0),
                    protocol: Some("tcp".into()),
                    range: None,
                });
            }
            if !mappings.is_empty() {
                builder = builder.portmappings(mappings);
            }
            builder = super::with_resource_limits(builder, source)?;
            podman
                .containers()
                .create(&builder.build())
                .await
                .map_err(|e| format!("Could not create candidate: {e}"))?;
            podman
                .containers()
                .get(&rollout.name)
                .start(None)
                .await
                .map_err(|e| format!("Could not start candidate: {e}"))?;
        }
        rollout
            .promote(
                state,
                app,
                runtime,
                &rollout.name,
                &targets,
                &[rollout.name.clone()],
            )
            .await
    }
    .await;
    if let Err(error) = result {
        // A failed rollback may still have routes pointing at the candidate.
        // Keep both processes for repair rather than turn that into an outage.
        if !error.contains("ingress rollback failed") {
            rollout.cleanup_candidate(runtime).await;
        }
        return Err(error);
    }
    if runtime.is_remote() {
        if let Err(error) =
            super::runtime_endpoints::remove_stale_for_application(state, app, port).await
        {
            tracing::warn!(application_id = app, %error, "obsolete endpoint cleanup failed after promotion");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    #[tokio::test]
    async fn tcp_listening_accepts_silent_or_banner_and_rejects_closed() {
        use tokio::io::AsyncWriteExt;
        let silent = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let silent_port = silent.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (_held, _) = silent.accept().await.unwrap();
            tokio::time::sleep(Duration::from_secs(5)).await;
        });
        assert!(tcp_listening("127.0.0.1", silent_port).await);

        let banner = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let banner_port = banner.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut stream, _) = banner.accept().await.unwrap();
            stream.write_all(b"SSH-2.0\r\n").await.unwrap();
            tokio::time::sleep(Duration::from_secs(5)).await;
        });
        assert!(tcp_listening("127.0.0.1", banner_port).await);

        // A forwarder with nothing behind it: accepts, then hangs up.
        let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let closed_port = closed.local_addr().unwrap().port();
        tokio::spawn(async move {
            drop(closed.accept().await.unwrap());
        });
        assert!(!tcp_listening("127.0.0.1", closed_port).await);
    }

    #[test]
    fn stateless_replacement_rejects_shared_storage_ports_and_identity() {
        assert!(validate_stateless_compose("services:\n  api:\n    image: api\n").is_ok());
        for declaration in [
            "volumes: [data:/data]",
            "ports: ['8080:80']",
            "container_name: fixed",
            "network_mode: host",
        ] {
            assert!(
                validate_stateless_compose(&format!(
                    "services:\n  api:\n    image: api\n    {declaration}\n"
                ))
                .is_err()
            );
        }
        assert!(
            validate_stateless_compose(
                "services:\n  api: {}\nnetworks:\n  shared:\n    external: true\n"
            )
            .is_err()
        );
    }

    #[test]
    fn stateful_compose_uses_an_in_place_recreate_but_invalid_yaml_is_rejected() {
        assert!(!requires_in_place_recreate("services:\n  api:\n    image: api\n").unwrap());
        assert!(
            requires_in_place_recreate(
                "services:\n  api:\n    image: api\n    volumes: [data:/data]\n"
            )
            .unwrap()
        );
        assert!(requires_in_place_recreate("services: [").is_err());
    }

    #[test]
    fn only_clean_exits_without_restart_policy_count_as_completed_jobs() {
        let fact = |status: &str, code: i64, restart: &str| {
            serde_json::json!({
                "State": {"Status": status, "ExitCode": code},
                "HostConfig": {"RestartPolicy": {"Name": restart}}
            })
        };
        assert!(completed_job(&fact("exited", 0, "no")));
        assert!(completed_job(&fact("exited", 0, "")));
        assert!(!completed_job(&fact("exited", 1, "no")));
        assert!(!completed_job(&fact("exited", 0, "unless-stopped")));
        assert!(!completed_job(&fact("running", 0, "no")));
    }
    #[test]
    fn release_project_is_stable_for_lifecycle_and_keeps_legacy_names() {
        assert_eq!(compose_project("app-123", "api-123"), "app-123-compose");
        assert_eq!(
            compose_project("app-123", "api-123-release-abcdef012345"),
            "app-123-compose-release-abcdef012345"
        );
        assert!(!has_persistent_mounts(
            &serde_json::json!({"Mounts": [{"Type":"tmpfs"}]})
        ));
        assert!(has_persistent_mounts(
            &serde_json::json!({"Mounts": [{"Type":"volume"}]})
        ));
    }

    #[tokio::test]
    async fn promotion_preserves_old_routes_on_proxy_or_database_failure() {
        use axum::{
            Json, Router,
            extract::State,
            http::StatusCode,
            routing::{get, post},
        };
        use serde_json::json;
        type Proxy = (Arc<tokio::sync::Mutex<Value>>, Arc<AtomicBool>);
        let old_route = json!({"@id":"litepod-route-domain", "match":[{"host":["api.example.com"]}],
            "handle":[{"handler":"reverse_proxy","upstreams":[{"dial":"old:3000"}]}]});
        let live = Arc::new(tokio::sync::Mutex::new(
            json!({"apps":{"http":{"servers":{"srv0":{
                "listen":[":80"], "routes":[old_route.clone()]
            }}}}}),
        ));
        let reject = Arc::new(AtomicBool::new(true));
        let proxy = Router::new()
            .route(
                "/config/",
                get(
                    |State((live, _)): State<Proxy>| async move { Json(live.lock().await.clone()) },
                ),
            )
            .route(
                "/load",
                post(
                    |State((live, reject)): State<Proxy>, Json(config): Json<Value>| async move {
                        if reject.swap(false, Ordering::SeqCst) {
                            return StatusCode::SERVICE_UNAVAILABLE;
                        }
                        *live.lock().await = config;
                        StatusCode::OK
                    },
                ),
            )
            .with_state((live.clone(), reject));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, proxy).await.unwrap();
        });
        let (mut state, path) = crate::testing::test_state().await;
        Arc::get_mut(&mut state).unwrap().caddy_admin_url = url;
        let (_, _, app) = crate::testing::seed_application(&state, "organization-a").await;
        super::super::ensure_deployment(&state, &app).await.unwrap();
        sqlx::query("UPDATE application_deployments SET container_name='old', status='running', deployed_at=1 WHERE application_id=?1")
            .bind(&app).execute(&state.db).await.unwrap();
        sqlx::query("INSERT INTO application_domains (id, application_id, host, port, https, is_primary, created_at, updated_at) VALUES ('domain', ?1, 'api.example.com', 3000, 0, 1, 1, 1)")
            .bind(&app).execute(&state.db).await.unwrap();
        let rollout = Rollout {
            name: "candidate".into(),
            previous_name: "old".into(),
            project: "candidate-project".into(),
            previous_project: "old-project".into(),
            previous_exists: true,
            previous_containers: vec![],
            previous_endpoints: vec![],
            compose: false,
            keep_candidate: Arc::new(AtomicBool::new(false)),
        };
        let runtime = ServerRuntime::Local {
            podman_socket: String::new(),
        };
        let targets = vec![("domain".into(), "candidate".into(), 3000)];
        assert!(
            rollout
                .switch(
                    &state,
                    &app,
                    &runtime,
                    "candidate",
                    &targets,
                    HashMap::new()
                )
                .await
                .is_err()
        );
        assert_eq!(
            live.lock().await["apps"]["http"]["servers"]["srv0"]["routes"][0],
            old_route
        );

        sqlx::query("CREATE TRIGGER reject_promotion BEFORE UPDATE OF container_name ON application_deployments WHEN NEW.container_name='candidate' BEGIN SELECT RAISE(ABORT, 'test database failure'); END")
            .execute(&state.db).await.unwrap();
        let error = rollout
            .switch(
                &state,
                &app,
                &runtime,
                "candidate",
                &targets,
                HashMap::new(),
            )
            .await
            .unwrap_err();
        assert!(error.contains("test database failure"));
        assert_eq!(
            live.lock().await["apps"]["http"]["servers"]["srv0"]["routes"][0],
            old_route
        );
        let name: String = sqlx::query_scalar(
            "SELECT container_name FROM application_deployments WHERE application_id=?1",
        )
        .bind(&app)
        .fetch_one(&state.db)
        .await
        .unwrap();
        assert_eq!(name, "old");

        sqlx::query("DROP TRIGGER reject_promotion")
            .execute(&state.db)
            .await
            .unwrap();
        rollout
            .switch(
                &state,
                &app,
                &runtime,
                "candidate",
                &targets,
                HashMap::new(),
            )
            .await
            .unwrap();
        assert_eq!(
            live.lock().await["apps"]["http"]["servers"]["srv0"]["routes"][0]["handle"][0]["upstreams"]
                [0]["dial"],
            "candidate:3000"
        );
        let name: String = sqlx::query_scalar(
            "SELECT container_name FROM application_deployments WHERE application_id=?1",
        )
        .bind(&app)
        .fetch_one(&state.db)
        .await
        .unwrap();
        assert_eq!(name, "candidate");
        server.abort();
        state.db.close().await;
        crate::testing::cleanup_database(&path);
    }
}
