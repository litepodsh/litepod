use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::servers::model::{IngressMode, PodmanMode, ServerState};
use crate::servers::repository::ServerRecord;
use crate::servers::setup::ServerFacts;

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CapacityDto {
    pub cpu_cores: Option<f64>,
    pub memory_bytes: Option<u64>,
    pub disk_free_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ServerFactsDto {
    pub os: Option<String>,
    pub version: Option<String>,
    pub architecture: Option<String>,
    pub podman_version: Option<String>,
    pub caddy_version: Option<String>,
    pub swap_bytes: Option<u64>,
    /// No swap and 16 GiB of RAM or less; the UI offers to create a swap file.
    pub swap_recommended: bool,
    pub capacity: CapacityDto,
}

/// An application that keeps a server from being removed. Named by project and environment
/// because an application name alone is not unique across an organization.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ServerAssignedApplicationDto {
    pub id: String,
    pub name: String,
    pub environment_id: String,
    pub environment_name: String,
    /// With the environment name, enough for the UI to link straight to the application.
    pub project_id: String,
    pub project_name: String,
    pub deploy_type: String,
    pub template_kind: Option<String>,
    pub icon_url: Option<String>,
    pub catalog_icon_url: Option<String>,
}

/// Result of running a throwaway container on a server. A failed run still returns 200 with
/// `success: false` — the output is the answer the user asked for either way.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ServerTestDeployDto {
    pub success: bool,
    pub image: String,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ServerMaintenanceResultDto {
    pub action: String,
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ServerErrorDto {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ServerSummaryDto {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub state: ServerState,
    pub ingress_mode: IngressMode,
    pub capacity: CapacityDto,
    /// Platform family, only known for the local server. Remote hosts report a
    /// distribution id instead, which already implies Linux.
    pub os: Option<String>,
    pub distribution: Option<String>,
    pub os_version: Option<String>,
}

/// Platform, distribution and version for a server, as `(os, distribution, os_version)`.
///
/// The local row is never probed over SSH, so its mark comes from the host litepod itself runs
/// on. A remote host only reports a distribution id, which already implies Linux.
pub fn os_marks(
    kind: &str,
    facts: &ServerFacts,
) -> (Option<String>, Option<String>, Option<String>) {
    if kind == "local" {
        (
            Some(std::env::consts::OS.to_string()),
            crate::handler::runtime::host_distribution(),
            None,
        )
    } else {
        (None, facts.os.clone(), facts.version.clone())
    }
}

/// `(os, distribution, address)` for a server row, as replica views show it.
pub fn replica_marks(server: &ServerRecord) -> (Option<String>, Option<String>, Option<String>) {
    let facts = serde_json::from_str::<ServerFacts>(&server.facts_json).unwrap_or_default();
    let (os, distribution, _) = os_marks(&server.kind, &facts);
    let address = server
        .public_address
        .clone()
        .or_else(|| server.ssh_host.clone())
        .filter(|address| !address.is_empty());
    (os, distribution, address)
}

/// The one place an application's placement is turned into the shape the UI shows, so a
/// card on a project page and the server picker always name the same host the same way.
pub fn server_summary(server: &ServerRecord, facts: &ServerFacts) -> ServerSummaryDto {
    let (os, distribution, os_version) = os_marks(&server.kind, facts);
    ServerSummaryDto {
        id: server.id.clone(),
        kind: server.kind.clone(),
        name: server.name.clone(),
        state: server.state,
        ingress_mode: server.ingress_mode,
        capacity: CapacityDto {
            cpu_cores: facts.cpu_cores,
            memory_bytes: facts.memory_bytes,
            disk_free_bytes: facts.disk_free_bytes,
        },
        os,
        distribution,
        os_version,
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ServerDto {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub ingress_mode: IngressMode,
    pub podman_mode: PodmanMode,
    pub state: ServerState,
    pub public_address: Option<String>,
    pub facts: ServerFactsDto,
    pub last_error: Option<ServerErrorDto>,
    pub assigned_application_count: u64,
    pub ssh_host: Option<String>,
    pub ssh_port: Option<u16>,
    pub ssh_user: Option<String>,
    pub host_key_fingerprint: Option<String>,
    /// `null` marks the shared `local` server of a self-hosted install, which
    /// every organization on the instance can deploy to. Remote servers always
    /// name their owner, so the UI can tell "ours" from "the instance's".
    pub organization_id: Option<String>,
}

/// One pin on the fleet map. Coordinates are absent whenever the server's
/// address could not be placed; `unlocated_reason` says why, so the UI can list
/// the server instead of silently dropping it.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ServerGeoDto {
    pub server_id: String,
    pub name: String,
    pub kind: String,
    pub state: ServerState,
    pub ingress_mode: IngressMode,
    /// The instance's own host, the one every remote server hangs off.
    pub is_main: bool,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub city: Option<String>,
    pub country: Option<String>,
    /// IANA zone, so the UI can show the server's own wall clock.
    pub timezone: Option<String>,
    pub asn: Option<String>,
    /// `no_address`, `private_address` or `lookup_failed`. `None` when located.
    pub unlocated_reason: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateServerRequest {
    pub name: String,
    pub host: String,
    pub port: Option<u16>,
    pub user: String,
    pub ingress_mode: IngressMode,
    /// Defaults to rootless, which is the supported and recommended configuration.
    #[serde(default)]
    pub podman_mode: PodmanMode,
    pub public_address: Option<String>,
    pub credential: ServerCredentialInput,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RenameServerRequest {
    pub name: String,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ServerCredentialInput {
    Generate,
    Import { private_key: String },
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ServerJobDto {
    pub id: String,
    pub server_id: String,
    pub kind: String,
    pub status: String,
    pub phase: String,
    pub log: String,
    pub error: Option<ServerErrorDto>,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ServerRegistrationDto {
    pub server: ServerDto,
    pub generated_public_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ServerPublicKeyDto {
    pub public_key: String,
    /// Paste-ready one-liner that installs `public_key` for the configured SSH user.
    pub authorized_keys_command: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ServerPreflightDto {
    pub server: ServerDto,
    /// Present only before the sysadmin confirms first contact. It is never persisted until
    /// `confirm-host-key` receives the exact value.
    pub observed_host_key_fingerprint: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RemoteServerCapacityDto {
    pub available: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ConfirmHostKeyRequest {
    pub fingerprint: String,
}
