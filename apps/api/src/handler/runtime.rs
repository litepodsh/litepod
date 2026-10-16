use axum::{extract::State, response::IntoResponse};
use chrono::{DateTime, Local};
use podman_api::Podman;

use crate::{
    dtos::{response::ok, runtime::RuntimeResponse},
    state::state::{PodmanHostInfo, SharedState},
};

/// Reads the distribution id out of an `/etc/os-release` body, so the UI can show
/// a distro mark instead of the generic Linux one.
fn distribution_id(os_release: &str) -> Option<String> {
    os_release
        .lines()
        .find_map(|line| line.strip_prefix("ID="))
        .map(|value| value.trim().trim_matches('"').to_ascii_lowercase())
        .filter(|value| !value.is_empty())
}

fn os_release_value(os_release: &str, key: &str) -> Option<String> {
    os_release
        .lines()
        .find_map(|line| line.strip_prefix(key))
        .map(|value| value.trim().trim_matches('"').to_string())
        .filter(|value| !value.is_empty())
}

pub fn host_distribution() -> Option<String> {
    distribution_id(&std::fs::read_to_string("/etc/os-release").ok()?)
}

#[cfg(target_os = "macos")]
fn host_version() -> Option<String> {
    std::process::Command::new("sw_vers")
        .arg("-productVersion")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|version| version.trim().to_string())
        .filter(|version| !version.is_empty())
}

#[cfg(not(target_os = "macos"))]
fn host_version() -> Option<String> {
    os_release_value(
        &std::fs::read_to_string("/etc/os-release").ok()?,
        "VERSION_ID=",
    )
}

#[cfg(target_os = "linux")]
fn host_kernel_version() -> Option<String> {
    std::process::Command::new("uname")
        .arg("-r")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|version| version.trim().to_string())
        .filter(|version| !version.is_empty())
}

#[cfg(not(target_os = "linux"))]
fn host_kernel_version() -> Option<String> {
    None
}

pub fn runtime_response(
    started_at: DateTime<Local>,
    now: DateTime<Local>,
    podman_host: Option<PodmanHostInfo>,
) -> RuntimeResponse {
    RuntimeResponse {
        server_time: now.to_rfc3339(),
        started_at: started_at.to_rfc3339(),
        utc_offset_seconds: now.offset().local_minus_utc(),
        host_os: std::env::consts::OS.to_string(),
        host_version: host_version(),
        host_kernel_version: host_kernel_version(),
        host_arch: std::env::consts::ARCH.to_string(),
        host_distribution: host_distribution(),
        podman_host_os: podman_host.as_ref().map(|host| host.os.clone()),
        podman_host_arch: podman_host.map(|host| host.arch),
    }
}

/// Podman's own OS never changes while the service is up, so it is resolved once
/// and kept. A failed lookup leaves the cell empty so a later request can retry.
async fn podman_host(state: &SharedState) -> Option<PodmanHostInfo> {
    state
        .podman_host
        .get_or_try_init(|| async {
            let info = Podman::new(&state.podman_socket)?.info().await?;
            let host = info.host.ok_or(podman_api::Error::InvalidResponse(
                "podman reported no host info".into(),
            ))?;
            Ok::<_, podman_api::Error>(PodmanHostInfo {
                os: host.os.unwrap_or_default(),
                arch: host.arch.unwrap_or_default(),
            })
        })
        .await
        .ok()
        .cloned()
}

#[utoipa::path(
    get,
    path = "/api/v1/runtime",
    responses((status = 200, description = "API server runtime status", body = RuntimeResponse)),
    tag = "health"
)]
pub async fn get_runtime_handler(State(state): State<SharedState>) -> impl IntoResponse {
    let podman = podman_host(&state).await;
    ok(runtime_response(state.started_at, Local::now(), podman))
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, Local};

    use super::{PodmanHostInfo, distribution_id, os_release_value, runtime_response};

    #[test]
    fn runtime_response_preserves_server_local_timestamps_and_offset() {
        let now = Local::now();
        let started_at = now - Duration::minutes(5);
        let response = runtime_response(started_at, now, None);

        assert_eq!(response.utc_offset_seconds, now.offset().local_minus_utc());
        assert_eq!(response.server_time, now.to_rfc3339());
        assert_eq!(response.started_at, started_at.to_rfc3339());
        assert_eq!(response.host_os, std::env::consts::OS);
        assert!(response.podman_host_os.is_none());
    }

    #[test]
    fn runtime_response_surfaces_the_podman_host_when_known() {
        let now = Local::now();
        let response = runtime_response(
            now,
            now,
            Some(PodmanHostInfo {
                os: "linux".into(),
                arch: "aarch64".into(),
            }),
        );

        assert_eq!(response.podman_host_os.as_deref(), Some("linux"));
        assert_eq!(response.podman_host_arch.as_deref(), Some("aarch64"));
    }

    #[test]
    fn distribution_ids_are_unquoted_and_lowercased() {
        assert_eq!(
            distribution_id("NAME=\"Ubuntu\"\nID=ubuntu\nVERSION_ID=\"24.04\"").as_deref(),
            Some("ubuntu")
        );
        assert_eq!(distribution_id("ID=\"Fedora\"").as_deref(), Some("fedora"));
        assert_eq!(distribution_id("NAME=\"Alpine Linux\""), None);
    }

    #[test]
    fn reads_os_release_versions() {
        assert_eq!(
            os_release_value("ID=ubuntu\nVERSION_ID=\"24.04\"", "VERSION_ID=").as_deref(),
            Some("24.04")
        );
    }
}
