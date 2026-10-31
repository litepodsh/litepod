//! The provider-agnostic event vocabulary.
//!
//! Everything that can be notified about is reduced to an [`EventPayload`]
//! here, *before* any provider is involved. `render.rs` then turns that single
//! shape into Slack blocks / a Discord embed / a Teams MessageCard / Telegram
//! Markdown. Keeping the two apart means adding a provider never touches the
//! emitting code, and adding an event never touches the four renderers.
//!
//! A payload is serialized into `notification_deliveries.payload` and may be
//! replayed minutes later by the worker, so it must be self-contained and must
//! never carry a credential.

use serde::{Deserialize, Serialize};

/// What happened. The string form is the value stored in
/// `notification_subscriptions.action` and matched by the migration's CHECK.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    AppDeploy,
    AppBuildError,
    LitepodBackup,
    DockerCleanup,
    LitepodRestart,
    ServerReboot,
    DatabaseBackup,
    SecurityAlert,
}

impl Action {
    pub const ALL: [Action; 8] = [
        Action::AppDeploy,
        Action::AppBuildError,
        Action::LitepodBackup,
        Action::DockerCleanup,
        Action::LitepodRestart,
        Action::ServerReboot,
        Action::DatabaseBackup,
        Action::SecurityAlert,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Action::AppDeploy => "app_deploy",
            Action::AppBuildError => "app_build_error",
            Action::LitepodBackup => "litepod_backup",
            Action::DockerCleanup => "docker_cleanup",
            Action::LitepodRestart => "litepod_restart",
            Action::ServerReboot => "server_reboot",
            Action::DatabaseBackup => "database_backup",
            Action::SecurityAlert => "security_alert",
        }
    }

    pub fn parse(value: &str) -> Option<Action> {
        Action::ALL.into_iter().find(|a| a.key() == value)
    }

    /// Platform-wide actions carry information about the litepod install itself
    /// rather than about one organization's applications. Only the root organization may
    /// subscribe to them — enforced in the handler.
    pub fn is_global(self) -> bool {
        matches!(
            self,
            Action::LitepodBackup
                | Action::DockerCleanup
                | Action::LitepodRestart
                | Action::SecurityAlert
        )
    }
}

/// Drives colour and emoji in every renderer. Deliberately coarse — providers
/// only really distinguish "good", "bad" and "neutral".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Success,
    Failure,
    Info,
}

impl Outcome {
    /// Packed RGB, used directly by the Discord embed and (as hex) by Teams.
    pub fn color(self) -> u32 {
        match self {
            Outcome::Success => 0x22C55E,
            Outcome::Failure => 0xEF4444,
            Outcome::Info => 0x3B82F6,
        }
    }

    pub fn emoji(self) -> &'static str {
        match self {
            Outcome::Success => "✅",
            Outcome::Failure => "❌",
            Outcome::Info => "ℹ️",
        }
    }
}

/// A label/value pair rendered as a Slack field, a Discord embed field, a
/// Teams fact, or a `label: value` line in Telegram.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Field {
    pub label: String,
    pub value: String,
}

impl Field {
    pub fn new(label: impl Into<String>, value: impl Into<String>) -> Field {
        Field {
            label: label.into(),
            value: value.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventPayload {
    pub action: String,
    pub title: String,
    pub outcome: Outcome,
    #[serde(default)]
    pub fields: Vec<Field>,
    /// Free text below the title — an error message, typically. Already
    /// sanitized and truncated by the builder.
    #[serde(default)]
    pub description: Option<String>,
    /// Deep link back into litepod.
    #[serde(default)]
    pub url: Option<String>,
    pub timestamp: i64,
}

/// Who a given event belongs to. `Global` fans out to the root organization's
/// providers only.
#[derive(Debug, Clone)]
pub enum Scope {
    Organization(String),
    Global,
}

#[derive(Debug, Clone)]
pub struct Event {
    pub action: Action,
    pub scope: Scope,
    pub payload: EventPayload,
}

/// Error text arrives from build logs and container output: it can carry ANSI
/// escapes and can be megabytes long. Strip and clamp before it ever reaches a
/// chat client.
pub fn sanitize_description(raw: &str) -> String {
    const MAX: usize = 500;
    let cleaned = crate::deployment::log_filter::strip_ansi_sequences(raw);
    let trimmed = cleaned.trim();
    if trimmed.chars().count() <= MAX {
        return trimmed.to_string();
    }
    let mut out: String = trimmed.chars().take(MAX).collect();
    out.push('…');
    out
}

pub fn human_bytes(bytes: i64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

pub fn human_duration(millis: i64) -> String {
    let total_secs = (millis.max(0) / 1000) as u64;
    if total_secs < 60 {
        return format!("{total_secs}s");
    }
    let minutes = total_secs / 60;
    let seconds = total_secs % 60;
    if minutes < 60 {
        return format!("{minutes}m {seconds}s");
    }
    format!("{}h {}m", minutes / 60, minutes % 60)
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

// ── Builders ──────────────────────────────────────────────────────────────
//
// Each builder takes already-resolved data. Looking that data up (joining a
// run to its organization, reading the app name) is `repository.rs`'s job — these stay
// pure so they can be unit-tested without a database.

/// Context shared by both deployment events.
pub struct DeploymentContext {
    pub organization_id: String,
    pub application_name: String,
    pub project_name: String,
    pub environment_name: String,
    pub commit_sha: Option<String>,
    pub commit_message: Option<String>,
    pub triggered_by: String,
    pub duration_ms: Option<i64>,
    pub url: Option<String>,
}

fn deployment_fields(context: &DeploymentContext) -> Vec<Field> {
    let mut fields = vec![
        Field::new("Application", &context.application_name),
        Field::new(
            "Environment",
            format!("{} / {}", context.project_name, context.environment_name),
        ),
    ];
    if let Some(sha) = &context.commit_sha {
        let short: String = sha.chars().take(7).collect();
        let value = match &context.commit_message {
            Some(message) if !message.trim().is_empty() => {
                let subject = message.lines().next().unwrap_or("").trim();
                format!("`{short}` {subject}")
            }
            _ => format!("`{short}`"),
        };
        fields.push(Field::new("Commit", value));
    }
    fields.push(Field::new("Triggered by", &context.triggered_by));
    if let Some(duration) = context.duration_ms {
        fields.push(Field::new("Duration", human_duration(duration)));
    }
    fields
}

pub fn app_deploy(context: DeploymentContext) -> Event {
    Event {
        action: Action::AppDeploy,
        scope: Scope::Organization(context.organization_id.clone()),
        payload: EventPayload {
            action: Action::AppDeploy.key().to_string(),
            title: format!("Deployed {}", context.application_name),
            outcome: Outcome::Success,
            fields: deployment_fields(&context),
            description: None,
            url: context.url.clone(),
            timestamp: now_ms(),
        },
    }
}

/// `phase` is `building` or `deploying` — read from the run's status before it
/// is overwritten with the terminal one.
pub fn app_build_error(context: DeploymentContext, phase: &str, error: Option<&str>) -> Event {
    let mut fields = deployment_fields(&context);
    fields.push(Field::new(
        "Failed during",
        if phase == "deploying" {
            "Deploy"
        } else {
            "Build"
        },
    ));
    Event {
        action: Action::AppBuildError,
        scope: Scope::Organization(context.organization_id.clone()),
        payload: EventPayload {
            action: Action::AppBuildError.key().to_string(),
            title: format!("Deployment of {} failed", context.application_name),
            outcome: Outcome::Failure,
            fields,
            description: error.map(sanitize_description),
            url: context.url.clone(),
            timestamp: now_ms(),
        },
    }
}

pub fn litepod_backup(
    succeeded: bool,
    triggered_by: &str,
    size_bytes: Option<i64>,
    s3_key: Option<&str>,
    duration_ms: Option<i64>,
    error: Option<&str>,
) -> Event {
    let mut fields = vec![Field::new("Triggered by", triggered_by)];
    if let Some(size) = size_bytes {
        fields.push(Field::new("Size", human_bytes(size)));
    }
    if let Some(key) = s3_key {
        fields.push(Field::new("Object", key));
    }
    if let Some(duration) = duration_ms {
        fields.push(Field::new("Duration", human_duration(duration)));
    }
    Event {
        action: Action::LitepodBackup,
        scope: Scope::Global,
        payload: EventPayload {
            action: Action::LitepodBackup.key().to_string(),
            title: if succeeded {
                "litepod backup completed".to_string()
            } else {
                "litepod backup failed".to_string()
            },
            outcome: if succeeded {
                Outcome::Success
            } else {
                Outcome::Failure
            },
            fields,
            description: error.map(sanitize_description),
            url: None,
            timestamp: now_ms(),
        },
    }
}

/// Fired whenever a per-database backup run (ad-hoc or scheduled) finishes.
/// Scoped to the application's organization so each org subscribes to its own
/// databases. `size_bytes`/`error` mirror the run row; the error is sanitized.
pub fn database_backup(
    organization_id: &str,
    application_name: &str,
    engine: &str,
    succeeded: bool,
    triggered_by: &str,
    size_bytes: Option<i64>,
    error: Option<&str>,
) -> Event {
    let mut fields = vec![
        Field::new("Database", format!("{application_name} ({engine})")),
        Field::new("Triggered by", triggered_by),
    ];
    if let Some(size) = size_bytes {
        fields.push(Field::new("Size", human_bytes(size)));
    }
    Event {
        action: Action::DatabaseBackup,
        scope: Scope::Organization(organization_id.to_string()),
        payload: EventPayload {
            action: Action::DatabaseBackup.key().to_string(),
            title: if succeeded {
                format!("Database backup of {} completed", application_name)
            } else {
                format!("Database backup of {} failed", application_name)
            },
            outcome: if succeeded {
                Outcome::Success
            } else {
                Outcome::Failure
            },
            fields,
            description: error.map(sanitize_description),
            url: None,
            timestamp: now_ms(),
        },
    }
}

pub struct CleanupSummary {
    pub containers_freed: i64,
    pub containers_space: i64,
    pub images_freed: i64,
    pub images_space: i64,
    pub volumes_freed: i64,
    pub volumes_space: i64,
}

pub fn docker_cleanup(summary: CleanupSummary, actor: Option<&str>) -> Event {
    let reclaimed = summary.containers_space + summary.images_space + summary.volumes_space;
    let mut fields = vec![
        Field::new("Containers", summary.containers_freed.to_string()),
        Field::new("Images", summary.images_freed.to_string()),
        Field::new("Volumes", summary.volumes_freed.to_string()),
        Field::new("Reclaimed", human_bytes(reclaimed)),
    ];
    if let Some(actor) = actor {
        fields.push(Field::new("Run by", actor));
    }
    Event {
        action: Action::DockerCleanup,
        scope: Scope::Global,
        payload: EventPayload {
            action: Action::DockerCleanup.key().to_string(),
            title: "Cleanup finished".to_string(),
            outcome: Outcome::Info,
            fields,
            description: None,
            url: None,
            timestamp: now_ms(),
        },
    }
}

pub fn litepod_restart(target: &str, actor: Option<&str>) -> Event {
    let mut fields = vec![Field::new("Target", target)];
    if let Some(actor) = actor {
        fields.push(Field::new("Requested by", actor));
    }
    Event {
        action: Action::LitepodRestart,
        scope: Scope::Global,
        payload: EventPayload {
            action: Action::LitepodRestart.key().to_string(),
            title: format!("Restarting {target}"),
            outcome: Outcome::Info,
            fields,
            description: None,
            url: None,
            timestamp: now_ms(),
        },
    }
}

/// A sign-in or failed-login pattern `login_anomaly` flagged.
pub fn security_alert(
    flag: &str,
    subject: &str,
    ip: &str,
    place: Option<&str>,
    url: Option<String>,
) -> Event {
    let title = match flag {
        "impossible_travel" => format!("Impossible travel for {subject}"),
        "new_country" => format!("Sign-in from a new country for {subject}"),
        _ => format!("Failed logins from many IPs for {subject}"),
    };
    let mut fields = vec![Field::new("Account", subject), Field::new("IP", ip)];
    if let Some(place) = place.filter(|place| !place.is_empty()) {
        fields.push(Field::new("Location", place));
    }
    Event {
        action: Action::SecurityAlert,
        scope: Scope::Global,
        payload: EventPayload {
            action: Action::SecurityAlert.key().to_string(),
            title,
            outcome: Outcome::Failure,
            fields,
            description: None,
            url,
            timestamp: now_ms(),
        },
    }
}

/// What a reboot restore did on one server, for one organization.
#[derive(Debug, Default)]
pub struct RebootReport {
    pub started: Vec<String>,
    /// Application name and a short error.
    pub failed: Vec<(String, String)>,
    /// Left stopped on purpose (`notify` policy) or unrecoverable without a redeploy.
    pub pending: Vec<String>,
}

/// One summary per organization per reboot. `None` when nothing needed doing, so an
/// organization whose apps were all deliberately stopped hears nothing.
pub fn server_reboot(
    organization_id: &str,
    server_name: &str,
    report: &RebootReport,
) -> Option<Event> {
    if report.started.is_empty() && report.failed.is_empty() && report.pending.is_empty() {
        return None;
    }
    let outcome = if !report.failed.is_empty() {
        Outcome::Failure
    } else if !report.started.is_empty() {
        Outcome::Success
    } else {
        Outcome::Info
    };
    let mut fields = vec![Field::new("Server", server_name)];
    if !report.started.is_empty() {
        fields.push(Field::new("Started", report.started.join(", ")));
    }
    if !report.failed.is_empty() {
        let names: Vec<&str> = report
            .failed
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        fields.push(Field::new("Failed", names.join(", ")));
    }
    if !report.pending.is_empty() {
        fields.push(Field::new("Left stopped", report.pending.join(", ")));
    }
    let description = (!report.failed.is_empty()).then(|| {
        let lines: Vec<String> = report
            .failed
            .iter()
            .map(|(name, error)| format!("{name}: {error}"))
            .collect();
        sanitize_description(&lines.join("\n"))
    });
    Some(Event {
        action: Action::ServerReboot,
        scope: Scope::Organization(organization_id.to_string()),
        payload: EventPayload {
            action: Action::ServerReboot.key().to_string(),
            title: format!("{server_name} restarted"),
            outcome,
            fields,
            description,
            url: None,
            timestamp: now_ms(),
        },
    })
}

/// The message the "Test Notification" button sends. Uses `Info` so a test can
/// never be mistaken for a real success in a channel's history.
pub fn test_event() -> EventPayload {
    EventPayload {
        action: "test".to_string(),
        title: "litepod test notification".to_string(),
        outcome: Outcome::Info,
        fields: vec![Field::new("Status", "Your webhook is configured correctly")],
        description: Some(
            "If you can read this, litepod can reach this channel. No further action is needed."
                .to_string(),
        ),
        url: None,
        timestamp: now_ms(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_keys_round_trip() {
        for action in Action::ALL {
            assert_eq!(Action::parse(action.key()), Some(action));
        }
        assert_eq!(Action::parse("not_an_action"), None);
    }

    #[test]
    fn only_platform_actions_are_global() {
        assert!(!Action::AppDeploy.is_global());
        assert!(!Action::AppBuildError.is_global());
        assert!(Action::LitepodBackup.is_global());
        assert!(Action::DockerCleanup.is_global());
        assert!(Action::LitepodRestart.is_global());
        assert!(!Action::ServerReboot.is_global());
    }

    #[test]
    fn server_reboot_summarizes_and_stays_quiet_when_nothing_happened() {
        assert!(server_reboot("org", "vps", &RebootReport::default()).is_none());

        let started = RebootReport {
            started: vec!["umami".into(), "vaultwarden".into()],
            ..Default::default()
        };
        let event = server_reboot("org", "vps", &started).unwrap();
        assert_eq!(event.action, Action::ServerReboot);
        assert!(matches!(event.scope, Scope::Organization(ref id) if id == "org"));
        assert_eq!(event.payload.outcome, Outcome::Success);
        assert!(
            event
                .payload
                .fields
                .iter()
                .any(|f| f.label == "Started" && f.value == "umami, vaultwarden")
        );

        let failed = RebootReport {
            started: vec!["umami".into()],
            failed: vec![("ente".into(), "Podman command failed: boom".into())],
            ..Default::default()
        };
        let event = server_reboot("org", "vps", &failed).unwrap();
        assert_eq!(event.payload.outcome, Outcome::Failure);
        assert!(
            event
                .payload
                .description
                .unwrap()
                .contains("ente: Podman command failed: boom")
        );

        let pending = RebootReport {
            pending: vec!["mailpit".into()],
            ..Default::default()
        };
        assert_eq!(
            server_reboot("org", "vps", &pending)
                .unwrap()
                .payload
                .outcome,
            Outcome::Info
        );
    }

    #[test]
    fn description_is_stripped_and_clamped() {
        let noisy = format!("\u{1b}[31mboom\u{1b}[0m {}", "x".repeat(600));
        let out = sanitize_description(&noisy);
        assert!(!out.contains('\u{1b}'));
        assert!(out.starts_with("boom"));
        assert_eq!(out.chars().count(), 501); // 500 + ellipsis
    }

    #[test]
    fn byte_and_duration_formatting() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1024), "1.0 KB");
        assert_eq!(human_bytes(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(human_duration(4_000), "4s");
        assert_eq!(human_duration(95_000), "1m 35s");
        assert_eq!(human_duration(3_720_000), "1h 2m");
    }

    #[test]
    fn build_error_records_the_failing_phase() {
        let context = DeploymentContext {
            organization_id: "organization-1".into(),
            application_name: "api".into(),
            project_name: "litepod".into(),
            environment_name: "production".into(),
            commit_sha: Some("0123456789abcdef".into()),
            commit_message: Some("fix: thing\nbody".into()),
            triggered_by: "manual".into(),
            duration_ms: Some(12_000),
            url: None,
        };
        let event = app_build_error(context, "building", Some("compile error"));
        assert_eq!(event.action, Action::AppBuildError);
        assert_eq!(event.payload.outcome, Outcome::Failure);
        let commit = event
            .payload
            .fields
            .iter()
            .find(|f| f.label == "Commit")
            .expect("commit field");
        assert_eq!(commit.value, "`0123456` fix: thing");
        let phase = event
            .payload
            .fields
            .iter()
            .find(|f| f.label == "Failed during")
            .expect("phase field");
        assert_eq!(phase.value, "Build");
    }
}
