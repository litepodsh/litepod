//! HTTP request analytics from the central Caddy's access logs.
//!
//! Every `edit_caddy_config` round trip (the 30s route recovery included) makes sure
//! Caddy writes its access log as JSON lines to a `net` writer pointed at a small TCP
//! listener here. Lines are mapped to an application by host (`application_domains`),
//! summed in memory per minute and flushed to DuckDB once a minute: one row per
//! (minute, application, host, status) plus a bounded per-minute top-paths table.
//! Retention rides along with `container_samples` (see `apply_retention`).
//!
//! Env: `LITEPOD_ACCESS_LOG_LISTEN` (default `[::]:6514`, `off` disables),
//! `LITEPOD_ACCESS_LOG_DIAL` (what Caddy dials; default: the host of
//! `CADDY_SERVER_UPSTREAM` with the listen port, e.g. `litepod-api:6514` in prod,
//! `host.containers.internal:6514` in dev).

use std::{
    collections::{HashMap, HashSet},
    net::IpAddr,
    sync::{
        LazyLock, Mutex, OnceLock, RwLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use axum::{
    extract::{Query, State},
    response::IntoResponse,
};
use duckdb::{Connection, params, params_from_iter};
use serde::{Deserialize, Serialize};
use sqlx::Row;

use super::with_db;
use crate::{
    dtos::response::{AppError, ok},
    security::authz::{
        Permission, Target,
        extract::{RequireApp, RequireOrg, perm},
    },
    state::state::SharedState,
};

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS request_rollups (
    ts TIMESTAMP NOT NULL,
    organization_id VARCHAR NOT NULL,
    application_id VARCHAR NOT NULL,
    -- NULL: the central Caddy; otherwise the public-edge server whose Caddy served it.
    server_id VARCHAR,
    host VARCHAR NOT NULL,
    status SMALLINT NOT NULL,
    requests BIGINT NOT NULL,
    bytes_out BIGINT NOT NULL,
    bytes_in BIGINT NOT NULL,
    duration_sum_ms DOUBLE NOT NULL,
    duration_max_ms DOUBLE NOT NULL,
    lat_0 BIGINT NOT NULL, lat_1 BIGINT NOT NULL, lat_2 BIGINT NOT NULL,
    lat_3 BIGINT NOT NULL, lat_4 BIGINT NOT NULL, lat_5 BIGINT NOT NULL,
    lat_6 BIGINT NOT NULL, lat_7 BIGINT NOT NULL, lat_8 BIGINT NOT NULL
);
CREATE TABLE IF NOT EXISTS request_paths (
    ts TIMESTAMP NOT NULL,
    organization_id VARCHAR NOT NULL,
    application_id VARCHAR NOT NULL,
    path VARCHAR NOT NULL,
    requests BIGINT NOT NULL,
    errors BIGINT NOT NULL,
    duration_sum_ms DOUBLE NOT NULL
);";

/// Tables `apply_retention` prunes alongside `container_samples`.
pub const TABLES: [&str; 2] = ["request_rollups", "request_paths"];

/// Upper bounds (ms) of the latency histogram columns `lat_0..lat_8`; the last is open.
const LATENCY_BOUNDS_MS: [f64; 9] = [
    10.0,
    25.0,
    50.0,
    100.0,
    250.0,
    500.0,
    1_000.0,
    2_500.0,
    f64::INFINITY,
];
const MINUTE_MS: i64 = 60_000;
const FLUSH_EVERY: Duration = Duration::from_secs(60);
/// ponytail: per-minute cap on distinct paths held in memory and kept per application;
/// raise or move to a sketch if real top-N accuracy matters.
const MAX_PATH_KEYS: usize = 20_000;
const PATHS_PER_APP_MINUTE: usize = 20;
const MAX_PATH_LEN: usize = 160;
const TOP_LIMIT: usize = 10;
const ACCESS_LOGGER: &str = "litepod_access";

// ---------------------------------------------------------------------------
// Caddy config
// ---------------------------------------------------------------------------

/// Set once the listener is bound; `edit_caddy_config` adds the access log only then,
/// so tests and installs with analytics off never see it.
static DIAL: OnceLock<String> = OnceLock::new();

pub fn access_log_dial() -> Option<&'static str> {
    DIAL.get().map(String::as_str)
}

/// Idempotent: routes every HTTP server's access log to one JSON `net` log that dials
/// `dial`. `soft_start` keeps a `/load` from failing while the API is down.
pub fn ensure_access_log(config: &mut serde_json::Value, dial: &str) {
    let Some(root) = config.as_object_mut() else {
        return;
    };
    if let Some(servers) = root
        .get_mut("apps")
        .and_then(|apps| apps.pointer_mut("/http/servers"))
        .and_then(serde_json::Value::as_object_mut)
    {
        for server in servers.values_mut() {
            if let Some(server) = server.as_object_mut() {
                let logs = server
                    .entry("logs")
                    .or_insert_with(|| serde_json::json!({}));
                if let Some(logs) = logs.as_object_mut() {
                    logs.insert("default_logger_name".into(), ACCESS_LOGGER.into());
                }
            }
        }
    }
    let logging = root
        .entry("logging")
        .or_insert_with(|| serde_json::json!({}));
    let Some(logging) = logging.as_object_mut() else {
        return;
    };
    let logs = logging
        .entry("logs")
        .or_insert_with(|| serde_json::json!({}));
    if let Some(logs) = logs.as_object_mut() {
        logs.insert(
            ACCESS_LOGGER.into(),
            serde_json::json!({
                "writer": { "output": "net", "address": dial, "dial_timeout": "2s", "soft_start": true },
                "encoder": { "format": "json" },
                "include": [format!("http.log.access.{ACCESS_LOGGER}")]
            }),
        );
    }
}

// ---------------------------------------------------------------------------
// Parsing and in-memory aggregation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
struct AccessLine {
    ts_ms: i64,
    host: String,
    path: String,
    status: i64,
    duration_ms: f64,
    bytes_out: i64,
    bytes_in: i64,
}

fn parse_line(line: &str, now_ms: i64) -> Option<AccessLine> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    if !value["logger"]
        .as_str()
        .is_some_and(|logger| logger.starts_with("http.log.access"))
    {
        return None;
    }
    let request = &value["request"];
    let host = request["host"].as_str()?;
    // Strip a port; IPv6 literals keep their brackets.
    let host = match host.rsplit_once(':') {
        Some((name, port)) if !name.ends_with(':') && port.chars().all(|c| c.is_ascii_digit()) => {
            name
        }
        _ => host,
    };
    let uri = request["uri"].as_str().unwrap_or("/");
    let mut path: String = uri.split(['?', '#']).next().unwrap_or("/").to_string();
    if path.len() > MAX_PATH_LEN {
        let mut cut = MAX_PATH_LEN;
        while !path.is_char_boundary(cut) {
            cut -= 1;
        }
        path.truncate(cut);
    }
    Some(AccessLine {
        ts_ms: value["ts"]
            .as_f64()
            .map(|seconds| (seconds * 1_000.0) as i64)
            .unwrap_or(now_ms),
        host: host.to_ascii_lowercase(),
        path,
        // Caddy logs 0 when no handler wrote a header; the client got its implicit 200.
        status: value["status"]
            .as_i64()
            .filter(|status| *status > 0)
            .unwrap_or(200),
        duration_ms: value["duration"].as_f64().unwrap_or(0.0) * 1_000.0,
        bytes_out: value["size"].as_i64().unwrap_or(0),
        bytes_in: value["bytes_read"].as_i64().unwrap_or(0),
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Owner {
    organization_id: String,
    application_id: String,
}

#[derive(Debug, Default, Clone, PartialEq)]
struct Rollup {
    requests: i64,
    bytes_out: i64,
    bytes_in: i64,
    duration_sum_ms: f64,
    duration_max_ms: f64,
    latency: [i64; 9],
}

#[derive(Debug, Default, Clone, PartialEq)]
struct PathRollup {
    requests: i64,
    errors: i64,
    duration_sum_ms: f64,
}

type RollupKey = (i64, Owner, Option<String>, String, i64);
type PathKey = (i64, Owner, String);
type Drained = (Vec<(RollupKey, Rollup)>, Vec<(PathKey, PathRollup)>);

#[derive(Debug, Default)]
struct Aggregator {
    rollups: HashMap<RollupKey, Rollup>,
    paths: HashMap<PathKey, PathRollup>,
}

impl Aggregator {
    fn record(&mut self, line: AccessLine, owner: Owner, server_id: Option<&str>) {
        let minute = line.ts_ms.div_euclid(MINUTE_MS) * MINUTE_MS;
        let rollup = self
            .rollups
            .entry((
                minute,
                owner.clone(),
                server_id.map(str::to_owned),
                line.host,
                line.status,
            ))
            .or_default();
        rollup.requests += 1;
        rollup.bytes_out += line.bytes_out;
        rollup.bytes_in += line.bytes_in;
        rollup.duration_sum_ms += line.duration_ms;
        rollup.duration_max_ms = rollup.duration_max_ms.max(line.duration_ms);
        let bucket = LATENCY_BOUNDS_MS
            .iter()
            .position(|bound| line.duration_ms <= *bound)
            .unwrap_or(LATENCY_BOUNDS_MS.len() - 1);
        rollup.latency[bucket] += 1;

        let key = (minute, owner, line.path);
        if self.paths.len() >= MAX_PATH_KEYS && !self.paths.contains_key(&key) {
            return;
        }
        let path = self.paths.entry(key).or_default();
        path.requests += 1;
        path.errors += i64::from(line.status >= 500);
        path.duration_sum_ms += line.duration_ms;
    }

    /// Everything so far; partial minutes are fine because every query sums.
    fn drain(&mut self) -> Drained {
        let rollups = self.rollups.drain().collect();
        let mut by_app: HashMap<(i64, Owner), Vec<(String, PathRollup)>> = HashMap::new();
        for ((minute, owner, path), rollup) in self.paths.drain() {
            by_app
                .entry((minute, owner))
                .or_default()
                .push((path, rollup));
        }
        let mut paths = Vec::new();
        for ((minute, owner), mut rows) in by_app {
            rows.sort_by(|a, b| b.1.requests.cmp(&a.1.requests).then(a.0.cmp(&b.0)));
            rows.truncate(PATHS_PER_APP_MINUTE);
            paths.extend(
                rows.into_iter()
                    .map(|(path, rollup)| ((minute, owner.clone(), path), rollup)),
            );
        }
        (rollups, paths)
    }
}

static AGGREGATOR: LazyLock<Mutex<Aggregator>> = LazyLock::new(Mutex::default);
/// Lowercased host -> owner, refreshed every flush.
static HOSTS: LazyLock<RwLock<HashMap<String, Owner>>> = LazyLock::new(RwLock::default);

fn ingest(line: &str, now_ms: i64, server_id: Option<&str>) {
    let Some(line) = parse_line(line, now_ms) else {
        return;
    };
    // Unknown hosts (the console itself, scanners) are dropped here so memory stays bounded.
    let Some(owner) = HOSTS
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&line.host)
        .cloned()
    else {
        return;
    };
    AGGREGATOR
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .record(line, owner, server_id);
}

// ---------------------------------------------------------------------------
// Storage (pure, sync; tested against an in-memory DuckDB)
// ---------------------------------------------------------------------------

fn ts(ms: i64) -> duckdb::types::Value {
    duckdb::types::Value::Timestamp(duckdb::types::TimeUnit::Millisecond, ms)
}

fn insert(
    conn: &Connection,
    rollups: &[(RollupKey, Rollup)],
    paths: &[(PathKey, PathRollup)],
) -> duckdb::Result<()> {
    let mut appender = conn.appender("request_rollups")?;
    for ((minute, owner, server_id, host, status), r) in rollups {
        let l = &r.latency;
        appender.append_row(params![
            ts(*minute),
            owner.organization_id,
            owner.application_id,
            server_id,
            host,
            status,
            r.requests,
            r.bytes_out,
            r.bytes_in,
            r.duration_sum_ms,
            r.duration_max_ms,
            l[0],
            l[1],
            l[2],
            l[3],
            l[4],
            l[5],
            l[6],
            l[7],
            l[8],
        ])?;
    }
    appender.flush()?;
    let mut appender = conn.appender("request_paths")?;
    for ((minute, owner, path), r) in paths {
        appender.append_row(params![
            ts(*minute),
            owner.organization_id,
            owner.application_id,
            path,
            r.requests,
            r.errors,
            r.duration_sum_ms,
        ])?;
    }
    appender.flush()
}

pub fn request_row_count(conn: &Connection) -> duckdb::Result<i64> {
    conn.query_row("SELECT count(*) FROM request_rollups", [], |row| row.get(0))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
pub enum AnalyticsRange {
    #[serde(rename = "1h")]
    Hour,
    #[default]
    #[serde(rename = "24h")]
    Day,
    #[serde(rename = "7d")]
    Week,
    #[serde(rename = "30d")]
    Month,
}

impl AnalyticsRange {
    /// (window, bucket) in seconds: 60, 48, 56 and 60 points.
    const fn spans(self) -> (i64, i64) {
        match self {
            Self::Hour => (3_600, 60),
            Self::Day => (86_400, 1_800),
            Self::Week => (7 * 86_400, 3 * 3_600),
            Self::Month => (30 * 86_400, 12 * 3_600),
        }
    }
}

#[derive(Debug, Default, Serialize, PartialEq)]
pub struct AnalyticsPoint {
    /// Bucket start, epoch milliseconds.
    pub ts: i64,
    pub requests: i64,
    pub errors_4xx: i64,
    pub errors_5xx: i64,
    pub bytes_out: i64,
    pub avg_ms: Option<f64>,
    pub p95_ms: Option<f64>,
    /// Requests per application in this bucket, busiest first (organization view only).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub applications: Vec<BucketApplication>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct BucketApplication {
    pub key: String,
    pub requests: i64,
}

#[derive(Debug, Default, Serialize, PartialEq)]
pub struct AnalyticsTotals {
    pub requests: i64,
    pub errors_4xx: i64,
    pub errors_5xx: i64,
    pub bytes_out: i64,
    pub bytes_in: i64,
    pub avg_ms: Option<f64>,
    pub p95_ms: Option<f64>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct StatusCount {
    pub status: i64,
    pub requests: i64,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct TopEntry {
    /// Host, path or application id, depending on the list.
    pub key: String,
    pub requests: i64,
    pub errors: i64,
    pub bytes_out: Option<i64>,
    pub avg_ms: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct AnalyticsDto {
    pub range: AnalyticsRange,
    pub bucket_seconds: i64,
    pub series: Vec<AnalyticsPoint>,
    pub totals: AnalyticsTotals,
    pub statuses: Vec<StatusCount>,
    pub hosts: Vec<TopEntry>,
    pub paths: Vec<TopEntry>,
    /// Per-application breakdown (organization view only; empty for one application).
    pub applications: Vec<TopEntry>,
}

/// ponytail: p95 from a fixed 9-bucket histogram, reported as the bucket's upper bound
/// (capped by the real max); store a t-digest if exact percentiles are ever needed.
fn p95(latency: &[i64; 9], max_ms: f64) -> Option<f64> {
    let total: i64 = latency.iter().sum();
    if total == 0 {
        return None;
    }
    let target = (total as f64 * 0.95).ceil() as i64;
    let mut seen = 0;
    for (count, bound) in latency.iter().zip(LATENCY_BOUNDS_MS) {
        seen += count;
        if seen >= target {
            return Some(bound.min(max_ms));
        }
    }
    Some(max_ms)
}

/// `organization_id = ? AND application_id IN (?, ...) AND ts >= ?` plus its params.
fn scope_filter(
    organization_id: &str,
    applications: &[String],
    since_ms: i64,
) -> (String, Vec<duckdb::types::Value>) {
    let marks = vec!["?"; applications.len()].join(", ");
    let mut values = vec![duckdb::types::Value::Text(organization_id.to_string())];
    values.extend(
        applications
            .iter()
            .map(|id| duckdb::types::Value::Text(id.clone())),
    );
    values.push(ts(since_ms));
    (
        format!("organization_id = ? AND application_id IN ({marks}) AND ts >= ?"),
        values,
    )
}

fn analytics(
    conn: &Connection,
    organization_id: &str,
    applications: &[String],
    range: AnalyticsRange,
    with_applications: bool,
    now_ms: i64,
) -> duckdb::Result<AnalyticsDto> {
    let (window, bucket_seconds) = range.spans();
    let bucket_ms = bucket_seconds * 1_000;
    let since_ms = ((now_ms - window * 1_000) / bucket_ms + 1) * bucket_ms;
    let mut dto = AnalyticsDto {
        range,
        bucket_seconds,
        series: (0..)
            .map(|step| since_ms + step * bucket_ms)
            .take_while(|ts| *ts <= now_ms)
            .map(|ts| AnalyticsPoint {
                ts,
                ..Default::default()
            })
            .collect(),
        totals: AnalyticsTotals::default(),
        statuses: Vec::new(),
        hosts: Vec::new(),
        paths: Vec::new(),
        applications: Vec::new(),
    };
    if applications.is_empty() {
        return Ok(dto);
    }
    let (filter, values) = scope_filter(organization_id, applications, since_ms);

    let mut total_latency = [0i64; 9];
    let mut total_duration = 0.0;
    let mut total_max = 0.0f64;
    let sql = format!(
        "SELECT epoch_ms(time_bucket(to_seconds(?::BIGINT), ts, TIMESTAMP '1970-01-01')) AS bucket,
                sum(requests), sum(requests) FILTER (status BETWEEN 400 AND 499),
                sum(requests) FILTER (status >= 500), sum(bytes_out), sum(bytes_in),
                sum(duration_sum_ms), max(duration_max_ms),
                sum(lat_0), sum(lat_1), sum(lat_2), sum(lat_3), sum(lat_4),
                sum(lat_5), sum(lat_6), sum(lat_7), sum(lat_8)
         FROM request_rollups WHERE {filter} GROUP BY bucket"
    );
    let mut bucket_values = vec![duckdb::types::Value::BigInt(bucket_seconds)];
    bucket_values.extend(values.iter().cloned());
    let mut statement = conn.prepare(&sql)?;
    let mut rows = statement.query(params_from_iter(bucket_values))?;
    while let Some(row) = rows.next()? {
        let bucket: i64 = row.get(0)?;
        let requests: i64 = row.get::<_, Option<i64>>(1)?.unwrap_or(0);
        let errors_4xx = row.get::<_, Option<i64>>(2)?.unwrap_or(0);
        let errors_5xx = row.get::<_, Option<i64>>(3)?.unwrap_or(0);
        let bytes_out = row.get::<_, Option<i64>>(4)?.unwrap_or(0);
        let bytes_in = row.get::<_, Option<i64>>(5)?.unwrap_or(0);
        let duration: f64 = row.get::<_, Option<f64>>(6)?.unwrap_or(0.0);
        let max_ms: f64 = row.get::<_, Option<f64>>(7)?.unwrap_or(0.0);
        let mut latency = [0i64; 9];
        for (index, slot) in latency.iter_mut().enumerate() {
            *slot = row.get::<_, Option<i64>>(8 + index)?.unwrap_or(0);
        }
        dto.totals.requests += requests;
        dto.totals.errors_4xx += errors_4xx;
        dto.totals.errors_5xx += errors_5xx;
        dto.totals.bytes_out += bytes_out;
        dto.totals.bytes_in += bytes_in;
        total_duration += duration;
        total_max = total_max.max(max_ms);
        for (sum, count) in total_latency.iter_mut().zip(latency) {
            *sum += count;
        }
        if let Some(point) = dto.series.iter_mut().find(|point| point.ts == bucket) {
            point.requests = requests;
            point.errors_4xx = errors_4xx;
            point.errors_5xx = errors_5xx;
            point.bytes_out = bytes_out;
            point.avg_ms = (requests > 0).then(|| duration / requests as f64);
            point.p95_ms = p95(&latency, max_ms);
        }
    }
    drop(rows);
    if dto.totals.requests > 0 {
        dto.totals.avg_ms = Some(total_duration / dto.totals.requests as f64);
        dto.totals.p95_ms = p95(&total_latency, total_max);
    }

    let mut statement = conn.prepare(&format!(
        "SELECT status, sum(requests) AS n FROM request_rollups WHERE {filter}
         GROUP BY status ORDER BY n DESC, status LIMIT {TOP_LIMIT}"
    ))?;
    dto.statuses = statement
        .query_map(params_from_iter(values.iter()), |row| {
            Ok(StatusCount {
                status: row.get(0)?,
                requests: row.get(1)?,
            })
        })?
        .collect::<duckdb::Result<_>>()?;

    let top = |column: &str, limit: Option<usize>| -> duckdb::Result<Vec<TopEntry>> {
        let limit = limit.map(|n| format!("LIMIT {n}")).unwrap_or_default();
        let mut statement = conn.prepare(&format!(
            "SELECT {column}, sum(requests) AS n, coalesce(sum(requests) FILTER (status >= 500), 0),
                    sum(bytes_out), sum(duration_sum_ms) / sum(requests)
             FROM request_rollups WHERE {filter}
             GROUP BY {column} ORDER BY n DESC, {column} {limit}"
        ))?;
        statement
            .query_map(params_from_iter(values.iter()), |row| {
                Ok(TopEntry {
                    key: row.get(0)?,
                    requests: row.get(1)?,
                    errors: row.get(2)?,
                    bytes_out: row.get(3)?,
                    avg_ms: row.get(4)?,
                })
            })?
            .collect()
    };
    dto.hosts = top("host", Some(TOP_LIMIT))?;
    if with_applications {
        dto.applications = top("application_id", None)?;
        let mut bucket_values = vec![duckdb::types::Value::BigInt(bucket_seconds)];
        bucket_values.extend(values.iter().cloned());
        let mut statement = conn.prepare(&format!(
            "SELECT epoch_ms(time_bucket(to_seconds(?::BIGINT), ts, TIMESTAMP '1970-01-01')) AS bucket,
                    application_id, sum(requests) AS n
             FROM request_rollups WHERE {filter}
             GROUP BY bucket, application_id ORDER BY bucket, n DESC, application_id"
        ))?;
        let rows = statement.query_map(params_from_iter(bucket_values), |row| {
            Ok((
                row.get::<_, i64>(0)?,
                BucketApplication {
                    key: row.get(1)?,
                    requests: row.get(2)?,
                },
            ))
        })?;
        for row in rows {
            let (bucket, entry) = row?;
            if let Some(point) = dto.series.iter_mut().find(|point| point.ts == bucket) {
                point.applications.push(entry);
            }
        }
    }

    let mut statement = conn.prepare(&format!(
        "SELECT path, sum(requests) AS n, sum(errors), sum(duration_sum_ms) / sum(requests)
         FROM request_paths WHERE {filter}
         GROUP BY path ORDER BY n DESC, path LIMIT {TOP_LIMIT}"
    ))?;
    dto.paths = statement
        .query_map(params_from_iter(values.iter()), |row| {
            Ok(TopEntry {
                key: row.get(0)?,
                requests: row.get(1)?,
                errors: row.get(2)?,
                bytes_out: None,
                avg_ms: row.get(3)?,
            })
        })?
        .collect::<duckdb::Result<_>>()?;
    Ok(dto)
}

#[derive(Debug, Serialize)]
pub struct SparklinesDto {
    pub bucket_seconds: i64,
    /// First bucket start, epoch milliseconds.
    pub since: i64,
    /// Application id -> requests per bucket, oldest first. Apps without traffic are absent.
    pub applications: HashMap<String, Vec<i64>>,
}

fn sparklines(
    conn: &Connection,
    organization_id: &str,
    applications: &[String],
    now_ms: i64,
) -> duckdb::Result<SparklinesDto> {
    let (window, bucket_seconds) = AnalyticsRange::Day.spans();
    let bucket_ms = bucket_seconds * 1_000;
    let since_ms = ((now_ms - window * 1_000) / bucket_ms + 1) * bucket_ms;
    let points = ((now_ms - since_ms) / bucket_ms + 1) as usize;
    let mut dto = SparklinesDto {
        bucket_seconds,
        since: since_ms,
        applications: HashMap::new(),
    };
    if applications.is_empty() {
        return Ok(dto);
    }
    let (filter, values) = scope_filter(organization_id, applications, since_ms);
    let mut all = vec![duckdb::types::Value::BigInt(bucket_seconds)];
    all.extend(values);
    let mut statement = conn.prepare(&format!(
        "SELECT application_id,
                epoch_ms(time_bucket(to_seconds(?::BIGINT), ts, TIMESTAMP '1970-01-01')) AS bucket,
                sum(requests)
         FROM request_rollups WHERE {filter} GROUP BY application_id, bucket"
    ))?;
    let rows = statement.query_map(params_from_iter(all), |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    for row in rows {
        let (application_id, bucket, requests) = row?;
        let index = ((bucket - since_ms) / bucket_ms) as usize;
        if index < points {
            dto.applications
                .entry(application_id)
                .or_insert_with(|| vec![0; points])[index] = requests;
        }
    }
    Ok(dto)
}

// ---------------------------------------------------------------------------
// Listener and flusher
// ---------------------------------------------------------------------------

async fn refresh_hosts(state: &SharedState) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(
        "SELECT lower(d.host), d.application_id, p.organization_id FROM application_domains d \
         JOIN applications a ON a.id = d.application_id \
         JOIN environments e ON e.id = a.environment_id \
         JOIN projects p ON p.id = e.project_id",
    )
    .fetch_all(&conn)
    .await?;
    let mut hosts = HashMap::with_capacity(rows.len());
    for row in &rows {
        hosts.insert(
            row.try_get::<String, _>(0)?,
            Owner {
                application_id: row.try_get(1)?,
                organization_id: row.try_get(2)?,
            },
        );
    }
    *HOSTS
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = hosts;
    Ok(())
}

async fn flush() -> Result<(), AppError> {
    let (rollups, paths) = AGGREGATOR
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .drain();
    if rollups.is_empty() && paths.is_empty() {
        return Ok(());
    }
    with_db(move |conn| insert(conn, &rollups, &paths)).await
}

/// Caddy's own address(es) plus loopback (dev, where podman forwards from the host).
/// Anything else on the network (a deployed app) could otherwise forge traffic.
async fn trusted_peers(state: &SharedState) -> HashSet<IpAddr> {
    let mut peers = HashSet::new();
    for url in [&state.caddy_admin_url, &state.caddy_url] {
        let Ok(url) = reqwest::Url::parse(url) else {
            continue;
        };
        if let Some(host) = url.host_str()
            && let Ok(addrs) = tokio::net::lookup_host((host, 0)).await
        {
            peers.extend(addrs.map(|addr| addr.ip().to_canonical()));
        }
    }
    peers
}

async fn handle(stream: tokio::net::TcpStream) {
    use tokio::io::AsyncBufReadExt;
    let mut lines = tokio::io::BufReader::new(stream).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        ingest(&line, chrono::Utc::now().timestamp_millis(), None);
    }
}

/// Last (inode, byte offset) read from each public-edge server's access log.
static EDGE_OFFSETS: LazyLock<Mutex<HashMap<String, (u64, u64)>>> = LazyLock::new(Mutex::default);
/// ponytail: bounded read per minute; a server writing more than this per minute falls
/// behind and loses the unread tail when Caddy rolls the file. Raise or pull more often.
const EDGE_READ_MAX: u64 = 4 * 1024 * 1024;

/// What `serverctl access-log-read` printed: `inode size start` then the raw lines.
/// Returns the log's inode, the next offset to ask for, and the complete lines read.
/// On first contact (`known` is None) nothing is ingested: the read starts at the end,
/// so an API restart never counts the same lines twice.
fn parse_edge_read(output: &str, known: Option<(u64, u64)>) -> Option<(u64, u64, Vec<&str>)> {
    let (header, data) = output.split_once('\n').unwrap_or((output, ""));
    let mut fields = header.split_whitespace().map(str::parse::<u64>);
    let (inode, size, start) = (
        fields.next()?.ok()?,
        fields.next()?.ok()?,
        fields.next()?.ok()?,
    );
    if known.is_none() {
        return Some((inode, size, Vec::new()));
    }
    let complete = data.rfind('\n').map_or(0, |index| index + 1);
    Some((
        inode,
        start + complete as u64,
        data[..complete].lines().collect(),
    ))
}

async fn pull_edge(server: &crate::servers::repository::ServerRecord) -> Result<(), AppError> {
    let known = EDGE_OFFSETS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&server.id)
        .copied();
    let (inode, offset, max) =
        known.map_or((0, 0, 0), |(inode, offset)| (inode, offset, EDGE_READ_MAX));
    let output = crate::servers::setup::caddy_helper(
        server,
        &[
            "access-log-read".into(),
            inode.to_string().into(),
            offset.to_string().into(),
            max.to_string().into(),
        ],
    )
    .await?;
    let (inode, next, lines) = parse_edge_read(&output, known)
        .ok_or_else(|| AppError::InternalError("unreadable access-log-read output".into()))?;
    let now_ms = chrono::Utc::now().timestamp_millis();
    for line in lines {
        ingest(line, now_ms, Some(&server.id));
    }
    EDGE_OFFSETS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(server.id.clone(), (inode, next));
    Ok(())
}

/// Public-edge servers run their own Caddy, which writes a rolled JSON access log
/// locally; the API pulls new lines over SSH (the server may not reach the API).
async fn pull_edges(state: &SharedState) {
    use crate::servers::model::{IngressMode, ServerState};
    let Ok(servers) = crate::servers::repository::list_monitorable(state).await else {
        return;
    };
    let edges: Vec<_> = servers
        .into_iter()
        .filter(|server| server.kind == "ssh" && server.state == ServerState::Ready)
        .filter(|server| matches!(server.ingress_mode, IngressMode::PublicEdge))
        .collect();
    let results = futures_util::future::join_all(edges.iter().map(pull_edge)).await;
    for (server, result) in edges.iter().zip(results) {
        if let Err(error) = result {
            // A helper from before access logs exits 64 until the server is repaired.
            tracing::debug!(server = %server.name, %error, "edge access log pull skipped");
        }
    }
}

/// Binds the access-log listener and starts the minute flusher. Called after the DuckDB
/// file opened; any failure is logged and the API runs without request analytics.
pub fn spawn(state: SharedState) {
    let listen = std::env::var("LITEPOD_ACCESS_LOG_LISTEN").unwrap_or_else(|_| "[::]:6514".into());
    if listen.trim().is_empty() || listen.trim() == "off" {
        return;
    }
    let port = listen
        .rsplit_once(':')
        .map(|(_, port)| port)
        .unwrap_or("6514");
    let dial = std::env::var("LITEPOD_ACCESS_LOG_DIAL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            let host = state
                .caddy_server_upstream
                .rsplit_once(':')
                .map(|(host, _)| host)
                .unwrap_or(&state.caddy_server_upstream);
            format!("{host}:{port}")
        });
    let listener_state = state.clone();
    tokio::spawn(async move {
        let listener = match tokio::net::TcpListener::bind(&listen).await {
            Ok(listener) => listener,
            Err(error) => {
                tracing::warn!(%listen, %error, "request analytics disabled: could not bind the access log listener");
                return;
            }
        };
        let _ = DIAL.set(dial.clone());
        tracing::info!(%listen, %dial, "request analytics: listening for Caddy access logs");
        let warned = AtomicBool::new(false);
        loop {
            let Ok((stream, peer)) = listener.accept().await else {
                continue;
            };
            let ip = peer.ip().to_canonical();
            if !ip.is_loopback() && !trusted_peers(&listener_state).await.contains(&ip) {
                if !warned.swap(true, Ordering::Relaxed) {
                    tracing::warn!(%ip, "request analytics: dropped an access log connection from an unknown peer");
                }
                continue;
            }
            tokio::spawn(handle(stream));
        }
    });
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(FLUSH_EVERY);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if let Err(error) = refresh_hosts(&state).await {
                tracing::warn!(%error, "request analytics: host map refresh failed");
            }
            pull_edges(&state).await;
            if let Err(error) = flush().await {
                tracing::warn!(%error, "request analytics flush failed");
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct AnalyticsQuery {
    #[serde(default)]
    pub range: AnalyticsRange,
}

/// Same permission as the application's container metrics.
pub async fn application_analytics_handler(
    RequireApp(app, _): RequireApp<perm::AppRead>,
    Query(query): Query<AnalyticsQuery>,
) -> Result<impl IntoResponse, AppError> {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let organization_id = app.organization_id;
    let applications = vec![app.application_id];
    Ok(ok(with_db(move |conn| {
        analytics(
            conn,
            &organization_id,
            &applications,
            query.range,
            false,
            now_ms,
        )
    })
    .await?))
}

/// The organization's applications this caller may read (`app.read` per environment).
async fn readable_applications(
    state: &SharedState,
    org: &crate::security::middleware::org_scope::OrgScope,
) -> Result<Vec<String>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(
        "SELECT a.id, e.project_id, a.environment_id FROM applications a \
         JOIN environments e ON e.id = a.environment_id \
         JOIN projects p ON p.id = e.project_id WHERE p.organization_id = ?1",
    )
    .bind(&org.organization_id)
    .fetch_all(&conn)
    .await?;
    let mut ids = Vec::new();
    for row in &rows {
        let (id, project_id, environment_id): (String, String, String) =
            (row.try_get(0)?, row.try_get(1)?, row.try_get(2)?);
        if org.permissions.allows(
            Permission::AppRead,
            Target::Environment {
                project_id: &project_id,
                environment_id: &environment_id,
            },
        ) {
            ids.push(id);
        }
    }
    Ok(ids)
}

pub async fn organization_analytics_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgRead>,
    Query(query): Query<AnalyticsQuery>,
) -> Result<impl IntoResponse, AppError> {
    let applications = readable_applications(&state, &org).await?;
    let now_ms = chrono::Utc::now().timestamp_millis();
    let organization_id = org.organization_id;
    Ok(ok(with_db(move |conn| {
        analytics(
            conn,
            &organization_id,
            &applications,
            query.range,
            true,
            now_ms,
        )
    })
    .await?))
}

/// One call per page for every card's 24h requests sparkline.
pub async fn organization_sparklines_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgRead>,
) -> Result<impl IntoResponse, AppError> {
    let applications = readable_applications(&state, &org).await?;
    let now_ms = chrono::Utc::now().timestamp_millis();
    let organization_id = org.organization_id;
    Ok(ok(with_db(move |conn| {
        sparklines(conn, &organization_id, &applications, now_ms)
    })
    .await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_790_000_000_000;

    fn owner(app: &str) -> Owner {
        Owner {
            organization_id: "org".into(),
            application_id: app.into(),
        }
    }

    fn line(ts_ms: i64, host: &str, path: &str, status: i64, duration_ms: f64) -> AccessLine {
        AccessLine {
            ts_ms,
            host: host.into(),
            path: path.into(),
            status,
            duration_ms,
            bytes_out: 100,
            bytes_in: 10,
        }
    }

    fn db() -> Connection {
        super::super::open(None).unwrap()
    }

    #[test]
    fn parses_a_caddy_access_line() {
        let raw = r#"{"level":"info","ts":1790000000.5,"logger":"http.log.access.litepod_access","msg":"handled request","request":{"remote_ip":"1.2.3.4","proto":"HTTP/2.0","method":"GET","host":"Shop.Example.com:8443","uri":"/api/items?page=2","headers":{}},"bytes_read":12,"duration":0.0421,"size":5120,"status":200,"resp_headers":{}}"#;
        let parsed = parse_line(raw, 0).unwrap();
        assert_eq!(parsed.ts_ms, 1_790_000_000_500);
        assert_eq!(parsed.host, "shop.example.com");
        assert_eq!(parsed.path, "/api/items");
        assert_eq!(parsed.status, 200);
        assert!((parsed.duration_ms - 42.1).abs() < 1e-6);
        assert_eq!(parsed.bytes_out, 5120);
        assert_eq!(parsed.bytes_in, 12);
        assert!(parse_line(r#"{"logger":"admin.api","msg":"x"}"#, 0).is_none());
        assert!(parse_line("not json", 0).is_none());
        let ipv6 =
            r#"{"logger":"http.log.access","request":{"host":"[::1]","uri":"/"},"status":204}"#;
        assert_eq!(parse_line(ipv6, NOW).unwrap().host, "[::1]");
        assert_eq!(parse_line(ipv6, NOW).unwrap().ts_ms, NOW);
    }

    #[test]
    fn access_log_config_is_idempotent_and_keeps_routes() {
        let mut config = serde_json::json!({ "apps": { "http": { "servers": {
            "srv0": { "listen": [":80"], "routes": [{ "@id": "r" }] },
            "srv1": { "logs": { "skip_hosts": ["a"] } }
        } } } });
        ensure_access_log(&mut config, "litepod-api:6514");
        let once = config.clone();
        ensure_access_log(&mut config, "litepod-api:6514");
        assert_eq!(config, once);
        assert_eq!(
            config["apps"]["http"]["servers"]["srv0"]["routes"][0]["@id"],
            "r"
        );
        assert_eq!(
            config["apps"]["http"]["servers"]["srv1"]["logs"],
            serde_json::json!({ "skip_hosts": ["a"], "default_logger_name": ACCESS_LOGGER })
        );
        let log = &config["logging"]["logs"][ACCESS_LOGGER];
        assert_eq!(log["writer"]["address"], "litepod-api:6514");
        assert_eq!(log["writer"]["soft_start"], true);
        assert_eq!(log["include"][0], "http.log.access.litepod_access");
    }

    #[test]
    fn p95_reads_the_histogram() {
        assert_eq!(p95(&[0; 9], 0.0), None);
        // 100 requests: 94 under 10ms, 6 in the 250ms bucket.
        let mut latency = [0; 9];
        latency[0] = 94;
        latency[4] = 6;
        assert_eq!(p95(&latency, 240.0), Some(240.0));
        latency[0] = 96;
        latency[4] = 4;
        assert_eq!(p95(&latency, 240.0), Some(10.0));
    }

    #[test]
    fn aggregates_flushes_and_queries() {
        let conn = db();
        let mut aggregator = Aggregator::default();
        let base = NOW - 10 * MINUTE_MS;
        for _ in 0..3 {
            aggregator.record(line(base, "a.test", "/", 200, 5.0), owner("app"), None);
        }
        aggregator.record(
            line(base + 1_000, "a.test", "/boom", 502, 300.0),
            owner("app"),
            None,
        );
        aggregator.record(line(base, "b.test", "/x", 404, 20.0), owner("app"), None);
        aggregator.record(
            line(base, "other.test", "/", 200, 1.0),
            owner("other"),
            None,
        );
        let (rollups, paths) = aggregator.drain();
        assert!(aggregator.rollups.is_empty() && aggregator.paths.is_empty());
        insert(&conn, &rollups, &paths).unwrap();
        // A second flush for the same minute just adds rows; queries sum them.
        aggregator.record(
            line(base + 2_000, "a.test", "/", 200, 5.0),
            owner("app"),
            None,
        );
        let (rollups, paths) = aggregator.drain();
        insert(&conn, &rollups, &paths).unwrap();

        let dto = analytics(
            &conn,
            "org",
            &["app".into()],
            AnalyticsRange::Hour,
            false,
            NOW,
        )
        .unwrap();
        assert_eq!(dto.bucket_seconds, 60);
        assert_eq!(dto.series.len(), 60);
        assert_eq!(dto.totals.requests, 6);
        assert_eq!(dto.totals.errors_4xx, 1);
        assert_eq!(dto.totals.errors_5xx, 1);
        assert_eq!(dto.totals.bytes_out, 600);
        assert_eq!(dto.series.iter().map(|p| p.requests).sum::<i64>(), 6);
        let point = dto.series.iter().find(|p| p.requests > 0).unwrap();
        assert_eq!(point.ts, base / MINUTE_MS * MINUTE_MS);
        assert_eq!(
            dto.statuses[0],
            StatusCount {
                status: 200,
                requests: 4
            }
        );
        assert_eq!(dto.hosts[0].key, "a.test");
        assert_eq!(dto.hosts[0].requests, 5);
        assert_eq!(dto.hosts[0].errors, 1);
        assert_eq!(dto.paths[0].key, "/");
        assert_eq!(dto.paths[0].requests, 4);
        assert!(dto.applications.is_empty());

        let org = analytics(
            &conn,
            "org",
            &["app".into(), "other".into()],
            AnalyticsRange::Day,
            true,
            NOW,
        )
        .unwrap();
        assert_eq!(org.series.len(), 48);
        assert_eq!(org.totals.requests, 7);
        assert_eq!(org.applications.len(), 2);
        assert_eq!(org.applications[0].key, "app");
        // Each bucket splits its requests by application, busiest first.
        let busy = org.series.iter().find(|p| p.requests > 0).unwrap();
        assert_eq!(
            busy.applications.iter().map(|a| a.requests).sum::<i64>(),
            busy.requests
        );
        assert!(dto.series.iter().all(|p| p.applications.is_empty()));
        // Another organization's id never matches.
        let foreign = analytics(
            &conn,
            "else",
            &["app".into()],
            AnalyticsRange::Hour,
            true,
            NOW,
        )
        .unwrap();
        assert_eq!(foreign.totals.requests, 0);
        assert_eq!(request_row_count(&conn).unwrap(), 5);

        let lines = sparklines(&conn, "org", &["app".into(), "missing".into()], NOW).unwrap();
        assert_eq!(lines.applications.len(), 1);
        assert_eq!(lines.applications["app"].len(), 48);
        assert_eq!(lines.applications["app"].iter().sum::<i64>(), 6);
    }

    #[test]
    fn edge_reads_track_offsets_and_skip_partial_lines() {
        // First contact: jump to the end, ingest nothing.
        assert_eq!(parse_edge_read("42 900 0\n", None), Some((42, 900, vec![])));
        let output = "42 1000 900\n{\"a\":1}\n{\"b\":2}\n{\"partial";
        let (inode, next, lines) = parse_edge_read(output, Some((42, 900))).unwrap();
        assert_eq!(inode, 42);
        assert_eq!(lines, vec!["{\"a\":1}", "{\"b\":2}"]);
        assert_eq!(next, 900 + 16);
        assert_eq!(
            parse_edge_read("42 916 916\n", Some((42, 916))),
            Some((42, 916, vec![]))
        );
        assert_eq!(parse_edge_read("garbage", Some((1, 1))), None);
    }

    #[test]
    fn paths_are_capped_per_application_minute() {
        let mut aggregator = Aggregator::default();
        for index in 0..(PATHS_PER_APP_MINUTE + 5) {
            aggregator.record(
                line(NOW, "a.test", &format!("/p{index}"), 200, 1.0),
                owner("app"),
                None,
            );
        }
        let (rollups, paths) = aggregator.drain();
        assert_eq!(rollups.len(), 1);
        assert_eq!(paths.len(), PATHS_PER_APP_MINUTE);
    }

    #[test]
    fn retention_covers_request_tables() {
        let conn = db();
        let mut aggregator = Aggregator::default();
        aggregator.record(
            line(NOW - 10 * 86_400_000, "a.test", "/", 200, 1.0),
            owner("app"),
            None,
        );
        aggregator.record(
            line(NOW - 1_000, "a.test", "/", 200, 1.0),
            owner("app"),
            None,
        );
        let (rollups, paths) = aggregator.drain();
        insert(&conn, &rollups, &paths).unwrap();
        let deleted = super::super::apply_retention(&conn, &[], 7, NOW).unwrap();
        assert_eq!(deleted, 2);
        assert_eq!(request_row_count(&conn).unwrap(), 1);
    }
}
