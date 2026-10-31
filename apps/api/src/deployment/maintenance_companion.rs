//! The hidden "companion" application behind an uploaded maintenance site.
//!
//! Building, running and health-checking are all keyed by an `applications` row,
//! so a maintenance site that Railpack builds needs one. It is created when a ZIP
//! is uploaded (and only then), built like any uploaded source, and then
//! **parked**: its container is stopped with `podman stop`, never removed. Litepod's
//! own stop removes the container and its start rebuilds, which would make turning
//! maintenance on a full rebuild; a parked container comes back with a plain
//! `podman start`.
//!
//! A companion is invisible (`applications.maintenance_for` is set, and every
//! listing filters it out), has no domains and no published ports, and goes with
//! its parent.

use std::{ffi::OsString, time::Duration};

use chrono::Utc;
use sqlx::Row;
use uuid::Uuid;

use crate::{dtos::response::AppError, state::state::SharedState};

/// Railpack's static-site default (Caddy on `:$PORT`, 80 when unset).
pub const CONTAINER_PORT: i64 = 80;

/// A page is small; its build must not be able to starve real applications.
const BUILD_CPU_CORES: f64 = 1.0;
const BUILD_MEMORY_MIB: i64 = 1024;

fn db(error: sqlx::Error) -> AppError {
    AppError::DatabaseError(error.to_string())
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

/// The companion of `parent_id`, if one exists.
pub async fn find(state: &SharedState, parent_id: &str) -> Result<Option<String>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query_scalar("SELECT id FROM applications WHERE maintenance_for = ?1")
        .bind(parent_id)
        .fetch_optional(&conn)
        .await
        .map_err(db)
}

/// The parent of `application_id` when it is a companion.
pub async fn parent_of(
    state: &SharedState,
    application_id: &str,
) -> Result<Option<String>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let parent: Option<Option<String>> =
        sqlx::query_scalar("SELECT maintenance_for FROM applications WHERE id = ?1")
            .bind(application_id)
            .fetch_optional(&conn)
            .await
            .map_err(db)?;
    Ok(parent.flatten())
}

pub(crate) async fn set_status(
    state: &SharedState,
    parent_id: &str,
    status: &str,
    error: Option<&str>,
) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "INSERT INTO application_maintenance (application_id, enabled, page_html, companion_status, companion_error, updated_at) \
         VALUES (?1, 0, NULL, ?2, ?3, ?4) \
         ON CONFLICT(application_id) DO UPDATE SET \
             companion_status = excluded.companion_status, companion_error = excluded.companion_error, \
             updated_at = excluded.updated_at",
    )
    .bind(parent_id)
    .bind(status)
    .bind(error)
    .bind(now_ms())
    .execute(&conn)
    .await
    .map_err(db)?;
    Ok(())
}

/// `(status, error)` as the UI shows them.
pub async fn status(
    state: &SharedState,
    parent_id: &str,
) -> Result<(String, Option<String>), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(
        "SELECT companion_status, companion_error FROM application_maintenance WHERE application_id = ?1",
    )
    .bind(parent_id)
    .fetch_optional(&conn)
    .await
    .map_err(db)?;
    match row {
        Some(row) => Ok((row.try_get(0).map_err(db)?, row.try_get(1).map_err(db)?)),
        None => Ok(("none".into(), None)),
    }
}

/// Creates the companion when it does not exist yet: same environment and server
/// as the parent, Railpack, port 80, tight build limits, and nothing that makes it
/// visible or reachable.
async fn create(state: &SharedState, parent_id: &str) -> Result<String, AppError> {
    if let Some(id) = find(state, parent_id).await? {
        return Ok(id);
    }
    let conn = crate::git::repositories::db::get_conn(state).await;
    let parent = sqlx::query("SELECT environment_id, name, server_id FROM applications WHERE id = ?1")
        .bind(parent_id)
        .fetch_optional(&conn)
        .await
        .map_err(db)?
        .ok_or(AppError::ApplicationNotFound)?;
    let environment_id: String = parent.try_get(0).map_err(db)?;
    let parent_name: String = parent.try_get(1).map_err(db)?;
    let server_id: Option<String> = parent.try_get(2).map_err(db)?;

    let id = Uuid::now_v7().to_string();
    let now = now_ms();
    // The name only has to be human-readable in logs and the build queue.
    sqlx::query(
        "INSERT INTO applications (id, environment_id, name, description, server_id, maintenance_for, \
             deploy_type, build_method, container_port, build_cpu_cores, build_memory_mib, auto_deploy, restart_policy, created_at, updated_at) \
         VALUES (?1, ?2, ?3, 'Maintenance page', ?4, ?5, 'git', 'railpack', ?6, ?7, ?8, 0, 'no', ?9, ?9)",
    )
    .bind(&id)
    .bind(&environment_id)
    .bind(format!("Maintenance page · {parent_name}"))
    .bind(server_id)
    .bind(parent_id)
    .bind(CONTAINER_PORT)
    .bind(BUILD_CPU_CORES)
    .bind(BUILD_MEMORY_MIB)
    .bind(now)
    .execute(&conn)
    .await
    .map_err(db)?;
    super::ensure_deployment(state, &id).await?;
    Ok(id)
}

/// Keeps `zip` as the companion's source and queues its Railpack build. Returns
/// immediately; the build's outcome arrives through [`on_build_finished`].
pub async fn build(
    state: &SharedState,
    parent_id: &str,
    zip: &[u8],
) -> Result<(), AppError> {
    let companion = create(state, parent_id).await?;
    // A rebuild replaces the container behind the route. Until it is parked again
    // the static copy serves, and `on_build_finished` puts the new build back.
    set_live(state, parent_id, false).await?;
    let _ = super::maintenance::refresh_routes(state, parent_id).await;
    super::source_archive::store(state, &companion, zip).await?;
    // `store` resets the source to a git upload; the companion is always Railpack on 80.
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE applications SET build_method = 'railpack', container_port = ?1, auto_deploy = 0 WHERE id = ?2",
    )
    .bind(CONTAINER_PORT)
    .bind(&companion)
    .execute(&conn)
    .await
    .map_err(db)?;

    let ids = sqlx::query(
        "SELECT p.organization_id, p.id, e.id FROM applications a \
         JOIN environments e ON e.id = a.environment_id JOIN projects p ON p.id = e.project_id WHERE a.id = ?1",
    )
    .bind(&companion)
    .fetch_one(&conn)
    .await
    .map_err(db)?;
    let organization_id: String = ids.try_get(0).map_err(db)?;
    let project_id: String = ids.try_get(1).map_err(db)?;
    let environment_id: String = ids.try_get(2).map_err(db)?;

    set_status(state, parent_id, "building", None).await?;
    if let Err(error) = super::git::enqueue(
        state,
        &organization_id,
        &project_id,
        &environment_id,
        &companion,
        // `triggered_by` is a CHECK-constrained list; nobody reads this hidden run's label.
        "manual_deploy",
        None,
    )
    .await
    {
        set_status(state, parent_id, "failed", Some(&error.to_string())).await?;
        return Err(error);
    }
    Ok(())
}

/// Stops the companion's container but keeps it, and records that the user wants
/// it down (so a server reboot does not revive it).
async fn park(state: &SharedState, companion: &str) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let container: String =
        sqlx::query_scalar("SELECT container_name FROM application_deployments WHERE application_id = ?1")
            .bind(companion)
            .fetch_optional(&conn)
            .await
            .map_err(db)?
            .ok_or(AppError::ApplicationNotFound)?;
    // `restore` boots a container only when the user wants it running, and
    // parking is exactly "the user wants it stopped".
    sqlx::query("UPDATE applications SET boot_policy = 'restore' WHERE id = ?1")
        .bind(companion)
        .execute(&conn)
        .await
        .map_err(db)?;
    super::boot::mark_stopped(state, companion).await?;
    let runtime = crate::servers::runtime::ServerRuntime::for_application(state, companion).await?;
    runtime
        .podman(&[OsString::from("stop"), OsString::from(&container)])
        .await?;
    sqlx::query("UPDATE application_deployments SET status = 'stopped', updated_at = ?1 WHERE application_id = ?2")
        .bind(now_ms())
        .bind(companion)
        .execute(&conn)
        .await
        .map_err(db)?;
    Ok(())
}

/// Called when the pipeline of `application_id` ends. Does nothing for an ordinary
/// application. For a companion: a good build is parked, a bad one is recorded,
/// and in neither case is anything routed.
pub async fn on_build_finished(
    state: &SharedState,
    application_id: &str,
    outcome: &Result<(), String>,
) {
    let Ok(Some(parent)) = parent_of(state, application_id).await else {
        return;
    };
    let result = match outcome {
        Ok(()) => match park(state, application_id).await {
            Ok(()) => {
                let marked = set_status(state, &parent, "ready", None).await;
                // Maintenance was already on while this rebuilt: bring the new
                // build into service the same way turning the mode on does.
                if enabled_now(state, &parent).await {
                    if let Err(error) = activate(state, &parent).await {
                        tracing::info!(parent, %error, "keeping the static copy after a rebuild");
                    }
                    let _ = super::maintenance::refresh_routes(state, &parent).await;
                }
                marked
            }
            Err(error) => {
                set_status(state, &parent, "failed", Some(&error.to_string())).await
            }
        },
        Err(error) => set_status(state, &parent, "failed", Some(error)).await,
    };
    if let Err(error) = result {
        tracing::warn!(parent, %error, "could not record the maintenance companion's state");
    }
}

/// Removes the companion, its container, image files and row. Idempotent. Run it
/// before the parent's row is deleted: the cascade removes the row but not the
/// container.
pub async fn teardown(state: &SharedState, parent_id: &str) {
    let Ok(Some(companion)) = find(state, parent_id).await else {
        return;
    };
    super::cleanup(state, &companion).await;
    let conn = crate::git::repositories::db::get_conn(state).await;
    let _ = sqlx::query("DELETE FROM applications WHERE id = ?1 AND maintenance_for = ?2")
        .bind(&companion)
        .bind(parent_id)
        .execute(&conn)
        .await;
    let _ = set_status(state, parent_id, "none", None).await;
}

/// How long a started companion has to answer its health check.
const HEALTH_BUDGET: Duration = Duration::from_secs(30);

pub(crate) async fn set_live(state: &SharedState, parent_id: &str, live: bool) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("UPDATE application_maintenance SET companion_live = ?1, updated_at = ?2 WHERE application_id = ?3")
        .bind(i64::from(live))
        .bind(now_ms())
        .bind(parent_id)
        .execute(&conn)
        .await
        .map_err(db)?;
    Ok(())
}

/// Where routes may dial the companion: `container:80` while it is live, running
/// and local. Anything else is `None`, which sends visitors to the static copy.
pub(crate) async fn live_upstream(
    state: &SharedState,
    parent_id: &str,
) -> Result<Option<String>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(
        "SELECT d.container_name, d.status FROM application_maintenance m \
         JOIN applications c ON c.maintenance_for = m.application_id \
         JOIN application_deployments d ON d.application_id = c.id \
         WHERE m.application_id = ?1 AND m.companion_live = 1 AND m.companion_status = 'ready'",
    )
    .bind(parent_id)
    .fetch_optional(&conn)
    .await
    .map_err(db)?;
    let Some(row) = row else {
        return Ok(None);
    };
    let container: String = row.try_get(0).map_err(db)?;
    let status: String = row.try_get(1).map_err(db)?;
    // `status` follows what Podman reports, so a container that died while live
    // stops being routed the next time a route is built.
    Ok((status == "running" && !container.is_empty()).then(|| format!("{container}:{CONTAINER_PORT}")))
}

/// Waits for the started container to answer `GET /health` with a success.
async fn wait_healthy(
    state: &SharedState,
    runtime: &crate::servers::runtime::ServerRuntime,
    container: &str,
) -> Result<(), String> {
    // Same reachability rule as a normal rollout: in dev the API sits outside the
    // container network and uses the published loopback port.
    let (host, port) = if state.dev_mode {
        let published = super::rollout::published_port(runtime, container, CONTAINER_PORT as u16).await?;
        ("127.0.0.1".to_string(), published)
    } else {
        (container.to_string(), CONTAINER_PORT as u16)
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .map_err(|error| error.to_string())?;
    let url = format!("http://{host}:{port}/health");
    let deadline = tokio::time::Instant::now() + HEALTH_BUDGET;
    while tokio::time::Instant::now() < deadline {
        if let Ok(response) = client.get(&url).send().await
            && response.status().is_success()
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Err("The maintenance page did not answer its health check in time".into())
}

/// Starts the parked container and waits until it is serving. On any failure it is
/// stopped again and nothing is marked live, so the caller falls back to the static
/// copy. Order matters: the route may only be moved after this returns `Ok`.
pub async fn activate(state: &SharedState, parent_id: &str) -> Result<(), String> {
    let Some(companion) = find(state, parent_id).await.map_err(|e| e.to_string())? else {
        return Err("There is no compiled maintenance page yet".into());
    };
    let (status, _) = status(state, parent_id).await.map_err(|e| e.to_string())?;
    if status != "ready" {
        return Err("The maintenance page is not compiled yet".into());
    }
    let runtime = crate::servers::runtime::ServerRuntime::for_application(state, &companion)
        .await
        .map_err(|e| e.to_string())?;
    if runtime.is_remote() {
        return Err("A compiled maintenance page is not available on remote servers yet".into());
    }
    let conn = crate::git::repositories::db::get_conn(state).await;
    let container: String =
        sqlx::query_scalar("SELECT container_name FROM application_deployments WHERE application_id = ?1")
            .bind(&companion)
            .fetch_one(&conn)
            .await
            .map_err(|e| e.to_string())?;
    // A container someone removed is not recreated here: rebuilding takes minutes
    // and activation must be quick. The caller serves the static copy instead.
    runtime
        .podman(&[OsString::from("start"), OsString::from(&container)])
        .await
        .map_err(|_| "The maintenance page's container is missing; upload the ZIP again to rebuild it".to_string())?;
    if let Err(error) = wait_healthy(state, &runtime, &container).await {
        let _ = runtime
            .podman(&[OsString::from("stop"), OsString::from(&container)])
            .await;
        return Err(error);
    }
    sqlx::query("UPDATE application_deployments SET status = 'running', desired_state = 'running', updated_at = ?1 WHERE application_id = ?2")
        .bind(now_ms())
        .bind(&companion)
        .execute(&conn)
        .await
        .map_err(|e| e.to_string())?;
    set_live(state, parent_id, true).await.map_err(|e| e.to_string())
}

/// Takes the companion out of service and stops it. Marks it not live first, so
/// no route built from here on names it. Run it after the route has moved back.
pub async fn deactivate(state: &SharedState, parent_id: &str) {
    let _ = set_live(state, parent_id, false).await;
    let Ok(Some(companion)) = find(state, parent_id).await else {
        return;
    };
    if let Err(error) = park(state, &companion).await {
        tracing::warn!(parent_id, %error, "could not park the maintenance companion");
    }
}

async fn enabled_now(state: &SharedState, parent_id: &str) -> bool {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query_scalar::<_, i64>("SELECT enabled FROM application_maintenance WHERE application_id = ?1")
        .bind(parent_id)
        .fetch_optional(&conn)
        .await
        .ok()
        .flatten()
        .is_some_and(|enabled| enabled != 0)
}
