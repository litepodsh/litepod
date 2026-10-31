//! Keeps two facts honest that nothing else re-derives after a restart: whether a
//! server is reachable, and whether an application's container is actually running.
//!
//! Both come from the same single SSH round trip per server (`podman ps --all
//! --format json`), so the cost is one remote call per server per tick, not one per
//! application. Without this, `application_deployments.status` is written only by the
//! deploy pipeline: a container that died out-of-band stays `running` forever, and a
//! server briefly unreachable while a cloud VM reboots stays `degraded` until someone
//! clicks Health check - which blocks every live read, because `ServerRuntime` refuses
//! any server that is not `Ready`.

use std::{collections::HashMap, ffi::OsString, sync::Arc, time::Duration};

use chrono::Utc;

use crate::{
    deployment::compose_standalone::json_string,
    dtos::response::AppError,
    servers::{
        model::ServerState,
        repository::{self, ServerRecord},
        runtime::ServerRuntime,
        setup, ssh,
    },
    state::state::SharedState,
};

/// One remote round trip per server. Short enough that a page opened after a restart
/// sees the truth, long enough that a fleet is not hammered over SSH.
const TICK: Duration = Duration::from_secs(45);
/// This is a probe, not a build step.
const PROBE_TIMEOUT: Duration = Duration::from_secs(20);
/// One dropped SSH session must not take a healthy server out of service.
const FAILURES_BEFORE_DEMOTE: u32 = 3;
/// Consecutive failures back the probe off so an unreachable host - a rebooting cloud
/// VM - is not dialled every 45s for hours.
const MAX_BACKOFF: Duration = Duration::from_secs(600);
/// Boot must not stall behind a fleet of unreachable hosts.
const BOOT_BUDGET: Duration = Duration::from_secs(90);
const BOOT_CONCURRENCY: usize = 8;

/// At boot no deploy pipeline can exist in this process, so a row left at `deploying`
/// by the previous process is fair game. While running, that row belongs to a live
/// pipeline and must be left alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Boot,
    Periodic,
}

impl Scope {
    const fn claims_deploying(self) -> i64 {
        match self {
            Self::Boot => 1,
            Self::Periodic => 0,
        }
    }
}

/// One entry of `podman ps --all --format json`.
#[derive(Debug, Clone)]
pub struct ContainerFact {
    pub name: String,
    pub state: String,
    pub exit_code: Option<i64>,
}

impl ContainerFact {
    pub(crate) fn is_running(&self) -> bool {
        let state = self.state.to_ascii_lowercase();
        state.starts_with("running") || state.starts_with("up")
    }
}

/// What to do with `last_error` alongside a status write.
enum ErrorWrite {
    /// The container is up; whatever failed before no longer describes it.
    Clear,
    Set(String),
    /// A clean stop is not a reason to erase the error that explains the last failure.
    Keep,
}

#[derive(Debug, Default, Clone, Copy)]
struct ProbeHistory {
    consecutive_failures: u32,
    skip_ticks: u32,
}

pub fn spawn(state: SharedState) {
    tokio::spawn(async move {
        let mut history: HashMap<String, ProbeHistory> = HashMap::new();
        let mut interval = tokio::time::interval(TICK);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // The first tick fires immediately and `reconcile_at_boot` already ran.
        interval.tick().await;
        loop {
            interval.tick().await;
            if let Err(error) = tick(&state, &mut history).await {
                tracing::warn!(%error, "server monitor tick failed");
            }
        }
    });
}

async fn tick(
    state: &SharedState,
    history: &mut HashMap<String, ProbeHistory>,
) -> Result<(), AppError> {
    let servers = repository::list_monitorable(state).await?;
    history.retain(|id, _| servers.iter().any(|server| &server.id == id));
    for server in servers {
        let entry = history.entry(server.id.clone()).or_default();
        if entry.skip_ticks > 0 {
            entry.skip_ticks -= 1;
            continue;
        }
        let failures = entry.consecutive_failures;
        let reachable = check_server(state, &server, Scope::Periodic, failures).await;
        let entry = history.entry(server.id.clone()).or_default();
        if reachable {
            *entry = ProbeHistory::default();
        } else {
            entry.consecutive_failures = entry.consecutive_failures.saturating_add(1);
            let ceiling = (MAX_BACKOFF.as_secs() / TICK.as_secs()) as u32;
            entry.skip_ticks = 1u32
                .checked_shl(entry.consecutive_failures - 1)
                .unwrap_or(ceiling)
                .min(ceiling);
        }
    }
    Ok(())
}

/// Probes every server before any other boot bookkeeping runs, so an application whose
/// container survived the restart is never mistaken for an interrupted deployment.
pub async fn reconcile_at_boot(state: &SharedState) -> Result<(), AppError> {
    let servers = repository::list_monitorable(state).await?;
    let total = servers.len();
    if total == 0 {
        return Ok(());
    }
    // The servers are independent and one unreachable host would otherwise serialize
    // the whole boot behind its SSH timeout.
    let permits = Arc::new(tokio::sync::Semaphore::new(BOOT_CONCURRENCY));
    let mut tasks = tokio::task::JoinSet::new();
    for server in servers {
        let state = state.clone();
        let permits = permits.clone();
        tasks.spawn(async move {
            let _permit = permits.acquire().await;
            // A boot probe never demotes: a remote VM rebooting alongside litepod is
            // the expected case, and the periodic tick decides that after 3 failures.
            check_server(&state, &server, Scope::Boot, 0).await
        });
    }
    let drain = async {
        let mut reachable = 0usize;
        while let Some(joined) = tasks.join_next().await {
            if joined.unwrap_or(false) {
                reachable += 1;
            }
        }
        reachable
    };
    match tokio::time::timeout(BOOT_BUDGET, drain).await {
        Ok(reachable) => {
            tracing::info!(
                servers = total,
                reachable,
                "reconciled container state at boot"
            )
        }
        Err(_) => tracing::warn!(
            servers = total,
            "boot container reconcile timed out; the monitor will finish it on the next tick"
        ),
    }
    Ok(())
}

/// Probes one server, records what that says about its state, and corrects the
/// applications placed on it. Returns whether the probe reached the host.
async fn check_server(
    state: &SharedState,
    server: &ServerRecord,
    scope: Scope,
    consecutive_failures: u32,
) -> bool {
    match probe(state, server).await {
        Ok(facts) => {
            if let Err(error) = apply_probe_success(state, server).await {
                tracing::warn!(server = %server.name, %error, "could not record a successful server probe");
            }
            match reconcile_containers(state, &server.id, &facts, scope).await {
                Ok(corrected) if corrected > 0 => {
                    tracing::info!(server = %server.name, corrected, "corrected container status from Podman")
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(server = %server.name, %error, "could not reconcile container status")
                }
            }
            // Spawned: a restore can take minutes and must not hold the boot budget or
            // the periodic tick.
            tokio::spawn(super::boot_restore::on_probe(
                state.clone(),
                server.clone(),
                facts,
            ));
            true
        }
        Err(error) => {
            apply_probe_failure(state, server, &error, consecutive_failures + 1).await;
            false
        }
    }
}

async fn probe(state: &SharedState, server: &ServerRecord) -> Result<Vec<ContainerFact>, AppError> {
    let runtime = ServerRuntime::from_server_unchecked(state, server.clone()).await?;
    let output = runtime
        .podman_with_timeout(
            &[
                OsString::from("ps"),
                OsString::from("--all"),
                OsString::from("--format"),
                OsString::from("json"),
            ],
            PROBE_TIMEOUT,
        )
        .await?;
    Ok(parse_container_facts(output.stdout.as_bytes()))
}

async fn apply_probe_success(state: &SharedState, server: &ServerRecord) -> Result<(), AppError> {
    if server.kind != "ssh" {
        return Ok(());
    }
    // Re-read: the row may have moved while the probe was in flight, and a Setup or
    // Repair job owns `setting_up` outright.
    let Some(current) = repository::get(state, &server.id).await? else {
        return Ok(());
    };
    if repository::has_active_setup_job(state, &server.id).await? {
        return Ok(());
    }
    if current.state == ServerState::Ready {
        refresh_helper_version(state, &current).await;
        return repository::mark_checked(state, &server.id).await;
    }
    if !matches!(current.state, ServerState::Degraded | ServerState::Offline) {
        return Ok(());
    }
    // `ps` answering only proves the host is reachable. A server demoted mid-setup may
    // still be genuinely half-configured, so promotion runs the full check - once per
    // recovery, since it is a transition and not a per-tick cost.
    match setup::health_check(&current).await {
        Ok((facts, next)) => {
            let facts_json = serde_json::to_string(&facts)
                .map_err(|error| AppError::InternalError(error.to_string()))?;
            repository::set_facts_and_state(state, &current.id, &facts_json, next).await?;
            tracing::info!(server = %current.name, state = next.as_str(), "server recovered");
            Ok(())
        }
        Err(error) => {
            let message = ssh::redact(&error.to_string());
            tracing::warn!(server = %current.name, error = %message, "server answered but is still not healthy");
            repository::set_state_error(
                state,
                &current.id,
                current.state,
                "health_check_failed",
                &message,
            )
            .await
        }
    }
}

/// Keeps the stored `serverctl` version current without a full health check. Only a server
/// that is not known to be on the latest script is asked, so an up-to-date fleet costs nothing
/// and an outdated one notices the update within a tick.
async fn refresh_helper_version(state: &SharedState, server: &ServerRecord) {
    if server.kind != "ssh" || !matches!(server.ingress_mode, super::model::IngressMode::PublicEdge)
    {
        return;
    }
    let mut facts =
        serde_json::from_str::<setup::ServerFacts>(&server.facts_json).unwrap_or_default();
    if facts.helper_version == Some(super::helper::HELPER_VERSION) {
        return;
    }
    let Some(version) = super::helper::installed_version(server).await else {
        return;
    };
    if facts.helper_version == Some(version) {
        return;
    }
    facts.helper_version = Some(version);
    let Ok(facts_json) = serde_json::to_string(&facts) else {
        return;
    };
    if let Err(error) =
        repository::set_facts_and_state(state, &server.id, &facts_json, server.state).await
    {
        tracing::warn!(server = %server.name, %error, "could not record the helper version");
    }
}

async fn apply_probe_failure(
    state: &SharedState,
    server: &ServerRecord,
    error: &AppError,
    failures: u32,
) {
    let message = ssh::redact(&error.to_string());
    if server.kind != "ssh" || server.state != ServerState::Ready {
        tracing::warn!(server = %server.name, failures, error = %message, "server probe failed");
        return;
    }
    if failures < FAILURES_BEFORE_DEMOTE {
        tracing::warn!(server = %server.name, failures, error = %message, "server probe failed");
        return;
    }
    match repository::has_active_setup_job(state, &server.id).await {
        Ok(true) | Err(_) => return,
        Ok(false) => {}
    }
    if !matches!(repository::get(state, &server.id).await, Ok(Some(ref current)) if current.state == ServerState::Ready)
    {
        return;
    }
    tracing::warn!(server = %server.name, failures, error = %message, "server is unreachable; marking it degraded");
    if let Err(update_error) = repository::set_state_error(
        state,
        &server.id,
        ServerState::Degraded,
        "probe_failed",
        &message,
    )
    .await
    {
        tracing::warn!(server = %server.name, error = %update_error, "could not record the failed server probe");
    }
}

/// Rewrites `application_deployments.status` for the applications on this server to
/// match what Podman actually reports.
pub(crate) async fn reconcile_containers(
    state: &SharedState,
    server_id: &str,
    facts: &[ContainerFact],
    scope: Scope,
) -> Result<usize, AppError> {
    use sqlx::Row;
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(
        // A deploy in flight owns the row; only a boot pass, where no pipeline of
        // this process can exist, may touch a `deploying` one.
        "SELECT a.id, a.deploy_type, a.build_method, d.container_name, d.status, d.updated_at \
         FROM application_deployments d \
         JOIN applications a ON a.id = d.application_id \
         WHERE a.server_id = ?1 \
           AND d.status <> 'not_deployed' \
           AND (?2 = 1 OR d.status <> 'deploying') \
           AND NOT EXISTS (SELECT 1 FROM deployment_jobs j \
                           WHERE j.application_id = a.id \
                             AND j.status IN ('queued', 'building', 'deploying')) \
           AND NOT EXISTS (SELECT 1 FROM application_deployment_runs r \
                           WHERE r.application_id = a.id \
                             AND r.status IN ('building', 'deploying'))",
    )
    .bind(server_id)
    .bind(scope.claims_deploying())
    .fetch_all(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    let mut candidates = Vec::new();
    for row in &rows {
        let get = |index: usize| -> Result<String, AppError> {
            row.try_get(index)
                .map_err(|error| AppError::DatabaseError(error.to_string()))
        };
        candidates.push((
            get(0)?,
            get(1)?,
            get(2)?,
            get(3)?,
            get(4)?,
            row.try_get::<i64, _>(5)
                .map_err(|error| AppError::DatabaseError(error.to_string()))?,
        ));
    }

    let mut corrected = 0usize;
    for (application_id, deploy_type, build_method, container_name, status, updated_at) in
        candidates
    {
        if container_name.is_empty() {
            continue;
        }
        let compose = deploy_type == "compose" || build_method == "compose";
        let matched: Vec<&ContainerFact> = facts
            .iter()
            .filter(|fact| matches_container(&fact.name, &container_name, compose))
            .collect();
        let (desired, error_write) = desired_status(&matched);
        if desired == status {
            continue;
        }
        let changed = match error_write {
            ErrorWrite::Keep => {
                sqlx::query(
                    "UPDATE application_deployments SET status = ?1, updated_at = ?2 \
                     WHERE application_id = ?3 AND status = ?4 AND updated_at = ?5",
                )
                .bind(desired.clone())
                .bind(Utc::now().timestamp_millis())
                .bind(application_id.clone())
                .bind(status.clone())
                .bind(updated_at)
                .execute(&conn)
                .await
            }
            ErrorWrite::Clear | ErrorWrite::Set(_) => {
                let last_error = match &error_write {
                    ErrorWrite::Set(message) => Some(message.clone()),
                    _ => None,
                };
                sqlx::query(
                    "UPDATE application_deployments SET status = ?1, last_error = ?2, updated_at = ?3 \
                     WHERE application_id = ?4 AND status = ?5 AND updated_at = ?6",
                )
                .bind(desired.clone())
                .bind(last_error)
                .bind(Utc::now().timestamp_millis())
                .bind(application_id.clone())
                .bind(status.clone())
                .bind(updated_at)
                .execute(&conn)
                .await
            }
        }
        // The CAS on (status, updated_at) means a deploy pipeline that wrote between
        // the read and this update always wins.
        .map_err(|error| AppError::DatabaseError(error.to_string()))?
        .rows_affected();
        if changed > 0 {
            corrected += 1;
            tracing::info!(application_id, from = %status, to = %desired, "container status corrected from Podman");
        }
    }
    Ok(corrected)
}

/// Compose deployments spread over `<runtime_name>-<service>` containers, so they match
/// on that separator rather than a bare `contains`: `shop-prod-api` must not claim
/// `shop-prod-api2-web`.
pub(crate) fn matches_container(fact_name: &str, container_name: &str, compose: bool) -> bool {
    fact_name == container_name || (compose && fact_name.starts_with(&format!("{container_name}-")))
}

fn desired_status(matched: &[&ContainerFact]) -> (String, ErrorWrite) {
    if matched.iter().any(|fact| fact.is_running()) {
        return ("running".to_string(), ErrorWrite::Clear);
    }
    if matched.is_empty() {
        // `stopped`, not `not_deployed`: that is what an explicit stop writes, and it
        // keeps the UI offering Start instead of pretending the app was never set up.
        return (
            "stopped".to_string(),
            ErrorWrite::Set("The container is no longer present on the server".to_string()),
        );
    }
    match matched
        .iter()
        .filter_map(|fact| fact.exit_code)
        .find(|code| *code != 0)
    {
        Some(code) => (
            "failed".to_string(),
            ErrorWrite::Set(format!("Container exited with code {code}")),
        ),
        None => ("stopped".to_string(), ErrorWrite::Keep),
    }
}

pub(crate) fn parse_container_facts(output: &[u8]) -> Vec<ContainerFact> {
    if output.iter().all(u8::is_ascii_whitespace) {
        return Vec::new();
    }
    let value = match serde_json::from_slice::<serde_json::Value>(output) {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(%error, "could not parse podman ps JSON while probing a server");
            return Vec::new();
        }
    };
    let entries = value.as_array().cloned().unwrap_or_else(|| vec![value]);
    entries
        .into_iter()
        .filter_map(|entry| {
            let name = json_string(
                entry
                    .get("Names")
                    .or_else(|| entry.get("names"))
                    .or_else(|| entry.get("Name"))
                    .or_else(|| entry.get("name")),
            )?
            .to_string();
            let state = json_string(
                entry
                    .get("State")
                    .or_else(|| entry.get("state"))
                    .or_else(|| entry.get("Status"))
                    .or_else(|| entry.get("status")),
            )
            .unwrap_or("unknown")
            .to_string();
            Some(ContainerFact {
                name,
                state,
                exit_code: entry
                    .get("ExitCode")
                    .or_else(|| entry.get("exitCode"))
                    .and_then(serde_json::Value::as_i64),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{migrations, state::state::AppState};
    use std::collections::HashSet;
    use std::sync::Mutex;

    use uuid::Uuid;

    const PS_JSON: &str = r#"[
        {"Names":["shop-prod-web"],"State":"running","ExitCode":0},
        {"Names":["shop-prod-worker"],"State":"exited","ExitCode":137},
        {"Name":"shop-prod-cache","Status":"Up 3 hours","ExitCode":0}
    ]"#;

    fn fact(name: &str, state: &str, exit_code: Option<i64>) -> ContainerFact {
        ContainerFact {
            name: name.to_string(),
            state: state.to_string(),
            exit_code,
        }
    }

    #[test]
    fn parses_both_podman_name_shapes() {
        let facts = parse_container_facts(PS_JSON.as_bytes());
        assert_eq!(facts.len(), 3);
        assert_eq!(facts[0].name, "shop-prod-web");
        assert!(facts[0].is_running());
        assert_eq!(facts[1].exit_code, Some(137));
        assert!(!facts[1].is_running());
        // `Status: "Up 3 hours"` is the only state a plain `Name` entry carries.
        assert_eq!(facts[2].name, "shop-prod-cache");
        assert!(facts[2].is_running());
    }

    #[test]
    fn tolerates_empty_and_malformed_output() {
        assert!(parse_container_facts(b"").is_empty());
        assert!(parse_container_facts(b"   \n").is_empty());
        assert!(parse_container_facts(b"[]").is_empty());
        assert!(parse_container_facts(b"not json at all").is_empty());
    }

    #[test]
    fn status_follows_what_podman_reports() {
        let running = fact("app", "running", None);
        let clean = fact("app", "exited", Some(0));
        let crashed = fact("app", "exited", Some(137));

        assert_eq!(desired_status(&[&running]).0, "running");
        assert_eq!(desired_status(&[&clean]).0, "stopped");
        assert_eq!(desired_status(&[&crashed]).0, "failed");
        assert_eq!(desired_status(&[]).0, "stopped");
        // One live replica is enough for the application to count as running.
        assert_eq!(desired_status(&[&crashed, &running]).0, "running");
    }

    #[test]
    fn compose_matches_its_services_without_swallowing_a_sibling_app() {
        assert!(matches_container(
            "shop-prod-api-web",
            "shop-prod-api",
            true
        ));
        assert!(matches_container("shop-prod-api-db", "shop-prod-api", true));
        assert!(matches_container("shop-prod-api", "shop-prod-api", true));
        assert!(!matches_container(
            "shop-prod-api2-web",
            "shop-prod-api",
            true
        ));
        // A single-container app only ever owns its exact name.
        assert!(matches_container("shop-prod-api", "shop-prod-api", false));
        assert!(!matches_container(
            "shop-prod-api-web",
            "shop-prod-api",
            false
        ));
    }

    async fn test_state() -> (SharedState, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!("litepod-monitor-{}.db", Uuid::now_v7()));
        let db = crate::testing::sqlite_pool_at(&path).await;
        migrations::run(&db).await;
        let state: SharedState = Arc::new(AppState {
            started_at: chrono::Local::now(),
            db,
            caddy_url: String::new(),
            caddy_admin_url: String::new(),
            caddyfile_path: Default::default(),
            caddy_server_upstream: String::new(),
            podman_socket: String::new(),
            pending_execs: Mutex::new(HashMap::new()),
            data_dir: Default::default(),
            data_volume: None,
            host_data_dir: Default::default(),
            podman_network: String::new(),
            public_url: String::new(),
            oauth_states: Mutex::new(HashMap::new()),
            public_ip: String::new(),
            is_cloud: false,
            polar: None,
            dev_mode: false,
            port_range: (0, 0),
            reserved_ports: HashSet::new(),
            volume_access: Default::default(),
            podman_host: Default::default(),
            queue_runtime: Default::default(),
            version: String::new(),
            git_sha: String::new(),
            image_repo: String::new(),
            image_tag: String::new(),
            update_channel: Default::default(),
            version_manifest_url: String::new(),
            update_interval_secs: 0,
            container_name: String::new(),
            caddy_container: String::new(),
            stack_env_file: None,
            update_state: Default::default(),
            backup_lock: Default::default(),
            rate_limiter: tokio::sync::RwLock::new(
                crate::security::rate_limit::RateLimiterState::from_config(Default::default()),
            ),
            ephemeral: Default::default(),
        });
        let conn = state.db.clone();
        sqlx::query(
            "INSERT INTO users (id, email, hashed_password, name) VALUES ('user-1', 'user@example.com', 'hash', 'User')",
        ).execute(&conn).await.unwrap();
        sqlx::query(
            "INSERT INTO organizations (id, name, slug, created_by) VALUES ('organization-1', 'Organization', 'organization', 'user-1')",
        ).execute(&conn).await.unwrap();
        sqlx::query(
            "INSERT INTO projects (id, organization_id, name, created_at, updated_at) VALUES ('project-1', 'organization-1', 'Project', 1, 1)",
        ).execute(&conn).await.unwrap();
        sqlx::query(
            "INSERT INTO environments (id, project_id, name, is_production, created_at, updated_at) VALUES ('environment-1', 'project-1', 'production', 1, 1, 1)",
        ).execute(&conn).await.unwrap();
        sqlx::query(
            "INSERT INTO applications (id, environment_id, name, server_id, created_at, updated_at) VALUES ('application-1', 'environment-1', 'app', 'local', 1, 1)",
        ).execute(&conn).await.unwrap();
        (state, path)
    }

    async fn seed_deployment(state: &SharedState, status: &str) {
        let conn = state.db.clone();
        sqlx::query(
            "INSERT INTO application_deployments (application_id, status, container_name, hostname, updated_at) VALUES ('application-1', ?1, 'app', 'app.test', 1)",
        )
        .bind(status)
        .execute(&conn)
        .await
        .unwrap();
    }

    async fn stored_status(state: &SharedState) -> String {
        use sqlx::Row;
        let conn = state.db.clone();
        sqlx::query(
            "SELECT status FROM application_deployments WHERE application_id = 'application-1'",
        )
        .fetch_one(&conn)
        .await
        .unwrap()
        .try_get(0)
        .unwrap()
    }

    fn cleanup(path: std::path::PathBuf) {
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn a_dead_container_is_corrected_and_a_live_one_is_left_alone() {
        let (state, path) = test_state().await;
        seed_deployment(&state, "running").await;

        let corrected = reconcile_containers(&state, "local", &[], Scope::Periodic)
            .await
            .unwrap();
        assert_eq!(corrected, 1);
        assert_eq!(stored_status(&state).await, "stopped");

        let facts = vec![fact("app", "running", None)];
        assert_eq!(
            reconcile_containers(&state, "local", &facts, Scope::Periodic)
                .await
                .unwrap(),
            1
        );
        assert_eq!(stored_status(&state).await, "running");
        // Already in agreement with Podman: no write at all.
        assert_eq!(
            reconcile_containers(&state, "local", &facts, Scope::Periodic)
                .await
                .unwrap(),
            0
        );
        cleanup(path);
    }

    #[tokio::test]
    async fn only_the_boot_pass_claims_a_deploying_row() {
        let (state, path) = test_state().await;
        seed_deployment(&state, "deploying").await;
        let facts = vec![fact("app", "running", None)];

        assert_eq!(
            reconcile_containers(&state, "local", &facts, Scope::Periodic)
                .await
                .unwrap(),
            0
        );
        assert_eq!(stored_status(&state).await, "deploying");

        assert_eq!(
            reconcile_containers(&state, "local", &facts, Scope::Boot)
                .await
                .unwrap(),
            1
        );
        assert_eq!(stored_status(&state).await, "running");
        cleanup(path);
    }

    #[tokio::test]
    async fn a_live_deploy_run_owns_the_row_even_at_boot() {
        let (state, path) = test_state().await;
        seed_deployment(&state, "deploying").await;
        let conn = state.db.clone();
        sqlx::query(
            "INSERT INTO application_deployment_runs (id, application_id, source_type, build_method, status, started_at, triggered_by) VALUES ('run-1', 'application-1', 'git', 'docker', 'building', 1, 'manual')",
        ).execute(&conn).await.unwrap();

        assert_eq!(
            reconcile_containers(&state, "local", &[], Scope::Boot)
                .await
                .unwrap(),
            0
        );
        assert_eq!(stored_status(&state).await, "deploying");
        cleanup(path);
    }

    #[test]
    fn a_running_container_clears_the_stale_error() {
        assert!(matches!(
            desired_status(&[&fact("app", "running", None)]).1,
            ErrorWrite::Clear
        ));
        assert!(matches!(
            desired_status(&[&fact("app", "exited", Some(0))]).1,
            ErrorWrite::Keep
        ));
        assert!(matches!(desired_status(&[]).1, ErrorWrite::Set(_)));
    }
}
