//! Checks a server's OS packages against known vulnerabilities with Syft and Grype.
//!
//! Scanning is off until someone turns it on for a server, because the vulnerability database
//! takes more than 3 GiB of that server's disk. Once on, it runs daily and after a package
//! update. Everything runs in throwaway containers, under the closed `server-vulnerability-scan`
//! script: request data never reaches the remote command.

use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    sync::Arc,
    time::Duration,
};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use utoipa::ToSchema;

use crate::{
    dtos::response::AppError,
    servers::{
        repository::{self, ServerRecord},
        setup::{credentials, install_root_file, target},
        ssh::{SshExecutor, SshRun, clamp_tail},
    },
    state::state::SharedState,
};

const SCRIPT_PATH: &str = "/usr/local/lib/litepod/server-vulnerability-scan";

/// Runs as the user that owns Podman on the host, never as root on a rootless server, so the
/// images and the database stay in that user's storage. The package databases are world
/// readable. Syft and the Grype match run with no network, so the package list never leaves
/// the host; only the database download has one.
const SCRIPT: &str = r#"#!/bin/sh
set -eu
PATH=/usr/sbin:/usr/bin:/sbin:/bin
syft=ghcr.io/anchore/syft:v1.57.0@sha256:3c70e6d5979364cefdc22b51617a78fd264574d3bd6b4f84a351204147025e2a
grype=ghcr.io/anchore/grype:v0.121.2@sha256:6a36234da13f9faa327306ac0b3dab3405cfa2b4eb20c8f904125a8707863913
volume=litepod-grype-db
# The unpacked database is 3.3 GiB, and an update holds the old and the new copy for a moment.
min_free_kb=8388608
report_limit=16777216
common="--rm --user 0:0 --read-only --tmpfs /tmp --cap-drop=all --security-opt=no-new-privileges"

scan() {
    store=$(podman info --format '{{.Store.GraphRoot}}')
    free=$(df -Pk "$store" | awk 'NR == 2 { print $4 }')
    if [ "$free" -lt "$min_free_kb" ]; then
        echo "The scanner needs 8 GiB free on the disk that holds Podman's storage." >&2
        exit 4
    fi
    # A rootless user without the memory controller delegated cannot set a limit at all.
    if podman info --format '{{.Host.CgroupControllers}}' | grep -qw memory; then
        common="$common --memory=1g"
    fi

    work=$(mktemp -d)
    trap 'rm -rf "$work"' EXIT
    trap 'exit 1' HUP INT TERM
    mkdir "$work/host" "$work/out" "$work/template"

    for path in /etc/os-release /usr/lib/os-release /var/lib/dpkg/status /var/lib/rpm /usr/lib/sysimage/rpm /var/lib/pacman/local; do
        # On Fedora this is a link to /usr/lib/sysimage/rpm, which is copied on its own.
        if [ "$path" = /var/lib/rpm ] && [ -L "$path" ]; then continue; fi
        if [ -e "$path" ]; then
            mkdir -p "$(dirname "$work/host$path")"
            cp -RL "$path" "$work/host$path" 2>/dev/null || true
        fi
    done
    if [ ! -e "$work/host/var/lib/dpkg/status" ] && [ ! -e "$work/host/var/lib/rpm" ] \
        && [ ! -e "$work/host/usr/lib/sysimage/rpm" ] && [ ! -e "$work/host/var/lib/pacman/local" ]; then
        exit 3
    fi

    nice -n 19 podman run $common --network=none -v "$work/host:/in:ro,Z" -v "$work/out:/out:Z" "$syft" \
        scan dir:/in --override-default-catalogers dpkg-db-cataloger,rpm-db-cataloger,alpm-db-cataloger \
        -o syft-json=/out/sbom.json -q >/dev/null

    # A failed download is not fatal: Grype itself refuses a database that is too old.
    nice -n 19 podman run $common -v "$volume:/db" -e GRYPE_DB_CACHE_DIR=/db -e GRYPE_CHECK_FOR_APP_UPDATE=false \
        "$grype" db update >/dev/null || echo "Could not update the vulnerability database." >&2

    cat >"$work/template/report.tmpl" <<'TEMPLATE'
{{- range .Matches}}{"id":{{toJson .Vulnerability.ID}},"severity":{{toJson .Vulnerability.Severity}},"package":{{toJson .Artifact.Name}},"version":{{toJson .Artifact.Version}},"fix_state":{{toJson .Vulnerability.Fix.State}},"fix_versions":{{toJson .Vulnerability.Fix.Versions}},"exploited":{{if .Vulnerability.KnownExploited}}true{{else}}false{{end}},"risk":{{.Vulnerability.Risk}},"url":{{toJson .Vulnerability.DataSource}}}
{{end -}}
TEMPLATE

    # Binary packages built from one source repeat the same problems thousands of times, and the
    # distribution will never ship a fix for wont-fix ones. `sh` has no pipefail, hence the file.
    {
        rc=0
        nice -n 19 podman run $common --network=none -v "$volume:/db:ro" -e GRYPE_DB_CACHE_DIR=/db \
            -e GRYPE_DB_AUTO_UPDATE=false -e GRYPE_CHECK_FOR_APP_UPDATE=false \
            -v "$work/out:/out:ro,Z" -v "$work/template:/template:ro,Z" "$grype" \
            sbom:/out/sbom.json -o template -t /template/report.tmpl -q || rc=$?
        echo "$rc" >"$work/rc"
    } | awk 'index($0, "\"fix_state\":\"wont-fix\"") { next }
             match($0, /"id":"[^"]*"/) { key = substr($0, RSTART, RLENGTH); if (seen[key]++) next; print }' >"$work/report"
    rc=$(cat "$work/rc")
    if [ "$rc" -ne 0 ]; then exit "$rc"; fi
    head -c "$report_limit" "$work/report"
}

remove() {
    podman volume rm -f "$volume" >/dev/null 2>&1 || true
    podman image rm -f "$syft" "$grype" >/dev/null 2>&1 || true
}

case "${1:-}" in
    scan) scan ;;
    remove) remove ;;
    *) echo "usage: server-vulnerability-scan scan|remove" >&2; exit 2 ;;
esac
"#;

/// Must match `report_limit` in the script.
const REPORT_LIMIT: usize = 16 * 1024 * 1024;
const MISSING_PACKAGE_DATABASE_EXIT: i32 = 3;
const LOW_DISK_EXIT: i32 = 4;
const OUT_OF_MEMORY_EXIT: i32 = 137;

const SCAN_BUDGET: Duration = Duration::from_secs(30 * 60);
const CLEANUP_BUDGET: Duration = Duration::from_secs(120);
const TICK: Duration = Duration::from_secs(15 * 60);
const RESCAN_AFTER_MS: i64 = 24 * 60 * 60 * 1000;
const MAX_CONCURRENT_SCANS: usize = 3;
/// The ranking puts what needs attention first, so the cut only drops the least urgent. The
/// summary still counts everything.
const MAX_STORED_FINDINGS: usize = 500;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct VulnerabilitySummary {
    pub critical: u32,
    pub high: u32,
    pub medium: u32,
    pub low: u32,
    /// Negligible or not rated.
    pub other: u32,
    pub fixable: u32,
    pub known_exploited: u32,
    /// Critical or known to be exploited; what the alerts and the server card report.
    pub urgent: u32,
    pub total: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct VulnerabilityFinding {
    pub id: String,
    /// `critical`, `high`, `medium`, `low`, `negligible` or `unknown`.
    pub severity: String,
    pub package: String,
    pub version: String,
    pub fixed_in: Option<String>,
    pub known_exploited: bool,
    pub risk: f64,
    /// Advisory link; `None` unless it is plain http(s), as the host's output is not trusted.
    pub url: Option<String>,
}

impl VulnerabilityFinding {
    fn is_urgent(&self) -> bool {
        self.known_exploited || self.severity == "critical"
    }
}

#[derive(Debug, Clone, Default, Serialize, ToSchema)]
pub struct VulnerabilityScanDto {
    pub enabled: bool,
    pub status: Option<String>,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    /// Last completed scan; older than `finished_at` when the latest one failed.
    pub scanned_at: Option<i64>,
    pub error: Option<String>,
    pub summary: Option<VulnerabilitySummary>,
    pub findings: Vec<VulnerabilityFinding>,
    /// Some findings were left out; the summary may also undercount if the report was cut.
    pub truncated: bool,
}

#[derive(Debug, Deserialize)]
struct RawFinding {
    id: String,
    severity: String,
    package: String,
    version: String,
    fix_state: String,
    fix_versions: Option<Vec<String>>,
    exploited: bool,
    risk: f64,
    url: String,
}

#[derive(Debug, Default)]
struct Report {
    summary: VulnerabilitySummary,
    findings: Vec<VulnerabilityFinding>,
    truncated: bool,
}

fn clamped(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

fn normalized_severity(value: &str) -> &'static str {
    match value.to_ascii_lowercase().as_str() {
        "critical" => "critical",
        "high" => "high",
        "medium" => "medium",
        "low" => "low",
        "negligible" => "negligible",
        _ => "unknown",
    }
}

fn severity_rank(value: &str) -> u8 {
    match value {
        "critical" => 0,
        "high" => 1,
        "medium" => 2,
        "low" => 3,
        _ => 4,
    }
}

/// The report comes from the scanned host, which may be compromised: every value is bounded
/// here and nothing but the parsed fields goes any further.
fn parse_report(output: &str) -> Report {
    let mut report = Report {
        truncated: output.len() >= REPORT_LIMIT,
        ..Default::default()
    };
    for line in output.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(raw) = serde_json::from_str::<RawFinding>(line) else {
            report.truncated = true;
            continue;
        };
        let fixed = raw.fix_state == "fixed";
        let finding = VulnerabilityFinding {
            id: clamped(&raw.id, 128),
            severity: normalized_severity(&raw.severity).into(),
            package: clamped(&raw.package, 200),
            version: clamped(&raw.version, 200),
            fixed_in: raw
                .fix_versions
                .and_then(|versions| versions.into_iter().next())
                .filter(|_| fixed)
                .map(|version| clamped(&version, 200)),
            known_exploited: raw.exploited,
            risk: raw.risk,
            url: (raw.url.len() <= 2048
                && (raw.url.starts_with("https://") || raw.url.starts_with("http://")))
            .then_some(raw.url),
        };
        let summary = &mut report.summary;
        match finding.severity.as_str() {
            "critical" => summary.critical += 1,
            "high" => summary.high += 1,
            "medium" => summary.medium += 1,
            "low" => summary.low += 1,
            _ => summary.other += 1,
        }
        summary.fixable += u32::from(fixed);
        summary.known_exploited += u32::from(finding.known_exploited);
        summary.urgent += u32::from(finding.is_urgent());
        summary.total += 1;
        report.findings.push(finding);
    }
    report.findings.sort_by(|a, b| {
        (
            !a.known_exploited,
            severity_rank(&a.severity),
            a.fixed_in.is_none(),
        )
            .cmp(&(
                !b.known_exploited,
                severity_rank(&b.severity),
                b.fixed_in.is_none(),
            ))
            .then(b.risk.total_cmp(&a.risk))
    });
    if report.findings.len() > MAX_STORED_FINDINGS {
        report.findings.truncate(MAX_STORED_FINDINGS);
        report.truncated = true;
    }
    report
}

fn database_error(error: sqlx::Error) -> AppError {
    AppError::DatabaseError(error.to_string())
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

pub async fn get(state: &SharedState, server_id: &str) -> Result<VulnerabilityScanDto, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(
        "SELECT status, started_at, finished_at, scanned_at, error, summary_json, findings_json, truncated \
         FROM server_vulnerability_scans WHERE server_id = ?1",
    )
    .bind(server_id)
    .fetch_optional(&conn)
    .await
    .map_err(database_error)?;
    let Some(row) = row else {
        return Ok(VulnerabilityScanDto::default());
    };
    Ok(VulnerabilityScanDto {
        enabled: true,
        status: Some(row.try_get("status").map_err(database_error)?),
        started_at: row.try_get("started_at").map_err(database_error)?,
        finished_at: row.try_get("finished_at").map_err(database_error)?,
        scanned_at: row.try_get("scanned_at").map_err(database_error)?,
        error: row.try_get("error").map_err(database_error)?,
        summary: row
            .try_get::<Option<String>, _>("summary_json")
            .map_err(database_error)?
            .and_then(|json| serde_json::from_str(&json).ok()),
        findings: row
            .try_get::<Option<String>, _>("findings_json")
            .map_err(database_error)?
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default(),
        truncated: row.try_get::<i64, _>("truncated").map_err(database_error)? != 0,
    })
}

/// Latest counts of every scanned server, for the server list.
pub async fn summaries(
    state: &SharedState,
) -> Result<HashMap<String, VulnerabilitySummary>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(
        "SELECT server_id, summary_json FROM server_vulnerability_scans WHERE summary_json IS NOT NULL",
    )
    .fetch_all(&conn)
    .await
    .map_err(database_error)?;
    let mut summaries = HashMap::with_capacity(rows.len());
    for row in rows {
        let json: String = row.try_get("summary_json").map_err(database_error)?;
        if let Ok(summary) = serde_json::from_str(&json) {
            summaries.insert(row.try_get("server_id").map_err(database_error)?, summary);
        }
    }
    Ok(summaries)
}

/// Exactly one caller gets `true`, so a server never runs two scans at once.
async fn claim(state: &SharedState, server_id: &str) -> Result<bool, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let result = sqlx::query(
        "UPDATE server_vulnerability_scans SET status = 'running', started_at = ?2 \
         WHERE server_id = ?1 AND status != 'running'",
    )
    .bind(server_id)
    .bind(now_ms())
    .execute(&conn)
    .await
    .map_err(database_error)?;
    Ok(result.rows_affected() == 1)
}

fn start(state: &SharedState, server: ServerRecord) {
    let state = state.clone();
    tokio::spawn(async move { run(&state, &server).await });
}

/// Turns scanning on for the server if it was off and starts a scan in the background, unless
/// one is already running.
pub async fn scan_now(
    state: &SharedState,
    server: ServerRecord,
) -> Result<VulnerabilityScanDto, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("INSERT OR IGNORE INTO server_vulnerability_scans (server_id) VALUES (?1)")
        .bind(&server.id)
        .execute(&conn)
        .await
        .map_err(database_error)?;
    let id = server.id.clone();
    if claim(state, &id).await? {
        start(state, server);
    }
    get(state, &id).await
}

/// New package versions change the answer, so the result on screen is out of date. A failure
/// here must not fail the package update that already happened.
pub async fn scan_after_package_update(state: &SharedState, server: ServerRecord) {
    match claim(state, &server.id).await {
        Ok(true) => start(state, server),
        Ok(false) => {}
        Err(error) => tracing::warn!(%error, "could not queue a vulnerability scan"),
    }
}

/// Turns scanning off and frees the disk. Returns false when the server could not be reached:
/// scanning is off anyway, but the database is still on its disk.
pub async fn disable(state: &SharedState, server: &ServerRecord) -> Result<bool, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let removed = sqlx::query(
        "DELETE FROM server_vulnerability_scans WHERE server_id = ?1 AND status != 'running'",
    )
    .bind(&server.id)
    .execute(&conn)
    .await
    .map_err(database_error)?;
    if removed.rows_affected() == 0 {
        return Err(AppError::ValidationError(
            "Wait for the running scan to finish, or scanning is already off".into(),
        ));
    }
    match run_script(server, "remove", CLEANUP_BUDGET).await {
        Ok(run) if run.exit_code == 0 => Ok(true),
        Ok(run) => {
            tracing::warn!(server = %server.id, stderr = %run.stderr, "could not remove the scanner data");
            Ok(false)
        }
        Err(error) => {
            tracing::warn!(server = %server.id, %error, "could not remove the scanner data");
            Ok(false)
        }
    }
}

async fn run_script(
    server: &ServerRecord,
    subcommand: &str,
    budget: Duration,
) -> Result<SshRun, AppError> {
    install_root_file(
        server,
        SCRIPT,
        "server-vulnerability-scan",
        SCRIPT_PATH,
        "0755",
    )
    .await?;
    let target = target(server)?;
    let (key, fingerprint, known_hosts) = credentials(server)?;
    let command = server
        .podman_mode
        .podman_prefix(server.ssh_user.as_deref().unwrap_or_default())
        .iter()
        .chain(&[SCRIPT_PATH, subcommand])
        .map(OsString::from)
        .collect::<Vec<_>>();
    SshExecutor::run_allow_failure(&target, key, fingerprint, known_hosts, &command, budget).await
}

fn failure_message(run: &SshRun) -> String {
    match run.exit_code {
        MISSING_PACKAGE_DATABASE_EXIT => {
            "No supported package database was found on this server.".into()
        }
        LOW_DISK_EXIT => clamp_tail(&run.stderr, 500),
        OUT_OF_MEMORY_EXIT => "The scanner ran out of memory.".into(),
        code => format!(
            "The scan failed (exit {code}): {}",
            clamp_tail(&run.stderr, 500)
        ),
    }
}

/// The caller holds the claim. Every outcome leaves the row out of `running`.
async fn run(state: &SharedState, server: &ServerRecord) {
    let outcome = match run_script(server, "scan", SCAN_BUDGET).await {
        Ok(run) if run.exit_code == 0 => complete(state, server, parse_report(&run.stdout)).await,
        Ok(run) => fail(state, server, failure_message(&run)).await,
        Err(error) => fail(state, server, error.to_string()).await,
    };
    if let Err(error) = outcome {
        tracing::warn!(server = %server.id, %error, "could not save the vulnerability scan");
    }
}

async fn complete(
    state: &SharedState,
    server: &ServerRecord,
    report: Report,
) -> Result<(), AppError> {
    let previous = get(state, &server.id).await?;
    let known = previous
        .findings
        .iter()
        .filter(|finding| finding.is_urgent())
        .map(|finding| finding.id.as_str())
        .collect::<HashSet<_>>();
    let new_urgent = report
        .findings
        .iter()
        .filter(|finding| finding.is_urgent() && !known.contains(finding.id.as_str()))
        .map(|finding| finding.id.clone())
        .collect::<Vec<_>>();
    let summary_json = serde_json::to_string(&report.summary)
        .map_err(|e| AppError::InternalError(e.to_string()))?;
    let findings_json = serde_json::to_string(&report.findings)
        .map_err(|e| AppError::InternalError(e.to_string()))?;
    let now = now_ms();
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE server_vulnerability_scans SET status = 'completed', finished_at = ?2, scanned_at = ?2, \
         error = NULL, summary_json = ?3, findings_json = ?4, truncated = ?5 WHERE server_id = ?1",
    )
    .bind(&server.id)
    .bind(now)
    .bind(summary_json)
    .bind(findings_json)
    .bind(i64::from(report.truncated))
    .execute(&conn)
    .await
    .map_err(database_error)?;
    if let (false, Some(organization_id)) = (new_urgent.is_empty(), &server.organization_id) {
        crate::notifications::emit::notify_server_vulnerabilities(
            state,
            organization_id,
            &server.name,
            &new_urgent,
            report.summary.known_exploited,
            report.summary.critical,
        )
        .await;
    }
    Ok(())
}

/// Keeps the findings of the last completed scan. Alerts on the first failure only, so a server
/// that stays broken does not repeat it every day.
async fn fail(state: &SharedState, server: &ServerRecord, message: String) -> Result<(), AppError> {
    let first_failure = get(state, &server.id).await?.error.is_none();
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE server_vulnerability_scans SET status = 'failed', finished_at = ?2, error = ?3 WHERE server_id = ?1",
    )
    .bind(&server.id)
    .bind(now_ms())
    .bind(&message)
    .execute(&conn)
    .await
    .map_err(database_error)?;
    if let (true, Some(organization_id)) = (first_failure, &server.organization_id) {
        crate::notifications::emit::notify_server_vulnerability_scan_failed(
            state,
            organization_id,
            &server.name,
            &message,
        )
        .await;
    }
    Ok(())
}

async fn due_server_ids(state: &SharedState) -> Result<Vec<String>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(
        "SELECT v.server_id FROM server_vulnerability_scans v JOIN servers s ON s.id = v.server_id \
         WHERE s.kind = 'ssh' AND s.state = 'ready' AND s.host_key_fingerprint IS NOT NULL \
         AND v.status != 'running' AND COALESCE(v.finished_at, 0) <= ?1 \
         ORDER BY COALESCE(v.finished_at, 0)",
    )
    .bind(now_ms() - RESCAN_AFTER_MS)
    .fetch_all(&conn)
    .await
    .map_err(database_error)?;
    rows.iter()
        .map(|row| row.try_get("server_id").map_err(database_error))
        .collect()
}

/// A scan only dies mid-way when the API restarts. `finished_at` stays as it was, so the
/// scheduler picks the server up again on its next tick.
async fn fail_interrupted(state: &SharedState) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE server_vulnerability_scans SET status = 'failed', error = 'Interrupted by a litepod restart.' WHERE status = 'running'",
    )
    .execute(&conn)
    .await
    .map_err(database_error)?;
    Ok(())
}

async fn tick(state: &SharedState) -> Result<(), AppError> {
    let permits = Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_SCANS));
    let mut tasks = tokio::task::JoinSet::new();
    for id in due_server_ids(state).await? {
        // A scan competes with builds for CPU and disk; the next tick tries again.
        if repository::has_active_deployment(state, &id).await?
            || repository::has_active_setup_job(state, &id).await?
        {
            continue;
        }
        let Some(server) = repository::get(state, &id).await? else {
            continue;
        };
        let server = repository::ensure_host_key_known_hosts(state, server).await?;
        let state = state.clone();
        let permits = permits.clone();
        tasks.spawn(async move {
            let _permit = permits.acquire().await;
            match claim(&state, &server.id).await {
                Ok(true) => run(&state, &server).await,
                Ok(false) => {}
                Err(error) => tracing::warn!(server = %server.id, %error, "could not start a vulnerability scan"),
            }
        });
    }
    while tasks.join_next().await.is_some() {}
    Ok(())
}

pub fn spawn(state: SharedState) {
    tokio::spawn(async move {
        if let Err(error) = fail_interrupted(&state).await {
            tracing::warn!(%error, "could not close interrupted vulnerability scans");
        }
        let mut interval = tokio::time::interval(TICK);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        interval.tick().await;
        loop {
            interval.tick().await;
            if let Err(error) = tick(&state).await {
                tracing::warn!(%error, "vulnerability scan tick failed");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_parses() {
        let status = std::process::Command::new("sh")
            .args(["-n", "-c", SCRIPT])
            .status()
            .unwrap();
        assert!(status.success());
    }

    fn line(id: &str, severity: &str, fix_state: &str, exploited: bool, risk: f64) -> String {
        format!(
            r#"{{"id":"{id}","severity":"{severity}","package":"openssl","version":"3.0","fix_state":"{fix_state}","fix_versions":["3.1"],"exploited":{exploited},"risk":{risk},"url":"https://example.com/{id}"}}"#
        )
    }

    #[test]
    fn report_ranks_what_needs_attention_first_and_counts_everything() {
        let output = [
            line("LOW", "Low", "not-fixed", false, 0.9),
            line("HIGH-OPEN", "High", "not-fixed", false, 0.9),
            line("HIGH-FIX", "High", "fixed", false, 0.1),
            line("CRIT", "Critical", "fixed", false, 0.5),
            line("KEV", "Medium", "not-fixed", true, 0.1),
            line("ODD", "Whatever", "unknown", false, 0.0),
            String::new(),
        ]
        .join("\n");
        let report = parse_report(&output);
        let order = report
            .findings
            .iter()
            .map(|finding| finding.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            order,
            ["KEV", "CRIT", "HIGH-FIX", "HIGH-OPEN", "LOW", "ODD"]
        );
        assert_eq!(
            report.summary,
            VulnerabilitySummary {
                critical: 1,
                high: 2,
                medium: 1,
                low: 1,
                other: 1,
                fixable: 2,
                known_exploited: 1,
                urgent: 2,
                total: 6,
            }
        );
        assert!(!report.truncated);
        assert_eq!(report.findings[1].fixed_in.as_deref(), Some("3.1"));
        assert_eq!(report.findings[0].fixed_in, None);
    }

    #[test]
    fn report_bounds_what_a_compromised_host_can_send() {
        let hostile = format!(
            r#"{{"id":"{}","severity":"High","package":"p","version":"1","fix_state":"fixed","fix_versions":null,"exploited":false,"risk":0.1,"url":"javascript:alert(1)"}}"#,
            "A".repeat(5000)
        );
        let report = parse_report(&format!("{hostile}\nnot json\n"));
        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.findings[0].id.len(), 128);
        assert_eq!(report.findings[0].url, None);
        assert_eq!(report.findings[0].fixed_in, None);
        assert!(report.truncated);
    }

    #[test]
    fn report_keeps_the_stored_list_small_but_counts_all() {
        let output = (0..MAX_STORED_FINDINGS + 1)
            .map(|index| line(&format!("CVE-{index}"), "Low", "fixed", false, 0.1))
            .collect::<Vec<_>>()
            .join("\n");
        let report = parse_report(&output);
        assert_eq!(report.findings.len(), MAX_STORED_FINDINGS);
        assert_eq!(report.summary.total as usize, MAX_STORED_FINDINGS + 1);
        assert!(report.truncated);
    }

    async fn seed_server(state: &SharedState, id: &str) {
        sqlx::query(
            "INSERT INTO servers (id, kind, name, organization_id, ingress_mode, state, ssh_host, ssh_port, ssh_user, host_key_fingerprint, created_at, updated_at) VALUES (?1, 'ssh', ?1, NULL, 'public_edge', 'ready', '198.51.100.1', 22, 'litepod', 'SHA256:x', 1, 1)",
        )
        .bind(id)
        .execute(&state.db)
        .await
        .unwrap();
        sqlx::query("INSERT INTO server_vulnerability_scans (server_id) VALUES (?1)")
            .bind(id)
            .execute(&state.db)
            .await
            .unwrap();
    }

    fn record(id: &str) -> ServerRecord {
        ServerRecord {
            id: id.into(),
            kind: "ssh".into(),
            name: id.into(),
            ssh_host: None,
            ssh_port: None,
            ssh_user: None,
            host_key_fingerprint: None,
            host_key_known_hosts: None,
            private_key_encrypted: None,
            ingress_mode: crate::servers::model::IngressMode::PublicEdge,
            podman_mode: crate::servers::model::PodmanMode::Rootless,
            state: crate::servers::model::ServerState::Ready,
            public_address: None,
            facts_json: "{}".into(),
            last_error_code: None,
            last_error_message: None,
            organization_id: None,
            latitude: None,
            longitude: None,
            geo_city: None,
            geo_country: None,
            geo_asn: None,
            geo_timezone: None,
            geo_resolved_at: None,
        }
    }

    #[tokio::test]
    async fn a_failed_scan_keeps_the_last_findings_and_stays_due_only_after_a_day() {
        let (state, path) = crate::testing::test_state().await;
        seed_server(&state, "a").await;
        assert_eq!(due_server_ids(&state).await.unwrap(), ["a"]);

        assert!(claim(&state, "a").await.unwrap());
        assert!(
            !claim(&state, "a").await.unwrap(),
            "a second scan must not start"
        );
        assert!(due_server_ids(&state).await.unwrap().is_empty());

        let server = record("a");
        complete(
            &state,
            &server,
            parse_report(&line("CVE-1", "Critical", "fixed", false, 0.5)),
        )
        .await
        .unwrap();
        assert!(due_server_ids(&state).await.unwrap().is_empty());

        assert!(claim(&state, "a").await.unwrap());
        fail(&state, &server, "boom".into()).await.unwrap();
        let scan = get(&state, "a").await.unwrap();
        assert_eq!(scan.status.as_deref(), Some("failed"));
        assert_eq!(scan.error.as_deref(), Some("boom"));
        assert_eq!(scan.findings.len(), 1);
        assert_eq!(scan.summary.unwrap().urgent, 1);
        assert_eq!(summaries(&state).await.unwrap()["a"].critical, 1);

        crate::testing::cleanup_database(&path);
    }

    #[tokio::test]
    async fn a_restart_closes_running_scans_and_makes_them_due_again() {
        let (state, path) = crate::testing::test_state().await;
        seed_server(&state, "a").await;
        assert!(claim(&state, "a").await.unwrap());
        fail_interrupted(&state).await.unwrap();
        assert_eq!(
            get(&state, "a").await.unwrap().status.as_deref(),
            Some("failed")
        );
        assert_eq!(due_server_ids(&state).await.unwrap(), ["a"]);
        crate::testing::cleanup_database(&path);
    }
}
