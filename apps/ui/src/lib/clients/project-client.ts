import type { CertificateInput, CertificateProvider } from './server-settings-client';
import { API_URL, api, apiForm } from './base-client';
import { workspaceClient } from './volume-workspace-client';

export type ProjectDto = {
	id: string;
	organization_id: string;
	name: string;
	description: string;
	icon_url: string | null;
	created_at: number;
	updated_at: number;
};

export type ApplicationDto = {
	id: string;
	environment_id: string;
	name: string;
	description: string;
	template_kind: string | null;
	catalog_template_id: string | null;
	catalog_icon_url: string | null;
	icon_url: string | null;
	/** `null` for a group-targeted application, whose target is `server_group`. */
	server: ServerSummaryDto | null;
	/** `null` for a single-server application. */
	server_group: ServerGroupSummaryDto | null;
	created_at: number;
	updated_at: number;
};

export type ArtifactDto = {
	id: string;
	filename: string;
	kind: string;
	byte_size: number;
	sha256: string;
	created_at: number;
};
export type DomainInput = CertificateInput & {
	host: string;
	port: number;
	https: boolean;
	is_primary?: boolean;
	/** Server pin for a group-targeted application; ignored for single-server apps. */
	server_id?: string | null;
};
export type DomainDto = {
	id: string;
	application_id: string;
	host: string;
	port: number;
	https: boolean;
	certificate_provider?: CertificateProvider;
	has_custom_certificate?: boolean;
	certificate_id?: string | null;
	certificate_name?: string | null;
	is_primary: boolean;
	url: string;
	is_local: boolean;
	required_dns_target: string | null;
	ingress_mode: 'central' | 'public_edge' | 'private_relay' | 'private_only';
	route_health: 'ready' | 'pending' | 'unavailable' | 'private_only';
	route_health_reason: string | null;
	/** Which server this domain routes to; `null` for a single-server app. */
	server_id: string | null;
	created_at: number;
	updated_at: number;
};
export type PortConstraintsDto = {
	reserved_ports: number[];
	taken_ports: number[];
};
export type DomainDnsValidationResult = {
	/** `unresolved`: no A/AAAA record yet. `error`: the lookup itself failed. */
	status: 'valid' | 'cdn' | 'invalid' | 'unresolved' | 'error';
	resolved_addresses: string[];
	expected_addresses: string[];
	cname: string | null;
	cdn_provider: string | null;
	message: string | null;
};

export type DeploymentDto = {
	status: 'not_deployed' | 'ready' | 'deploying' | 'running' | 'failed' | 'stopped';
	container_port: number;
	domains: DomainDto[];
	public_url: string | null;
	config_stale: boolean;
	last_error: string | null;
	deployed_at: number | null;
	artifact: ArtifactDto | null;
	/** Per-server replica state for a group-targeted application; empty otherwise. */
	replicas: ApplicationReplicaDto[];
};

export type ApplicationReplicaDto = {
	server_id: string;
	server_name: string;
	os: string | null;
	distribution: string | null;
	/** Public address when known, otherwise the SSH host. */
	address: string | null;
	status: 'not_deployed' | 'deploying' | 'running' | 'failed' | 'stopped';
	last_error: string | null;
	deployed_at: number | null;
};
export type EnvironmentVariableDto = {
	scope: 'runtime' | 'build';
	name: string;
	value: string;
	masked: boolean;
};
export type VolumeDto = {
	mount_type: string;
	source: string;
	destination: string;
	read_only: boolean;
	services: string[];
};
export type ManagedVolumeDto = {
	id: string;
	name: string;
	mount_path: string;
	compose_service: string | null;
	attached: boolean;
	created_at: number;
	updated_at: number;
};
export type CreateManagedVolumeInput = {
	name: string;
	mount_path: string;
	compose_service: string | null;
};
export type BuildMethod = 'docker' | 'compose' | 'railpack';
export type PortMapping = {
	service: string;
	host_port: number;
	container_port: number;
	protocol: 'tcp' | 'udp';
};

export async function randomSslipHost(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
): Promise<string> {
	const response = await api<{ host: string }>(
		deploymentPath(
			organizationId,
			projectId,
			environmentId,
			applicationId,
			'domains/random-sslip-host',
		),
	);
	return response.data!.host;
}

export type ComposeContent = {
	content: string;
	port_mappings: PortMapping[];
	catalog?: {
		services: { name: string; suggested_internal_ports: number[] }[];
	} | null;
};

export type ComposeProxyRoute = {
	domain_id: string;
	application_id: string;
	service: string;
	container_port: number;
};

export async function listComposeProxyRoutes(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
): Promise<ComposeProxyRoute[]> {
	const response = await api<ComposeProxyRoute[]>(
		deploymentPath(organizationId, projectId, environmentId, applicationId, 'compose/proxy-routes'),
	);
	return response.data ?? [];
}
export type DomainMarkerDto = {
	name: string;
	marker: string;
	service: string | null;
	port: number | null;
	domain_id: string | null;
	resolved: string | null;
	error: string | null;
};
export type DomainMarkersDto = {
	requires_https: boolean;
	url_persisted: boolean;
	markers: DomainMarkerDto[];
};

/** `{{url}}`-style markers in the runtime env and what each resolves to now. */
export async function getDomainMarkers(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
): Promise<DomainMarkersDto> {
	const response = await api<DomainMarkersDto>(
		deploymentPath(
			organizationId,
			projectId,
			environmentId,
			applicationId,
			'environment/domain-markers',
		),
	);
	return response.data ?? { requires_https: false, url_persisted: false, markers: [] };
}

export async function saveComposeProxyRoute(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	input: { domain_id: string; service: string; container_port: number },
): Promise<ComposeProxyRoute> {
	const response = await api<ComposeProxyRoute>(
		deploymentPath(organizationId, projectId, environmentId, applicationId, 'compose/proxy-routes'),
		{ method: 'POST', body: JSON.stringify(input) },
	);
	return response.data!;
}

export type ValidationError = {
	line?: number;
	column?: number;
	message: string;
};

export type ValidationResponse = {
	valid: boolean;
	errors: ValidationError[];
};

export type ComposeServiceStatus = {
	service: string;
	status: string;
	ports: string[];
	container_name: string;
};

export type SourceConfigDto = {
	deploy_type: 'static' | 'git' | 'compose' | 'image';
	build_method: BuildMethod;
	git_provider_id: string | null;
	git_url: string | null;
	git_branch: string | null;
	dockerfile_path: string | null;
	compose_file_path: string | null;
	compose_public_service: string | null;
	container_port: number | null;
	railpack_project_path: string;
	railpack_start_command: string | null;
	/** Registry image reference for `deploy_type === "image"`. */
	image_url: string | null;
	/** Whether registry credentials are on file for a private image. */
	has_registry_credentials: boolean;
	/** Container restart policy applied when this source's container is created.
	 * Sent as `null` when the save should keep the stored value. */
	restart_policy: 'no' | 'always' | 'unless-stopped' | 'on-failure' | null;
	/** New registry credentials to store when saving an image source (input only). */
	registry_username?: string | null;
	registry_password?: string | null;
	auto_deploy: boolean;
	/** Comma-separated path prefixes. When set, a push only triggers auto-deploy if it
	 * touched a file under one of these paths (monorepo scoping). `null`/empty means any
	 * push to `git_branch` redeploys. */
	watch_paths: string | null;
	clean_build: boolean;
	build_cpu_cores: number | null;
	build_memory_mib: number | null;
	build_swap_mib: number | null;
	/** Credentials embedded in the pasted git URL (no saved provider) are on file
	 * for this application. The secret itself never round-trips to the client. */
	has_git_credentials: boolean;
};

export type DeployHookDto = {
	token: string;
	url: string;
};
export type DeploymentRunDto = {
	id: string;
	source_type: string;
	build_method: string;
	status: string;
	started_at: number;
	finished_at: number | null;
	duration_ms: number | null;
	last_error: string | null;
	commit_sha: string | null;
	commit_message: string | null;
	commit_author_name: string | null;
	commit_author_email: string | null;
	triggered_by: string | null;
	branch: string | null;
	job_id: string | null;
	actor: DeploymentRunActorDto | null;
	/** Per-server progress inside the run; empty for a single-server application. */
	replicas: DeploymentRunReplicaDto[];
};
export type DeploymentRunReplicaDto = {
	server_id: string;
	server_name: string;
	os: string | null;
	distribution: string | null;
	address: string | null;
	status: 'queued' | 'deploying' | 'succeeded' | 'failed' | 'canceled';
	started_at: number | null;
	finished_at: number | null;
	last_error: string | null;
};
export type DeploymentRunActorDto = {
	id: string;
	name: string;
	email: string;
};
export type DeploymentRunLogDto = {
	sequence: number;
	level: string;
	message: string;
	created_at: number;
	/** The replica that wrote the line; `null` for run-level lines. */
	server_id?: string | null;
};
// The API flattens the run onto the detail object (`#[serde(flatten)]`), so the
// run's fields sit at the top level rather than under a `run` key.
export type DeploymentRunStepDto = {
	/** `clone`, `build`, `pull`, `prepare`, `start`, `health_check`, `route`. */
	name: string;
	status: 'pending' | 'running' | 'succeeded' | 'failed' | 'skipped';
	started_at: number | null;
	finished_at: number | null;
	/** The replica this step belongs to; `null` for a single-server run. */
	server_id: string | null;
};
export type DeploymentRunDetailDto = DeploymentRunDto & {
	/** Empty for runs recorded before steps existed and for lifecycle runs. */
	steps: DeploymentRunStepDto[];
	logs: DeploymentRunLogDto[];
	total_log_count: number;
	truncated: boolean;
};
export type DeploymentJobDto = {
	id: string;
	application_id: string;
	application_name: string;
	project_id: string;
	project_name: string;
	environment_id: string;
	organization_id: string;
	source_type: string;
	build_method: string;
	status: 'queued' | 'building' | 'deploying' | 'succeeded' | 'failed' | 'canceled';
	queued_at: number;
	started_at: number | null;
	finished_at: number | null;
	cancel_requested: boolean;
	canceled_at: number | null;
	builder_container_name: string | null;
	builder_container_id: string | null;
	run_id: string;
	sanitized_error: string | null;
	duration_ms: number | null;
	commit_sha: string | null;
	commit_message: string | null;
	branch: string | null;
	commit_author_name: string | null;
	commit_author_email: string | null;
	triggered_by: string | null;
	actor: DeploymentRunActorDto | null;
};
export type DeploymentQueueEvent = {
	// A `resync` event reports that this subscriber fell behind and carries no
	// job id, so consumers must handle it before filtering by job.
	job_id: string;
	// `steps` carries the run's whole step list as JSON in `payload`.
	kind: 'log' | 'status' | 'terminal' | 'resync' | 'steps';
	sequence: number | null;
	level: 'info' | 'error' | null;
	payload: string;
	/** Replica that produced a `log` event, for a group deploy. */
	server_id?: string;
};

export type EnvironmentDto = {
	id: string;
	project_id: string;
	name: string;
	description: string;
	is_production: boolean;
	created_at: number;
	updated_at: number;
	applications: ApplicationDto[];
};

export type ProjectDetailDto = {
	project: ProjectDto;
	environments: EnvironmentDto[];
};

export type CreateProjectRequest = {
	name: string;
	description?: string;
};

export type UpdateProjectRequest = {
	name?: string;
	description?: string;
	/** Omit to leave unchanged, `null` to clear back to the default icon. */
	icon_url?: string | null;
};

export type CreateEnvironmentRequest = {
	name: string;
	description?: string;
};

export type UpdateEnvironmentRequest = {
	name: string;
	description?: string;
};

export type CreateApplicationRequest = {
	name: string;
	description?: string;
	server_id?: string;
	server_group_id?: string;
};

// Re-exported so a new server state or ingress mode only has to be added in one place.
export type { IngressMode as ServerIngressMode, ServerState } from './server-client';
import type { IngressMode as ServerIngressMode, ServerState } from './server-client';

export type ServerSummaryDto = {
	id: string;
	kind: string;
	name: string;
	state: ServerState;
	ingress_mode: ServerIngressMode;
	capacity: {
		cpu_cores: number | null;
		memory_bytes: number | null;
		disk_free_bytes: number | null;
	};
	/** Only the local server reports a platform; remote hosts report a distribution instead. */
	os: string | null;
	distribution: string | null;
	os_version: string | null;
};

export type ServerGroupSummaryDto = {
	id: string;
	name: string;
	description: string;
	member_count: number;
};

export type UpdateApplicationRequest = {
	name?: string;
	description?: string;
	/** Omit to leave unchanged, `null` to clear back to the catalog default. */
	icon_url?: string | null;
	/** Omit to leave unchanged, `null` to clear the target (exactly one target must remain). */
	server_id?: string | null;
	server_group_id?: string | null;
};

export type PostgresTemplateInput = {
	application_name: string;
	description?: string;
	server_id: string;
	docker_image?: string;
	database_name?: string;
	database_user?: string;
	password?: string;
	volume_name?: string;
	public_host_port?: number | null;
	runtime_cpu_cores?: number | null;
	runtime_memory_mib?: number | null;
	template_instance_id?: string;
};

export type TemplateVariableDto = { name: string; masked: boolean };
export type TemplateVolumeDto = {
	name: string;
	mount_path: string;
	service: string;
};

export type PostgresTemplatePreview = {
	template_instance_id: string;
	resolved_password: string;
	database_name: string;
	database_user: string;
	docker_image: string;
	private_alias: string;
	private_connection_string: string;
	public_connection_string: string | null;
	compose: string;
	port_mappings: PortMapping[];
	variables: TemplateVariableDto[];
	volumes: TemplateVolumeDto[];
};

export type PostgresTemplateCreated = {
	application: ApplicationDto;
	preview: PostgresTemplatePreview;
};

export type PostgresResourceDetail = {
	application: ApplicationDto;
	docker_image: string;
	database_name: string;
	database_user: string;
	volume: TemplateVolumeDto;
	private_alias: string;
	internal_port: number;
	public_host_port: number | null;
	deployment_status: DeploymentDto['status'];
	config_stale: boolean;
	password: string | null;
	private_connection_string: string | null;
	public_connection_string: string | null;
	compose: string;
	variables: TemplateVariableDto[];
	runtime_cpu_cores: number | null;
	runtime_memory_mib: number | null;
};

export type PostgresTemplateConfigUpdate = {
	docker_image: string;
	public_host_port: number | null;
	runtime_cpu_cores: number | null;
	runtime_memory_mib: number | null;
	database_name?: string;
	database_user?: string;
	password?: string;
};

export type MariaDbTemplateInput = {
	application_name: string;
	description?: string;
	server_id: string;
	docker_image?: string;
	database_name?: string;
	database_user?: string;
	password?: string;
	root_password?: string;
	volume_name?: string;
	public_host_port?: number | null;
	runtime_cpu_cores?: number | null;
	runtime_memory_mib?: number | null;
	template_instance_id?: string;
};

export type MariaDbTemplatePreview = {
	template_instance_id: string;
	resolved_password: string;
	resolved_root_password: string;
	database_name: string;
	database_user: string;
	docker_image: string;
	private_alias: string;
	private_connection_string: string;
	private_root_connection_string: string;
	public_connection_string: string | null;
	public_root_connection_string: string | null;
	compose: string;
	port_mappings: PortMapping[];
	variables: TemplateVariableDto[];
	volumes: TemplateVolumeDto[];
};

export type MariaDbTemplateCreated = {
	application: ApplicationDto;
	preview: MariaDbTemplatePreview;
};

export type MariaDbResourceDetail = {
	application: ApplicationDto;
	docker_image: string;
	database_name: string;
	database_user: string;
	password: string | null;
	root_password: string | null;
	volume: TemplateVolumeDto;
	private_alias: string;
	internal_port: number;
	public_host_port: number | null;
	deployment_status: DeploymentDto['status'];
	config_stale: boolean;
	private_connection_string: string | null;
	private_root_connection_string: string | null;
	public_connection_string: string | null;
	public_root_connection_string: string | null;
	compose: string;
	variables: TemplateVariableDto[];
	runtime_cpu_cores: number | null;
	runtime_memory_mib: number | null;
};

export type MariaDbTemplateConfigUpdate = {
	docker_image: string;
	public_host_port: number | null;
	runtime_cpu_cores: number | null;
	runtime_memory_mib: number | null;
	database_name?: string;
	database_user?: string;
	password?: string;
	root_password?: string;
};

export type MariaDbPasswordRotationRequest = {
	credential: 'database' | 'root';
	password?: string;
};

export type DragonflyDbTemplateInput = {
	application_name: string;
	description?: string;
	server_id: string;
	docker_image?: string;
	password?: string;
	snapshot_cron?: string;
	max_memory_mib?: number | null;
	cache_mode?: boolean;
	volume_name?: string;
	public_host_port?: number | null;
	runtime_cpu_cores?: number | null;
	runtime_memory_mib?: number | null;
	template_instance_id?: string;
};

export type DragonflyDbTemplatePreview = {
	template_instance_id: string;
	resolved_password: string;
	docker_image: string;
	snapshot_cron: string;
	max_memory_mib: number | null;
	cache_mode: boolean;
	private_alias: string;
	private_connection_string: string;
	public_connection_string: string | null;
	compose: string;
	port_mappings: PortMapping[];
	variables: TemplateVariableDto[];
	volumes: TemplateVolumeDto[];
};

export type DragonflyDbTemplateCreated = {
	application: ApplicationDto;
	preview: DragonflyDbTemplatePreview;
};

export type DragonflyDbResourceDetail = {
	application: ApplicationDto;
	docker_image: string;
	snapshot_cron: string;
	max_memory_mib: number | null;
	cache_mode: boolean;
	password: string | null;
	volume: TemplateVolumeDto;
	private_alias: string;
	internal_port: number;
	public_host_port: number | null;
	deployment_status: DeploymentDto['status'];
	config_stale: boolean;
	private_connection_string: string | null;
	public_connection_string: string | null;
	compose: string;
	variables: TemplateVariableDto[];
	runtime_cpu_cores: number | null;
	runtime_memory_mib: number | null;
};

export type DragonflyDbTemplateConfigUpdate = {
	docker_image: string;
	public_host_port: number | null;
	runtime_cpu_cores: number | null;
	runtime_memory_mib: number | null;
	snapshot_cron?: string;
	max_memory_mib?: number | null;
	cache_mode?: boolean;
	password?: string;
};

export type DragonflyDbPasswordRotationRequest = { password?: string };

export type KeyDbTemplateInput = {
	application_name: string;
	description?: string;
	server_id: string;
	docker_image?: string;
	password?: string;
	append_only?: boolean;
	append_fsync?: string;
	max_memory_mib?: number | null;
	eviction_policy?: string;
	volume_name?: string;
	public_host_port?: number | null;
	runtime_cpu_cores?: number | null;
	runtime_memory_mib?: number | null;
	template_instance_id?: string;
};

export type KeyDbTemplatePreview = {
	template_instance_id: string;
	resolved_password: string;
	docker_image: string;
	append_only: boolean;
	append_fsync: string;
	max_memory_mib: number | null;
	eviction_policy: string;
	private_alias: string;
	private_connection_string: string;
	public_connection_string: string | null;
	compose: string;
	port_mappings: PortMapping[];
	variables: TemplateVariableDto[];
	volumes: TemplateVolumeDto[];
};

export type KeyDbTemplateCreated = {
	application: ApplicationDto;
	preview: KeyDbTemplatePreview;
};

export type KeyDbResourceDetail = {
	application: ApplicationDto;
	docker_image: string;
	append_only: boolean;
	append_fsync: string;
	max_memory_mib: number | null;
	eviction_policy: string;
	password: string | null;
	volume: TemplateVolumeDto;
	private_alias: string;
	internal_port: number;
	public_host_port: number | null;
	deployment_status: DeploymentDto['status'];
	config_stale: boolean;
	private_connection_string: string | null;
	public_connection_string: string | null;
	compose: string;
	variables: TemplateVariableDto[];
	runtime_cpu_cores: number | null;
	runtime_memory_mib: number | null;
};

export type KeyDbTemplateConfigUpdate = {
	docker_image: string;
	public_host_port: number | null;
	runtime_cpu_cores: number | null;
	runtime_memory_mib: number | null;
	append_only?: boolean;
	append_fsync?: string;
	max_memory_mib?: number | null;
	eviction_policy?: string;
	password?: string;
};

export type KeyDbPasswordRotationRequest = { password?: string };

export type SqlServerTemplateInput = {
	application_name: string;
	description?: string;
	server_id: string;
	docker_image?: string;
	password?: string;
	volume_name?: string;
	public_host_port?: number | null;
	runtime_cpu_cores?: number | null;
	runtime_memory_mib?: number | null;
	template_instance_id?: string;
};

export type SqlServerTemplatePreview = {
	template_instance_id: string;
	resolved_password: string;
	docker_image: string;
	private_alias: string;
	private_connection_string: string;
	private_ado_connection_string: string;
	public_connection_string: string | null;
	public_ado_connection_string: string | null;
	compose: string;
	port_mappings: PortMapping[];
	variables: TemplateVariableDto[];
	volumes: TemplateVolumeDto[];
};

export type SqlServerTemplateCreated = {
	application: ApplicationDto;
	preview: SqlServerTemplatePreview;
};

export type SqlServerResourceDetail = {
	application: ApplicationDto;
	docker_image: string;
	password: string | null;
	volume: TemplateVolumeDto;
	private_alias: string;
	internal_port: number;
	public_host_port: number | null;
	deployment_status: DeploymentDto['status'];
	config_stale: boolean;
	private_connection_string: string | null;
	private_ado_connection_string: string | null;
	public_connection_string: string | null;
	public_ado_connection_string: string | null;
	compose: string;
	variables: TemplateVariableDto[];
	runtime_cpu_cores: number | null;
	runtime_memory_mib: number | null;
};

export type SqlServerTemplateConfigUpdate = {
	docker_image: string;
	public_host_port: number | null;
	runtime_cpu_cores: number | null;
	runtime_memory_mib: number | null;
	password?: string;
};

export type SqlServerPasswordRotationRequest = { password?: string };

export type MongoDbTemplateInput = {
	application_name: string;
	description?: string;
	server_id: string;
	docker_image?: string;
	database_name?: string;
	database_user?: string;
	password?: string;
	volume_name?: string;
	public_host_port?: number | null;
	runtime_cpu_cores?: number | null;
	runtime_memory_mib?: number | null;
	template_instance_id?: string;
};
export type MongoDbTemplatePreview = {
	template_instance_id: string;
	resolved_password: string;
	database_name: string;
	database_user: string;
	docker_image: string;
	private_alias: string;
	private_connection_string: string;
	public_connection_string: string | null;
	compose: string;
	port_mappings: PortMapping[];
	variables: TemplateVariableDto[];
	volumes: TemplateVolumeDto[];
};
export type MongoDbTemplateCreated = {
	application: ApplicationDto;
	preview: MongoDbTemplatePreview;
};
export type MongoDbResourceDetail = {
	application: ApplicationDto;
	docker_image: string;
	database_name: string;
	database_user: string;
	volume: TemplateVolumeDto;
	private_alias: string;
	internal_port: number;
	public_host_port: number | null;
	deployment_status: string;
	config_stale: boolean;
	password: string | null;
	private_connection_string: string | null;
	public_connection_string: string | null;
	compose: string;
	variables: TemplateVariableDto[];
	runtime_cpu_cores: number | null;
	runtime_memory_mib: number | null;
};
export type MongoDbTemplateConfigUpdate = {
	docker_image: string;
	public_host_port: number | null;
	runtime_cpu_cores: number | null;
	runtime_memory_mib: number | null;
	database_name?: string;
	database_user?: string;
	password?: string;
};
export type MongoDbPasswordRotationRequest = { password?: string };

function projectPath(organizationId: string, projectId?: string): string {
	const base = `/api/v1/organizations/${encodeURIComponent(organizationId)}/projects`;
	return projectId ? `${base}/${encodeURIComponent(projectId)}` : base;
}

function environmentPath(
	organizationId: string,
	projectId: string,
	environmentId?: string,
): string {
	const base = `${projectPath(organizationId, projectId)}/environments`;
	return environmentId ? `${base}/${encodeURIComponent(environmentId)}` : base;
}

function applicationPath(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId?: string,
): string {
	const base = `${environmentPath(organizationId, projectId, environmentId)}/applications`;
	return applicationId ? `${base}/${encodeURIComponent(applicationId)}` : base;
}

function deploymentPath(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	suffix: string,
) {
	return `${applicationPath(organizationId, projectId, environmentId, applicationId)}/${suffix}`;
}

function templatePath(
	organizationId: string,
	projectId: string,
	environmentId: string,
	suffix: string,
): string {
	return `${environmentPath(organizationId, projectId, environmentId)}/templates/${suffix}`;
}

export async function listProjects(organizationId: string): Promise<ProjectDto[]> {
	const response = await api<ProjectDto[]>(projectPath(organizationId));
	return response.data ?? [];
}

export async function createProject(
	organizationId: string,
	input: CreateProjectRequest,
): Promise<ProjectDto> {
	const response = await api<ProjectDto>(projectPath(organizationId), {
		method: 'POST',
		body: JSON.stringify(input),
	});
	return response.data!;
}

export async function getProject(
	organizationId: string,
	projectId: string,
): Promise<ProjectDetailDto> {
	const response = await api<ProjectDetailDto>(projectPath(organizationId, projectId));
	return response.data!;
}

export async function updateProject(
	organizationId: string,
	projectId: string,
	input: UpdateProjectRequest,
): Promise<ProjectDto> {
	const response = await api<ProjectDto>(projectPath(organizationId, projectId), {
		method: 'PATCH',
		body: JSON.stringify(input),
	});
	return response.data!;
}

export async function uploadProjectIcon(
	organizationId: string,
	projectId: string,
	icon: File,
): Promise<ProjectDto> {
	const form = new FormData();
	form.append('icon', icon);
	const response = await apiForm<ProjectDto>(
		`${projectPath(organizationId, projectId)}/icon`,
		form,
	);
	return response.data!;
}

export async function deleteProject(organizationId: string, projectId: string): Promise<void> {
	await api<void>(projectPath(organizationId, projectId), { method: 'DELETE' });
}

export async function createEnvironment(
	organizationId: string,
	projectId: string,
	input: CreateEnvironmentRequest,
): Promise<EnvironmentDto> {
	const response = await api<EnvironmentDto>(environmentPath(organizationId, projectId), {
		method: 'POST',
		body: JSON.stringify(input),
	});
	return response.data!;
}

export async function updateEnvironment(
	organizationId: string,
	projectId: string,
	environmentId: string,
	input: UpdateEnvironmentRequest,
): Promise<EnvironmentDto> {
	const response = await api<EnvironmentDto>(
		environmentPath(organizationId, projectId, environmentId),
		{
			method: 'PATCH',
			body: JSON.stringify(input),
		},
	);
	return response.data!;
}

export async function deleteEnvironment(
	organizationId: string,
	projectId: string,
	environmentId: string,
): Promise<void> {
	await api<void>(environmentPath(organizationId, projectId, environmentId), {
		method: 'DELETE',
	});
}

export async function createApplication(
	organizationId: string,
	projectId: string,
	environmentId: string,
	input: CreateApplicationRequest,
): Promise<ApplicationDto> {
	const response = await api<ApplicationDto>(
		applicationPath(organizationId, projectId, environmentId),
		{
			method: 'POST',
			body: JSON.stringify(input),
		},
	);
	return response.data!;
}

export async function listReadyServers(organizationId: string): Promise<ServerSummaryDto[]> {
	const response = await api<ServerSummaryDto[]>(
		`/api/v1/organizations/${encodeURIComponent(organizationId)}/servers/available`,
	);
	return response.data ?? [];
}

export async function previewPostgresTemplate(
	organizationId: string,
	projectId: string,
	environmentId: string,
	input: PostgresTemplateInput,
): Promise<PostgresTemplatePreview> {
	const response = await api<PostgresTemplatePreview>(
		templatePath(organizationId, projectId, environmentId, 'postgres/preview'),
		{ method: 'POST', body: JSON.stringify(input) },
	);
	return response.data!;
}

export async function createPostgresTemplate(
	organizationId: string,
	projectId: string,
	environmentId: string,
	input: PostgresTemplateInput,
): Promise<PostgresTemplateCreated> {
	const response = await api<PostgresTemplateCreated>(
		templatePath(organizationId, projectId, environmentId, 'postgres'),
		{ method: 'POST', body: JSON.stringify(input) },
	);
	return response.data!;
}

function postgresResourcePath(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	suffix = '',
): string {
	const base = `${applicationPath(organizationId, projectId, environmentId, applicationId)}/templates/postgres/resource`;
	return suffix ? `${base}/${suffix}` : base;
}

export async function getPostgresResource(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
): Promise<PostgresResourceDetail> {
	const response = await api<PostgresResourceDetail>(
		postgresResourcePath(organizationId, projectId, environmentId, applicationId),
	);
	return response.data!;
}

export async function updatePostgresResource(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	input: PostgresTemplateConfigUpdate,
): Promise<PostgresResourceDetail> {
	const response = await api<PostgresResourceDetail>(
		postgresResourcePath(organizationId, projectId, environmentId, applicationId),
		{ method: 'PATCH', body: JSON.stringify(input) },
	);
	return response.data!;
}

export async function rotatePostgresPassword(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	input: { password?: string },
): Promise<PostgresResourceDetail> {
	const response = await api<PostgresResourceDetail>(
		postgresResourcePath(
			organizationId,
			projectId,
			environmentId,
			applicationId,
			'rotate-password',
		),
		{ method: 'POST', body: JSON.stringify(input) },
	);
	return response.data!;
}

function templateResourcePath(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	kind: 'mariadb' | 'dragonflydb' | 'keydb' | 'sqlserver' | 'mongodb',
	suffix = '',
): string {
	const base = `${applicationPath(organizationId, projectId, environmentId, applicationId)}/templates/${kind}/resource`;
	return suffix ? `${base}/${suffix}` : base;
}

export async function previewMariaDbTemplate(
	organizationId: string,
	projectId: string,
	environmentId: string,
	input: MariaDbTemplateInput,
): Promise<MariaDbTemplatePreview> {
	const response = await api<MariaDbTemplatePreview>(
		templatePath(organizationId, projectId, environmentId, 'mariadb/preview'),
		{ method: 'POST', body: JSON.stringify(input) },
	);
	return response.data!;
}

export async function createMariaDbTemplate(
	organizationId: string,
	projectId: string,
	environmentId: string,
	input: MariaDbTemplateInput,
): Promise<MariaDbTemplateCreated> {
	const response = await api<MariaDbTemplateCreated>(
		templatePath(organizationId, projectId, environmentId, 'mariadb'),
		{ method: 'POST', body: JSON.stringify(input) },
	);
	return response.data!;
}

export async function getMariaDbResource(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
): Promise<MariaDbResourceDetail> {
	const response = await api<MariaDbResourceDetail>(
		templateResourcePath(organizationId, projectId, environmentId, applicationId, 'mariadb'),
	);
	return response.data!;
}

export async function updateMariaDbResource(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	input: MariaDbTemplateConfigUpdate,
): Promise<MariaDbResourceDetail> {
	const response = await api<MariaDbResourceDetail>(
		templateResourcePath(organizationId, projectId, environmentId, applicationId, 'mariadb'),
		{ method: 'PATCH', body: JSON.stringify(input) },
	);
	return response.data!;
}

export async function rotateMariaDbPassword(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	input: MariaDbPasswordRotationRequest,
): Promise<MariaDbResourceDetail> {
	const response = await api<MariaDbResourceDetail>(
		templateResourcePath(
			organizationId,
			projectId,
			environmentId,
			applicationId,
			'mariadb',
			'rotate-password',
		),
		{ method: 'POST', body: JSON.stringify(input) },
	);
	return response.data!;
}

export async function previewDragonflyDbTemplate(
	organizationId: string,
	projectId: string,
	environmentId: string,
	input: DragonflyDbTemplateInput,
): Promise<DragonflyDbTemplatePreview> {
	const response = await api<DragonflyDbTemplatePreview>(
		templatePath(organizationId, projectId, environmentId, 'dragonflydb/preview'),
		{ method: 'POST', body: JSON.stringify(input) },
	);
	return response.data!;
}

export async function createDragonflyDbTemplate(
	organizationId: string,
	projectId: string,
	environmentId: string,
	input: DragonflyDbTemplateInput,
): Promise<DragonflyDbTemplateCreated> {
	const response = await api<DragonflyDbTemplateCreated>(
		templatePath(organizationId, projectId, environmentId, 'dragonflydb'),
		{ method: 'POST', body: JSON.stringify(input) },
	);
	return response.data!;
}

export async function getDragonflyDbResource(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
): Promise<DragonflyDbResourceDetail> {
	const response = await api<DragonflyDbResourceDetail>(
		templateResourcePath(organizationId, projectId, environmentId, applicationId, 'dragonflydb'),
	);
	return response.data!;
}

export async function updateDragonflyDbResource(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	input: DragonflyDbTemplateConfigUpdate,
): Promise<DragonflyDbResourceDetail> {
	const response = await api<DragonflyDbResourceDetail>(
		templateResourcePath(organizationId, projectId, environmentId, applicationId, 'dragonflydb'),
		{ method: 'PATCH', body: JSON.stringify(input) },
	);
	return response.data!;
}

export async function rotateDragonflyDbPassword(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	input: DragonflyDbPasswordRotationRequest,
): Promise<DragonflyDbResourceDetail> {
	const response = await api<DragonflyDbResourceDetail>(
		templateResourcePath(
			organizationId,
			projectId,
			environmentId,
			applicationId,
			'dragonflydb',
			'rotate-password',
		),
		{ method: 'POST', body: JSON.stringify(input) },
	);
	return response.data!;
}

export async function previewKeyDbTemplate(
	organizationId: string,
	projectId: string,
	environmentId: string,
	input: KeyDbTemplateInput,
): Promise<KeyDbTemplatePreview> {
	const response = await api<KeyDbTemplatePreview>(
		templatePath(organizationId, projectId, environmentId, 'keydb/preview'),
		{ method: 'POST', body: JSON.stringify(input) },
	);
	return response.data!;
}

export async function createKeyDbTemplate(
	organizationId: string,
	projectId: string,
	environmentId: string,
	input: KeyDbTemplateInput,
): Promise<KeyDbTemplateCreated> {
	const response = await api<KeyDbTemplateCreated>(
		templatePath(organizationId, projectId, environmentId, 'keydb'),
		{ method: 'POST', body: JSON.stringify(input) },
	);
	return response.data!;
}

export async function getKeyDbResource(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
): Promise<KeyDbResourceDetail> {
	const response = await api<KeyDbResourceDetail>(
		templateResourcePath(organizationId, projectId, environmentId, applicationId, 'keydb'),
	);
	return response.data!;
}

export async function updateKeyDbResource(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	input: KeyDbTemplateConfigUpdate,
): Promise<KeyDbResourceDetail> {
	const response = await api<KeyDbResourceDetail>(
		templateResourcePath(organizationId, projectId, environmentId, applicationId, 'keydb'),
		{ method: 'PATCH', body: JSON.stringify(input) },
	);
	return response.data!;
}

export async function rotateKeyDbPassword(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	input: KeyDbPasswordRotationRequest,
): Promise<KeyDbResourceDetail> {
	const response = await api<KeyDbResourceDetail>(
		templateResourcePath(
			organizationId,
			projectId,
			environmentId,
			applicationId,
			'keydb',
			'rotate-password',
		),
		{ method: 'POST', body: JSON.stringify(input) },
	);
	return response.data!;
}

export async function previewSqlServerTemplate(
	organizationId: string,
	projectId: string,
	environmentId: string,
	input: SqlServerTemplateInput,
): Promise<SqlServerTemplatePreview> {
	const response = await api<SqlServerTemplatePreview>(
		templatePath(organizationId, projectId, environmentId, 'sqlserver/preview'),
		{ method: 'POST', body: JSON.stringify(input) },
	);
	return response.data!;
}

export async function createSqlServerTemplate(
	organizationId: string,
	projectId: string,
	environmentId: string,
	input: SqlServerTemplateInput,
): Promise<SqlServerTemplateCreated> {
	const response = await api<SqlServerTemplateCreated>(
		templatePath(organizationId, projectId, environmentId, 'sqlserver'),
		{ method: 'POST', body: JSON.stringify(input) },
	);
	return response.data!;
}

export async function getSqlServerResource(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
): Promise<SqlServerResourceDetail> {
	const response = await api<SqlServerResourceDetail>(
		templateResourcePath(organizationId, projectId, environmentId, applicationId, 'sqlserver'),
	);
	return response.data!;
}

export async function updateSqlServerResource(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	input: SqlServerTemplateConfigUpdate,
): Promise<SqlServerResourceDetail> {
	const response = await api<SqlServerResourceDetail>(
		templateResourcePath(organizationId, projectId, environmentId, applicationId, 'sqlserver'),
		{ method: 'PATCH', body: JSON.stringify(input) },
	);
	return response.data!;
}

export async function rotateSqlServerPassword(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	input: SqlServerPasswordRotationRequest,
): Promise<SqlServerResourceDetail> {
	const response = await api<SqlServerResourceDetail>(
		templateResourcePath(
			organizationId,
			projectId,
			environmentId,
			applicationId,
			'sqlserver',
			'rotate-password',
		),
		{ method: 'POST', body: JSON.stringify(input) },
	);
	return response.data!;
}

export async function previewMongoDbTemplate(
	organizationId: string,
	projectId: string,
	environmentId: string,
	input: MongoDbTemplateInput,
): Promise<MongoDbTemplatePreview> {
	const response = await api<MongoDbTemplatePreview>(
		templatePath(organizationId, projectId, environmentId, 'mongodb/preview'),
		{ method: 'POST', body: JSON.stringify(input) },
	);
	return response.data!;
}
export async function createMongoDbTemplate(
	organizationId: string,
	projectId: string,
	environmentId: string,
	input: MongoDbTemplateInput,
): Promise<MongoDbTemplateCreated> {
	const response = await api<MongoDbTemplateCreated>(
		templatePath(organizationId, projectId, environmentId, 'mongodb'),
		{ method: 'POST', body: JSON.stringify(input) },
	);
	return response.data!;
}
export async function getMongoDbResource(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
): Promise<MongoDbResourceDetail> {
	const response = await api<MongoDbResourceDetail>(
		templateResourcePath(organizationId, projectId, environmentId, applicationId, 'mongodb'),
	);
	return response.data!;
}
export async function updateMongoDbResource(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	input: MongoDbTemplateConfigUpdate,
): Promise<MongoDbResourceDetail> {
	const response = await api<MongoDbResourceDetail>(
		templateResourcePath(organizationId, projectId, environmentId, applicationId, 'mongodb'),
		{ method: 'PATCH', body: JSON.stringify(input) },
	);
	return response.data!;
}
export async function rotateMongoDbPassword(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	input: MongoDbPasswordRotationRequest,
): Promise<MongoDbResourceDetail> {
	const response = await api<MongoDbResourceDetail>(
		templateResourcePath(
			organizationId,
			projectId,
			environmentId,
			applicationId,
			'mongodb',
			'rotate-password',
		),
		{ method: 'POST', body: JSON.stringify(input) },
	);
	return response.data!;
}

export async function getApplication(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
): Promise<ApplicationDto> {
	const response = await api<ApplicationDto>(
		applicationPath(organizationId, projectId, environmentId, applicationId),
	);
	return response.data!;
}

export async function getResourceBaseline(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
): Promise<{ cpu_cores: number; memory_bytes: number } | null> {
	const response = await api<{
		cpu_cores: number;
		memory_bytes: number;
	} | null>(
		deploymentPath(organizationId, projectId, environmentId, applicationId, 'resource-baseline'),
	);
	return response.data ?? null;
}

export async function updateApplication(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	input: UpdateApplicationRequest,
): Promise<ApplicationDto> {
	const response = await api<ApplicationDto>(
		applicationPath(organizationId, projectId, environmentId, applicationId),
		{
			method: 'PATCH',
			body: JSON.stringify(input),
		},
	);
	return response.data!;
}

export async function deleteApplication(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
): Promise<void> {
	await api<void>(applicationPath(organizationId, projectId, environmentId, applicationId), {
		method: 'DELETE',
	});
}

export async function getDeployment(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
): Promise<DeploymentDto> {
	const response = await api<DeploymentDto>(
		deploymentPath(organizationId, projectId, environmentId, applicationId, 'deployment'),
	);
	return response.data!;
}

export type ApplicationStatsDto = {
	status: string;
	cpu_percent: number | null;
	mem_usage_bytes: number | null;
	mem_limit_bytes: number | null;
};

export async function getApplicationStats(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
): Promise<ApplicationStatsDto> {
	const response = await api<ApplicationStatsDto>(
		deploymentPath(organizationId, projectId, environmentId, applicationId, 'stats'),
	);
	return response.data!;
}

export type ContainerStatusDto = {
	status: string;
	label: string;
	container_name: string | null;
};

export type ApplicationLogTargetDto = {
	container_name: string;
	service: string | null;
	status: string;
};

export async function listApplicationLogTargets(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
): Promise<ApplicationLogTargetDto[]> {
	const response = await api<ApplicationLogTargetDto[]>(
		deploymentPath(organizationId, projectId, environmentId, applicationId, 'log-targets'),
	);
	return response.data ?? [];
}

export async function inspectApplicationContainer(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	containerName: string,
	serverId?: string,
): Promise<Record<string, unknown>> {
	const response = await api<Record<string, unknown>>(
		deploymentPath(
			organizationId,
			projectId,
			environmentId,
			applicationId,
			`containers/${encodeURIComponent(containerName)}/inspect${replicaQuery(serverId)}`,
		),
	);
	return response.data!;
}

export type ApplicationNetworkAttachmentDto = {
	name: string;
	ip_address: string | null;
	aliases: string[];
};

export type ApplicationNetworkContainerDto = {
	name: string;
	service: string | null;
	networks: ApplicationNetworkAttachmentDto[];
};

export type ApplicationNetworkDto = {
	name: string;
	driver: string | null;
	scope: 'private' | 'shared';
	application_member_count: number;
	external_member_count: number;
};

export type ApplicationNetworkFindingDto = {
	code: string;
	severity: 'warning' | 'info';
	message: string;
	/** Container the UI can act on (detach). Null for aggregate findings. */
	container: string | null;
	/** Network the remediation applies to. Null unless `container` is set. */
	network: string | null;
};

export type ApplicationNetworkOverviewDto = {
	containers: ApplicationNetworkContainerDto[];
	networks: ApplicationNetworkDto[];
	findings: ApplicationNetworkFindingDto[];
	observed_at: string;
};

/** One internal database resource this application can join, and whether it does. */
export type ResourceNetworkLinkDto = {
	resource_application_id: string;
	name: string;
	template_kind: string | null;
	network_name: string;
	linked: boolean;
};

/** An application that has joined a database resource's own network. */
export type LinkedApplicationDto = {
	application_id: string;
	name: string;
};

/** `consumers` is only ever populated when this application IS a database resource. */
export type ResourceNetworksDto = {
	linkable: ResourceNetworkLinkDto[];
	consumers: LinkedApplicationDto[];
};

export type NetworkProbePairDto = {
	from: string;
	to: string;
	from_service: string | null;
	to_service: string | null;
	port: number | null;
	/** reachable | resolved | connection_refused | unreachable | name_not_resolved | no_tools | probe_failed | unknown */
	reason: string;
};

export type ApplicationNetworkProbeDto = {
	pairs: NetworkProbePairDto[];
	observed_at: string;
};

export type ContainerNetStatDto = {
	container: string;
	net_rx_bytes: number;
	net_tx_bytes: number;
};

export type ApplicationNetworkStatsDto = {
	containers: ContainerNetStatDto[];
	observed_at: string;
};

export async function getApplicationNetworks(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
): Promise<ApplicationNetworkOverviewDto> {
	const response = await api<ApplicationNetworkOverviewDto>(
		deploymentPath(organizationId, projectId, environmentId, applicationId, 'networks'),
	);
	return response.data!;
}

function resourceNetworkPath(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	resourceId?: string,
) {
	const base = deploymentPath(
		organizationId,
		projectId,
		environmentId,
		applicationId,
		'resource-networks',
	);
	return resourceId ? `${base}/${encodeURIComponent(resourceId)}` : base;
}

export async function listResourceNetworks(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
): Promise<ResourceNetworksDto> {
	return (
		(
			await api<ResourceNetworksDto>(
				resourceNetworkPath(organizationId, projectId, environmentId, applicationId),
			)
		).data ?? { linkable: [], consumers: [] }
	);
}

export async function attachResourceNetwork(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	resourceId: string,
): Promise<ResourceNetworksDto> {
	return (
		(
			await api<ResourceNetworksDto>(
				resourceNetworkPath(organizationId, projectId, environmentId, applicationId, resourceId),
				{ method: 'POST' },
			)
		).data ?? { linkable: [], consumers: [] }
	);
}

export async function detachResourceNetwork(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	resourceId: string,
): Promise<ResourceNetworksDto> {
	return (
		(
			await api<ResourceNetworksDto>(
				resourceNetworkPath(organizationId, projectId, environmentId, applicationId, resourceId),
				{ method: 'DELETE' },
			)
		).data ?? { linkable: [], consumers: [] }
	);
}

/** Active reachability probe: dials between the app's own containers. */
export async function probeApplicationNetwork(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
): Promise<ApplicationNetworkProbeDto> {
	const response = await api<ApplicationNetworkProbeDto>(
		deploymentPath(organizationId, projectId, environmentId, applicationId, 'network-probe'),
		{ method: 'POST' },
	);
	return response.data!;
}

/** Cumulative per-container network counters (`podman stats`); diff two to get a rate. */
export async function getApplicationNetworkStats(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
): Promise<ApplicationNetworkStatsDto> {
	const response = await api<ApplicationNetworkStatsDto>(
		deploymentPath(organizationId, projectId, environmentId, applicationId, 'network-stats'),
	);
	return response.data!;
}

export async function getContainerStatus(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
): Promise<ContainerStatusDto> {
	const response = await api<ContainerStatusDto>(
		deploymentPath(organizationId, projectId, environmentId, applicationId, 'container-status'),
	);
	return response.data!;
}

export async function getApplicationSource(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
): Promise<SourceConfigDto> {
	const response = await api<SourceConfigDto>(
		deploymentPath(organizationId, projectId, environmentId, applicationId, 'source'),
	);
	return response.data!;
}
export async function listDeploymentRuns(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
): Promise<DeploymentRunDto[]> {
	const response = await api<DeploymentRunDto[]>(
		deploymentPath(organizationId, projectId, environmentId, applicationId, 'deployment-runs'),
	);
	return response.data ?? [];
}
export async function getDeploymentRun(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	runId: string,
	tail = 500,
): Promise<DeploymentRunDetailDto> {
	const response = await api<DeploymentRunDetailDto>(
		`${deploymentPath(organizationId, projectId, environmentId, applicationId, `deployment-runs/${encodeURIComponent(runId)}`)}?tail=${Math.min(Math.max(tail, 1), 1000)}`,
	);
	return response.data!;
}
export async function deleteDeploymentRun(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	runId: string,
): Promise<void> {
	await api(
		deploymentPath(
			organizationId,
			projectId,
			environmentId,
			applicationId,
			`deployment-runs/${encodeURIComponent(runId)}`,
		),
		{ method: 'DELETE' },
	);
}
export async function deleteDeploymentRuns(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
): Promise<void> {
	await api(
		deploymentPath(organizationId, projectId, environmentId, applicationId, 'deployment-runs'),
		{
			method: 'DELETE',
		},
	);
}

export async function saveApplicationSource(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	input: SourceConfigDto,
): Promise<SourceConfigDto> {
	const response = await api<SourceConfigDto>(
		deploymentPath(organizationId, projectId, environmentId, applicationId, 'source'),
		{ method: 'PUT', body: JSON.stringify(input) },
	);
	return response.data!;
}

export async function getDeployHook(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
): Promise<DeployHookDto> {
	const response = await api<DeployHookDto>(
		deploymentPath(organizationId, projectId, environmentId, applicationId, 'deploy-hook'),
	);
	return response.data!;
}

export async function rotateDeployHook(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
): Promise<DeployHookDto> {
	const response = await api<DeployHookDto>(
		deploymentPath(organizationId, projectId, environmentId, applicationId, 'deploy-hook/rotate'),
		{ method: 'POST' },
	);
	return response.data!;
}

export type HealthCheckProtocol = 'http' | 'tcp';
export type HealthCheckStatus = 'unknown' | 'healthy' | 'unhealthy';

export type HealthCheckDto = {
	application_id: string;
	enabled: boolean;
	protocol: HealthCheckProtocol;
	path: string;
	port: number | null;
	interval_seconds: number;
	timeout_seconds: number;
	unhealthy_threshold: number;
	restart_on_unhealthy: boolean;
	consecutive_failures: number;
	last_status: HealthCheckStatus;
	last_checked_at: number | null;
	last_error: string | null;
	last_restarted_at: number | null;
};

export type UpdateHealthCheckInput = {
	enabled: boolean;
	protocol: HealthCheckProtocol;
	path: string;
	port: number | null;
	interval_seconds: number;
	timeout_seconds: number;
	unhealthy_threshold: number;
	restart_on_unhealthy: boolean;
};

export async function getHealthCheck(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
): Promise<HealthCheckDto> {
	const response = await api<HealthCheckDto>(
		deploymentPath(organizationId, projectId, environmentId, applicationId, 'health-check'),
	);
	return response.data!;
}

export async function updateHealthCheck(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	input: UpdateHealthCheckInput,
): Promise<HealthCheckDto> {
	const response = await api<HealthCheckDto>(
		deploymentPath(organizationId, projectId, environmentId, applicationId, 'health-check'),
		{ method: 'PUT', body: JSON.stringify(input) },
	);
	return response.data!;
}

export type BootPolicy = 'restore' | 'always' | 'notify';

export async function getBootPolicy(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
): Promise<BootPolicy> {
	const response = await api<{ boot_policy: BootPolicy }>(
		deploymentPath(organizationId, projectId, environmentId, applicationId, 'boot-policy'),
	);
	return response.data!.boot_policy;
}

export async function updateBootPolicy(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	bootPolicy: BootPolicy,
): Promise<BootPolicy> {
	const response = await api<{ boot_policy: BootPolicy }>(
		deploymentPath(organizationId, projectId, environmentId, applicationId, 'boot-policy'),
		{ method: 'PUT', body: JSON.stringify({ boot_policy: bootPolicy }) },
	);
	return response.data!.boot_policy;
}

export async function getPortConstraints(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
): Promise<PortConstraintsDto> {
	const response = await api<PortConstraintsDto>(
		deploymentPath(
			organizationId,
			projectId,
			environmentId,
			applicationId,
			'deployment/port-constraints',
		),
	);
	return response.data ?? { reserved_ports: [], taken_ports: [] };
}

export type DatabaseBackupPolicyDto = {
	application_id: string;
	enabled: boolean;
	cron_expression: string;
	retention_count: number;
	next_run_at: number | null;
	updated_at: number;
	updated_by: string | null;
	s3_target_id: string | null;
};

export type DatabaseBackupDto = {
	id: string;
	application_id: string;
	status: 'running' | 'succeeded' | 'failed';
	engine: string;
	started_at: number;
	finished_at: number | null;
	size_bytes: number | null;
	s3_key: string | null;
	error_message: string | null;
	s3_target_id: string | null;
};

export type UpdateDatabaseBackupPolicyInput = {
	enabled: boolean;
	cron_expression: string;
	retention_count: number;
	s3_target_id?: string | null;
};

export async function getDatabaseBackupPolicy(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
): Promise<DatabaseBackupPolicyDto> {
	const response = await api<DatabaseBackupPolicyDto>(
		deploymentPath(organizationId, projectId, environmentId, applicationId, 'backup-policy'),
	);
	return response.data!;
}

export async function updateDatabaseBackupPolicy(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	input: UpdateDatabaseBackupPolicyInput,
): Promise<DatabaseBackupPolicyDto> {
	const response = await api<DatabaseBackupPolicyDto>(
		deploymentPath(organizationId, projectId, environmentId, applicationId, 'backup-policy'),
		{ method: 'PUT', body: JSON.stringify(input) },
	);
	return response.data!;
}

export async function runDatabaseBackup(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
): Promise<DatabaseBackupDto> {
	const response = await api<DatabaseBackupDto>(
		deploymentPath(organizationId, projectId, environmentId, applicationId, 'backups'),
		{ method: 'POST', body: JSON.stringify({}) },
	);
	return response.data!;
}

export async function listDatabaseBackups(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
): Promise<DatabaseBackupDto[]> {
	const response = await api<DatabaseBackupDto[]>(
		deploymentPath(organizationId, projectId, environmentId, applicationId, 'backups'),
	);
	return response.data ?? [];
}

export async function restoreDatabaseBackup(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	backupId: string,
): Promise<boolean> {
	const response = await api<boolean>(
		deploymentPath(
			organizationId,
			projectId,
			environmentId,
			applicationId,
			`backups/${backupId}/restore`,
		),
		{ method: 'POST', body: JSON.stringify({}) },
	);
	return response.data ?? true;
}

export async function deleteDatabaseBackup(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	backupId: string,
): Promise<boolean> {
	const response = await api<boolean>(
		deploymentPath(organizationId, projectId, environmentId, applicationId, `backups/${backupId}`),
		{ method: 'DELETE' },
	);
	return response.data ?? true;
}

export async function listDomains(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
): Promise<DomainDto[]> {
	const response = await api<DomainDto[]>(
		deploymentPath(organizationId, projectId, environmentId, applicationId, 'domains'),
	);
	return response.data ?? [];
}

export function domainDnsValidationPath(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	domainId: string,
): string {
	return `${deploymentPath(organizationId, projectId, environmentId, applicationId, 'domains')}/${encodeURIComponent(domainId)}/validate-dns`;
}

export async function validateDomainDns(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	domainId: string,
): Promise<DomainDnsValidationResult> {
	const response = await api<DomainDnsValidationResult>(
		domainDnsValidationPath(organizationId, projectId, environmentId, applicationId, domainId),
		{ method: 'POST' },
	);
	return response.data!;
}

/** Same check for a hostname typed in the add-domain dialog, before it is saved. */
export async function validateHostDns(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	host: string,
	serverId?: string | null,
): Promise<DomainDnsValidationResult> {
	const response = await api<DomainDnsValidationResult>(
		deploymentPath(organizationId, projectId, environmentId, applicationId, 'domains/validate-dns'),
		{ method: 'POST', body: JSON.stringify({ host, server_id: serverId || null }) },
	);
	return response.data!;
}

export async function createDomain(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	input: DomainInput,
): Promise<DomainDto> {
	const response = await api<DomainDto>(
		deploymentPath(organizationId, projectId, environmentId, applicationId, 'domains'),
		{ method: 'POST', body: JSON.stringify(input) },
	);
	return response.data!;
}

export async function generateDomain(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
): Promise<DomainDto> {
	const response = await api<DomainDto>(
		deploymentPath(organizationId, projectId, environmentId, applicationId, 'domains/generate'),
		{ method: 'POST' },
	);
	return response.data!;
}

export async function updateDomain(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	domainId: string,
	input: Partial<DomainInput>,
): Promise<DomainDto> {
	const response = await api<DomainDto>(
		`${deploymentPath(organizationId, projectId, environmentId, applicationId, 'domains')}/${domainId}`,
		{ method: 'PATCH', body: JSON.stringify(input) },
	);
	return response.data!;
}

export async function deleteDomain(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	domainId: string,
): Promise<void> {
	await api(
		`${deploymentPath(organizationId, projectId, environmentId, applicationId, 'domains')}/${domainId}`,
		{ method: 'DELETE' },
	);
}

export type BasicAuthUserDto = {
	id: string;
	username: string;
	has_password: boolean;
	created_at: number;
	updated_at: number;
};

export type BasicAuthConfigDto = {
	enabled: boolean;
	realm: string;
	users: BasicAuthUserDto[];
	protected_domain_ids: string[];
};

export type UpdateBasicAuthInput = {
	enabled?: boolean;
	realm?: string;
	protected_domain_ids?: string[];
};

export function basicAuthPath(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
): string {
	return deploymentPath(organizationId, projectId, environmentId, applicationId, 'basic-auth');
}

export async function getBasicAuth(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
): Promise<BasicAuthConfigDto> {
	const response = await api<BasicAuthConfigDto>(
		basicAuthPath(organizationId, projectId, environmentId, applicationId),
		{ method: 'GET' },
	);
	return response.data!;
}

export async function updateBasicAuth(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	input: UpdateBasicAuthInput,
): Promise<BasicAuthConfigDto> {
	const response = await api<BasicAuthConfigDto>(
		basicAuthPath(organizationId, projectId, environmentId, applicationId),
		{ method: 'PUT', body: JSON.stringify(input) },
	);
	return response.data!;
}

export async function addBasicAuthUser(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	input: { username: string; password: string },
): Promise<BasicAuthUserDto> {
	const response = await api<BasicAuthUserDto>(
		`${basicAuthPath(organizationId, projectId, environmentId, applicationId)}/users`,
		{ method: 'POST', body: JSON.stringify(input) },
	);
	return response.data!;
}

export async function updateBasicAuthUser(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	userId: string,
	input: { username?: string; password?: string },
): Promise<BasicAuthUserDto> {
	const response = await api<BasicAuthUserDto>(
		`${basicAuthPath(organizationId, projectId, environmentId, applicationId)}/users/${userId}`,
		{ method: 'PATCH', body: JSON.stringify(input) },
	);
	return response.data!;
}

export async function deleteBasicAuthUser(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	userId: string,
): Promise<void> {
	await api(
		`${basicAuthPath(organizationId, projectId, environmentId, applicationId)}/users/${userId}`,
		{ method: 'DELETE' },
	);
}

export async function uploadApplicationArtifact(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	artifact: File,
): Promise<DeploymentDto> {
	const form = new FormData();
	form.append('artifact', artifact);
	const { data } = await apiForm<DeploymentDto>(
		deploymentPath(organizationId, projectId, environmentId, applicationId, 'artifact'),
		form,
	);
	return data!;
}

export async function uploadApplicationIcon(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	icon: File,
): Promise<ApplicationDto> {
	const form = new FormData();
	form.append('icon', icon);
	const { data } = await apiForm<ApplicationDto>(
		deploymentPath(organizationId, projectId, environmentId, applicationId, 'icon'),
		form,
	);
	return data!;
}

export async function deleteApplicationArtifact(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
): Promise<DeploymentDto> {
	const { data } = await api<DeploymentDto>(
		deploymentPath(organizationId, projectId, environmentId, applicationId, 'artifact'),
		{ method: 'DELETE' },
	);
	return data!;
}

/** The deployed static site, as an HTML file or a ZIP of its files. */
export function applicationArtifactDownloadUrl(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
): string {
	return `${API_URL}${deploymentPath(organizationId, projectId, environmentId, applicationId, 'artifact')}`;
}

export async function getEnvironmentVariables(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	scope: 'runtime' | 'build' = 'runtime',
	reveal = false,
): Promise<EnvironmentVariableDto[]> {
	const response = await api<EnvironmentVariableDto[]>(
		`${deploymentPath(organizationId, projectId, environmentId, applicationId, 'environment')}?scope=${scope}${reveal ? '&reveal=true' : ''}`,
	);
	return response.data ?? [];
}

export async function saveEnvironmentVariables(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	content: string,
	scope: 'runtime' | 'build' = 'runtime',
): Promise<EnvironmentVariableDto[]> {
	const response = await api<EnvironmentVariableDto[]>(
		deploymentPath(organizationId, projectId, environmentId, applicationId, 'environment'),
		{ method: 'PUT', body: JSON.stringify({ content, scope }) },
	);
	return response.data ?? [];
}

export async function deployApplication(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	cleanBuild?: boolean,
): Promise<DeploymentDto | DeploymentJobDto> {
	const response = await api<DeploymentDto | DeploymentJobDto>(
		deploymentPath(organizationId, projectId, environmentId, applicationId, 'deploy'),
		{
			method: 'POST',
			body: JSON.stringify(cleanBuild === undefined ? {} : { clean_build: cleanBuild }),
		},
	);
	return response.data!;
}

/** Re-deploys one replica of a group-targeted application. */
export async function retryReplicaDeploy(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	serverId: string,
): Promise<DeploymentJobDto> {
	const response = await api<DeploymentJobDto>(
		deploymentPath(
			organizationId,
			projectId,
			environmentId,
			applicationId,
			`replicas/${encodeURIComponent(serverId)}/deploy`,
		),
		{ method: 'POST' },
	);
	return response.data!;
}

export async function rollbackDeploymentRun(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	runId: string,
): Promise<DeploymentJobDto> {
	const response = await api<DeploymentJobDto>(
		deploymentPath(
			organizationId,
			projectId,
			environmentId,
			applicationId,
			`deployment-runs/${encodeURIComponent(runId)}/rollback`,
		),
		{ method: 'POST' },
	);
	return response.data!;
}

export async function setCleanBuild(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	cleanBuild: boolean,
): Promise<SourceConfigDto> {
	const response = await api<SourceConfigDto>(
		deploymentPath(organizationId, projectId, environmentId, applicationId, 'source/clean-build'),
		{
			method: 'PATCH',
			body: JSON.stringify({ clean_build: cleanBuild }),
		},
	);
	return response.data!;
}

export async function stopApplication(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
): Promise<DeploymentDto> {
	const response = await api<DeploymentDto>(
		deploymentPath(organizationId, projectId, environmentId, applicationId, 'stop'),
		{ method: 'POST' },
	);
	return response.data!;
}

export async function startApplication(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
): Promise<DeploymentDto> {
	const response = await api<DeploymentDto>(
		deploymentPath(organizationId, projectId, environmentId, applicationId, 'start'),
		{ method: 'POST' },
	);
	return response.data!;
}

export async function removeApplicationRuntime(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
): Promise<DeploymentDto> {
	const response = await api<DeploymentDto>(
		deploymentPath(organizationId, projectId, environmentId, applicationId, 'remove-runtime'),
		{ method: 'POST' },
	);
	return response.data!;
}

export async function getApplicationVolumes(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
): Promise<VolumeDto[]> {
	const response = await api<VolumeDto[]>(
		deploymentPath(organizationId, projectId, environmentId, applicationId, 'volumes'),
	);
	return response.data ?? [];
}

export function applicationVolumeWorkspace(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	source: string,
) {
	return workspaceClient(
		`${deploymentPath(organizationId, projectId, environmentId, applicationId, 'volumes')}/${encodeURIComponent(source)}/workspace`,
	);
}

function managedVolumePath(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	volumeId?: string,
) {
	const base = deploymentPath(
		organizationId,
		projectId,
		environmentId,
		applicationId,
		'managed-volumes',
	);
	return volumeId ? `${base}/${encodeURIComponent(volumeId)}` : base;
}

export async function listManagedVolumes(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
): Promise<ManagedVolumeDto[]> {
	return (
		(
			await api<ManagedVolumeDto[]>(
				managedVolumePath(organizationId, projectId, environmentId, applicationId),
			)
		).data ?? []
	);
}

export async function createManagedVolume(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	input: CreateManagedVolumeInput,
): Promise<ManagedVolumeDto> {
	return (
		await api<ManagedVolumeDto>(
			managedVolumePath(organizationId, projectId, environmentId, applicationId),
			{ method: 'POST', body: JSON.stringify(input) },
		)
	).data!;
}

export async function attachManagedVolume(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	volumeId: string,
): Promise<ManagedVolumeDto> {
	return (
		await api<ManagedVolumeDto>(
			`${managedVolumePath(organizationId, projectId, environmentId, applicationId, volumeId)}/attach`,
			{ method: 'POST' },
		)
	).data!;
}

export async function detachManagedVolume(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	volumeId: string,
): Promise<ManagedVolumeDto> {
	return (
		await api<ManagedVolumeDto>(
			`${managedVolumePath(organizationId, projectId, environmentId, applicationId, volumeId)}/detach`,
			{ method: 'POST' },
		)
	).data!;
}

export async function deleteManagedVolume(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	volumeId: string,
): Promise<void> {
	await api<unknown>(
		managedVolumePath(organizationId, projectId, environmentId, applicationId, volumeId),
		{
			method: 'DELETE',
		},
	);
}

export function managedVolumeWorkspace(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	volumeId: string,
) {
	return workspaceClient(
		`${managedVolumePath(organizationId, projectId, environmentId, applicationId, volumeId)}/workspace`,
	);
}

export function streamDeploymentLogs(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	tail = 200,
	container?: string,
): EventSource {
	const query = new URLSearchParams({ tail: String(tail) });
	if (container) query.set('container', container);
	const url = `${API_URL}${deploymentPath(organizationId, projectId, environmentId, applicationId, 'logs/stream')}?${query}`;
	return new EventSource(url, { withCredentials: true });
}

export async function fetchDeploymentLogs(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	tail = 200,
	container?: string,
	serverId?: string,
): Promise<{
	lines: { text: string; ts: string | null }[];
	total: number;
	status: string;
}> {
	const query = new URLSearchParams({ tail: String(tail) });
	if (container) query.set('container', container);
	if (serverId) query.set('server_id', serverId);
	const response = await api<{
		lines: { text: string; ts: string | null }[];
		total: number;
		status: string;
	}>(`${deploymentPath(organizationId, projectId, environmentId, applicationId, 'logs')}?${query}`);
	return response.data!;
}

export async function listDeploymentQueue(
	organizationId: string,
	limit = 50,
): Promise<DeploymentJobDto[]> {
	const response = await api<DeploymentJobDto[]>(
		`/api/v1/organizations/${encodeURIComponent(organizationId)}/deployment-queue?limit=${limit}`,
	);
	return response.data ?? [];
}

export async function getDeploymentQueueJob(jobId: string): Promise<DeploymentJobDto> {
	const response = await api<DeploymentJobDto>(
		`/api/v1/deployment-queue/${encodeURIComponent(jobId)}`,
	);
	return response.data!;
}

export async function cancelDeploymentQueueJob(jobId: string): Promise<DeploymentJobDto> {
	const response = await api<DeploymentJobDto>(
		`/api/v1/deployment-queue/${encodeURIComponent(jobId)}`,
		{ method: 'POST' },
	);
	return response.data!;
}

export function streamDeploymentQueueEvents(): EventSource {
	return new EventSource(`${API_URL}/api/v1/deployment-queue/events`, {
		withCredentials: true,
	});
}

export async function composeContentGet(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
): Promise<ComposeContent> {
	const response = await api<ComposeContent>(
		deploymentPath(organizationId, projectId, environmentId, applicationId, 'compose/content'),
	);
	return response.data!;
}

export async function composeContentPut(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	input: ComposeContent,
): Promise<ComposeContent> {
	const response = await api<ComposeContent>(
		deploymentPath(organizationId, projectId, environmentId, applicationId, 'compose/content'),
		{ method: 'PUT', body: JSON.stringify(input) },
	);
	return response.data!;
}

export async function composeValidate(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	content: string,
): Promise<ValidationResponse> {
	const response = await api<ValidationResponse>(
		deploymentPath(organizationId, projectId, environmentId, applicationId, 'compose/validate'),
		{ method: 'POST', body: JSON.stringify({ content }) },
	);
	return response.data!;
}

export async function composeDeploy(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	confirmation?: string,
): Promise<DeploymentJobDto> {
	const response = await api<DeploymentJobDto>(
		deploymentPath(organizationId, projectId, environmentId, applicationId, 'compose/deploy'),
		{
			method: 'POST',
			body: JSON.stringify(confirmation ? { confirmation } : {}),
		},
	);
	return response.data!;
}

export async function composeStop(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
): Promise<DeploymentDto> {
	const response = await api<DeploymentDto>(
		deploymentPath(organizationId, projectId, environmentId, applicationId, 'compose/stop'),
		{ method: 'POST' },
	);
	return response.data!;
}

export async function composeStart(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
): Promise<DeploymentDto> {
	const response = await api<DeploymentDto>(
		deploymentPath(organizationId, projectId, environmentId, applicationId, 'compose/start'),
		{ method: 'POST' },
	);
	return response.data!;
}

export async function composeStatus(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
): Promise<ComposeServiceStatus[]> {
	const response = await api<ComposeServiceStatus[]>(
		deploymentPath(organizationId, projectId, environmentId, applicationId, 'compose/status'),
	);
	return response.data ?? [];
}

export type ContainerAction = 'start' | 'stop' | 'restart';

export async function containerAction(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	containerName: string,
	action: ContainerAction,
	serverId?: string,
): Promise<void> {
	await api(
		deploymentPath(
			organizationId,
			projectId,
			environmentId,
			applicationId,
			`containers/${encodeURIComponent(containerName)}/${action}${replicaQuery(serverId)}`,
		),
		{ method: 'POST' },
	);
}

/** Pins a container call to one replica of a group-targeted application. */
function replicaQuery(serverId?: string): string {
	return serverId ? `?server_id=${encodeURIComponent(serverId)}` : '';
}

export type ReplicaContainerDto = {
	container_name: string;
	service: string | null;
	status: string;
	ports: string[];
};

export type ReplicaContainersDto = {
	server_id: string;
	server_name: string;
	os: string | null;
	distribution: string | null;
	address: string | null;
	replica_status: ApplicationReplicaDto['status'];
	last_error: string | null;
	deployed_at: number | null;
	/** `false` when the server could not be reached; `containers` is then empty. */
	reachable: boolean;
	containers: ReplicaContainerDto[];
};

/** Live containers grouped by target server, for a group-targeted application. */
export async function listReplicaContainers(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
): Promise<ReplicaContainersDto[]> {
	const response = await api<ReplicaContainersDto[]>(
		deploymentPath(organizationId, projectId, environmentId, applicationId, 'replicas/containers'),
	);
	return response.data ?? [];
}
