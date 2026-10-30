use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ArtifactDto {
    pub id: String,
    pub filename: String,
    pub kind: String,
    pub byte_size: i64,
    pub sha256: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DeploymentDto {
    pub status: String,
    pub container_port: i64,
    pub domains: Vec<DomainDto>,
    /// Address of the primary domain; `None` when the application has no domains.
    pub public_url: Option<String>,
    /// True when a published port changed and the container has not been recreated.
    pub config_stale: bool,
    pub last_error: Option<String>,
    pub deployed_at: Option<i64>,
    pub artifact: Option<ArtifactDto>,
    /// Per-server replica state for a group-targeted application. Empty for a
    /// single-server application, which keeps its single `status`.
    pub replicas: Vec<ApplicationReplicaDto>,
}

/// One replica's deployment state for a group-targeted application.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ApplicationReplicaDto {
    pub server_id: String,
    pub server_name: String,
    /// Same platform marks as `ServerSummaryDto`, so the UI can draw the OS icon.
    pub os: Option<String>,
    pub distribution: Option<String>,
    /// Public address when known, otherwise the SSH host.
    pub address: Option<String>,
    pub status: String,
    pub last_error: Option<String>,
    pub deployed_at: Option<i64>,
}

/// Live resource usage for the container backing an application. Populated only
/// while the application is running; a stopped/never-deployed app still returns
/// `status` with the usage fields left `None` rather than erroring.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ApplicationStatsDto {
    pub status: String,
    pub cpu_percent: Option<f64>,
    pub mem_usage_bytes: Option<u64>,
    pub mem_limit_bytes: Option<u64>,
}

/// Real-time container state from Podman inspect, not the DB-stored status.
/// `status` is the raw Podman state; `label` is a user-friendly display string.
/// `container_name` is the canonical name assigned on deploy — shown regardless
/// of whether the container is running or stopped.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ContainerStatusDto {
    pub status: String,
    pub label: String,
    pub container_name: Option<String>,
    /// Release-independent name other containers on `litepod-network` can dial.
    /// `container_name` changes on every redeploy; this one does not.
    pub internal_host: Option<String>,
    /// Container port the application listens on, when litepod knows it.
    pub internal_port: Option<u16>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ApplicationNetworkOverviewDto {
    pub containers: Vec<ApplicationNetworkContainerDto>,
    pub networks: Vec<ApplicationNetworkDto>,
    pub findings: Vec<ApplicationNetworkFindingDto>,
    pub observed_at: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ApplicationNetworkContainerDto {
    pub name: String,
    pub service: Option<String>,
    pub networks: Vec<ApplicationNetworkAttachmentDto>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ApplicationNetworkAttachmentDto {
    pub name: String,
    pub ip_address: Option<String>,
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ApplicationNetworkDto {
    pub name: String,
    pub driver: Option<String>,
    pub scope: String,
    pub application_member_count: usize,
    pub external_member_count: usize,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ApplicationNetworkFindingDto {
    pub code: String,
    pub severity: String,
    pub message: String,
    /// The container the UI can act on (e.g. detach from a network). `None` for
    /// aggregate findings that name no single container.
    pub container: Option<String>,
    /// The network the remediation applies to. `None` unless `container` is set.
    pub network: Option<String>,
}

/// What the Networks tab needs about resource networks: the database resources
/// this application can join, and — when the application is itself a database
/// resource — the applications that have joined its own network.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ResourceNetworksDto {
    pub linkable: Vec<ResourceNetworkLinkDto>,
    pub consumers: Vec<LinkedApplicationDto>,
}

/// An application linked to this resource's network.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct LinkedApplicationDto {
    pub application_id: String,
    pub name: String,
}

/// One internal database resource an application can join on the resource's own
/// Podman network. `linked` is the current state of the switch in the UI.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ResourceNetworkLinkDto {
    pub resource_application_id: String,
    pub name: String,
    pub template_kind: Option<String>,
    pub network_name: String,
    pub linked: bool,
}

/// Result of actively dialing between an application's own containers on their
/// shared private network. An active probe, not a passive read.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ApplicationNetworkProbeDto {
    pub pairs: Vec<NetworkProbePairDto>,
    pub observed_at: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct NetworkProbePairDto {
    /// Container names, so the UI can match the peer edge it drew.
    pub from: String,
    pub to: String,
    pub from_service: Option<String>,
    pub to_service: Option<String>,
    /// The port dialed, when one is known from a persisted proxy route.
    pub port: Option<u16>,
    /// `reachable` | `resolved` | `connection_refused` | `unreachable`
    /// | `name_not_resolved` | `no_tools` | `probe_failed` | `unknown`
    pub reason: String,
}

/// Cumulative per-container network counters from `podman stats`. The client
/// polls this and diffs two samples to show a rate.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ApplicationNetworkStatsDto {
    pub containers: Vec<ContainerNetStatDto>,
    pub observed_at: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ContainerNetStatDto {
    pub container: String,
    pub net_rx_bytes: u64,
    pub net_tx_bytes: u64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DeploymentRunActorDto {
    pub id: String,
    pub name: String,
    pub email: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DeploymentRunDto {
    pub id: String,
    pub source_type: String,
    pub build_method: String,
    pub status: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub duration_ms: Option<i64>,
    pub last_error: Option<String>,
    pub commit_sha: Option<String>,
    pub commit_message: Option<String>,
    pub commit_author_name: Option<String>,
    pub commit_author_email: Option<String>,
    pub triggered_by: Option<String>,
    pub branch: Option<String>,
    pub job_id: Option<String>,
    pub actor: Option<DeploymentRunActorDto>,
    /// Per-server outcome inside this run, for a group-targeted application;
    /// empty for a single-server one.
    pub replicas: Vec<DeploymentRunReplicaDto>,
}

/// One replica's progress inside a deployment run.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DeploymentRunReplicaDto {
    pub server_id: String,
    pub server_name: String,
    pub os: Option<String>,
    pub distribution: Option<String>,
    pub address: Option<String>,
    pub status: String,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DeploymentRunLogDto {
    pub sequence: i64,
    pub level: String,
    pub message: String,
    pub created_at: i64,
    /// The replica that wrote the line; `None` for run-level lines.
    pub server_id: Option<String>,
}

/// One recorded pipeline step of a run (`clone`, `build`, `pull`, `prepare`,
/// `start`, `health_check`, `route`).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DeploymentRunStepDto {
    pub name: String,
    /// `pending`, `running`, `succeeded`, `failed` or `skipped`.
    pub status: String,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    /// The replica this step belongs to; `None` for a single-server run.
    pub server_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DeploymentRunDetailDto {
    #[serde(flatten)]
    pub run: DeploymentRunDto,
    /// Empty for runs recorded before steps existed and for lifecycle runs.
    pub steps: Vec<DeploymentRunStepDto>,
    pub logs: Vec<DeploymentRunLogDto>,
    pub total_log_count: i64,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DomainDto {
    pub certificate_id: Option<String>,
    pub certificate_name: Option<String>,
    pub certificate_provider: String,
    pub has_custom_certificate: bool,
    pub id: String,
    pub application_id: String,
    pub host: String,
    pub port: i64,
    pub https: bool,
    pub is_primary: bool,
    /// Fully resolved address, e.g. `http://localhost:10000`.
    pub url: String,
    /// True when the host is served by a published container port instead of Caddy.
    pub is_local: bool,
    /// The DNS A/AAAA target needed for this domain, when it is public. This
    /// never contains the remote server's SSH host or Caddy admin address.
    pub required_dns_target: Option<String>,
    /// `central`, `public_edge`, `private_relay`, or `private_only`.
    pub ingress_mode: String,
    /// `ready`, `pending`, `unavailable`, or `private_only`.
    pub route_health: String,
    /// Safe user-facing reason for a route that cannot currently serve traffic.
    pub route_health_reason: Option<String>,
    /// Which server this domain routes to. `None` for a single-server app (the
    /// app's own server); required for a group app.
    pub server_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Result of resolving an application domain against the selected public-edge server.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DomainDnsValidationDto {
    /// `valid`, `cdn`, `invalid`, `unresolved` (no A/AAAA record), or `error`
    /// (the lookup itself failed or timed out).
    pub status: String,
    /// A and AAAA addresses returned for the application domain.
    pub resolved_addresses: Vec<String>,
    /// First CNAME hop, when the host is an alias.
    pub cname: Option<String>,
    /// Addresses expected from the public-edge server's configured target.
    pub expected_addresses: Vec<String>,
    /// Set when validation recognizes a CDN in front of the server.
    pub cdn_provider: Option<String>,
    /// Safe, actionable detail for warnings and failed validation.
    pub message: Option<String>,
}

/// A route the database says Caddy should be serving.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ExpectedRouteDto {
    /// Caddy `@id` of the route, e.g. `litepod-route-<domain id>`.
    pub route_id: String,
    pub application_id: String,
    pub application_name: String,
    pub domain_id: String,
    pub host: String,
    pub https: bool,
    /// `container:port` Caddy has to dial, or `None` when the domain is not
    /// routable yet (deployment not running, or Compose domain with no service).
    pub dial: Option<String>,
}

/// One row of a public-edge server's route diff: a database domain, an
/// on-disk Caddy fragment, or both.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EdgeRouteRow {
    /// The domain id, which is also its edge fragment's `route_id`.
    pub domain_id: String,
    /// Empty for an orphaned fragment (no database row explains it).
    pub host: String,
    pub application_name: String,
    /// `ok` (routable and on disk), `missing` (routable, no fragment),
    /// `pending` (no loopback endpoint yet), or `orphaned` (fragment with no
    /// current domain).
    pub status: String,
}

/// The route diff for one remote server.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EdgeRoutesDto {
    pub rows: Vec<EdgeRouteRow>,
    /// True only for a public-edge server, which owns its Caddy and so has a
    /// per-server route helper to reconcile and prune. False for a
    /// private-relay or private-only server, whose routes live on the central
    /// proxy: the rows are informational and there is nothing to repair here.
    pub manages_routes: bool,
    /// True when a public-edge server's helper has no `list-routes` arm; the
    /// diff then only reflects the database, and pruning is unavailable until
    /// the server is repaired. Always false when `manages_routes` is false.
    pub stale_helper: bool,
}

/// A route currently loaded in Caddy, as read back from its admin API.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct LiveRouteDto {
    pub route_id: Option<String>,
    pub hosts: Vec<String>,
    pub dial: Option<String>,
}

/// One host's comparison between the database and Caddy.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RouteDiffEntryDto {
    pub host: String,
    pub route_id: String,
    pub application_id: Option<String>,
    pub application_name: Option<String>,
    /// `app` (belongs to a litepod application) or `manual` (a standalone route
    /// created from the Caddy console).
    pub source: String,
    /// `ok`, `missing` (in the database, absent from Caddy), `mismatched`
    /// (present but dialing something else), `unroutable` (database has no
    /// target), or `orphaned` (in Caddy with no database row).
    pub status: String,
    pub expected_dial: Option<String>,
    pub live_dial: Option<String>,
}

/// A standalone Caddy route managed from the console, not tied to any
/// application. `target` is `host:port` when `kind` is `reverse_proxy`, an
/// absolute URL when `kind` is `redirect`.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ManualRouteDto {
    pub id: String,
    pub server_id: String,
    pub host: String,
    pub kind: String,
    pub target: String,
    pub https: bool,
    pub redirect_status: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

/// What the debug page shows and what the reconcile button acts on.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RouteDiffDto {
    pub entries: Vec<RouteDiffEntryDto>,
    pub expected: Vec<ExpectedRouteDto>,
    pub live: Vec<LiveRouteDto>,
    /// Upstream litepod's own catch-all route dials, and whether it resolves.
    pub caddy_admin_url: String,
    pub needs_reconcile: usize,
}

/// Result of pressing the reconcile button.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RouteReconcileResultDto {
    pub pushed: usize,
    pub failed: usize,
}

/// Ports the domain dialog must refuse before submitting. `taken_ports` covers
/// every local domain across all applications, because a published host port
/// collides at the OS level regardless of which application claimed it.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PortConstraintsDto {
    pub reserved_ports: Vec<i64>,
    pub taken_ports: Vec<i64>,
}

/// A hostname typed in the add-domain dialog, checked before it is saved.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ValidateHostDnsRequest {
    pub host: String,
    /// Restrict the expected addresses to this server (a group pin).
    pub server_id: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateDomainRequest {
    #[serde(flatten)]
    pub certificate: crate::deployment::certificates::CertificateRequest,
    pub host: String,
    pub port: i64,
    pub https: Option<bool>,
    pub is_primary: Option<bool>,
    #[serde(default)]
    pub server_id: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateDomainRequest {
    #[serde(flatten)]
    pub certificate: crate::deployment::certificates::CertificateRequest,
    pub host: Option<String>,
    pub port: Option<i64>,
    pub https: Option<bool>,
    pub is_primary: Option<bool>,
    #[serde(default)]
    pub server_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EnvironmentVariableDto {
    pub scope: String,
    pub name: String,
    pub value: String,
    pub masked: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateEnvironmentRequest {
    pub content: String,
    pub scope: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SourceConfigDto {
    pub deploy_type: String,
    pub build_method: String,
    pub git_provider_id: Option<String>,
    pub git_url: Option<String>,
    pub git_branch: Option<String>,
    pub dockerfile_path: Option<String>,
    pub compose_file_path: Option<String>,
    pub compose_public_service: Option<String>,
    pub railpack_project_path: String,
    pub railpack_start_command: Option<String>,
    /// Registry-image deploys: the exact image reference (e.g. `ghcr.io/owner/app:tag`).
    pub image_url: Option<String>,
    /// Whether registry credentials are on file for a private registry image.
    /// The secret itself never leaves the backend.
    pub has_registry_credentials: bool,
    /// Container restart policy applied when this source's container is created
    /// (`no`, `always`, `unless-stopped`, `on-failure`).
    pub restart_policy: String,
    pub container_port: Option<i64>,
    /// Redeploy on every push to `git_branch`. Requires a GitHub App provider.
    pub auto_deploy: bool,
    /// Comma-separated path prefixes. When set, a push only triggers auto-deploy
    /// if it touched a file under one of these paths (monorepo scoping). `None`
    /// or empty means any push to `git_branch` redeploys, as before.
    pub watch_paths: Option<String>,
    /// Whether Railpack must rebuild without BuildKit layer cache.
    pub clean_build: bool,
    /// Optional CPU limit, expressed in CPU cores, applied to both the build
    /// and the running container/service.
    pub build_cpu_cores: Option<f64>,
    /// Optional memory limit, expressed in MiB, applied to both the build and
    /// the running container/service.
    pub build_memory_mib: Option<i64>,
    /// Extra swap, expressed in MiB, allowed on top of `build_memory_mib`.
    /// `None` or `0` means no extra swap.
    pub build_swap_mib: Option<i64>,
    /// Whether credentials embedded in a pasted git URL (no saved provider) are on
    /// file for this application. The secret itself never leaves the backend.
    pub has_git_credentials: bool,
    /// The source tree is an uploaded ZIP rather than a repository clone. It is
    /// set by uploading an archive; saving a repository URL clears it.
    pub source_upload: bool,
    /// Size of the stored ZIP when `source_upload` is set.
    pub source_archive_bytes: Option<u64>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateSourceRequest {
    pub deploy_type: String,
    pub build_method: String,
    pub git_provider_id: Option<String>,
    pub git_url: Option<String>,
    pub git_branch: Option<String>,
    pub auto_deploy: Option<bool>,
    pub watch_paths: Option<String>,
    pub dockerfile_path: Option<String>,
    pub compose_file_path: Option<String>,
    pub compose_public_service: Option<String>,
    pub container_port: Option<i64>,
    pub railpack_project_path: Option<String>,
    pub railpack_start_command: Option<String>,
    /// Registry-image deploys: image reference to pull+run. Clearing it (to switch
    /// this app away from the image source) removes any saved registry credentials.
    #[serde(default)]
    pub image_url: Option<String>,
    /// Private registry username. `Some("")` clears the saved username alongside
    /// `registry_password` when the image source is abandoned.
    #[serde(default)]
    pub registry_username: Option<String>,
    /// Private registry password or token. Empty string keeps the saved one when
    /// `image_url` is unchanged; a new value always replaces it.
    #[serde(default)]
    pub registry_password: Option<String>,
    /// Container restart policy (`no`, `always`, `unless-stopped`, `on-failure`).
    /// `None` keeps the current value (default `unless-stopped`).
    #[serde(default)]
    pub restart_policy: Option<String>,
    pub build_cpu_cores: Option<f64>,
    pub build_memory_mib: Option<i64>,
    pub build_swap_mib: Option<i64>,
}

#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct DeployRequest {
    pub clean_build: Option<bool>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SetCleanBuildRequest {
    pub clean_build: bool,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct VolumeDto {
    pub mount_type: String,
    pub source: String,
    pub destination: String,
    pub read_only: bool,
    #[serde(default)]
    pub services: Vec<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ManagedVolumeDto {
    pub id: String,
    pub name: String,
    pub mount_path: String,
    pub compose_service: Option<String>,
    pub attached: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateManagedVolumeRequest {
    pub name: String,
    pub mount_path: String,
    pub compose_service: Option<String>,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize, ToSchema)]
pub struct ComposeProxyRouteDto {
    pub domain_id: String,
    pub application_id: String,
    pub service: String,
    pub container_port: u16,
}

/// One `{{url}}`-style marker in an application's runtime env and what it
/// resolves to right now.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DomainMarkerDto {
    pub name: String,
    pub marker: String,
    pub service: Option<String>,
    pub port: Option<u16>,
    pub domain_id: Option<String>,
    pub resolved: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DomainMarkersDto {
    /// The catalog template does not fully work over plain http.
    pub requires_https: bool,
    /// The catalog template's app stores its URL internally on first boot.
    pub url_persisted: bool,
    pub markers: Vec<DomainMarkerDto>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct WorkspaceEntryDto {
    pub path: String,
    pub name: String,
    pub kind: String,
    pub mime_type: Option<String>,
    pub byte_size: Option<u64>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct WorkspaceFileDto {
    pub path: String,
    pub mime_type: Option<String>,
    pub is_text: bool,
    pub content: Option<String>,
    pub byte_size: u64,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateWorkspaceTextRequest {
    pub content: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateWorkspaceEntryRequest {
    pub path: String,
    pub kind: String,
    pub content: Option<String>,
}

/// Body for `POST .../compose/deploy`. When the application is already running
/// the caller must echo back the application name in `confirmation`; the API
/// rejects the request otherwise. On a first deploy (no services running) the
/// field is ignored so existing call paths keep working.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RedeployRequest {
    pub confirmation: Option<String>,
}

/// One row of the Advanced tab "Basic Authentication" card. Passwords are
/// bcrypt-hashed in the DB and never returned; `has_password` is the only
/// signal the UI gets to show "Set password" for a freshly imported row.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BasicAuthUserDto {
    pub id: String,
    pub username: String,
    pub has_password: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Full Basic Auth configuration for one application.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BasicAuthConfigDto {
    pub enabled: bool,
    pub realm: String,
    pub users: Vec<BasicAuthUserDto>,
    /// Domain ids this application's basicauth gate applies to. Domains not in
    /// the list are served unprotected.
    pub protected_domain_ids: Vec<String>,
}

/// PATCH body for the Advanced tab. All fields optional; absent keys are left
/// unchanged. `protected_domain_ids` is a full replace.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateBasicAuthRequest {
    pub enabled: Option<bool>,
    pub realm: Option<String>,
    pub protected_domain_ids: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AddBasicAuthUserRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateBasicAuthUserRequest {
    pub username: Option<String>,
    pub password: Option<String>,
}
