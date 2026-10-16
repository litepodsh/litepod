//! Brings applications back after their server reboots.
//!
//! Rootless Podman has no daemon, and `podman-restart.service` only starts
//! `restart=always` containers, so every `unless-stopped` app stays `Exited` after a
//! reboot unless litepod starts it. The monitor calls [`on_probe`] after each
//! successful probe; a changed kernel `boot_id` means the host rebooted.

use std::{
    collections::{BTreeMap, HashSet},
    ffi::OsString,
    sync::{Arc, LazyLock, Mutex},
};

use sqlx::Row;

use crate::{
    dtos::response::AppError,
    notifications::{emit, event::RebootReport},
    servers::{
        monitor::{ContainerFact, matches_container},
        repository::ServerRecord,
        runtime::ServerRuntime,
    },
    state::state::SharedState,
};

const BOOT_ID_PATH: &str = "/proc/sys/kernel/random/boot_id";
/// A reboot must not stampede the host.
const START_CONCURRENCY: usize = 2;

/// Servers with a restore in flight: a tick landing mid-restore must not start a second.
static RESTORING: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(Default::default);

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum BootCheck {
    /// First sighting: store the id, restore nothing (safe upgrade).
    Record,
    Unchanged,
    Rebooted,
}

pub(crate) fn compare_boot_id(stored: Option<&str>, current: &str) -> BootCheck {
    match stored {
        None => BootCheck::Record,
        Some(stored) if stored == current => BootCheck::Unchanged,
        Some(_) => BootCheck::Rebooted,
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Plan {
    Start,
    Pending,
    Skip,
}

pub(crate) fn classify(policy: &str, desired: &str, running: bool, busy: bool) -> Plan {
    if running || busy {
        return Plan::Skip;
    }
    match (policy, desired) {
        ("always", _) | ("restore", "running") => Plan::Start,
        ("notify", "running") => Plan::Pending,
        _ => Plan::Skip,
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Candidate {
    pub application_id: String,
    pub name: String,
    pub organization_id: String,
    pub policy: String,
    pub desired: String,
    pub container_name: String,
    pub compose: bool,
    pub busy: bool,
}

pub(crate) async fn load_candidates(
    state: &SharedState,
    server_id: &str,
) -> Result<Vec<Candidate>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(
        // Same "a deploy in flight owns the app" rule as `monitor::reconcile_containers`.
        "SELECT a.id, a.name, p.organization_id, a.boot_policy, d.desired_state, d.container_name, \
                (a.deploy_type = 'compose' OR a.build_method = 'compose'), \
                (EXISTS (SELECT 1 FROM deployment_jobs j WHERE j.application_id = a.id \
                         AND j.status IN ('queued', 'building', 'deploying')) \
                 OR EXISTS (SELECT 1 FROM application_deployment_runs r WHERE r.application_id = a.id \
                         AND r.status IN ('building', 'deploying'))) \
         FROM application_deployments d \
         JOIN applications a ON a.id = d.application_id \
         JOIN environments e ON e.id = a.environment_id \
         JOIN projects p ON p.id = e.project_id \
         WHERE a.server_id = ?1 AND d.status <> 'not_deployed' AND d.container_name <> '' \
         ORDER BY a.name",
    )
    .bind(server_id)
    .fetch_all(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    rows.iter()
        .map(|row| {
            let text = |index: usize| -> Result<String, AppError> {
                row.try_get(index)
                    .map_err(|error| AppError::DatabaseError(error.to_string()))
            };
            let flag = |index: usize| -> Result<bool, AppError> {
                row.try_get::<i64, _>(index)
                    .map(|value| value != 0)
                    .map_err(|error| AppError::DatabaseError(error.to_string()))
            };
            Ok(Candidate {
                application_id: text(0)?,
                name: text(1)?,
                organization_id: text(2)?,
                policy: text(3)?,
                desired: text(4)?,
                container_name: text(5)?,
                compose: flag(6)?,
                busy: flag(7)?,
            })
        })
        .collect()
}

/// Called (spawned) after every successful probe. Cheap when nothing rebooted: one
/// row read, and the boot id is only fetched when it has never been stored or some
/// container is down, since a reboot always leaves app containers down.
pub async fn on_probe(state: SharedState, server: ServerRecord, facts: Vec<ContainerFact>) {
    let stored = match stored_boot_id(&state, &server.id).await {
        Ok(stored) => stored,
        Err(error) => {
            tracing::warn!(server = %server.name, %error, "[boot-restore] could not read the stored boot id");
            return;
        }
    };
    if stored.is_some() && facts.iter().all(ContainerFact::is_running) {
        return;
    }
    let Some(current) = read_boot_id(&state, &server).await else {
        tracing::debug!(server = %server.name, "[boot-restore] boot id unavailable; skipping reboot detection");
        return;
    };
    match compare_boot_id(stored.as_deref(), &current) {
        BootCheck::Unchanged => {}
        BootCheck::Record => {
            tracing::info!(
                server = %server.name,
                boot_id = %current,
                "[boot-restore] first boot id recorded; nothing restored on a first sighting"
            );
            store_boot_id(&state, &server, &current).await;
        }
        BootCheck::Rebooted => {
            if !RESTORING.lock().unwrap().insert(server.id.clone()) {
                tracing::debug!(server = %server.name, "[boot-restore] restore already in flight");
                return;
            }
            tracing::info!(
                server = %server.name,
                previous_boot_id = stored.as_deref().unwrap_or_default(),
                boot_id = %current,
                "[boot-restore] server rebooted; restoring applications"
            );
            restore(&state, &server, &facts).await;
            // Stored last: if litepod dies mid-restore, the next probe runs it again and
            // the apps that already came up are skipped as running.
            store_boot_id(&state, &server, &current).await;
            RESTORING.lock().unwrap().remove(&server.id);
        }
    }
}

async fn stored_boot_id(state: &SharedState, server_id: &str) -> Result<Option<String>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("SELECT last_boot_id FROM servers WHERE id = ?1")
        .bind(server_id)
        .fetch_optional(&conn)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?
        .map(|row| row.try_get::<Option<String>, _>(0))
        .transpose()
        .map(Option::flatten)
        .map_err(|error| AppError::DatabaseError(error.to_string()))
}

async fn store_boot_id(state: &SharedState, server: &ServerRecord, boot_id: &str) {
    let conn = crate::git::repositories::db::get_conn(state).await;
    if let Err(error) = sqlx::query("UPDATE servers SET last_boot_id = ?1 WHERE id = ?2")
        .bind(boot_id)
        .bind(&server.id)
        .execute(&conn)
        .await
    {
        tracing::warn!(server = %server.name, %error, "[boot-restore] could not store the boot id");
    }
}

/// The API container shares the host kernel, so the local file is the host's id.
async fn read_boot_id(state: &SharedState, server: &ServerRecord) -> Option<String> {
    let raw = if server.kind == "local" {
        tokio::fs::read_to_string(BOOT_ID_PATH).await.ok()?
    } else {
        let runtime = ServerRuntime::from_server_unchecked(state, server.clone())
            .await
            .ok()?;
        runtime
            .remote_command(&[OsString::from("cat"), OsString::from(BOOT_ID_PATH)])
            .await
            .ok()?
            .stdout
    };
    let id = raw.trim();
    (!id.is_empty()).then(|| id.to_string())
}

async fn restore(state: &SharedState, server: &ServerRecord, facts: &[ContainerFact]) {
    let candidates = match load_candidates(state, &server.id).await {
        Ok(candidates) => candidates,
        Err(error) => {
            tracing::warn!(server = %server.name, %error, "[boot-restore] could not load applications to restore");
            return;
        }
    };
    let runtime = match ServerRuntime::from_server_unchecked(state, server.clone()).await {
        Ok(runtime) => Arc::new(runtime),
        Err(error) => {
            tracing::warn!(server = %server.name, %error, "[boot-restore] could not reach the server to restore");
            return;
        }
    };
    let begun = std::time::Instant::now();
    tracing::info!(
        server = %server.name,
        applications = candidates.len(),
        concurrency = START_CONCURRENCY,
        "[boot-restore] evaluating applications"
    );
    let permits = Arc::new(tokio::sync::Semaphore::new(START_CONCURRENCY));
    let mut reports: BTreeMap<String, RebootReport> = BTreeMap::new();
    let mut tasks = tokio::task::JoinSet::new();
    for candidate in candidates {
        let owned: Vec<&ContainerFact> = facts
            .iter()
            .filter(|fact| {
                matches_container(&fact.name, &candidate.container_name, candidate.compose)
            })
            .collect();
        let running = owned.iter().any(|fact| fact.is_running());
        let plan = classify(
            &candidate.policy,
            &candidate.desired,
            running,
            candidate.busy,
        );
        // `restore` resumes what was there and never rebuilds: litepod's own Stop
        // removes containers, so a missing one may be a stop older than the audit log.
        let missing = plan == Plan::Start && owned.is_empty() && candidate.policy != "always";
        let decision = match plan {
            Plan::Skip if running => "skip: already running",
            Plan::Skip if candidate.busy => "skip: deployment in progress",
            Plan::Skip => "skip: stopped by the user",
            Plan::Pending => "leave stopped: notify policy",
            Plan::Start if missing => "leave stopped: container missing, redeploy it",
            Plan::Start if owned.is_empty() => "redeploy: always policy, no container",
            Plan::Start => "start",
        };
        tracing::info!(
            server = %server.name,
            application = %candidate.name,
            application_id = %candidate.application_id,
            policy = %candidate.policy,
            desired_state = %candidate.desired,
            containers = owned.len(),
            decision,
            "[boot-restore] decision"
        );
        let report = reports
            .entry(candidate.organization_id.clone())
            .or_default();
        match plan {
            Plan::Skip => {}
            Plan::Pending => report.pending.push(candidate.name),
            Plan::Start if missing => report.pending.push(format!(
                "{} (container missing, redeploy it)",
                candidate.name
            )),
            Plan::Start => {
                let names: Vec<String> = owned.iter().map(|fact| fact.name.clone()).collect();
                let (state, runtime, permits) = (state.clone(), runtime.clone(), permits.clone());
                let server_name = server.name.clone();
                tasks.spawn(async move {
                    let _permit = permits.acquire().await;
                    let started_at = std::time::Instant::now();
                    tracing::info!(
                        server = %server_name,
                        application = %candidate.name,
                        containers = %names.join(" "),
                        "[boot-restore] starting"
                    );
                    let result =
                        start_one(&state, &runtime, &candidate.application_id, names).await;
                    let elapsed_ms = started_at.elapsed().as_millis() as u64;
                    match &result {
                        Ok(()) => tracing::info!(
                            server = %server_name,
                            application = %candidate.name,
                            elapsed_ms,
                            "[boot-restore] started"
                        ),
                        Err(error) => tracing::warn!(
                            server = %server_name,
                            application = %candidate.name,
                            elapsed_ms,
                            %error,
                            "[boot-restore] start failed"
                        ),
                    }
                    (candidate.organization_id, candidate.name, result)
                });
            }
        }
    }
    while let Some(joined) = tasks.join_next().await {
        let (organization_id, name, result) = match joined {
            Ok(outcome) => outcome,
            Err(error) => {
                tracing::warn!(server = %server.name, %error, "[boot-restore] start task panicked");
                continue;
            }
        };
        let report = reports.entry(organization_id).or_default();
        match result {
            Ok(()) => report.started.push(name),
            Err(error) => report.failed.push((name, error)),
        }
    }
    let count = |pick: fn(&RebootReport) -> usize| reports.values().map(pick).sum::<usize>();
    tracing::info!(
        server = %server.name,
        started = count(|report| report.started.len()),
        failed = count(|report| report.failed.len()),
        left_stopped = count(|report| report.pending.len()),
        elapsed_ms = begun.elapsed().as_millis() as u64,
        "[boot-restore] finished"
    );
    for (organization_id, mut report) in reports {
        report.started.sort();
        emit::notify_server_reboot(state, &organization_id, &server.name, &report).await;
    }
}

async fn start_one(
    state: &SharedState,
    runtime: &ServerRuntime,
    application_id: &str,
    containers: Vec<String>,
) -> Result<(), String> {
    if containers.is_empty() {
        // Only `always` gets here: a redeploy is the only way back once containers are gone.
        return crate::deployment::start(state, application_id, None)
            .await
            .map(|_| ())
            .map_err(|error| error.to_string());
    }
    let mut args = vec![OsString::from("start")];
    args.extend(containers.into_iter().map(OsString::from));
    runtime
        .podman(&args)
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boot_id_first_sighting_only_records() {
        assert_eq!(compare_boot_id(None, "a"), BootCheck::Record);
        assert_eq!(compare_boot_id(Some("a"), "a"), BootCheck::Unchanged);
        assert_eq!(compare_boot_id(Some("a"), "b"), BootCheck::Rebooted);
    }

    #[test]
    fn classify_follows_policy_and_intent() {
        // Already up or owned by a deploy: never touched.
        assert_eq!(classify("always", "running", true, false), Plan::Skip);
        assert_eq!(classify("always", "running", false, true), Plan::Skip);
        // always ignores intent.
        assert_eq!(classify("always", "stopped", false, false), Plan::Start);
        // restore follows intent.
        assert_eq!(classify("restore", "running", false, false), Plan::Start);
        assert_eq!(classify("restore", "stopped", false, false), Plan::Skip);
        // notify never starts, only reports what would have come back.
        assert_eq!(classify("notify", "running", false, false), Plan::Pending);
        assert_eq!(classify("notify", "stopped", false, false), Plan::Skip);
    }

    #[tokio::test]
    async fn candidates_carry_policy_intent_org_and_busy_flag() {
        let (state, path) = crate::testing::test_state().await;
        let (_, _, application_id) =
            crate::testing::seed_application(&state, "organization-a").await;
        sqlx::query(
            "UPDATE applications SET deploy_type = 'compose', boot_policy = 'notify' WHERE id = ?1",
        )
        .bind(&application_id)
        .execute(&state.db)
        .await
        .unwrap();
        sqlx::query("INSERT INTO application_deployments (application_id, status, desired_state, container_name, hostname, updated_at) VALUES (?1, 'stopped', 'running', 'shop', 'shop.test', 1)")
            .bind(&application_id)
            .execute(&state.db)
            .await
            .unwrap();

        let candidates = load_candidates(&state, "organization-a-server")
            .await
            .unwrap();
        assert_eq!(candidates.len(), 1);
        let candidate = &candidates[0];
        assert_eq!(candidate.application_id, application_id);
        assert_eq!(candidate.organization_id, "organization-a");
        assert_eq!(candidate.policy, "notify");
        assert_eq!(candidate.desired, "running");
        assert_eq!(candidate.container_name, "shop");
        assert!(candidate.compose);
        assert!(!candidate.busy);
        assert!(
            load_candidates(&state, "other-server")
                .await
                .unwrap()
                .is_empty()
        );
        crate::testing::cleanup_database(&path);
    }
}
