//! Anonymous install counter. Sends
//! `{instance_id, version, is_cloud, podman_mode, arch, os_family}` to the Litepod status
//! service once a day so the project can tell how many self-hosted instances exist and how
//! they run. Nothing else leaves the server: no domains, emails, app names or IPs.
//!
//! On by default; the sysadmin can turn it off in System > Telemetry, or an operator
//! can force it off with `LITEPOD_TELEMETRY=off`.

use std::time::Duration;

use crate::state::state::SharedState;

const DEFAULT_BASE_URL: &str = "https://status.litepod.sh";
const ALIVE_PATH: &str = "/alive";
/// Wait after boot before the first ping, so startup work is not competing with it.
const FIRST_PING_DELAY: Duration = Duration::from_secs(60);
/// Time between pings after the first one: once every 24 hours.
const INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
/// Give up on a single ping request after 10 seconds.
const TIMEOUT: Duration = Duration::from_secs(10);

/// Operator override that wins over the stored setting.
pub(super) fn env_disabled() -> bool {
    std::env::var("LITEPOD_TELEMETRY").is_ok_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "off" | "false" | "0" | "no" | "disabled"
        )
    })
}

/// The instance id, created as a UUID v7 (hex, no dashes) the first time it is needed.
pub(super) async fn instance_id(pool: &sqlx::SqlitePool) -> Result<String, sqlx::Error> {
    let existing: Option<String> =
        sqlx::query_scalar("SELECT instance_id FROM instance_telemetry WHERE id = 1")
            .fetch_one(pool)
            .await?;
    if let Some(id) = existing.filter(|id| !id.is_empty()) {
        return Ok(id);
    }
    let id = uuid::Uuid::now_v7().simple().to_string();
    // `IS NULL` keeps a concurrent first call from replacing the id the other one stored.
    sqlx::query(
        "UPDATE instance_telemetry SET instance_id = ?1 WHERE id = 1 AND instance_id IS NULL",
    )
    .bind(&id)
    .execute(pool)
    .await?;
    sqlx::query_scalar("SELECT instance_id FROM instance_telemetry WHERE id = 1")
        .fetch_one(pool)
        .await
}

async fn stored_enabled(pool: &sqlx::SqlitePool) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar::<_, i64>("SELECT enabled FROM instance_telemetry WHERE id = 1")
        .fetch_one(pool)
        .await
        .map(|enabled| enabled != 0)
}

/// How Podman runs for this install, read from the endpoint the API was started with.
/// Rootless sockets live under `/run/user/<uid>`; rootful uses the system socket. `None`
/// for an endpoint we do not recognise, so the status service shows "unknown" instead of
/// guessing.
pub(super) fn podman_mode(endpoint: &str) -> Option<&'static str> {
    let endpoint = endpoint.trim();
    if endpoint.contains("/run/user/") {
        Some("rootless")
    } else if endpoint.ends_with("/run/podman/podman.sock") {
        Some("rootful")
    } else {
        None
    }
}

/// Host distro family, written into `.env` by the installer. The API runs in a container,
/// so its own `/etc/os-release` describes the image, not the machine. Only known families
/// are sent; anything else is omitted.
fn os_family(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "debian" => Some("debian"),
        "fedora" => Some("fedora"),
        "arch" => Some("arch"),
        _ => None,
    }
}

fn host_os_family() -> Option<&'static str> {
    os_family(&std::env::var("LITEPOD_HOST_OS_FAMILY").ok()?)
}

/// `toggle` is `Some("enabled" | "disabled")` when the sysadmin flips the setting, so the
/// status service can tell a manual opt-out apart from an instance that just stopped pinging.
async fn ping(state: &SharedState, toggle: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let id = instance_id(&state.db).await?;
    let base =
        std::env::var("LITEPOD_TELEMETRY_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());
    let url = format!("{}{ALIVE_PATH}", base.trim_end_matches('/'));
    let mut body = serde_json::json!({
        "instance_id": id,
        "version": state.version,
        "is_cloud": state.is_cloud,
        "arch": std::env::consts::ARCH,
    });
    if let Some(mode) = podman_mode(&state.podman_socket) {
        body["podman_mode"] = mode.into();
    }
    if let Some(family) = host_os_family() {
        body["os_family"] = family.into();
    }
    if let Some(toggle) = toggle {
        body["telemetry"] = toggle.into();
    }
    reqwest::Client::builder()
        .timeout(TIMEOUT)
        .build()?
        .post(url)
        .json(&body)
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

/// One-off ping reporting that the sysadmin turned telemetry on or off. Skipped for dev builds
/// and when `LITEPOD_TELEMETRY=off` forces it off (the toggle has no effect then).
pub(super) fn notify_toggle(state: &SharedState, enabled: bool) {
    if state.dev_mode || state.version == "dev" || env_disabled() {
        return;
    }
    let state = state.clone();
    tokio::spawn(async move {
        let toggle = if enabled { "enabled" } else { "disabled" };
        if let Err(error) = ping(&state, Some(toggle)).await {
            tracing::debug!(%error, "telemetry toggle ping failed");
        }
    });
}

/// Daily ping loop. Skipped for dev builds. Litepod Cloud pings too, flagged `is_cloud`,
/// so the status service can show which instance is the cloud one.
/// Failures are logged at debug level and never affect the API.
pub fn spawn(state: SharedState) {
    if state.dev_mode || state.version == "dev" {
        return;
    }
    tokio::spawn(async move {
        tokio::time::sleep(FIRST_PING_DELAY).await;
        loop {
            // Re-read the setting every round so a toggle takes effect without a restart.
            let enabled = !env_disabled() && stored_enabled(&state.db).await.unwrap_or(false);
            if enabled && let Err(error) = ping(&state, None).await {
                tracing::debug!(%error, "telemetry ping failed");
            }
            tokio::time::sleep(INTERVAL).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{os_family, podman_mode};

    #[test]
    fn rootless_socket_reports_rootless() {
        assert_eq!(
            podman_mode("unix:///run/user/1000/podman/podman.sock"),
            Some("rootless")
        );
    }

    #[test]
    fn rootful_socket_reports_rootful() {
        assert_eq!(
            podman_mode("unix:///run/podman/podman.sock"),
            Some("rootful")
        );
    }

    #[test]
    fn unknown_endpoint_reports_nothing() {
        assert_eq!(podman_mode("tcp://podman.example:8080"), None);
        assert_eq!(podman_mode(""), None);
    }

    #[test]
    fn os_family_accepts_only_known_families() {
        assert_eq!(os_family("debian"), Some("debian"));
        assert_eq!(os_family(" Fedora\n"), Some("fedora"));
        assert_eq!(os_family("arch"), Some("arch"));
        assert_eq!(os_family("ubuntu"), None);
        assert_eq!(os_family(""), None);
    }
}
