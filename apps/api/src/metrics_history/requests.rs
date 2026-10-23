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

use super::{geo::Geo, with_db};
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
    duration_sum_ms DOUBLE NOT NULL,
    -- Added after the table shipped; rows from before stay 0 (see the ALTERs below).
    duration_max_ms DOUBLE DEFAULT 0,
    lat_0 BIGINT DEFAULT 0, lat_1 BIGINT DEFAULT 0, lat_2 BIGINT DEFAULT 0,
    lat_3 BIGINT DEFAULT 0, lat_4 BIGINT DEFAULT 0, lat_5 BIGINT DEFAULT 0,
    lat_6 BIGINT DEFAULT 0, lat_7 BIGINT DEFAULT 0, lat_8 BIGINT DEFAULT 0
);
ALTER TABLE request_paths ADD COLUMN IF NOT EXISTS duration_max_ms DOUBLE DEFAULT 0;
ALTER TABLE request_paths ADD COLUMN IF NOT EXISTS lat_0 BIGINT DEFAULT 0;
ALTER TABLE request_paths ADD COLUMN IF NOT EXISTS lat_1 BIGINT DEFAULT 0;
ALTER TABLE request_paths ADD COLUMN IF NOT EXISTS lat_2 BIGINT DEFAULT 0;
ALTER TABLE request_paths ADD COLUMN IF NOT EXISTS lat_3 BIGINT DEFAULT 0;
ALTER TABLE request_paths ADD COLUMN IF NOT EXISTS lat_4 BIGINT DEFAULT 0;
ALTER TABLE request_paths ADD COLUMN IF NOT EXISTS lat_5 BIGINT DEFAULT 0;
ALTER TABLE request_paths ADD COLUMN IF NOT EXISTS lat_6 BIGINT DEFAULT 0;
ALTER TABLE request_paths ADD COLUMN IF NOT EXISTS lat_7 BIGINT DEFAULT 0;
ALTER TABLE request_paths ADD COLUMN IF NOT EXISTS lat_8 BIGINT DEFAULT 0;
-- Bounded per-minute top values of one request attribute (method, protocol, client, referrer).
CREATE TABLE IF NOT EXISTS request_dimensions (
    ts TIMESTAMP NOT NULL,
    organization_id VARCHAR NOT NULL,
    application_id VARCHAR NOT NULL,
    dimension VARCHAR NOT NULL,
    value VARCHAR NOT NULL,
    requests BIGINT NOT NULL,
    errors BIGINT NOT NULL
);
-- One row per (minute, application, visitor): a salted hash of day, client IP and
-- User-Agent, never the IP itself. The day in the salt means visitors are unique per day.
CREATE TABLE IF NOT EXISTS request_visitors (
    ts TIMESTAMP NOT NULL,
    organization_id VARCHAR NOT NULL,
    application_id VARCHAR NOT NULL,
    visitor BIGINT NOT NULL
);
-- One row per (minute, application, client IP, browser, OS, agent), bounded per minute.
-- Unlike request_visitors this keeps the IP itself, plus where it geolocates to.
CREATE TABLE IF NOT EXISTS request_clients (
    ts TIMESTAMP NOT NULL,
    organization_id VARCHAR NOT NULL,
    application_id VARCHAR NOT NULL,
    ip VARCHAR NOT NULL,
    -- NULL for bots and scripts, like request_visitors.
    visitor BIGINT,
    client VARCHAR NOT NULL,
    os VARCHAR NOT NULL,
    country VARCHAR,
    region VARCHAR,
    city VARCHAR,
    requests BIGINT NOT NULL,
    errors BIGINT NOT NULL,
    bytes_out BIGINT NOT NULL,
    -- Named agent for bots and scripts (see `agent_label`); empty for browsers.
    -- Added after the table shipped; rows from before keep the empty string.
    agent VARCHAR DEFAULT ''
);
ALTER TABLE request_clients ADD COLUMN IF NOT EXISTS agent VARCHAR DEFAULT '';
";

/// Tables `apply_retention` prunes alongside `container_samples`.
pub const TABLES: [&str; 5] = [
    "request_rollups",
    "request_paths",
    "request_dimensions",
    "request_visitors",
    "request_clients",
];

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
/// Kept on top of the busiest paths: the slowest of the rest, so a rarely hit but slow
/// endpoint still makes "slowest endpoints".
const SLOW_PATHS_PER_APP_MINUTE: usize = 10;
/// shortcut: per-minute cap on distinct visitors held in memory; past it a minute's
/// unique count undercounts. Raise or switch to a sketch for very high traffic.
const MAX_VISITOR_KEYS: usize = 100_000;
/// Client IPs kept per application and minute, busiest first.
const CLIENTS_PER_APP_MINUTE: usize = 200;
/// Rows in the "recent visitors" list.
const RECENT_VISITORS: usize = 100;
const DIMENSIONS: [&str; 4] = ["method", "protocol", "client", "referrer"];
const METHODS: [&str; 7] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];
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
    /// One of [`METHODS`] or `OTHER`.
    method: &'static str,
    /// `HTTP/1.1`, `HTTP/2.0`, `HTTP/3.0`; None when Caddy did not log one.
    protocol: Option<String>,
    /// See [`client_family`].
    client: &'static str,
    /// Referer host, None when absent or the site itself.
    referrer: Option<String>,
    /// Salted hash of (day, client IP, User-Agent); None for bots and scripts.
    visitor: Option<i64>,
    /// Client IP as Caddy resolved it (trusted proxies honored).
    ip: Option<String>,
    /// See [`os_family`].
    os: &'static str,
    /// See [`agent_label`]; empty for mainstream browsers.
    agent: String,
    /// Filled in by `ingest`, once the host is known to belong to an application.
    geo: Geo,
}

/// Coarse client from a User-Agent. Order matters: Edge and Opera also say Chrome,
/// and Chrome also says Safari.
fn client_family(user_agent: &str) -> &'static str {
    const BOTS: [&str; 10] = [
        "bot",
        "crawl",
        "spider",
        "slurp",
        "facebookexternalhit",
        "preview",
        "monitor",
        "uptime",
        "headless",
        "lighthouse",
    ];
    const TOOLS: [&str; 14] = [
        "curl", "wget", "python", "go-http", "okhttp", "axios", "node", "java/", "httpie",
        "postman", "insomnia", "libwww", "reqwest", "ruby",
    ];
    let ua = user_agent.to_ascii_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|needle| ua.contains(needle));
    if ua.is_empty() {
        "unknown"
    } else if has(&BOTS) {
        "bot"
    } else if has(&TOOLS) {
        "tool"
    } else if has(&["edg/", "edga/", "edgios/"]) {
        "edge"
    } else if has(&["opr/", "opera"]) {
        "opera"
    } else if has(&["firefox/", "fxios/"]) {
        "firefox"
    } else if has(&["chrome/", "crios/", "chromium/"]) {
        "chrome"
    } else if ua.contains("safari/") {
        "safari"
    } else {
        "other"
    }
}

/// Coarse operating system from a User-Agent. iOS and Android before macOS and Linux,
/// whose tokens their User-Agents also carry.
fn os_family(user_agent: &str) -> &'static str {
    let ua = user_agent.to_ascii_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|needle| ua.contains(needle));
    if ua.is_empty() {
        "unknown"
    } else if has(&["iphone", "ipad", "ipod"]) {
        "ios"
    } else if ua.contains("android") {
        "android"
    } else if ua.contains("windows") {
        "windows"
    } else if ua.contains("cros") {
        "chromeos"
    } else if has(&["macintosh", "mac os x"]) {
        "macos"
    } else if has(&["linux", "x11"]) {
        "linux"
    } else {
        "other"
    }
}

/// Named agent behind a User-Agent: the canonical name of a well-known bot or tool,
/// or its first product token (`Name/1.2`) when unknown. Empty for mainstream
/// browsers, whose family the `client` column already carries.
fn agent_label(user_agent: &str, client: &str) -> String {
    if matches!(client, "chrome" | "safari" | "firefox" | "edge" | "opera") {
        return String::new();
    }
    // Canonical names for bots whose product token is not what people know them by,
    // or that bury it after browser boilerplate (mobile Googlebot).
    const KNOWN: [(&str, &str); 36] = [
        ("googlebot", "Googlebot"),
        ("google-inspectiontool", "Google-InspectionTool"),
        ("storebot-google", "GoogleStoreBot"),
        ("bingbot", "bingbot"),
        ("bingpreview", "BingPreview"),
        ("yandexbot", "YandexBot"),
        ("yandeximages", "YandexImages"),
        ("duckduckbot", "DuckDuckBot"),
        ("baiduspider", "Baiduspider"),
        ("bytespider", "Bytespider"),
        ("applebot", "Applebot"),
        ("petalbot", "PetalBot"),
        ("semrushbot", "SemrushBot"),
        ("ahrefsbot", "AhrefsBot"),
        ("mj12bot", "MJ12bot"),
        ("dotbot", "DotBot"),
        ("gptbot", "GPTBot"),
        ("chatgpt-user", "ChatGPT-User"),
        ("oai-searchbot", "OAI-SearchBot"),
        ("claudebot", "ClaudeBot"),
        ("claude-web", "Claude-Web"),
        ("anthropic-ai", "Anthropic-AI"),
        ("perplexitybot", "PerplexityBot"),
        ("cohere-ai", "Cohere-AI"),
        ("ccbot", "CCBot"),
        ("amazonbot", "AmazonBot"),
        ("meta-externalagent", "Meta-ExternalAgent"),
        ("facebookexternalhit", "facebookexternalhit"),
        ("twitterbot", "Twitterbot"),
        ("linkedinbot", "LinkedInBot"),
        ("slackbot", "Slackbot"),
        ("discordbot", "Discordbot"),
        ("telegrambot", "TelegramBot"),
        ("uptimerobot", "UptimeRobot"),
        ("pingdom", "Pingdom"),
        ("lighthouse", "Lighthouse"),
    ];
    let ua = user_agent.to_ascii_lowercase();
    if let Some((_, name)) = KNOWN.iter().find(|(needle, _)| ua.contains(needle)) {
        return (*name).to_owned();
    }
    // Product tokens (`Name/1.2`) minus the browser boilerplate; a bot-ish name wins
    // over whatever comes first, since crawlers sometimes pose as browsers.
    const SKIP: [&str; 8] = [
        "mozilla",
        "applewebkit",
        "khtml",
        "gecko",
        "like",
        "compatible",
        "version",
        "mobile",
    ];
    let tokens: Vec<(&str, &str)> = user_agent
        .split([' ', ';'])
        .map(|token| {
            let token = token.trim_matches(|c: char| c == '(' || c == ')' || c == ',');
            (token.split('/').next().unwrap_or(""), token)
        })
        .filter(|(name, _)| {
            !name.is_empty() && !SKIP.iter().any(|skip| name.eq_ignore_ascii_case(skip))
        })
        .collect();
    let botish = |(name, _): &(&str, &str)| {
        let name = name.to_ascii_lowercase();
        [
            "bot", "crawl", "spider", "slurp", "headless", "monitor", "uptime",
        ]
        .iter()
        .any(|needle| name.contains(needle))
    };
    tokens
        .iter()
        .find(|token| botish(token))
        .or_else(|| tokens.first())
        .map(|(_, token)| token.chars().take(40).collect())
        .unwrap_or_default()
}

fn visitor_id(day: i64, ip: &str, user_agent: &str) -> i64 {
    use sha2::{Digest, Sha256};
    // Validated at startup in main.rs; the salt keeps the hash from mapping back to an IP.
    static SECRET: LazyLock<String> =
        LazyLock::new(|| std::env::var("HASH_SECRET").unwrap_or_default());
    let digest = Sha256::new()
        .chain_update(SECRET.as_bytes())
        .chain_update(day.to_le_bytes())
        .chain_update(ip.as_bytes())
        .chain_update([0])
        .chain_update(user_agent.as_bytes())
        .finalize();
    i64::from_le_bytes(digest[..8].try_into().expect("sha256 is 32 bytes"))
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
    let ts_ms = value["ts"]
        .as_f64()
        .map(|seconds| (seconds * 1_000.0) as i64)
        .unwrap_or(now_ms);
    let host = host.to_ascii_lowercase();
    // Caddy logs each header as a list of values under its canonical name.
    let header = |name: &str| request["headers"][name][0].as_str().unwrap_or("");
    let user_agent = header("User-Agent");
    let client = client_family(user_agent);
    let agent = agent_label(user_agent, client);
    let method = request["method"].as_str().unwrap_or("");
    let method = METHODS
        .into_iter()
        .find(|known| *known == method)
        .unwrap_or("OTHER");
    let protocol = request["proto"]
        .as_str()
        .filter(|proto| proto.starts_with("HTTP/") && proto.len() <= 8)
        .map(str::to_owned);
    let referrer = reqwest::Url::parse(header("Referer"))
        .ok()
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
        .filter(|referrer| *referrer != host);
    // `client_ip` honors trusted proxies (a CDN in front); older Caddy only has `remote_ip`.
    let ip = request["client_ip"]
        .as_str()
        .or_else(|| request["remote_ip"].as_str())
        .filter(|ip| !ip.is_empty());
    let visitor = ip
        .filter(|_| !matches!(client, "bot" | "tool"))
        .map(|ip| visitor_id(ts_ms.div_euclid(86_400_000), ip, user_agent));
    Some(AccessLine {
        ts_ms,
        host,
        path,
        // Caddy logs 0 when no handler wrote a header; the client got its implicit 200.
        status: value["status"]
            .as_i64()
            .filter(|status| *status > 0)
            .unwrap_or(200),
        duration_ms: value["duration"].as_f64().unwrap_or(0.0) * 1_000.0,
        bytes_out: value["size"].as_i64().unwrap_or(0),
        bytes_in: value["bytes_read"].as_i64().unwrap_or(0),
        method,
        protocol,
        client,
        referrer,
        visitor,
        ip: ip.map(str::to_owned),
        os: os_family(user_agent),
        agent,
        geo: Geo::default(),
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
    duration_max_ms: f64,
    latency: [i64; 9],
}

#[derive(Debug, Default, Clone, PartialEq)]
struct Counts {
    requests: i64,
    errors: i64,
}

type RollupKey = (i64, Owner, Option<String>, String, i64);
type PathKey = (i64, Owner, String);
type DimensionKey = (i64, Owner, &'static str, String);
type VisitorKey = (i64, Owner, i64);
type ClientKey = (i64, Owner, String, &'static str, &'static str, String);

#[derive(Debug, Default, Clone, PartialEq)]
struct ClientRollup {
    visitor: Option<i64>,
    geo: Geo,
    requests: i64,
    errors: i64,
    bytes_out: i64,
}

#[derive(Debug, Default)]
struct Drained {
    rollups: Vec<(RollupKey, Rollup)>,
    paths: Vec<(PathKey, PathRollup)>,
    dimensions: Vec<(DimensionKey, Counts)>,
    visitors: Vec<VisitorKey>,
    clients: Vec<(ClientKey, ClientRollup)>,
}

impl Drained {
    fn is_empty(&self) -> bool {
        self.rollups.is_empty()
            && self.paths.is_empty()
            && self.dimensions.is_empty()
            && self.visitors.is_empty()
            && self.clients.is_empty()
    }
}

#[derive(Debug, Default)]
struct Aggregator {
    rollups: HashMap<RollupKey, Rollup>,
    paths: HashMap<PathKey, PathRollup>,
    dimensions: HashMap<DimensionKey, Counts>,
    visitors: HashSet<VisitorKey>,
    clients: HashMap<ClientKey, ClientRollup>,
}

/// The `limit` busiest rows of each group, ties broken by key.
fn top_per_group<G: Eq + std::hash::Hash + Clone, K: Ord, V>(
    rows: impl Iterator<Item = (G, K, V)>,
    limit: usize,
    requests: impl Fn(&V) -> i64,
) -> Vec<(G, K, V)> {
    let mut groups: HashMap<G, Vec<(K, V)>> = HashMap::new();
    for (group, key, value) in rows {
        groups.entry(group).or_default().push((key, value));
    }
    let mut out = Vec::new();
    for (group, mut rows) in groups {
        rows.sort_by(|a, b| requests(&b.1).cmp(&requests(&a.1)).then(a.0.cmp(&b.0)));
        rows.truncate(limit);
        out.extend(
            rows.into_iter()
                .map(|(key, value)| (group.clone(), key, value)),
        );
    }
    out
}

fn latency_bucket(duration_ms: f64) -> usize {
    LATENCY_BOUNDS_MS
        .iter()
        .position(|bound| duration_ms <= *bound)
        .unwrap_or(LATENCY_BOUNDS_MS.len() - 1)
}

/// Per application and minute: the [`PATHS_PER_APP_MINUTE`] busiest paths plus the
/// [`SLOW_PATHS_PER_APP_MINUTE`] slowest (by max duration) of the rest.
fn keep_paths(rows: impl Iterator<Item = (PathKey, PathRollup)>) -> Vec<(PathKey, PathRollup)> {
    let mut groups: HashMap<(i64, Owner), Vec<(String, PathRollup)>> = HashMap::new();
    for ((minute, owner, path), rollup) in rows {
        groups
            .entry((minute, owner))
            .or_default()
            .push((path, rollup));
    }
    let mut out = Vec::new();
    for ((minute, owner), mut rows) in groups {
        rows.sort_by(|a, b| b.1.requests.cmp(&a.1.requests).then(a.0.cmp(&b.0)));
        let mut rest = rows.split_off(rows.len().min(PATHS_PER_APP_MINUTE));
        rest.sort_by(|a, b| {
            b.1.duration_max_ms
                .total_cmp(&a.1.duration_max_ms)
                .then(a.0.cmp(&b.0))
        });
        rest.truncate(SLOW_PATHS_PER_APP_MINUTE);
        out.extend(
            rows.into_iter()
                .chain(rest)
                .map(|(path, rollup)| ((minute, owner.clone(), path), rollup)),
        );
    }
    out
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
        let bucket = latency_bucket(line.duration_ms);
        rollup.latency[bucket] += 1;
        let error = i64::from(line.status >= 500);

        let values = [
            Some(line.method.to_owned()),
            line.protocol,
            Some(line.client.to_owned()),
            line.referrer,
        ];
        for (dimension, value) in DIMENSIONS.into_iter().zip(values) {
            let Some(value) = value else { continue };
            let key = (minute, owner.clone(), dimension, value);
            if self.dimensions.len() >= MAX_PATH_KEYS && !self.dimensions.contains_key(&key) {
                continue;
            }
            let counts = self.dimensions.entry(key).or_default();
            counts.requests += 1;
            counts.errors += error;
        }
        if let Some(visitor) = line.visitor
            && self.visitors.len() < MAX_VISITOR_KEYS
        {
            self.visitors.insert((minute, owner.clone(), visitor));
        }
        if let Some(ip) = line.ip {
            let key = (minute, owner.clone(), ip, line.client, line.os, line.agent);
            if self.clients.len() < MAX_VISITOR_KEYS || self.clients.contains_key(&key) {
                let client = self.clients.entry(key).or_insert_with(|| ClientRollup {
                    visitor: line.visitor,
                    geo: line.geo,
                    ..Default::default()
                });
                client.requests += 1;
                client.errors += error;
                client.bytes_out += line.bytes_out;
            }
        }

        let key = (minute, owner, line.path);
        if self.paths.len() >= MAX_PATH_KEYS && !self.paths.contains_key(&key) {
            return;
        }
        let path = self.paths.entry(key).or_default();
        path.requests += 1;
        path.errors += error;
        path.duration_sum_ms += line.duration_ms;
        path.duration_max_ms = path.duration_max_ms.max(line.duration_ms);
        path.latency[bucket] += 1;
    }

    /// Everything so far; partial minutes are fine because every query sums.
    fn drain(&mut self) -> Drained {
        let paths = keep_paths(self.paths.drain());
        let dimensions = top_per_group(
            self.dimensions
                .drain()
                .map(|((minute, owner, dimension, value), counts)| {
                    ((minute, owner, dimension), value, counts)
                }),
            PATHS_PER_APP_MINUTE,
            |counts| counts.requests,
        );
        let clients = top_per_group(
            self.clients
                .drain()
                .map(|((minute, owner, ip, client, os, agent), rollup)| {
                    ((minute, owner), (ip, client, os, agent), rollup)
                }),
            CLIENTS_PER_APP_MINUTE,
            |rollup| rollup.requests,
        );
        Drained {
            clients: clients
                .into_iter()
                .map(|((minute, owner), (ip, client, os, agent), rollup)| {
                    ((minute, owner, ip, client, os, agent), rollup)
                })
                .collect(),
            rollups: self.rollups.drain().collect(),
            paths,
            dimensions: dimensions
                .into_iter()
                .map(|((minute, owner, dimension), value, counts)| {
                    ((minute, owner, dimension, value), counts)
                })
                .collect(),
            visitors: self.visitors.drain().collect(),
        }
    }
}

static AGGREGATOR: LazyLock<Mutex<Aggregator>> = LazyLock::new(Mutex::default);
/// Lowercased host -> owner, refreshed every flush.
static HOSTS: LazyLock<RwLock<HashMap<String, Owner>>> = LazyLock::new(RwLock::default);

fn ingest(line: &str, now_ms: i64, server_id: Option<&str>) {
    let Some(mut line) = parse_line(line, now_ms) else {
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
    if let Some(ip) = &line.ip {
        line.geo = super::geo::lookup(ip);
    }
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

fn insert(conn: &Connection, batch: &Drained) -> duckdb::Result<()> {
    let mut appender = conn.appender("request_rollups")?;
    for ((minute, owner, server_id, host, status), r) in &batch.rollups {
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
    for ((minute, owner, path), r) in &batch.paths {
        let l = &r.latency;
        appender.append_row(params![
            ts(*minute),
            owner.organization_id,
            owner.application_id,
            path,
            r.requests,
            r.errors,
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
    let mut appender = conn.appender("request_dimensions")?;
    for ((minute, owner, dimension, value), counts) in &batch.dimensions {
        appender.append_row(params![
            ts(*minute),
            owner.organization_id,
            owner.application_id,
            dimension,
            value,
            counts.requests,
            counts.errors,
        ])?;
    }
    appender.flush()?;
    let mut appender = conn.appender("request_visitors")?;
    for (minute, owner, visitor) in &batch.visitors {
        appender.append_row(params![
            ts(*minute),
            owner.organization_id,
            owner.application_id,
            visitor,
        ])?;
    }
    appender.flush()?;
    let mut appender = conn.appender("request_clients")?;
    for ((minute, owner, ip, client, os, agent), r) in &batch.clients {
        appender.append_row(params![
            ts(*minute),
            owner.organization_id,
            owner.application_id,
            ip,
            r.visitor,
            client,
            os,
            r.geo.country,
            r.geo.region,
            r.geo.city,
            r.requests,
            r.errors,
            r.bytes_out,
            agent
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
    /// Distinct non-bot visitors in this bucket.
    pub visitors: i64,
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
    /// Distinct non-bot visitors over the range (a visitor is unique per day).
    pub visitors: i64,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct StatusCount {
    pub status: i64,
    pub requests: i64,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct TopEntry {
    /// Host, path, application id or dimension value, depending on the list.
    pub key: String,
    pub requests: i64,
    pub errors: i64,
    pub bytes_out: Option<i64>,
    pub avg_ms: Option<f64>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct SlowEntry {
    /// Host, path or application id, depending on the list.
    pub key: String,
    /// The path's application (organization view, `slow_paths` only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub application_id: Option<String>,
    pub requests: i64,
    pub avg_ms: Option<f64>,
    pub p95_ms: Option<f64>,
    pub max_ms: Option<f64>,
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
    pub methods: Vec<TopEntry>,
    pub protocols: Vec<TopEntry>,
    /// Browser family, `bot`, `tool` (curl, SDKs), `other` or `unknown`.
    pub clients: Vec<TopEntry>,
    /// Named agents behind `bot` traffic (see `agent_label`), busiest first.
    pub bots: Vec<TopEntry>,
    /// Referer hosts other than the site itself.
    pub referrers: Vec<TopEntry>,
    /// Every country with traffic, busiest first.
    pub countries: Vec<CountryEntry>,
    /// The most recently seen client IPs.
    pub recent_visitors: Vec<VisitorEntry>,
    /// Slowest by average response time, slowest first.
    pub slow_hosts: Vec<SlowEntry>,
    pub slow_paths: Vec<SlowEntry>,
    /// Organization view only.
    pub slow_applications: Vec<SlowEntry>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct CountryEntry {
    /// ISO 3166-1 alpha-2.
    pub code: String,
    pub requests: i64,
    /// Distinct non-bot visitors (unique per day).
    pub visitors: i64,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct VisitorEntry {
    pub ip: String,
    /// Organization view only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub application_id: Option<String>,
    pub country: Option<String>,
    pub region: Option<String>,
    pub city: Option<String>,
    /// Latest browser family seen from this IP; see `clients`.
    pub client: String,
    pub os: String,
    /// Latest named agent seen from this IP (bots and scripts); see `bots`.
    pub agent: String,
    pub requests: i64,
    pub errors: i64,
    pub bytes_out: i64,
    /// Epoch milliseconds (minute precision).
    pub first_seen: i64,
    pub last_seen: i64,
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
        methods: Vec::new(),
        protocols: Vec::new(),
        clients: Vec::new(),
        bots: Vec::new(),
        referrers: Vec::new(),
        countries: Vec::new(),
        recent_visitors: Vec::new(),
        slow_hosts: Vec::new(),
        slow_paths: Vec::new(),
        slow_applications: Vec::new(),
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

    let mut bucket_values = vec![duckdb::types::Value::BigInt(bucket_seconds)];
    bucket_values.extend(values.iter().cloned());
    let mut statement = conn.prepare(&format!(
        "SELECT epoch_ms(time_bucket(to_seconds(?::BIGINT), ts, TIMESTAMP '1970-01-01')) AS bucket,
                count(DISTINCT visitor)
         FROM request_visitors WHERE {filter} GROUP BY bucket"
    ))?;
    let rows = statement.query_map(params_from_iter(bucket_values), |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
    })?;
    for row in rows {
        let (bucket, visitors) = row?;
        if let Some(point) = dto.series.iter_mut().find(|point| point.ts == bucket) {
            point.visitors = visitors;
        }
    }
    dto.totals.visitors = conn.query_row(
        &format!("SELECT count(DISTINCT visitor) FROM request_visitors WHERE {filter}"),
        params_from_iter(values.iter()),
        |row| row.get(0),
    )?;

    let mut statement = conn.prepare(&format!(
        "SELECT value, sum(requests) AS n, sum(errors) FROM request_dimensions
         WHERE {filter} AND dimension = ?
         GROUP BY value ORDER BY n DESC, value LIMIT {TOP_LIMIT}"
    ))?;
    let mut dimension = |name: &str| -> duckdb::Result<Vec<TopEntry>> {
        let mut params = values.clone();
        params.push(duckdb::types::Value::Text(name.to_string()));
        statement
            .query_map(params_from_iter(params), |row| {
                Ok(TopEntry {
                    key: row.get(0)?,
                    requests: row.get(1)?,
                    errors: row.get(2)?,
                    bytes_out: None,
                    avg_ms: None,
                })
            })?
            .collect()
    };
    dto.methods = dimension("method")?;
    dto.protocols = dimension("protocol")?;
    dto.clients = dimension("client")?;
    dto.referrers = dimension("referrer")?;
    drop(statement);

    let mut statement = conn.prepare(&format!(
        "SELECT country, sum(requests) AS n, count(DISTINCT visitor) FROM request_clients
         WHERE {filter} AND country IS NOT NULL
         GROUP BY country ORDER BY n DESC, country"
    ))?;
    dto.countries = statement
        .query_map(params_from_iter(values.iter()), |row| {
            Ok(CountryEntry {
                code: row.get(0)?,
                requests: row.get(1)?,
                visitors: row.get(2)?,
            })
        })?
        .collect::<duckdb::Result<_>>()?;

    let mut statement = conn.prepare(&format!(
        "SELECT ip, application_id, arg_max(country, ts), arg_max(region, ts), arg_max(city, ts),
                arg_max(client, ts), arg_max(os, ts), arg_max(agent, ts),
                sum(requests), sum(errors), sum(bytes_out),
                epoch_ms(min(ts)), epoch_ms(max(ts)) AS last_seen
         FROM request_clients WHERE {filter}
         GROUP BY ip, application_id ORDER BY last_seen DESC, ip LIMIT {RECENT_VISITORS}"
    ))?;
    dto.recent_visitors = statement
        .query_map(params_from_iter(values.iter()), |row| {
            Ok(VisitorEntry {
                ip: row.get(0)?,
                application_id: with_applications.then(|| row.get(1)).transpose()?,
                country: row.get(2)?,
                region: row.get(3)?,
                city: row.get(4)?,
                client: row.get(5)?,
                os: row.get(6)?,
                agent: row.get(7)?,
                requests: row.get(8)?,
                errors: row.get(9)?,
                bytes_out: row.get(10)?,
                first_seen: row.get(11)?,
                last_seen: row.get(12)?,
            })
        })?
        .collect::<duckdb::Result<_>>()?;

    let mut statement = conn.prepare(&format!(
        "SELECT agent, sum(requests) AS n, sum(errors) FROM request_clients
         WHERE {filter} AND agent <> '' AND client = 'bot'
         GROUP BY agent ORDER BY n DESC, agent LIMIT {TOP_LIMIT}"
    ))?;
    dto.bots = statement
        .query_map(params_from_iter(values.iter()), |row| {
            Ok(TopEntry {
                key: row.get(0)?,
                requests: row.get(1)?,
                errors: row.get(2)?,
                bytes_out: None,
                avg_ms: None,
            })
        })?
        .collect::<duckdb::Result<_>>()?;

    // Grouped by application too, so the same path in two apps stays two rows.
    let slowest = |table: &str, column: &str, with_app: bool| -> duckdb::Result<Vec<SlowEntry>> {
        let mut statement = conn.prepare(&format!(
            "SELECT {column}, application_id, sum(requests),
                    sum(duration_sum_ms) / sum(requests) AS avg_ms, max(duration_max_ms),
                    sum(lat_0), sum(lat_1), sum(lat_2), sum(lat_3), sum(lat_4),
                    sum(lat_5), sum(lat_6), sum(lat_7), sum(lat_8)
             FROM {table} WHERE {filter}
             GROUP BY {column}, application_id
             ORDER BY avg_ms DESC NULLS LAST, {column} LIMIT {TOP_LIMIT}"
        ))?;
        statement
            .query_map(params_from_iter(values.iter()), |row| {
                let max_ms: Option<f64> = row.get(4)?;
                let mut latency = [0i64; 9];
                for (index, slot) in latency.iter_mut().enumerate() {
                    *slot = row.get::<_, Option<i64>>(5 + index)?.unwrap_or(0);
                }
                Ok(SlowEntry {
                    key: row.get(0)?,
                    application_id: with_app.then(|| row.get(1)).transpose()?,
                    requests: row.get(2)?,
                    avg_ms: row.get(3)?,
                    // Path rows from before the max and histogram columns carry 0s.
                    p95_ms: p95(&latency, max_ms.unwrap_or(0.0)),
                    max_ms: max_ms.filter(|max| *max > 0.0),
                })
            })?
            .collect()
    };
    dto.slow_hosts = slowest("request_rollups", "host", false)?;
    dto.slow_paths = slowest("request_paths", "path", with_applications)?;
    if with_applications {
        dto.slow_applications = slowest("request_rollups", "application_id", false)?;
    }
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
    let batch = AGGREGATOR
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .drain();
    if batch.is_empty() {
        return Ok(());
    }
    with_db(move |conn| insert(conn, &batch)).await
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
            method: "GET",
            protocol: Some("HTTP/2.0".into()),
            client: "chrome",
            referrer: None,
            visitor: Some(1),
            ip: Some("1.2.3.4".into()),
            os: "macos",
            agent: String::new(),
            geo: Geo {
                country: Some("MX".into()),
                region: Some("Jalisco".into()),
                city: Some("Guadalajara".into()),
            },
        }
    }

    #[test]
    fn classifies_operating_systems() {
        for (ua, os) in [
            (
                "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) Safari/604.1",
                "ios",
            ),
            (
                "Mozilla/5.0 (Linux; Android 15; Pixel 9) Chrome/130.0 Mobile",
                "android",
            ),
            (
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/130.0",
                "windows",
            ),
            (
                "Mozilla/5.0 (X11; CrOS x86_64 16002.0.0) Chrome/130.0",
                "chromeos",
            ),
            (
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) Safari/605.1",
                "macos",
            ),
            (
                "Mozilla/5.0 (X11; Linux x86_64; rv:131.0) Firefox/131.0",
                "linux",
            ),
            ("curl/8.7.1", "other"),
            ("", "unknown"),
        ] {
            assert_eq!(os_family(ua), os, "{ua}");
        }
    }

    #[test]
    fn reports_countries_and_recent_visitors() {
        let conn = db();
        let base = (NOW - 10 * MINUTE_MS) / MINUTE_MS * MINUTE_MS;
        let mut aggregator = Aggregator::default();
        aggregator.record(line(base, "a.test", "/", 200, 5.0), owner("app"), None);
        aggregator.record(
            line(base + MINUTE_MS, "a.test", "/", 502, 5.0),
            owner("app"),
            None,
        );
        let mut curl = line(base + 2 * MINUTE_MS, "a.test", "/api", 200, 5.0);
        curl.ip = Some("9.9.9.9".into());
        curl.client = "tool";
        curl.visitor = None;
        curl.geo = Geo {
            country: Some("DE".into()),
            ..Default::default()
        };
        aggregator.record(curl, owner("app"), None);
        let mut private = line(base, "a.test", "/", 200, 5.0);
        private.ip = Some("10.0.0.2".into());
        private.geo = Geo::default();
        aggregator.record(private, owner("app"), None);
        aggregator.record(line(base, "b.test", "/", 200, 5.0), owner("other"), None);
        insert(&conn, &aggregator.drain()).unwrap();

        let dto = analytics(
            &conn,
            "org",
            &["app".into()],
            AnalyticsRange::Hour,
            false,
            NOW,
        )
        .unwrap();
        assert_eq!(
            dto.countries,
            vec![
                CountryEntry {
                    code: "MX".into(),
                    requests: 2,
                    visitors: 1
                },
                CountryEntry {
                    code: "DE".into(),
                    requests: 1,
                    visitors: 0
                },
            ]
        );
        let ips: Vec<_> = dto.recent_visitors.iter().map(|v| v.ip.as_str()).collect();
        assert_eq!(ips, ["9.9.9.9", "1.2.3.4", "10.0.0.2"]);
        let mexico = &dto.recent_visitors[1];
        assert_eq!(mexico.application_id, None);
        assert_eq!(mexico.city.as_deref(), Some("Guadalajara"));
        assert_eq!((mexico.requests, mexico.errors), (2, 1));
        assert_eq!(mexico.first_seen, base);
        assert_eq!(mexico.last_seen, base + MINUTE_MS);
        assert_eq!(dto.recent_visitors[0].client, "tool");
        assert_eq!(dto.recent_visitors[2].country, None);

        let org = analytics(
            &conn,
            "org",
            &["app".into(), "other".into()],
            AnalyticsRange::Hour,
            true,
            NOW,
        )
        .unwrap();
        assert_eq!(org.countries[0].requests, 3);
        assert!(
            org.recent_visitors
                .iter()
                .any(|v| v.application_id.as_deref() == Some("other"))
        );
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
        assert_eq!(parse_line(ipv6, NOW).unwrap().client, "unknown");
        assert_eq!(parse_line(ipv6, NOW).unwrap().visitor, None);
    }

    #[test]
    fn parses_client_referrer_and_visitor() {
        let raw = |ip: &str, ua: &str, referer: &str| {
            format!(
                r#"{{"logger":"http.log.access","ts":1790000000,"status":200,"request":{{"client_ip":"{ip}","remote_ip":"10.0.0.1","proto":"HTTP/3.0","method":"PROPFIND","host":"shop.test","uri":"/","headers":{{"User-Agent":["{ua}"],"Referer":["{referer}"]}}}}}}"#
            )
        };
        let chrome = "Mozilla/5.0 (Macintosh) AppleWebKit/537.36 Chrome/130.0 Safari/537.36";
        let line = parse_line(
            &raw("1.2.3.4", chrome, "https://News.ycombinator.com/item?id=1"),
            0,
        )
        .unwrap();
        assert_eq!(line.method, "OTHER");
        assert_eq!(line.protocol.as_deref(), Some("HTTP/3.0"));
        assert_eq!(line.client, "chrome");
        assert_eq!(line.referrer.as_deref(), Some("news.ycombinator.com"));
        // client_ip wins over remote_ip; same day, IP and UA hash to the same visitor.
        let same = parse_line(&raw("1.2.3.4", chrome, ""), 0).unwrap();
        assert_eq!(same.visitor, line.visitor);
        assert_eq!(same.referrer, None);
        let other = parse_line(&raw("10.0.0.1", chrome, ""), 0).unwrap();
        assert_ne!(other.visitor, line.visitor);
        // Self-referrals are not referrers; bots are not visitors.
        let bot = parse_line(&raw("1.2.3.4", "Googlebot/2.1", "https://shop.test/a"), 0).unwrap();
        assert_eq!(bot.client, "bot");
        assert_eq!(bot.agent, "Googlebot");
        assert_eq!(bot.referrer, None);
        assert_eq!(bot.visitor, None);
    }

    #[test]
    fn names_agents() {
        for (ua, client, agent) in [
            ("Googlebot/2.1", "bot", "Googlebot"),
            ("Mozilla/5.0 (compatible; bingbot/2.0)", "bot", "bingbot"),
            // Mobile Googlebot hides behind an iPhone Safari User-Agent.
            (
                "Mozilla/5.0 (iPhone) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1 (compatible; Googlebot/2.1)",
                "bot",
                "Googlebot",
            ),
            (
                "Mozilla/5.0 (compatible; AhrefsBot/7.0; +http://ahrefs.com/robot/)",
                "bot",
                "AhrefsBot",
            ),
            ("HeadlessChrome/130.0", "bot", "HeadlessChrome/130.0"),
            // Unknown bots fall back to their first product token.
            (
                "Mozilla/5.0 (compatible; SomeCrawler/3.0; +https://example.com/bot)",
                "bot",
                "SomeCrawler/3.0",
            ),
            ("curl/8.7.1", "tool", "curl/8.7.1"),
            ("python-requests/2.31.0", "tool", "python-requests/2.31.0"),
            // Browsers are identified by their family already.
            (
                "Mozilla/5.0 (Macintosh) AppleWebKit/537.36 Chrome/130.0 Safari/537.36",
                "chrome",
                "",
            ),
            ("", "unknown", ""),
        ] {
            assert_eq!(agent_label(ua, client), agent, "{ua}");
        }
    }

    #[test]
    fn ranks_bot_agents() {
        let conn = db();
        let base = (NOW - 10 * MINUTE_MS) / MINUTE_MS * MINUTE_MS;
        let mut aggregator = Aggregator::default();
        let mut record = |ip: &str, client: &'static str, agent: &str| {
            let mut line = line(base, "a.test", "/", 200, 5.0);
            line.ip = Some(ip.into());
            line.client = client;
            line.agent = agent.into();
            line.visitor = None;
            line.geo = Geo::default();
            aggregator.record(line, owner("app"), None);
        };
        record("1.1.1.1", "bot", "Googlebot");
        record("2.2.2.2", "bot", "Googlebot");
        record("3.3.3.3", "bot", "GPTBot");
        record("9.9.9.9", "tool", "curl/8.7.1");
        insert(&conn, &aggregator.drain()).unwrap();

        let dto = analytics(
            &conn,
            "org",
            &["app".into()],
            AnalyticsRange::Hour,
            false,
            NOW,
        )
        .unwrap();
        let bots: Vec<_> = dto
            .bots
            .iter()
            .map(|entry| (entry.key.as_str(), entry.requests))
            .collect();
        // Tools and browsers stay out of the bot ranking.
        assert_eq!(bots, [("Googlebot", 2), ("GPTBot", 1)]);
        let agents: Vec<_> = dto
            .recent_visitors
            .iter()
            .map(|v| (v.ip.as_str(), v.agent.as_str()))
            .collect();
        assert_eq!(
            agents,
            [
                ("1.1.1.1", "Googlebot"),
                ("2.2.2.2", "Googlebot"),
                ("3.3.3.3", "GPTBot"),
                ("9.9.9.9", "curl/8.7.1"),
            ]
        );
    }

    #[test]
    fn classifies_clients() {
        for (ua, family) in [
            ("Mozilla/5.0 Chrome/130.0 Safari/537.36 Edg/130.0", "edge"),
            ("Mozilla/5.0 Chrome/130.0 Safari/537.36 OPR/114.0", "opera"),
            (
                "Mozilla/5.0 (X11; rv:131.0) Gecko/20100101 Firefox/131.0",
                "firefox",
            ),
            (
                "Mozilla/5.0 (iPhone) Version/18.0 Mobile/15E148 Safari/604.1",
                "safari",
            ),
            ("curl/8.7.1", "tool"),
            ("Mozilla/5.0 (compatible; bingbot/2.0)", "bot"),
            ("HeadlessChrome/130.0", "bot"),
            ("SomethingElse/1.0", "other"),
            ("", "unknown"),
        ] {
            assert_eq!(client_family(ua), family, "{ua}");
        }
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
    fn ranks_slowest_hosts_and_paths() {
        let conn = db();
        let base = (NOW - 10 * MINUTE_MS) / MINUTE_MS * MINUTE_MS;
        let mut aggregator = Aggregator::default();
        // More busy paths than a minute keeps; the one slow, rare path must survive.
        for index in 0..PATHS_PER_APP_MINUTE + 5 {
            for _ in 0..3 {
                aggregator.record(
                    line(base, "a.test", &format!("/busy/{index}"), 200, 5.0),
                    owner("app"),
                    None,
                );
            }
        }
        aggregator.record(
            line(base, "a.test", "/report", 200, 1_800.0),
            owner("app"),
            None,
        );
        aggregator.record(
            line(base, "b.test", "/report", 200, 300.0),
            owner("other"),
            None,
        );
        aggregator.record(line(base, "b.test", "/", 200, 100.0), owner("other"), None);
        insert(&conn, &aggregator.drain()).unwrap();

        let dto = analytics(
            &conn,
            "org",
            &["app".into()],
            AnalyticsRange::Hour,
            false,
            NOW,
        )
        .unwrap();
        let slow = &dto.slow_paths[0];
        assert_eq!(slow.key, "/report");
        assert_eq!(slow.application_id, None);
        assert_eq!((slow.requests, slow.max_ms), (1, Some(1_800.0)));
        assert_eq!(slow.p95_ms, Some(1_800.0));
        assert_eq!(dto.slow_hosts.len(), 1);
        assert!(dto.slow_applications.is_empty());

        let org = analytics(
            &conn,
            "org",
            &["app".into(), "other".into()],
            AnalyticsRange::Hour,
            true,
            NOW,
        )
        .unwrap();
        let paths: Vec<_> = org
            .slow_paths
            .iter()
            .take(3)
            .map(|entry| (entry.key.as_str(), entry.application_id.as_deref()))
            .collect();
        assert_eq!(
            paths,
            [
                ("/report", Some("app")),
                ("/report", Some("other")),
                ("/", Some("other"))
            ]
        );
        let hosts: Vec<_> = org
            .slow_hosts
            .iter()
            .map(|entry| entry.key.as_str())
            .collect();
        assert_eq!(hosts, ["b.test", "a.test"]);
        assert_eq!(org.slow_applications[0].key, "other");
    }

    #[test]
    fn adds_path_latency_columns_to_an_old_table() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE request_paths (ts TIMESTAMP NOT NULL, organization_id VARCHAR NOT NULL,
                application_id VARCHAR NOT NULL, path VARCHAR NOT NULL, requests BIGINT NOT NULL,
                errors BIGINT NOT NULL, duration_sum_ms DOUBLE NOT NULL);
             INSERT INTO request_paths VALUES (TIMESTAMP '2026-01-01', 'org', 'app', '/', 1, 0, 5);",
        )
        .unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        let mut aggregator = Aggregator::default();
        aggregator.record(line(NOW, "a.test", "/x", 200, 5.0), owner("app"), None);
        insert(&conn, &aggregator.drain()).unwrap();
        let old: (f64, i64) = conn
            .query_row(
                "SELECT duration_max_ms, lat_0 FROM request_paths WHERE path = '/'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(old, (0.0, 0));
    }

    #[test]
    fn adds_agent_column_to_an_old_clients_table() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE request_clients (ts TIMESTAMP NOT NULL, organization_id VARCHAR NOT NULL,
                application_id VARCHAR NOT NULL, ip VARCHAR NOT NULL, visitor BIGINT,
                client VARCHAR NOT NULL, os VARCHAR NOT NULL, country VARCHAR, region VARCHAR,
                city VARCHAR, requests BIGINT NOT NULL, errors BIGINT NOT NULL, bytes_out BIGINT NOT NULL);
             INSERT INTO request_clients VALUES (TIMESTAMP '2026-01-01', 'org', 'app', '1.2.3.4', NULL,
                'bot', 'other', NULL, NULL, NULL, 3, 0, 10);",
        )
        .unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        let mut aggregator = Aggregator::default();
        let mut bot = line(NOW, "a.test", "/", 200, 5.0);
        bot.client = "bot";
        bot.agent = "Googlebot".into();
        bot.visitor = None;
        aggregator.record(bot, owner("app"), None);
        insert(&conn, &aggregator.drain()).unwrap();
        let agents: Vec<String> = conn
            .prepare("SELECT agent FROM request_clients ORDER BY ts")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<duckdb::Result<_>>()
            .unwrap();
        // Rows from before the column existed keep the empty string.
        assert_eq!(agents, ["", "Googlebot"]);
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
        let batch = aggregator.drain();
        assert!(aggregator.rollups.is_empty() && aggregator.paths.is_empty());
        insert(&conn, &batch).unwrap();
        // A second flush for the same minute just adds rows; queries sum them.
        aggregator.record(
            line(base + 2_000, "a.test", "/", 200, 5.0),
            owner("app"),
            None,
        );
        let batch = aggregator.drain();
        insert(&conn, &batch).unwrap();

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

        // Every line above shares visitor 1 and the HTTP/2.0 Chrome GET defaults.
        assert_eq!(dto.totals.visitors, 1);
        assert_eq!(dto.series.iter().map(|p| p.visitors).max(), Some(1));
        assert_eq!(dto.methods[0].key, "GET");
        assert_eq!(dto.methods[0].requests, 6);
        assert_eq!(dto.methods[0].errors, 1);
        assert_eq!(dto.protocols[0].key, "HTTP/2.0");
        assert_eq!(dto.clients[0].key, "chrome");
        assert!(dto.referrers.is_empty());
        assert_eq!(foreign.totals.visitors, 0);

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
        for index in 0..(PATHS_PER_APP_MINUTE + SLOW_PATHS_PER_APP_MINUTE + 5) {
            aggregator.record(
                line(NOW, "a.test", &format!("/p{index}"), 200, 1.0),
                owner("app"),
                None,
            );
        }
        let batch = aggregator.drain();
        assert_eq!(batch.rollups.len(), 1);
        assert_eq!(
            batch.paths.len(),
            PATHS_PER_APP_MINUTE + SLOW_PATHS_PER_APP_MINUTE
        );
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
        let batch = aggregator.drain();
        insert(&conn, &batch).unwrap();
        let deleted = super::super::apply_retention(&conn, &[], 7, NOW).unwrap();
        // One rollup, path, visitor and client plus three dimensions (no referrer) for the old minute.
        assert_eq!(deleted, 7);
        assert_eq!(request_row_count(&conn).unwrap(), 1);
    }
}
