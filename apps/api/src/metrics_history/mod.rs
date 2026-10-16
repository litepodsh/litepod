//! Container metrics history, kept in an embedded DuckDB file next to the SQLite
//! database (`<dir of DATABASE_PATH>/metrics.duckdb`).
//!
//! A background sampler reads `podman stats` from every ready server on a fixed
//! interval and appends one row per running container. History endpoints bucket
//! those rows server-side so a chart never receives more than [`MAX_POINTS`].
//! Retention is per organization (`organizations.metrics_retention_days`, NULL =
//! install default).
//!
//! DuckDB's API is synchronous: the single connection sits behind a std Mutex and
//! is only ever touched from `spawn_blocking`.
//!
//! Host disk, RAM and core count are sampled every [`HOST_SAMPLE_INTERVAL`] into
//! `server_samples`, under the same retention. Each row keeps the capacity of that
//! moment, so history stays truthful after a server is resized.
//!
//! Env: `LITEPOD_METRICS_SAMPLE_INTERVAL_SECS` (default 15, min 5),
//! `LITEPOD_METRICS_RETENTION_DAYS` (default 90, clamped to 1..=365).

use std::{
    sync::{Mutex, OnceLock},
    time::Duration,
};

use axum::{
    Json,
    extract::{Path, Query, State},
    response::IntoResponse,
};
use duckdb::{Connection, params};
use serde::{Deserialize, Serialize};
use sqlx::Row;

use crate::{
    dtos::response::{AppError, ok},
    git::repositories::audit_log,
    podman::podman_metrics::{ContainerMetricState, ContainerMetricsSnapshot},
    security::authz::extract::{RequireApp, RequireOrg, perm},
    servers::{model::ServerState, repository, runtime::ServerRuntime},
    state::state::SharedState,
};

pub mod requests;

const MAX_POINTS: i64 = 120;
const SAMPLE_TIMEOUT: Duration = Duration::from_secs(10);
const RETENTION_TICK: Duration = Duration::from_secs(3600);
const MAX_RETENTION_DAYS: i64 = 365;
/// One SSH round trip per server per minute (`df`, `nproc`, `/proc/meminfo`).
const HOST_SAMPLE_INTERVAL: Duration = Duration::from_secs(60);
/// Never exits non-zero: a host missing one part (no `/proc/meminfo` on macOS) still
/// reports the rest.
const HOST_PROBE: &str = "df -Pk / 2>/dev/null; echo ---; nproc 2>/dev/null; echo ---; cat /proc/meminfo 2>/dev/null; true";

static DB: OnceLock<Mutex<Connection>> = OnceLock::new();

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS container_samples (
    ts TIMESTAMP NOT NULL,
    organization_id VARCHAR,
    server_id VARCHAR NOT NULL,
    application_id VARCHAR,
    container_name VARCHAR NOT NULL,
    cpu_percent DOUBLE,
    mem_bytes BIGINT,
    mem_limit_bytes BIGINT
);
CREATE TABLE IF NOT EXISTS server_samples (
    ts TIMESTAMP NOT NULL,
    organization_id VARCHAR,
    server_id VARCHAR NOT NULL,
    disk_used_bytes BIGINT,
    disk_total_bytes BIGINT,
    mem_used_bytes BIGINT,
    mem_total_bytes BIGINT,
    cpu_cores INTEGER
);";

fn env_number(key: &str, default: i64) -> i64 {
    std::env::var(key)
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(default)
}

fn sample_interval() -> Duration {
    Duration::from_secs(env_number("LITEPOD_METRICS_SAMPLE_INTERVAL_SECS", 15).max(5) as u64)
}

pub fn default_retention_days() -> i64 {
    env_number("LITEPOD_METRICS_RETENTION_DAYS", 90).clamp(1, MAX_RETENTION_DAYS)
}

fn validate_retention_days(days: i64) -> Result<i64, AppError> {
    if (1..=MAX_RETENTION_DAYS).contains(&days) {
        Ok(days)
    } else {
        Err(AppError::ValidationError(
            "Metrics retention must be between 1 and 365 days".into(),
        ))
    }
}

/// Next to the SQLite file, whatever form `DATABASE_PATH` takes.
fn db_path() -> std::path::PathBuf {
    let database_path =
        std::env::var("DATABASE_PATH").unwrap_or_else(|_| "./data/litepod.db".to_string());
    let file = database_path
        .trim_start_matches("sqlite://")
        .trim_start_matches("sqlite:");
    let file = file.split('?').next().unwrap_or(file);
    std::path::Path::new(file)
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."))
        .join("metrics.duckdb")
}

fn open(path: Option<&std::path::Path>) -> duckdb::Result<Connection> {
    let conn = match path {
        Some(path) => Connection::open(path)?,
        None => Connection::open_in_memory()?,
    };
    // DuckDB defaults to every core and 80% of RAM; this shares a host with the API.
    conn.execute_batch("SET threads TO 2; SET memory_limit = '256MB';")?;
    conn.execute_batch(SCHEMA)?;
    conn.execute_batch(requests::SCHEMA)?;
    Ok(conn)
}

async fn with_db<T: Send + 'static>(
    work: impl FnOnce(&Connection) -> duckdb::Result<T> + Send + 'static,
) -> Result<T, AppError> {
    tokio::task::spawn_blocking(move || {
        let db = DB
            .get()
            .ok_or_else(|| AppError::InternalError("Metrics history is not available".into()))?;
        let conn = db.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        work(&conn).map_err(|error| AppError::InternalError(error.to_string()))
    })
    .await
    .map_err(|error| AppError::InternalError(error.to_string()))?
}

// ---------------------------------------------------------------------------
// Storage (pure, sync; tested against an in-memory DuckDB)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    pub ts_ms: i64,
    pub organization_id: Option<String>,
    pub server_id: String,
    pub application_id: Option<String>,
    pub container_name: String,
    pub cpu_percent: Option<f64>,
    pub mem_bytes: Option<i64>,
    pub mem_limit_bytes: Option<i64>,
}

fn insert(conn: &Connection, samples: &[Sample]) -> duckdb::Result<()> {
    let mut appender = conn.appender("container_samples")?;
    for sample in samples {
        appender.append_row(params![
            duckdb::types::Value::Timestamp(duckdb::types::TimeUnit::Millisecond, sample.ts_ms),
            sample.organization_id,
            sample.server_id,
            sample.application_id,
            sample.container_name,
            sample.cpu_percent,
            sample.mem_bytes,
            sample.mem_limit_bytes,
        ])?;
    }
    appender.flush()
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct HostSample {
    pub ts_ms: i64,
    pub organization_id: Option<String>,
    pub server_id: String,
    pub disk_used_bytes: Option<i64>,
    pub disk_total_bytes: Option<i64>,
    pub mem_used_bytes: Option<i64>,
    pub mem_total_bytes: Option<i64>,
    pub cpu_cores: Option<i32>,
}

fn insert_host(conn: &Connection, samples: &[HostSample]) -> duckdb::Result<()> {
    let mut appender = conn.appender("server_samples")?;
    for sample in samples {
        appender.append_row(params![
            duckdb::types::Value::Timestamp(duckdb::types::TimeUnit::Millisecond, sample.ts_ms),
            sample.organization_id,
            sample.server_id,
            sample.disk_used_bytes,
            sample.disk_total_bytes,
            sample.mem_used_bytes,
            sample.mem_total_bytes,
            sample.cpu_cores,
        ])?;
    }
    appender.flush()
}

/// `(used, total)` bytes of the root filesystem from `df -Pk /`. Total is used plus
/// available, so blocks reserved for root do not read as free space.
fn df_usage(df: &str) -> Option<(i64, i64)> {
    let fields: Vec<&str> = df
        .lines()
        .find(|line| line.trim_end().ends_with(" /"))?
        .split_whitespace()
        .collect();
    let used = fields.get(2)?.parse::<i64>().ok()?.checked_mul(1024)?;
    let available = fields.get(3)?.parse::<i64>().ok()?.checked_mul(1024)?;
    Some((used, used.checked_add(available)?))
}

/// `(used, total)` bytes from `/proc/meminfo`; used is `MemTotal - MemAvailable`.
fn meminfo_usage(meminfo: &str) -> Option<(i64, i64)> {
    let field = |name: &str| -> Option<i64> {
        meminfo
            .lines()
            .find_map(|line| line.strip_prefix(name))?
            .split_whitespace()
            .next()?
            .parse::<i64>()
            .ok()?
            .checked_mul(1024)
    };
    let total = field("MemTotal:")?;
    Some((total - field("MemAvailable:")?, total))
}

/// `(disk, cores, memory)`, each `(used, total)` bytes where it applies.
type HostProbe = (Option<(i64, i64)>, Option<i32>, Option<(i64, i64)>);

/// Parses [`HOST_PROBE`] output; any part a host cannot answer is left empty.
fn parse_host_probe(output: &str) -> HostProbe {
    let mut parts = output.split("---\n");
    let disk = parts.next().and_then(df_usage);
    let cores = parts.next().and_then(|part| part.trim().parse().ok());
    let memory = parts.next().and_then(meminfo_usage);
    (disk, cores, memory)
}

/// What the SysAdmin debug page shows about this store. Counters only, never sample rows.
#[derive(Debug, Serialize, PartialEq)]
pub struct DebugStats {
    pub file_bytes: Option<u64>,
    pub wal_bytes: Option<u64>,
    pub sample_count: i64,
    /// Epoch milliseconds.
    pub oldest_sample_at: Option<i64>,
    pub newest_sample_at: Option<i64>,
    /// Distinct (server, container name) pairs.
    pub container_count: i64,
    pub sample_interval_secs: u64,
    pub default_retention_days: i64,
    /// Rows of the per-minute HTTP request rollups.
    pub request_rows: i64,
    /// Largest first.
    pub tables: Vec<TableUsage>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct TableUsage {
    pub name: String,
    /// Checkpointed blocks times the block size; rows still only in the WAL count as 0.
    pub bytes: i64,
    pub rows: i64,
}

fn table_usage(conn: &Connection) -> duckdb::Result<Vec<TableUsage>> {
    let block_size: i64 =
        conn.query_row("SELECT block_size FROM pragma_database_size()", [], |row| {
            row.get(0)
        })?;
    let mut stmt = conn.prepare(
        "SELECT table_name, estimated_size FROM duckdb_tables() WHERE NOT internal ORDER BY table_name",
    )?;
    let tables = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?
        .collect::<duckdb::Result<Vec<_>>>()?;
    let mut usage = tables
        .into_iter()
        .map(|(name, rows)| {
            // Table names come from the catalog, not from a request.
            let blocks: i64 = conn.query_row(
                &format!(
                    "SELECT count(DISTINCT block_id) FILTER (WHERE persistent) FROM pragma_storage_info('{}')",
                    name.replace('\'', "''")
                ),
                [],
                |row| row.get(0),
            )?;
            Ok(TableUsage { name, bytes: blocks * block_size, rows })
        })
        .collect::<duckdb::Result<Vec<_>>>()?;
    usage.sort_by(|a, b| b.bytes.cmp(&a.bytes).then(b.rows.cmp(&a.rows)));
    Ok(usage)
}

type SampleCounters = (i64, Option<i64>, Option<i64>, i64);

fn sample_counters(conn: &Connection) -> duckdb::Result<SampleCounters> {
    conn.query_row(
        "SELECT count(*), epoch_ms(min(ts)), epoch_ms(max(ts)),
                count(DISTINCT server_id || '/' || container_name)
         FROM container_samples",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )
}

/// Through the shared connection: the file is never opened a second time.
pub async fn debug_stats() -> Result<DebugStats, AppError> {
    let ((sample_count, oldest_sample_at, newest_sample_at, container_count), request_rows, tables) =
        with_db(|conn| {
            Ok((
                sample_counters(conn)?,
                requests::request_row_count(conn)?,
                table_usage(conn)?,
            ))
        })
        .await?;
    let path = db_path();
    let size = |path: &std::path::Path| std::fs::metadata(path).ok().map(|m| m.len());
    let mut wal = path.clone().into_os_string();
    wal.push(".wal");
    Ok(DebugStats {
        file_bytes: size(&path),
        wal_bytes: size(std::path::Path::new(&wal)),
        sample_count,
        oldest_sample_at,
        newest_sample_at,
        container_count,
        sample_interval_secs: sample_interval().as_secs(),
        default_retention_days: default_retention_days(),
        request_rows,
        tables,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
pub enum Range {
    #[default]
    #[serde(rename = "1h")]
    Hour,
    #[serde(rename = "6h")]
    SixHours,
    #[serde(rename = "24h")]
    Day,
    #[serde(rename = "7d")]
    Week,
    #[serde(rename = "30d")]
    Month,
    #[serde(rename = "90d")]
    Quarter,
    #[serde(rename = "365d")]
    Year,
    /// From the scope's oldest stored sample, at most a year back.
    #[serde(rename = "all")]
    All,
}

impl Range {
    const fn seconds(self) -> i64 {
        match self {
            Self::Hour => 3_600,
            Self::SixHours => 6 * 3_600,
            Self::Day => 24 * 3_600,
            Self::Week => 7 * 24 * 3_600,
            Self::Month => 30 * 24 * 3_600,
            Self::Quarter => 90 * 24 * 3_600,
            Self::Year | Self::All => MAX_RETENTION_DAYS * 24 * 3_600,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Scope<'a> {
    Application(&'a str),
    Server(&'a str),
}

#[derive(Debug, Serialize, PartialEq)]
pub struct HistoryPoint {
    /// Bucket start, epoch milliseconds.
    pub ts: i64,
    pub cpu_avg: Option<f64>,
    pub cpu_max: Option<f64>,
    pub mem_avg: Option<f64>,
    pub mem_limit: Option<i64>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct ContainerHistory {
    pub container_name: String,
    pub points: Vec<HistoryPoint>,
}

/// Host usage per bucket; totals are the largest capacity seen in the bucket.
#[derive(Debug, Serialize, PartialEq)]
pub struct HostPoint {
    /// Bucket start, epoch milliseconds.
    pub ts: i64,
    pub disk_used: Option<f64>,
    pub disk_total: Option<i64>,
    pub mem_used: Option<f64>,
    pub mem_total: Option<i64>,
    pub cpu_cores: Option<i32>,
}

#[derive(Debug, Serialize)]
pub struct MetricsHistoryDto {
    pub range: Range,
    /// Window actually covered, epoch milliseconds (`to` exclusive).
    pub from: i64,
    pub to: i64,
    pub bucket_seconds: i64,
    pub containers: Vec<ContainerHistory>,
    /// Host disk and RAM; empty for an application.
    pub host: Vec<HostPoint>,
}

/// What to show: a preset range ending now, or an explicit `[from, to)` window.
#[derive(Debug, Clone, Copy, Default)]
pub struct Window {
    pub range: Range,
    pub from: Option<i64>,
    pub to: Option<i64>,
}

fn scope_column(scope: Scope<'_>) -> (&'static str, &str) {
    match scope {
        Scope::Application(id) => ("application_id", id),
        Scope::Server(id) => ("server_id", id),
    }
}

/// `[from, to)` in epoch ms, never wider than the retention cap.
fn resolve_window(
    conn: &Connection,
    scope: Scope<'_>,
    window: Window,
    now_ms: i64,
) -> duckdb::Result<(i64, i64)> {
    const MIN_SPAN_MS: i64 = 60_000;
    let cap_ms = MAX_RETENTION_DAYS * 24 * 3_600_000;
    let to = window.to.unwrap_or(now_ms).min(now_ms);
    let from = match (window.from, window.range) {
        (Some(from), _) => from,
        (None, Range::All) => {
            let (column, id) = scope_column(scope);
            let oldest: Option<i64> = conn.query_row(
                &format!(
                    "SELECT epoch_ms(min(ts)) FROM (
                         SELECT ts FROM container_samples WHERE {column} = ?
                         UNION ALL
                         SELECT ts FROM server_samples WHERE ? AND server_id = ?)"
                ),
                params![id, matches!(scope, Scope::Server(_)), id],
                |row| row.get(0),
            )?;
            oldest.unwrap_or(to - 3_600_000)
        }
        (None, range) => to - range.seconds() * 1_000,
    };
    let from = from.max(to - cap_ms).min(to - MIN_SPAN_MS);
    Ok((from, to))
}

/// For the positive epoch values used here (`i64::div_ceil` is unstable).
const fn ceil_div(value: i64, divisor: i64) -> i64 {
    (value + divisor - 1) / divisor
}

fn history(
    conn: &Connection,
    scope: Scope<'_>,
    window: Window,
    now_ms: i64,
) -> duckdb::Result<MetricsHistoryDto> {
    let (from, to) = resolve_window(conn, scope, window, now_ms)?;
    // A preset starts on the next bucket boundary (buckets are aligned to the epoch
    // below), so it never spills into a partial 121st bucket. An explicit start or
    // `all` must keep its first sample, so it floors and leaves room for one more bucket.
    let anchored = window.from.is_some() || window.range == Range::All;
    let points = if anchored { MAX_POINTS - 1 } else { MAX_POINTS };
    let bucket_seconds = ceil_div((to - from) / 1_000, points).max(1);
    let bucket_ms = bucket_seconds * 1_000;
    let since_ms = if anchored {
        from / bucket_ms * bucket_ms
    } else {
        ceil_div(from, bucket_ms) * bucket_ms
    };
    let (column, id) = scope_column(scope);
    let sql = format!(
        "SELECT container_name,
                epoch_ms(time_bucket(to_seconds(?::BIGINT), ts, TIMESTAMP '1970-01-01')) AS bucket,
                avg(cpu_percent), max(cpu_percent), avg(mem_bytes), max(mem_limit_bytes)
         FROM container_samples
         WHERE {column} = ? AND ts >= make_timestamp(?::BIGINT * 1000)
           AND ts < make_timestamp(?::BIGINT * 1000)
         GROUP BY container_name, bucket
         ORDER BY container_name, bucket"
    );
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map(params![bucket_seconds, id, since_ms, to], |row| {
        Ok((
            row.get::<_, String>(0)?,
            HistoryPoint {
                ts: row.get(1)?,
                cpu_avg: row.get(2)?,
                cpu_max: row.get(3)?,
                mem_avg: row.get(4)?,
                mem_limit: row.get(5)?,
            },
        ))
    })?;
    let mut containers: Vec<ContainerHistory> = Vec::new();
    for row in rows {
        let (container_name, point) = row?;
        match containers.last_mut() {
            Some(last) if last.container_name == container_name => last.points.push(point),
            _ => containers.push(ContainerHistory {
                container_name,
                points: vec![point],
            }),
        }
    }
    let mut host = Vec::new();
    if let Scope::Server(server_id) = scope {
        let mut statement = conn.prepare(
            "SELECT epoch_ms(time_bucket(to_seconds(?::BIGINT), ts, TIMESTAMP '1970-01-01')) AS bucket,
                    avg(disk_used_bytes), max(disk_total_bytes),
                    avg(mem_used_bytes), max(mem_total_bytes), max(cpu_cores)
             FROM server_samples
             WHERE server_id = ? AND ts >= make_timestamp(?::BIGINT * 1000)
               AND ts < make_timestamp(?::BIGINT * 1000)
             GROUP BY bucket
             ORDER BY bucket",
        )?;
        host = statement
            .query_map(params![bucket_seconds, server_id, since_ms, to], |row| {
                Ok(HostPoint {
                    ts: row.get(0)?,
                    disk_used: row.get(1)?,
                    disk_total: row.get(2)?,
                    mem_used: row.get(3)?,
                    mem_total: row.get(4)?,
                    cpu_cores: row.get(5)?,
                })
            })?
            .collect::<duckdb::Result<_>>()?;
    }
    Ok(MetricsHistoryDto {
        range: window.range,
        from: since_ms,
        to,
        bucket_seconds,
        containers,
        host,
    })
}

/// Deletes rows past their organization's window; rows of organizations without an
/// override (and rows with no organization, e.g. the shared local server's own
/// containers) use `default_days`.
fn apply_retention(
    conn: &Connection,
    overrides: &[(String, i64)],
    default_days: i64,
    now_ms: i64,
) -> duckdb::Result<usize> {
    conn.execute_batch(
        "CREATE OR REPLACE TEMP TABLE retention_overrides (organization_id VARCHAR PRIMARY KEY, days BIGINT NOT NULL);",
    )?;
    {
        let mut appender = conn.appender("retention_overrides")?;
        for (organization_id, days) in overrides {
            appender.append_row(params![organization_id, days])?;
        }
    }
    let mut deleted = 0;
    for table in ["container_samples", "server_samples"]
        .into_iter()
        .chain(requests::TABLES)
    {
        deleted += conn.execute(
            &format!(
                "DELETE FROM {table}
                 WHERE ts < make_timestamp(?::BIGINT * 1000) - to_days(CAST(COALESCE(
                     (SELECT days FROM retention_overrides r
                      WHERE r.organization_id = {table}.organization_id),
                     ?::BIGINT) AS INTEGER))"
            ),
            params![now_ms, default_days],
        )?;
    }
    conn.execute_batch("DROP TABLE retention_overrides; CHECKPOINT;")?;
    Ok(deleted)
}

// ---------------------------------------------------------------------------
// Sampler
// ---------------------------------------------------------------------------

/// An application container this server is expected to run.
#[derive(Debug, Clone)]
struct Owner {
    application_id: String,
    organization_id: String,
    compose: bool,
    container_name: String,
}

/// Same container ownership rule as `servers::monitor::reconcile_containers`:
/// exact name, or `<name>-<service>` for compose.
fn to_samples(
    server_id: &str,
    server_organization_id: Option<&str>,
    snapshot: &ContainerMetricsSnapshot,
    owners: &[Owner],
) -> Vec<Sample> {
    snapshot
        .metrics
        .iter()
        .filter(|metric| metric.state == ContainerMetricState::Running)
        .filter(|metric| metric.cpu_percent.is_some() || metric.memory_usage_bytes.is_some())
        .map(|metric| {
            let owner = owners.iter().find(|owner| {
                crate::servers::monitor::matches_container(
                    &metric.name,
                    &owner.container_name,
                    owner.compose,
                )
            });
            Sample {
                ts_ms: metric.observed_at.timestamp_millis(),
                organization_id: owner
                    .map(|owner| owner.organization_id.clone())
                    .or_else(|| server_organization_id.map(str::to_owned)),
                server_id: server_id.to_string(),
                application_id: owner.map(|owner| owner.application_id.clone()),
                container_name: metric.name.clone(),
                cpu_percent: metric.cpu_percent,
                mem_bytes: metric.memory_usage_bytes.map(|bytes| bytes as i64),
                mem_limit_bytes: metric.memory_limit_bytes.map(|bytes| bytes as i64),
            }
        })
        .collect()
}

async fn owners_on(state: &SharedState, server_id: &str) -> Result<Vec<Owner>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    // Primary placement plus server-group replicas placed on this server.
    let rows = sqlx::query(
        "SELECT a.id, p.organization_id, (a.deploy_type = 'compose' OR a.build_method = 'compose'), d.container_name \
         FROM application_deployments d \
         JOIN applications a ON a.id = d.application_id \
         JOIN environments e ON e.id = a.environment_id \
         JOIN projects p ON p.id = e.project_id \
         WHERE a.server_id = ?1 AND d.container_name <> '' \
         UNION \
         SELECT a.id, p.organization_id, (a.deploy_type = 'compose' OR a.build_method = 'compose'), r.container_name \
         FROM application_replicas r \
         JOIN applications a ON a.id = r.application_id \
         JOIN environments e ON e.id = a.environment_id \
         JOIN projects p ON p.id = e.project_id \
         WHERE r.server_id = ?1 AND r.container_name <> ''",
    )
    .bind(server_id)
    .fetch_all(&conn)
    .await?;
    rows.iter()
        .map(|row| {
            Ok(Owner {
                application_id: row.try_get(0)?,
                organization_id: row.try_get(1)?,
                compose: row.try_get::<i64, _>(2)? != 0,
                container_name: row.try_get(3)?,
            })
        })
        .collect::<Result<_, sqlx::Error>>()
        .map_err(AppError::from)
}

async fn sample_server(
    state: &SharedState,
    server: &repository::ServerRecord,
) -> Result<Vec<Sample>, AppError> {
    let runtime = ServerRuntime::for_server(state, &server.id).await?;
    let snapshot = tokio::time::timeout(
        SAMPLE_TIMEOUT,
        crate::podman::podman_metrics::collect_runtime_container_metrics(&runtime, &[]),
    )
    .await
    .map_err(|_| AppError::InternalError("podman stats timed out".into()))??;
    let owners = owners_on(state, &server.id).await?;
    Ok(to_samples(
        &server.id,
        server.organization_id.as_deref(),
        &snapshot,
        &owners,
    ))
}

async fn sample_tick(state: SharedState) -> Result<(), AppError> {
    let servers = repository::list_monitorable(&state)
        .await?
        .into_iter()
        .filter(|server| server.state == ServerState::Ready)
        .collect::<Vec<_>>();
    let results =
        futures_util::future::join_all(servers.iter().map(|server| sample_server(&state, server)))
            .await;
    let mut samples = Vec::new();
    for (server, result) in servers.iter().zip(results) {
        match result {
            Ok(rows) => samples.extend(rows),
            Err(error) => {
                tracing::debug!(server = %server.name, %error, "metrics sample skipped")
            }
        }
    }
    if !samples.is_empty() {
        with_db(move |conn| insert(conn, &samples)).await?;
    }
    Ok(())
}

async fn sample_host(server: &repository::ServerRecord) -> Result<HostSample, AppError> {
    let output = if server.kind == "local" {
        let output = tokio::process::Command::new("sh")
            .args(["-c", HOST_PROBE])
            .output()
            .await
            .map_err(|error| AppError::InternalError(error.to_string()))?;
        String::from_utf8_lossy(&output.stdout).into_owned()
    } else {
        crate::servers::setup::command(server, &["sh", "-c", HOST_PROBE]).await?
    };
    let (disk, cpu_cores, memory) = parse_host_probe(&output);
    if disk.is_none() && memory.is_none() {
        return Err(AppError::InternalError(
            "unreadable host probe output".into(),
        ));
    }
    Ok(HostSample {
        ts_ms: chrono::Utc::now().timestamp_millis(),
        organization_id: server.organization_id.clone(),
        server_id: server.id.clone(),
        disk_used_bytes: disk.map(|(used, _)| used),
        disk_total_bytes: disk.map(|(_, total)| total),
        mem_used_bytes: memory.map(|(used, _)| used),
        mem_total_bytes: memory.map(|(_, total)| total),
        cpu_cores,
    })
}

async fn host_tick(state: SharedState) -> Result<(), AppError> {
    let servers = repository::list_monitorable(&state)
        .await?
        .into_iter()
        .filter(|server| server.state == ServerState::Ready)
        .collect::<Vec<_>>();
    let results = futures_util::future::join_all(
        servers
            .iter()
            .map(|server| tokio::time::timeout(SAMPLE_TIMEOUT, sample_host(server))),
    )
    .await;
    let mut samples = Vec::new();
    for (server, result) in servers.iter().zip(results) {
        match result {
            Ok(Ok(sample)) => samples.push(sample),
            Ok(Err(error)) => tracing::debug!(server = %server.name, %error, "host sample skipped"),
            Err(_) => tracing::debug!(server = %server.name, "host sample timed out"),
        }
    }
    if !samples.is_empty() {
        with_db(move |conn| insert_host(conn, &samples)).await?;
    }
    Ok(())
}

async fn retention_tick(state: SharedState) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(&state).await;
    let overrides = sqlx::query(
        "SELECT id, metrics_retention_days FROM organizations WHERE metrics_retention_days IS NOT NULL",
    )
    .fetch_all(&conn)
    .await?
    .iter()
    .map(|row| Ok((row.try_get(0)?, row.try_get(1)?)))
    .collect::<Result<Vec<(String, i64)>, sqlx::Error>>()?;
    let default_days = default_retention_days();
    let now_ms = chrono::Utc::now().timestamp_millis();
    let deleted =
        with_db(move |conn| apply_retention(conn, &overrides, default_days, now_ms)).await?;
    if deleted > 0 {
        tracing::info!(deleted, "pruned container metrics history");
    }
    Ok(())
}

/// Opens the DuckDB file and starts the sampler and the retention job. Any failure
/// here or in a tick is logged; the API keeps running without history.
pub fn spawn(state: SharedState) {
    tokio::spawn(async move {
        let path = db_path();
        // A self-update starts this container while the old one still runs and holds
        // the file lock until it is retired, so the first opens can fail: keep retrying.
        let mut attempt = 0;
        let conn = loop {
            let p = path.clone();
            match tokio::task::spawn_blocking(move || open(Some(&p))).await {
                Ok(Ok(conn)) => break conn,
                Ok(Err(error)) if attempt < OPEN_RETRIES => {
                    if attempt == 0 {
                        tracing::info!(path = %path.display(), %error, "DuckDB busy, retrying open");
                    }
                }
                Ok(Err(error)) => {
                    tracing::warn!(path = %path.display(), %error, "metrics history disabled: could not open DuckDB");
                    return;
                }
                Err(error) => {
                    tracing::warn!(%error, "metrics history disabled: DuckDB open panicked");
                    return;
                }
            }
            attempt += 1;
            tokio::time::sleep(OPEN_RETRY_DELAY).await;
        };
        let _ = DB.set(Mutex::new(conn));
        start(state);
    });
}

const OPEN_RETRIES: u32 = 60;
const OPEN_RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(5);

fn start(state: SharedState) {
    requests::spawn(state.clone());
    let sampler_state = state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(sample_interval());
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            // Spawned so a panic inside a tick is caught here instead of ending the loop.
            match tokio::spawn(sample_tick(sampler_state.clone())).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => tracing::warn!(%error, "metrics sampler tick failed"),
                Err(error) => tracing::warn!(%error, "metrics sampler tick panicked"),
            }
        }
    });
    let host_state = state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(HOST_SAMPLE_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            match tokio::spawn(host_tick(host_state.clone())).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => tracing::warn!(%error, "host sampler tick failed"),
                Err(error) => tracing::warn!(%error, "host sampler tick panicked"),
            }
        }
    });
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(RETENTION_TICK);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            match tokio::spawn(retention_tick(state.clone())).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => tracing::warn!(%error, "metrics retention tick failed"),
                Err(error) => tracing::warn!(%error, "metrics retention tick panicked"),
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct HistoryQuery {
    #[serde(default)]
    pub range: Range,
    /// Epoch milliseconds; together they override `range` with an explicit window.
    pub from: Option<i64>,
    pub to: Option<i64>,
}

impl HistoryQuery {
    fn window(&self) -> Result<Window, AppError> {
        if let (Some(from), Some(to)) = (self.from, self.to)
            && from >= to
        {
            return Err(AppError::ValidationError(
                "`from` must be before `to`".into(),
            ));
        }
        Ok(Window {
            range: self.range,
            from: self.from,
            to: self.to,
        })
    }
}

/// Same permission as the live container metrics of this application.
pub async fn application_history_handler(
    RequireApp(app, _): RequireApp<perm::AppRead>,
    Query(query): Query<HistoryQuery>,
) -> Result<impl IntoResponse, AppError> {
    let application_id = app.application_id;
    let window = query.window()?;
    let now_ms = chrono::Utc::now().timestamp_millis();
    Ok(ok(with_db(move |conn| {
        history(conn, Scope::Application(&application_id), window, now_ms)
    })
    .await?))
}

/// Same permission and tenancy check as the server's live metrics stream.
pub async fn server_history_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerRead>,
    Path((_, server_id)): Path<(String, String)>,
    Query(query): Query<HistoryQuery>,
) -> Result<impl IntoResponse, AppError> {
    repository::get_for_server_route(&state, &server_id, &org.organization_id)
        .await?
        .ok_or(AppError::ResourceNotFound)?;
    let window = query.window()?;
    let now_ms = chrono::Utc::now().timestamp_millis();
    Ok(ok(with_db(move |conn| {
        history(conn, Scope::Server(&server_id), window, now_ms)
    })
    .await?))
}

#[derive(Debug, Serialize)]
pub struct MetricsRetentionDto {
    pub retention_days: i64,
    pub default_days: i64,
    pub max_days: i64,
}

#[derive(Debug, Deserialize)]
pub struct UpdateMetricsRetentionRequest {
    pub retention_days: i64,
}

async fn read_retention(
    state: &SharedState,
    organization_id: &str,
) -> Result<MetricsRetentionDto, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let days: Option<i64> =
        sqlx::query_scalar("SELECT metrics_retention_days FROM organizations WHERE id = ?1")
            .bind(organization_id)
            .fetch_optional(&conn)
            .await?
            .ok_or(AppError::OrganizationNotFound)?;
    let default_days = default_retention_days();
    Ok(MetricsRetentionDto {
        retention_days: days.unwrap_or(default_days),
        default_days,
        max_days: MAX_RETENTION_DAYS,
    })
}

pub async fn get_retention_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgRead>,
) -> Result<impl IntoResponse, AppError> {
    Ok(ok(read_retention(&state, &org.organization_id).await?))
}

/// Same permission and bounds as the audit log retention (`org.update`, 1..=365).
pub async fn update_retention_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgUpdate>,
    Json(request): Json<UpdateMetricsRetentionRequest>,
) -> Result<impl IntoResponse, AppError> {
    let days = validate_retention_days(request.retention_days)?;
    let conn = crate::git::repositories::db::get_conn(&state).await;
    sqlx::query("UPDATE organizations SET metrics_retention_days = ?1 WHERE id = ?2")
        .bind(days)
        .bind(&org.organization_id)
        .execute(&conn)
        .await?;
    audit_log::record(
        &state,
        &org.organization_id,
        &org.user,
        "organization.updated",
        Some("organization"),
        Some(&org.organization_id),
        None,
        Some(&serde_json::json!({ "metrics_retention_days": days }).to_string()),
        None,
        None,
        None,
    )
    .await
    .ok();
    Ok(ok(read_retention(&state, &org.organization_id).await?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::podman::podman_metrics::ContainerMetric;
    use chrono::{TimeZone, Utc};

    const NOW: i64 = 1_790_000_000_000;
    const MINUTE: i64 = 60_000;
    const DAY: i64 = 24 * 60 * MINUTE;

    fn sample(ts_ms: i64, org: Option<&str>, app: Option<&str>, name: &str, cpu: f64) -> Sample {
        Sample {
            ts_ms,
            organization_id: org.map(str::to_owned),
            server_id: "server-1".into(),
            application_id: app.map(str::to_owned),
            container_name: name.into(),
            cpu_percent: Some(cpu),
            mem_bytes: Some(1_000),
            mem_limit_bytes: Some(4_000),
        }
    }

    fn preset(range: Range) -> Window {
        Window {
            range,
            ..Window::default()
        }
    }

    fn count(conn: &Connection) -> i64 {
        conn.query_row("SELECT count(*) FROM container_samples", [], |row| {
            row.get(0)
        })
        .unwrap()
    }

    #[test]
    fn table_usage_lists_every_table_with_its_rows() {
        let conn = open(None).unwrap();
        insert(&conn, &[sample(NOW, Some("org"), Some("app"), "web", 1.0)]).unwrap();
        let usage = table_usage(&conn).unwrap();
        let samples = usage
            .iter()
            .find(|t| t.name == "container_samples")
            .unwrap();
        assert_eq!(samples.rows, 1);
        // In memory nothing is checkpointed to blocks.
        assert_eq!(samples.bytes, 0);
        assert!(usage.iter().any(|t| t.name == "request_rollups"));
    }

    #[test]
    fn sample_counters_cover_empty_and_filled_stores() {
        let conn = open(None).unwrap();
        assert_eq!(sample_counters(&conn).unwrap(), (0, None, None, 0));
        insert(
            &conn,
            &[
                sample(NOW - DAY, Some("org"), Some("app"), "web", 1.0),
                sample(NOW, Some("org"), Some("app"), "web", 2.0),
                sample(NOW - MINUTE, None, None, "caddy", 3.0),
            ],
        )
        .unwrap();
        assert_eq!(
            sample_counters(&conn).unwrap(),
            (3, Some(NOW - DAY), Some(NOW), 2)
        );
    }

    #[test]
    fn inserts_and_buckets_per_container() {
        let conn = open(None).unwrap();
        // Two samples inside one 30s bucket (1h range), one in the next bucket.
        let base = (NOW - 10 * MINUTE) / 30_000 * 30_000;
        insert(
            &conn,
            &[
                sample(base, Some("org"), Some("app"), "web", 10.0),
                sample(base + 15_000, Some("org"), Some("app"), "web", 30.0),
                sample(base + 30_000, Some("org"), Some("app"), "web", 50.0),
                sample(base, Some("org"), Some("app"), "worker", 5.0),
                sample(base, Some("org"), Some("other"), "else", 99.0),
            ],
        )
        .unwrap();
        assert_eq!(count(&conn), 5);

        let result = history(&conn, Scope::Application("app"), preset(Range::Hour), NOW).unwrap();
        assert_eq!(result.bucket_seconds, 30);
        assert_eq!(result.containers.len(), 2);
        let web = &result.containers[0];
        assert_eq!(web.container_name, "web");
        assert_eq!(web.points.len(), 2);
        assert_eq!(web.points[0].ts, base);
        assert_eq!(web.points[0].cpu_avg, Some(20.0));
        assert_eq!(web.points[0].cpu_max, Some(30.0));
        assert_eq!(web.points[0].mem_avg, Some(1_000.0));
        assert_eq!(web.points[0].mem_limit, Some(4_000));
        assert_eq!(web.points[1].cpu_avg, Some(50.0));

        let server = history(&conn, Scope::Server("server-1"), preset(Range::Hour), NOW).unwrap();
        assert_eq!(server.containers.len(), 3);
    }

    #[test]
    fn every_range_stays_within_max_points() {
        let conn = open(None).unwrap();
        for range in [
            Range::Hour,
            Range::SixHours,
            Range::Day,
            Range::Week,
            Range::Month,
        ] {
            conn.execute_batch("DELETE FROM container_samples").unwrap();
            let span = range.seconds() * 1_000;
            // One sample every 15s over the whole window, plus older ones outside it.
            let samples: Vec<_> = (0..)
                .map(|step| NOW - span - 60_000 + step * 15_000)
                .take_while(|ts| *ts <= NOW)
                .step_by((range.seconds() / 3_600) as usize)
                .map(|ts| sample(ts, None, Some("app"), "web", 1.0))
                .collect();
            insert(&conn, &samples).unwrap();
            let result = history(&conn, Scope::Application("app"), preset(range), NOW).unwrap();
            let points = result.containers[0].points.len() as i64;
            assert!(points <= MAX_POINTS, "{range:?} returned {points} points");
            assert!(
                points >= MAX_POINTS - 1,
                "{range:?} returned {points} points"
            );
        }
    }

    #[test]
    fn explicit_window_and_all_range_bound_the_query() {
        let conn = open(None).unwrap();
        insert(
            &conn,
            &[
                sample(NOW - 10 * DAY, Some("org"), Some("app"), "web", 1.0),
                sample(NOW - 3 * DAY, Some("org"), Some("app"), "web", 2.0),
                sample(NOW - MINUTE, Some("org"), Some("app"), "web", 3.0),
            ],
        )
        .unwrap();
        // [now-4d, now-2d): only the middle sample.
        let window = Window {
            range: Range::Hour,
            from: Some(NOW - 4 * DAY),
            to: Some(NOW - 2 * DAY),
        };
        let result = history(&conn, Scope::Application("app"), window, NOW).unwrap();
        assert_eq!(
            result.bucket_seconds,
            ceil_div(2 * 24 * 3_600, MAX_POINTS - 1)
        );
        let points = &result.containers[0].points;
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].cpu_avg, Some(2.0));
        assert!(result.host.is_empty());

        // `all` starts at the oldest sample, not a year back.
        let all = history(&conn, Scope::Application("app"), preset(Range::All), NOW).unwrap();
        assert_eq!(
            all.bucket_seconds,
            ceil_div(10 * DAY / 1_000, MAX_POINTS - 1)
        );
        assert_eq!(all.containers[0].points.len(), 3);
        assert!(all.from <= NOW - 10 * DAY);
    }

    #[test]
    fn host_samples_bucket_per_server_with_the_largest_capacity() {
        let conn = open(None).unwrap();
        let host = |ts_ms, server: &str, used, total| HostSample {
            ts_ms,
            organization_id: Some("org".into()),
            server_id: server.into(),
            disk_used_bytes: Some(used),
            disk_total_bytes: Some(1_000),
            mem_used_bytes: Some(used),
            mem_total_bytes: Some(total),
            cpu_cores: None,
        };
        let base = (NOW - 10 * MINUTE) / 30_000 * 30_000;
        insert_host(
            &conn,
            &[
                // Resized mid-bucket: 2 GB, then 4 GB.
                host(base, "server-1", 100, 2_000),
                host(base + 10_000, "server-1", 300, 4_000),
                host(base, "server-2", 900, 8_000),
            ],
        )
        .unwrap();
        let result = history(&conn, Scope::Server("server-1"), preset(Range::Hour), NOW).unwrap();
        assert_eq!(
            result.host,
            vec![HostPoint {
                ts: base,
                disk_used: Some(200.0),
                disk_total: Some(1_000),
                mem_used: Some(200.0),
                mem_total: Some(4_000),
                cpu_cores: None,
            }]
        );
    }

    #[test]
    fn host_probe_parses_each_part() {
        let output = "Filesystem     1024-blocks    Used Available Capacity Mounted on\n\
                      /dev/vda1         41152736 8000000  31000000      21% /\n---\n4\n---\n\
                      MemTotal:        8000000 kB\nMemFree:  100 kB\nMemAvailable:    6000000 kB\n";
        assert_eq!(
            parse_host_probe(output),
            (
                Some((8_000_000 * 1024, 39_000_000 * 1024)),
                Some(4),
                Some((2_000_000 * 1024, 8_000_000 * 1024))
            )
        );
        // macOS dev host: no /proc/meminfo.
        let (disk, _, memory) = parse_host_probe("garbage\n---\n\n---\n");
        assert_eq!((disk, memory), (None, None));
    }

    #[test]
    fn retention_is_per_organization_with_a_default() {
        let conn = open(None).unwrap();
        insert(
            &conn,
            &[
                // org-short keeps 1 day.
                sample(NOW - 2 * DAY, Some("org-short"), None, "a", 1.0),
                sample(NOW - DAY / 2, Some("org-short"), None, "a", 1.0),
                // org-long keeps 30 days.
                sample(NOW - 20 * DAY, Some("org-long"), None, "b", 1.0),
                sample(NOW - 40 * DAY, Some("org-long"), None, "b", 1.0),
                // No override and no organization: the 7 day default.
                sample(NOW - 5 * DAY, Some("org-default"), None, "c", 1.0),
                sample(NOW - 8 * DAY, Some("org-default"), None, "c", 1.0),
                sample(NOW - 8 * DAY, None, None, "d", 1.0),
            ],
        )
        .unwrap();
        let deleted = apply_retention(
            &conn,
            &[("org-short".into(), 1), ("org-long".into(), 30)],
            7,
            NOW,
        )
        .unwrap();
        assert_eq!(deleted, 4);
        let mut remaining = conn
            .prepare("SELECT container_name, epoch_ms(ts) FROM container_samples ORDER BY 1")
            .unwrap()
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>();
        remaining.sort();
        assert_eq!(
            remaining,
            vec![
                ("a".into(), NOW - DAY / 2),
                ("b".into(), NOW - 20 * DAY),
                ("c".into(), NOW - 5 * DAY)
            ]
        );
    }

    #[test]
    fn retention_bounds_are_validated() {
        assert!(validate_retention_days(0).is_err());
        assert!(validate_retention_days(-3).is_err());
        assert!(validate_retention_days(366).is_err());
        assert_eq!(validate_retention_days(1).unwrap(), 1);
        assert_eq!(validate_retention_days(365).unwrap(), 365);
    }

    #[test]
    fn samples_map_running_containers_to_their_application() {
        let at = Utc.timestamp_millis_opt(NOW).unwrap();
        let metric = |name: &str, state: ContainerMetricState| ContainerMetric {
            id: name.into(),
            name: name.into(),
            state,
            cpu_percent: Some(1.0),
            memory_usage_bytes: Some(10),
            memory_limit_bytes: None,
            observed_at: at,
        };
        let snapshot = ContainerMetricsSnapshot {
            observed_at: at,
            metrics: vec![
                metric("shop-api-web", ContainerMetricState::Running),
                metric("shop-api2-web", ContainerMetricState::Running),
                metric("solo", ContainerMetricState::Running),
                metric("stopped", ContainerMetricState::NotRunning),
            ],
        };
        let owners = [
            Owner {
                application_id: "compose-app".into(),
                organization_id: "org-a".into(),
                compose: true,
                container_name: "shop-api".into(),
            },
            Owner {
                application_id: "solo-app".into(),
                organization_id: "org-b".into(),
                compose: false,
                container_name: "solo".into(),
            },
        ];
        let samples = to_samples("server-1", Some("org-server"), &snapshot, &owners);
        assert_eq!(samples.len(), 3);
        assert_eq!(samples[0].application_id.as_deref(), Some("compose-app"));
        assert_eq!(samples[0].organization_id.as_deref(), Some("org-a"));
        // A sibling app's prefix does not match; it falls back to the server's org.
        assert_eq!(samples[1].application_id, None);
        assert_eq!(samples[1].organization_id.as_deref(), Some("org-server"));
        assert_eq!(samples[2].application_id.as_deref(), Some("solo-app"));
        assert_eq!(samples[0].ts_ms, NOW);
    }

    #[test]
    fn database_path_sits_next_to_sqlite() {
        // Default DATABASE_PATH is ./data/litepod.db.
        if std::env::var("DATABASE_PATH").is_err() {
            assert_eq!(db_path(), std::path::Path::new("./data/metrics.duckdb"));
        }
    }
}
