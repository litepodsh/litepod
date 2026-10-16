use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::servers::ssh::SshUnixRelay;
use podman_api::api::Exec;
use tokio::sync::{OnceCell, RwLock};

pub type SharedState = Arc<AppState>;

/// Operating system the podman service runs on, which differs from the API host
/// whenever podman lives in a VM (macOS) or on a remote machine.
#[derive(Clone)]
pub struct PodmanHostInfo {
    pub os: String,
    pub arch: String,
}

/// Per-volume resolution of how the API reaches a named volume's contents.
/// `Some(path)` means the volume mountpoint is readable from this process and
/// can be walked directly; `None` means every operation needs a helper container.
pub type VolumeAccessCache = RwLock<HashMap<String, Option<PathBuf>>>;

pub struct PendingExec {
    pub exec: Exec,
    /// Keeps the SSH Unix-socket relay alive until the terminal WebSocket
    /// closes. Local sessions carry no relay.
    pub _remote_relay: Option<SshUnixRelay>,
}

pub struct AppState {
    pub started_at: chrono::DateTime<chrono::Local>,
    pub db: sqlx::SqlitePool,
    #[allow(dead_code)] // exposed for telemetry dashboards that aren't shipped yet.
    pub caddy_url: String,
    pub caddy_admin_url: String,
    pub caddyfile_path: std::path::PathBuf,
    /// The internal address Caddy uses for litepod's own UI/API server route.
    pub caddy_server_upstream: String,
    pub podman_socket: String,
    pub pending_execs: Mutex<HashMap<String, PendingExec>>,
    pub data_dir: std::path::PathBuf,
    pub data_volume: Option<String>,
    pub host_data_dir: std::path::PathBuf,
    pub podman_network: String,
    pub public_url: String,
    pub oauth_states: Mutex<HashMap<String, String>>,
    pub public_ip: String,
    /// True when this control plane may deploy only to registered SSH servers.
    pub is_cloud: bool,
    /// Optional Polar configuration. Standard/self-hosted installs continue
    /// without billing credentials.
    pub polar: Option<crate::billing::config::PolarConfig>,
    /// Local development: deployed apps are reached on `localhost:<host_port>`
    /// instead of their Caddy-routed sslip.io hostname.
    pub dev_mode: bool,
    pub port_range: (i64, i64),
    pub reserved_ports: std::collections::HashSet<i64>,
    pub volume_access: VolumeAccessCache,
    pub podman_host: OnceCell<PodmanHostInfo>,
    pub queue_runtime: crate::deployment::queue::QueueRuntime,
    // ── build-time identity (from Dockerfile ARGs) ────────────────────
    pub version: String,
    pub git_sha: String,
    // ── self-update config ────────────────────────────────────────────
    /// Full repo, e.g. `docker.io/litepod/litepod`
    pub image_repo: String,
    /// Tag this container was started with. The persisted pin wins over it
    /// once an update has been applied.
    pub image_tag: String,
    /// Release channel selected at boot from `LITEPOD_UPDATE_CHANNEL`.
    pub update_channel: crate::updater::UpdateChannel,
    /// URL of the published release manifest.
    pub version_manifest_url: String,
    /// Seconds between update checks; 0 disables
    pub update_interval_secs: u64,
    /// Container we are running in (for self-recreate). Default: hostname.
    pub container_name: String,
    pub caddy_container: String,
    pub stack_env_file: Option<std::path::PathBuf>,
    /// Live update state exposed by /api/v1/system/version
    pub update_state: RwLock<crate::updater::UpdateState>,
    /// Serializes backup and restore runs: only one may be in flight at a time.
    /// `Arc`-wrapped so a handler can take an owned guard (`try_lock_owned`)
    /// that lives inside a `tokio::spawn`ed background task.
    pub backup_lock: Arc<tokio::sync::Mutex<()>>,
    /// Global + auth request throttles and login failed-attempt bans. Held behind one lock
    /// so a settings update can swap the limiters and ban-tracking config atomically.
    pub rate_limiter: RwLock<crate::security::rate_limit::RateLimiterState>,
    /// Redis-compatible ephemeral state. A connection failure is non-fatal; callers fall back
    /// to process-local behavior for security limits and to direct reads for caches.
    pub ephemeral: crate::scheduled_tasks::ephemeral::EphemeralStore,
}
