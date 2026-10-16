use std::{collections::HashMap, str::FromStr};

use chrono::Utc;
use sqlx::Row;
use uuid::Uuid;

use crate::{
    dtos::response::AppError,
    servers::model::{IngressMode, PodmanMode, ServerState},
    state::state::SharedState,
};

#[derive(Debug, Clone)]
pub struct ServerRecord {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub ssh_host: Option<String>,
    pub ssh_port: Option<u16>,
    pub ssh_user: Option<String>,
    pub host_key_fingerprint: Option<String>,
    /// Confirmed public known_hosts entry. It is not secret, but binds the
    /// fingerprint to the exact host key SSH must accept.
    pub host_key_known_hosts: Option<String>,
    pub private_key_encrypted: Option<String>,
    pub ingress_mode: IngressMode,
    pub podman_mode: PodmanMode,
    pub state: ServerState,
    pub public_address: Option<String>,
    pub facts_json: String,
    pub last_error_code: Option<String>,
    pub last_error_message: Option<String>,
    /// Owning organization. `None` only for the shared `local` server of a
    /// self-hosted install; every `kind = 'ssh'` server has an owner.
    pub organization_id: Option<String>,
    /// Where the server is, resolved from its public address by `servers::geoip`.
    /// The columns are the cache, so a lookup survives a restart.
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub geo_city: Option<String>,
    pub geo_country: Option<String>,
    pub geo_asn: Option<String>,
    pub geo_timezone: Option<String>,
    /// Epoch millis of the last lookup attempt, successful or not.
    pub geo_resolved_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct ServerJobRecord {
    pub id: String,
    pub server_id: String,
    pub kind: String,
    pub status: String,
    pub phase: String,
    pub log: String,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
}

fn row(record: &sqlx::sqlite::SqliteRow) -> Result<ServerRecord, AppError> {
    let state: String = record
        .try_get(10)
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    let ingress: String = record
        .try_get(9)
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    Ok(ServerRecord {
        id: record
            .try_get(0)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?,
        kind: record
            .try_get(1)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?,
        name: record
            .try_get(2)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?,
        ssh_host: record
            .try_get(3)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?,
        ssh_port: record
            .try_get::<Option<i64>, _>(4)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?
            .and_then(|port| u16::try_from(port).ok()),
        ssh_user: record
            .try_get(5)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?,
        host_key_fingerprint: record
            .try_get(6)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?,
        host_key_known_hosts: record
            .try_get(7)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?,
        private_key_encrypted: record
            .try_get(8)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?,
        ingress_mode: IngressMode::from_str(&ingress)?,
        podman_mode: PodmanMode::from_str(
            &record
                .try_get::<String, _>(15)
                .map_err(|error| AppError::DatabaseError(error.to_string()))?,
        )?,
        state: ServerState::from_str(&state)?,
        public_address: record
            .try_get(11)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?,
        facts_json: record
            .try_get(12)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?,
        last_error_code: record
            .try_get(13)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?,
        last_error_message: record
            .try_get(14)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?,
        organization_id: record
            .try_get(16)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?,
        latitude: record
            .try_get(17)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?,
        longitude: record
            .try_get(18)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?,
        geo_city: record
            .try_get(19)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?,
        geo_country: record
            .try_get(20)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?,
        geo_asn: record
            .try_get(21)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?,
        geo_timezone: record
            .try_get(22)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?,
        geo_resolved_at: record
            .try_get(23)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?,
    })
}

/// Column order is the contract with `row`, which reads by index. New columns
/// go at the end so every existing index keeps pointing at the same column.
const SELECT: &str = "id, kind, name, ssh_host, ssh_port, ssh_user, host_key_fingerprint, host_key_known_hosts, private_key_encrypted, ingress_mode, state, public_address, facts_json, last_error_code, last_error_message, podman_mode, organization_id, latitude, longitude, geo_city, geo_country, geo_asn, geo_timezone, geo_resolved_at";

/// Every server an organization may use: the ones it owns, plus the shared
/// `local` server of a self-hosted install (`organization_id IS NULL`).
const OWNED_BY: &str = "(organization_id = ?1 OR organization_id IS NULL)";

pub async fn get(state: &SharedState, id: &str) -> Result<Option<ServerRecord>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(&format!("SELECT {SELECT} FROM servers WHERE id = ?1"))
        .bind(id)
        .fetch_optional(&conn)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?
        .map(|record| row(&record))
        .transpose()
}

/// The tenancy-safe `get`. A server id belonging to another organization comes
/// back as `None`, so callers surface it as "not found" rather than leaking
/// that the server exists. Every caller reached through an organization-scoped
/// route must use this, never `get`.
///
/// On cloud it never returns the `local` server (the control plane host), so a
/// server id taken from a request body can never reach it.
pub async fn get_for_organization(
    state: &SharedState,
    id: &str,
    organization_id: &str,
) -> Result<Option<ServerRecord>, AppError> {
    get_scoped(state, id, organization_id, !state.is_cloud).await
}

/// `get_for_organization` for `/servers/{server_id}/...` handlers only: their
/// `RequireOrg` already 404s the cloud `local` server for anyone but the SysAdmin,
/// so it may be returned here.
pub async fn get_for_server_route(
    state: &SharedState,
    id: &str,
    organization_id: &str,
) -> Result<Option<ServerRecord>, AppError> {
    get_scoped(state, id, organization_id, true).await
}

async fn get_scoped(
    state: &SharedState,
    id: &str,
    organization_id: &str,
    include_local: bool,
) -> Result<Option<ServerRecord>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let filter = if include_local {
        ""
    } else {
        " AND kind = 'ssh'"
    };
    sqlx::query(&format!(
        "SELECT {SELECT} FROM servers WHERE id = ?2 AND {OWNED_BY}{filter}"
    ))
    .bind(organization_id)
    .bind(id)
    .fetch_optional(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?
    .map(|record| row(&record))
    .transpose()
}

/// Servers whose liveness the monitor re-checks. `needs_verification`, `needs_setup`,
/// `setting_up` and `unsupported` are states no probe can resolve - only a human action
/// moves them - so probing them would be a wasted SSH round trip.
pub async fn list_monitorable(state: &SharedState) -> Result<Vec<ServerRecord>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let sql = format!(
        "SELECT {SELECT} FROM servers \
         WHERE kind = 'local' OR (kind = 'ssh' AND state IN ('ready', 'degraded', 'offline')) \
         ORDER BY kind, name"
    );
    let rows = sqlx::query(&sql)
        .fetch_all(&conn)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    rows.iter().map(row).collect()
}

/// A probe that succeeded on an already-`ready` server proves liveness without changing
/// anything. Recording the timestamp keeps the UI's "last checked" honest without
/// rewriting `facts_json` every tick.
pub async fn mark_checked(state: &SharedState, id: &str) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("UPDATE servers SET last_checked_at = ?1 WHERE id = ?2")
        .bind(Utc::now().timestamp_millis())
        .bind(id)
        .execute(&conn)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    Ok(())
}

pub async fn list_ready_for_organization(
    state: &SharedState,
    organization_id: &str,
) -> Result<Vec<ServerRecord>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let filter = if state.is_cloud {
        " AND kind = 'ssh'"
    } else {
        ""
    };
    let sql = format!(
        "SELECT {SELECT} FROM servers WHERE state = 'ready' AND {OWNED_BY}{filter} ORDER BY kind, name"
    );
    let rows = sqlx::query(&sql)
        .bind(organization_id)
        .fetch_all(&conn)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    rows.iter().map(row).collect()
}

/// `include_local`: on cloud the `local` server (the control plane host) is listed
/// only for a SysAdmin; self-hosted always lists it.
pub async fn list_for_organization(
    state: &SharedState,
    organization_id: &str,
    include_local: bool,
) -> Result<Vec<ServerRecord>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let filter = if state.is_cloud && !include_local {
        " AND kind = 'ssh'"
    } else {
        ""
    };
    let sql = format!("SELECT {SELECT} FROM servers WHERE {OWNED_BY}{filter} ORDER BY kind, name");
    let rows = sqlx::query(&sql)
        .bind(organization_id)
        .fetch_all(&conn)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    rows.iter().map(row).collect()
}

#[allow(dead_code)] // used by cqrs::commands::project integration tests.
pub async fn list(state: &SharedState) -> Result<Vec<ServerRecord>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let filter = if state.is_cloud {
        " WHERE kind = 'ssh'"
    } else {
        ""
    };
    let sql = format!("SELECT {SELECT} FROM servers{filter} ORDER BY kind, name");
    let rows = sqlx::query(&sql)
        .fetch_all(&conn)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    rows.iter().map(row).collect()
}

#[allow(clippy::too_many_arguments)]
pub async fn insert_pending(
    state: &SharedState,
    id: &str,
    organization_id: &str,
    name: &str,
    ssh_host: &str,
    ssh_port: u16,
    ssh_user: &str,
    private_key_encrypted: &str,
    ingress_mode: IngressMode,
    podman_mode: PodmanMode,
    public_address: Option<&str>,
) -> Result<ServerRecord, AppError> {
    // SQLite cannot express "kind='ssh' implies organization_id IS NOT NULL" as
    // a CHECK added by ALTER, so the invariant is enforced at the only place
    // that creates an SSH server.
    if organization_id.trim().is_empty() {
        return Err(AppError::ValidationError(
            "A remote server must belong to an organization".into(),
        ));
    }
    let now = Utc::now().timestamp_millis();
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "INSERT INTO servers (id, kind, name, ssh_host, ssh_port, ssh_user, private_key_encrypted, ingress_mode, podman_mode, state, public_address, organization_id, created_at, updated_at) VALUES (?1, 'ssh', ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'needs_verification', ?9, ?10, ?11, ?11)",
    )
    .bind(id)
    .bind(name)
    .bind(ssh_host)
    .bind(ssh_port as i64)
    .bind(ssh_user)
    .bind(private_key_encrypted)
    .bind(ingress_mode.as_str())
    .bind(podman_mode.as_str())
    .bind(public_address)
    .bind(organization_id)
    .bind(now)
    .execute(&conn)
    .await
    .map_err(|error| {
        let message = error.to_string();
        if message.to_ascii_lowercase().contains("unique") { AppError::AlreadyExists } else { AppError::DatabaseError(message) }
    })?;
    get(state, id).await?.ok_or(AppError::ResourceNotFound)
}

/// Renames a registered or local server.
pub async fn update_name(
    state: &SharedState,
    id: &str,
    name: &str,
) -> Result<ServerRecord, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE servers SET name = ?1, updated_at = ?2 WHERE id = ?3 AND kind IN ('ssh', 'local')",
    )
    .bind(name)
    .bind(Utc::now().timestamp_millis())
    .bind(id)
    .execute(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    get(state, id).await?.ok_or(AppError::ResourceNotFound)
}

/// Records the outcome of a geolocation lookup. A failed lookup still stamps
/// `geo_resolved_at`, so an address that will never resolve is not retried on
/// every request.
pub async fn set_geo(
    state: &SharedState,
    id: &str,
    geo: Option<&crate::servers::geoip::ServerGeo>,
) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE servers SET latitude = ?1, longitude = ?2, geo_city = ?3, geo_country = ?4, geo_asn = ?5, geo_timezone = ?6, geo_resolved_at = ?7 WHERE id = ?8",
    )
    .bind(geo.map(|geo| geo.latitude))
    .bind(geo.map(|geo| geo.longitude))
    .bind(geo.and_then(|geo| geo.city.clone()))
    .bind(geo.and_then(|geo| geo.country.clone()))
    .bind(geo.and_then(|geo| geo.asn.clone()))
    .bind(geo.and_then(|geo| geo.timezone.clone()))
    .bind(Utc::now().timestamp_millis())
    .bind(id)
    .execute(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    Ok(())
}

pub async fn confirm_host_key(
    state: &SharedState,
    id: &str,
    fingerprint: &str,
    known_hosts_line: &str,
    next: ServerState,
) -> Result<ServerRecord, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("UPDATE servers SET host_key_fingerprint = ?1, host_key_known_hosts = ?2, state = ?3, updated_at = ?4 WHERE id = ?5 AND kind = 'ssh'")
        .bind(fingerprint)
        .bind(known_hosts_line)
        .bind(next.as_str())
        .bind(Utc::now().timestamp_millis())
        .bind(id)
        .execute(&conn)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    get(state, id).await?.ok_or(AppError::ResourceNotFound)
}

/// Lazily fills the public host-key material for installations upgraded from a
/// fingerprint-only schema. Callers must compare the scanned fingerprint before
/// invoking this function.
pub async fn set_host_key_known_hosts(
    state: &SharedState,
    id: &str,
    known_hosts_line: &str,
) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE servers SET host_key_known_hosts = ?1, updated_at = ?2 WHERE id = ?3 AND kind = 'ssh' AND host_key_known_hosts IS NULL",
    )
    .bind(known_hosts_line)
    .bind(Utc::now().timestamp_millis())
    .bind(id)
    .execute(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    Ok(())
}

/// Upgrades a fingerprint-only SSH record in place. The observed public key is
/// accepted only when it matches the fingerprint already confirmed by an
/// administrator, so this never silently trusts a replacement host.
pub async fn ensure_host_key_known_hosts(
    state: &SharedState,
    server: ServerRecord,
) -> Result<ServerRecord, AppError> {
    if server.kind != "ssh"
        || server
            .host_key_known_hosts
            .as_deref()
            .is_some_and(|line| !line.trim().is_empty())
    {
        return Ok(server);
    }
    let target = crate::servers::model::SshTarget::parse(
        server
            .ssh_host
            .as_deref()
            .ok_or_else(|| AppError::InternalError("Remote server has no SSH host".into()))?,
        server
            .ssh_port
            .ok_or_else(|| AppError::InternalError("Remote server has no SSH port".into()))?,
        server
            .ssh_user
            .as_deref()
            .ok_or_else(|| AppError::InternalError("Remote server has no SSH user".into()))?,
    )?;
    let expected = server.host_key_fingerprint.as_deref().ok_or_else(|| {
        AppError::ValidationError("Confirm the SSH host fingerprint before connecting".into())
    })?;
    let observed = crate::servers::ssh::SshExecutor::discover_host_key(&target).await?;
    if observed.value != expected {
        return Err(AppError::ValidationError(
            "SSH host key changed; a sysadmin must verify the new fingerprint before connecting"
                .into(),
        ));
    }
    set_host_key_known_hosts(state, &server.id, observed.known_hosts_line()).await?;
    get(state, &server.id)
        .await?
        .ok_or(AppError::ResourceNotFound)
}

pub async fn set_facts_and_state(
    state: &SharedState,
    id: &str,
    facts_json: &str,
    next: ServerState,
) -> Result<ServerRecord, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE servers SET facts_json = ?1, state = ?2, last_checked_at = ?3, last_error_code = NULL, last_error_message = NULL, updated_at = ?3 WHERE id = ?4",
    )
    .bind(facts_json)
    .bind(next.as_str())
    .bind(Utc::now().timestamp_millis())
    .bind(id)
    .execute(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    get(state, id).await?.ok_or(AppError::ResourceNotFound)
}

pub async fn set_state_error(
    state: &SharedState,
    id: &str,
    next: ServerState,
    code: &str,
    message: &str,
) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE servers SET state = ?1, last_checked_at = ?2, last_error_code = ?3, last_error_message = ?4, updated_at = ?2 WHERE id = ?5",
    )
    .bind(next.as_str())
    .bind(Utc::now().timestamp_millis())
    .bind(code)
    .bind(message)
    .bind(id)
    .execute(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    Ok(())
}

pub async fn set_state(state: &SharedState, id: &str, next: ServerState) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE servers SET state = ?1, last_error_code = NULL, last_error_message = NULL, updated_at = ?2 WHERE id = ?3",
    )
    .bind(next.as_str())
    .bind(Utc::now().timestamp_millis())
    .bind(id)
    .execute(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    Ok(())
}

fn job_row(record: &sqlx::sqlite::SqliteRow) -> Result<ServerJobRecord, AppError> {
    Ok(ServerJobRecord {
        id: record
            .try_get(0)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?,
        server_id: record
            .try_get(1)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?,
        kind: record
            .try_get(2)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?,
        status: record
            .try_get(3)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?,
        phase: record
            .try_get(4)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?,
        log: record
            .try_get(5)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?,
        error_code: record
            .try_get(6)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?,
        error_message: record
            .try_get(7)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?,
        created_at: record
            .try_get(8)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?,
        started_at: record
            .try_get(9)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?,
        finished_at: record
            .try_get(10)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?,
    })
}

const SELECT_JOB: &str = "id, server_id, kind, status, phase, log_redacted, error_code, error_message, created_at, started_at, finished_at";

pub async fn create_job(
    state: &SharedState,
    server_id: &str,
    actor_user_id: &str,
    kind: &str,
) -> Result<ServerJobRecord, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let id = Uuid::now_v7().to_string();
    let now = Utc::now().timestamp_millis();
    sqlx::query(
        "INSERT INTO server_jobs (id, server_id, kind, status, actor_user_id, phase, created_at) VALUES (?1, ?2, ?3, 'queued', ?4, 'queued', ?5)",
    )
    .bind(id.clone())
    .bind(server_id)
    .bind(kind)
    .bind(actor_user_id)
    .bind(now)
    .execute(&conn)
    .await
    .map_err(|error| {
        let message = error.to_string();
        if message.to_ascii_lowercase().contains("unique")
            && matches!(kind, "setup" | "repair")
        {
            AppError::ResourceInUse("A Setup or Repair job is already running for this server".into())
        } else {
            AppError::DatabaseError(message)
        }
    })?;
    get_job(state, &id).await?.ok_or(AppError::ResourceNotFound)
}

pub async fn get_job(state: &SharedState, id: &str) -> Result<Option<ServerJobRecord>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(&format!(
        "SELECT {SELECT_JOB} FROM server_jobs WHERE id = ?1"
    ))
    .bind(id)
    .fetch_optional(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?
    .map(|record| job_row(&record))
    .transpose()
}

pub async fn list_jobs(
    state: &SharedState,
    server_id: &str,
) -> Result<Vec<ServerJobRecord>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(&format!(
        "SELECT {SELECT_JOB} FROM server_jobs WHERE server_id = ?1 ORDER BY created_at DESC"
    ))
    .bind(server_id)
    .fetch_all(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    rows.iter().map(job_row).collect()
}

/// A setup or repair job only exists as a `tokio::spawn` task inside the process that queued it,
/// so any row still queued or running at boot belongs to a run that died with the last process.
/// Left as-is those rows block Setup, Repair, and Remove for that server forever.
pub async fn fail_interrupted_jobs(state: &SharedState) -> Result<u64, AppError> {
    const MESSAGE: &str = "Setup was interrupted because litepod restarted";
    let conn = crate::git::repositories::db::get_conn(state).await;
    let now = Utc::now().timestamp_millis();
    let jobs = sqlx::query(
        "UPDATE server_jobs SET status = 'failed', phase = 'failed', error_code = 'interrupted', error_message = ?1, finished_at = ?2 WHERE status IN ('queued', 'running')",
    )
    .bind(MESSAGE)
    .bind(now)
    .execute(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?
    .rows_affected();
    // A server frozen in setting_up has no job left to move it, so it is dropped to degraded
    // where Repair is offered again.
    sqlx::query(
        "UPDATE servers SET state = 'degraded', last_error_code = 'interrupted', last_error_message = ?1, updated_at = ?2 WHERE state = 'setting_up'",
    )
    .bind(MESSAGE)
    .bind(now)
    .execute(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    Ok(jobs)
}

pub async fn has_active_setup_job(state: &SharedState, server_id: &str) -> Result<bool, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    Ok(sqlx::query(
        "SELECT 1 FROM server_jobs WHERE server_id = ?1 AND kind IN ('setup', 'repair') AND status IN ('queued', 'running') LIMIT 1",
    )
    .bind(server_id)
    .fetch_optional(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?
    .is_some())
}

pub async fn start_job(state: &SharedState, id: &str, phase: &str) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("UPDATE server_jobs SET status = 'running', phase = ?1, started_at = ?2 WHERE id = ?3 AND status = 'queued'")
        .bind(phase)
        .bind(Utc::now().timestamp_millis())
        .bind(id)
        .execute(&conn)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    Ok(())
}

pub async fn append_job_log(
    state: &SharedState,
    id: &str,
    phase: &str,
    line: &str,
) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("UPDATE server_jobs SET phase = ?1, log_redacted = substr(log_redacted || ?2 || char(10), -16000) WHERE id = ?3")
        .bind(phase)
        .bind(line)
        .bind(id)
        .execute(&conn)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    Ok(())
}

pub async fn finish_job(
    state: &SharedState,
    id: &str,
    succeeded: bool,
    phase: &str,
    error: Option<(&str, &str)>,
) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let (code, message) = error.unzip();
    sqlx::query(
        "UPDATE server_jobs SET status = ?1, phase = ?2, error_code = ?3, error_message = ?4, finished_at = ?5 WHERE id = ?6",
    )
    .bind(if succeeded { "succeeded" } else { "failed" })
    .bind(phase)
    .bind(code)
    .bind(message)
    .bind(Utc::now().timestamp_millis())
    .bind(id)
    .execute(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    Ok(())
}

pub async fn count_assigned_applications(
    state: &SharedState,
    server_id: &str,
) -> Result<u64, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let count: i64 = match sqlx::query("SELECT COUNT(*) FROM applications WHERE server_id = ?1")
        .bind(server_id)
        .fetch_optional(&conn)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?
    {
        Some(record) => record
            .try_get(0)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?,
        None => 0,
    };
    Ok(u64::try_from(count).unwrap_or(0))
}

/// An application that keeps a server from being removed. Carries what the removal dialog needs
/// to both name it and link straight to it.
#[derive(Debug, Clone)]
pub struct AssignedApplication {
    pub id: String,
    pub name: String,
    pub environment_id: String,
    pub environment_name: String,
    pub project_id: String,
    pub project_name: String,
    pub deploy_type: String,
    pub template_kind: Option<String>,
    pub icon_url: Option<String>,
    pub catalog_icon_url: Option<String>,
}

/// The applications that keep a server from being removed, named the way the user sees them so
/// the removal dialog can say exactly what has to be moved first.
/// Applications of `organization_id` placed on `server_id`. Scoped because the
/// self-hosted `local` server is shared: without it, every organization would
/// see the others' applications on it.
pub async fn assigned_applications(
    state: &SharedState,
    server_id: &str,
    organization_id: &str,
) -> Result<Vec<AssignedApplication>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(
        "SELECT a.id, a.name, a.environment_id, e.name, p.id, p.name,
                a.deploy_type, a.template_kind, a.icon_url, a.catalog_icon_url
         FROM applications a
         INNER JOIN environments e ON e.id = a.environment_id
         INNER JOIN projects p ON p.id = e.project_id
         WHERE a.server_id = ?1 AND p.organization_id = ?2
         ORDER BY p.name ASC, e.name ASC, a.name ASC",
    )
    .bind(server_id)
    .bind(organization_id)
    .fetch_all(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    let mut assigned = Vec::new();
    for record in &rows {
        let read = |index: usize| -> Result<String, AppError> {
            record
                .try_get(index)
                .map_err(|error| AppError::DatabaseError(error.to_string()))
        };
        let read_opt = |index: usize| -> Option<String> {
            record.try_get::<Option<String>, _>(index).ok().flatten()
        };
        assigned.push(AssignedApplication {
            id: read(0)?,
            name: read(1)?,
            environment_id: read(2)?,
            environment_name: read(3)?,
            project_id: read(4)?,
            project_name: read(5)?,
            deploy_type: read(6)?,
            template_kind: read_opt(7),
            icon_url: read_opt(8),
            catalog_icon_url: read_opt(9),
        });
    }
    Ok(assigned)
}

/// One grouped query so listing servers stays a single round trip.
pub async fn assigned_counts(state: &SharedState) -> Result<HashMap<String, u64>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query("SELECT server_id, COUNT(*) FROM applications WHERE server_id IS NOT NULL GROUP BY server_id")
        .fetch_all(&conn)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    let mut counts = HashMap::new();
    for record in &rows {
        let id: String = record
            .try_get(0)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?;
        let count: i64 = record
            .try_get(1)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?;
        counts.insert(id, u64::try_from(count).unwrap_or(0));
    }
    Ok(counts)
}

/// Removes a registered SSH server. `server_jobs` rows cascade; the caller must have already
/// refused the local server, running jobs, and servers that still host applications.
pub async fn delete(state: &SharedState, id: &str) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("DELETE FROM server_jobs WHERE server_id = ?1")
        .bind(id)
        .execute(&conn)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    sqlx::query("DELETE FROM application_runtime_endpoints WHERE server_id = ?1")
        .bind(id)
        .execute(&conn)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    sqlx::query("DELETE FROM application_relay_endpoints WHERE server_id = ?1")
        .bind(id)
        .execute(&conn)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    sqlx::query("DELETE FROM servers WHERE id = ?1 AND kind = 'ssh'")
        .bind(id)
        .execute(&conn)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    Ok(())
}

pub async fn ensure_local(state: &SharedState) -> Result<(), AppError> {
    let now = Utc::now().timestamp_millis();
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("INSERT OR IGNORE INTO servers (id, kind, name, ingress_mode, state, created_at, updated_at) VALUES ('local', 'local', 'Local server', 'central', 'ready', ?1, ?1)")
        .bind(now)
        .execute(&conn)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    Ok(())
}

/// Cloud installations must not retain the Podman-backed control-plane server as a deployment
/// target. Refuse to remove it when it already owns workloads so an environment-only mode switch
/// cannot silently strand applications.
pub async fn assert_no_local_applications(state: &SharedState) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let local_application_count: i64 =
        sqlx::query("SELECT COUNT(*) FROM applications WHERE server_id = 'local'")
            .fetch_optional(&conn)
            .await
            .map_err(|error| AppError::DatabaseError(error.to_string()))?
            .ok_or_else(|| AppError::DatabaseError("Could not count local applications".into()))?
            .try_get(0)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?;

    if local_application_count > 0 {
        return Err(AppError::ValidationError(
            "IS_CLOUD=true cannot start while applications are assigned to the local server".into(),
        ));
    }

    Ok(())
}
