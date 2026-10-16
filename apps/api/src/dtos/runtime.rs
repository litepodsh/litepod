use serde::Serialize;
use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
pub struct RuntimeResponse {
    /// RFC 3339 timestamp using the API server's local UTC offset.
    pub server_time: String,
    /// RFC 3339 timestamp for the current API process startup time.
    pub started_at: String,
    /// Current API server UTC offset in seconds.
    pub utc_offset_seconds: i32,
    /// Operating system the API process runs on, e.g. `macos` or `linux`.
    pub host_os: String,
    /// Host operating-system version when the platform can provide it.
    pub host_version: Option<String>,
    /// Linux kernel release. Absent on non-Linux hosts.
    pub host_kernel_version: Option<String>,
    /// CPU architecture the API process runs on, e.g. `aarch64` or `x86_64`.
    pub host_arch: String,
    /// Linux distribution id from `/etc/os-release`, e.g. `ubuntu` or `fedora`.
    /// Absent off Linux and when the file cannot be read.
    pub host_distribution: Option<String>,
    /// Operating system podman runs on, which is the VM guest on macOS.
    /// Absent while podman is unreachable.
    pub podman_host_os: Option<String>,
    /// CPU architecture podman runs on. Absent while podman is unreachable.
    pub podman_host_arch: Option<String>,
}
