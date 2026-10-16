import { api } from './base-client';

export type IngressMode = 'central' | 'public_edge' | 'private_relay' | 'private_only';
export type PodmanMode = 'rootless' | 'rootful';
export type ServerState =
	| 'needs_verification'
	| 'needs_setup'
	| 'setting_up'
	| 'ready'
	| 'degraded'
	| 'offline'
	| 'unsupported';
export type Server = {
	id: string;
	kind: string;
	name: string;
	ingress_mode: IngressMode;
	podman_mode: PodmanMode;
	state: ServerState;
	public_address: string | null;
	ssh_host: string | null;
	ssh_port: number | null;
	ssh_user: string | null;
	host_key_fingerprint: string | null;
	assigned_application_count: number;
	/** `null` is the instance-wide local server; anything else is owned by that organization. */
	organization_id: string | null;
	facts: {
		os: string | null;
		version: string | null;
		architecture: string | null;
		podman_version: string | null;
		caddy_version: string | null;
		swap_bytes: number | null;
		/** No swap and 16 GiB of RAM or less; the UI offers to create a swap file. */
		swap_recommended: boolean;
		capacity: {
			cpu_cores: number | null;
			memory_bytes: number | null;
			disk_free_bytes: number | null;
		};
	};
	last_error: { code: string; message: string } | null;
};
export type ServerJob = {
	id: string;
	server_id: string;
	kind: string;
	status: 'queued' | 'running' | 'succeeded' | 'failed';
	phase: string;
	log: string;
	error: { code: string; message: string } | null;
	created_at: number;
	started_at: number | null;
	finished_at: number | null;
};
/** One pin on the fleet map. Coordinates are `null` when the address could not be placed. */
export type ServerGeo = {
	server_id: string;
	name: string;
	kind: string;
	state: ServerState;
	ingress_mode: IngressMode;
	/** The instance's own host, the one every remote server hangs off. */
	is_main: boolean;
	latitude: number | null;
	longitude: number | null;
	city: string | null;
	country: string | null;
	/** IANA zone, for the server's own wall clock. */
	timezone: string | null;
	asn: string | null;
	unlocated_reason: 'no_address' | 'private_address' | 'lookup_failed' | null;
};
export type ServerRegistration = { server: Server; generated_public_key: string | null };
export type RemoteServerCapacity = { available: boolean; reason: string | null };
/** `authorized_keys_command` is the paste-ready installer for `public_key`. */
export type ServerPublicKey = { public_key: string; authorized_keys_command: string };
export type ServerPreflight = { server: Server; observed_host_key_fingerprint: string | null };
/** An application blocking a server's removal, named by project and environment. */
export type ServerAssignedApplication = {
	id: string;
	name: string;
	environment_id: string;
	environment_name: string;
	project_id: string;
	project_name: string;
	deploy_type: 'static' | 'git' | 'compose';
	template_kind: string | null;
	icon_url: string | null;
	catalog_icon_url: string | null;
};
/** One published host port, from one of the three sources litepod tracks. */
export type ServerPort = {
	application_id: string;
	application_name: string;
	service: string | null;
	host_ip: string | null;
	host_port: string;
	container_port: string;
	protocol: string;
	source: 'compose' | 'port-override' | 'managed-endpoint';
};
/** A container that exits non-zero still comes back as a result, so the output can be shown. */
export type ServerTestDeploy = {
	success: boolean;
	image: string;
	exit_code: number;
	stdout: string;
	stderr: string;
	duration_ms: number;
};
export type ServerMaintenanceResult = {
	action: string;
	success: boolean;
	stdout: string;
	stderr: string;
	duration_ms: number;
};

export type CreateServerInput = {
	name: string;
	host: string;
	port: number;
	user: string;
	ingress_mode: Exclude<IngressMode, 'central'>;
	podman_mode: PodmanMode;
	public_address?: string;
	credential: { kind: 'generate' } | { kind: 'import'; private_key: string };
};

/** Keeps registration payload construction testable and avoids UI-specific defaults. */
export function createServerRequest(input: CreateServerInput): CreateServerInput {
	return { ...input, credential: { ...input.credential } };
}

/** The primary next action is derived only from safe server state. */
export function serverActionLabel(server: Pick<Server, 'state'>): string {
	if (server.state === 'needs_setup') return 'Set up server';
	if (server.state === 'degraded') return 'Repair server';
	return 'Continue setup';
}

/** Servers are owned by an organization, so every call is scoped to one. */
const base = (organizationId: string) =>
	`/api/v1/organizations/${encodeURIComponent(organizationId)}/servers`;
const path = (organizationId: string, id: string, suffix = '') =>
	`${base(organizationId)}/${encodeURIComponent(id)}${suffix}`;

export async function listServers(organizationId: string): Promise<Server[]> {
	return (await api<Server[]>(base(organizationId))).data ?? [];
}
/** Every server this organization can see, geolocated from its public address. */
export async function listServerGeo(organizationId: string): Promise<ServerGeo[]> {
	return (await api<ServerGeo[]>(`${base(organizationId)}/geo`)).data ?? [];
}
export async function getRemoteServerCapacity(
	organizationId: string,
): Promise<RemoteServerCapacity> {
	return (await api<RemoteServerCapacity>(`${base(organizationId)}/remote-capacity`)).data!;
}
export async function getServer(organizationId: string, id: string): Promise<Server> {
	return (await api<Server>(path(organizationId, id))).data!;
}
export async function createServer(
	organizationId: string,
	input: CreateServerInput,
): Promise<ServerRegistration> {
	return (
		await api<ServerRegistration>(base(organizationId), {
			method: 'POST',
			body: JSON.stringify(createServerRequest(input)),
		})
	).data!;
}
export async function deleteServer(organizationId: string, id: string): Promise<void> {
	await api(path(organizationId, id), { method: 'DELETE' });
}
export async function renameServer(
	organizationId: string,
	id: string,
	name: string,
): Promise<Server> {
	return (
		await api<Server>(path(organizationId, id), {
			method: 'PATCH',
			body: JSON.stringify({ name }),
		})
	).data!;
}
export async function getServerPublicKey(
	organizationId: string,
	id: string,
): Promise<ServerPublicKey> {
	return (await api<ServerPublicKey>(path(organizationId, id, '/public-key'))).data!;
}
export async function preflightServer(
	organizationId: string,
	id: string,
): Promise<ServerPreflight> {
	return (await api<ServerPreflight>(path(organizationId, id, '/preflight'), { method: 'POST' }))
		.data!;
}
export async function confirmHostKey(
	organizationId: string,
	id: string,
	fingerprint: string,
): Promise<Server> {
	return (
		await api<Server>(path(organizationId, id, '/confirm-host-key'), {
			method: 'POST',
			body: JSON.stringify({ fingerprint }),
		})
	).data!;
}
export async function setupServer(
	organizationId: string,
	id: string,
	repair = false,
): Promise<ServerJob> {
	return (
		await api<ServerJob>(path(organizationId, id, repair ? '/repair' : '/setup'), {
			method: 'POST',
		})
	).data!;
}
export async function healthcheckServer(organizationId: string, id: string): Promise<Server> {
	return (await api<Server>(path(organizationId, id, '/healthcheck'), { method: 'POST' })).data!;
}
async function runMaintenance(
	organizationId: string,
	id: string,
	action: string,
): Promise<ServerMaintenanceResult> {
	return (
		await api<ServerMaintenanceResult>(path(organizationId, id, `/maintenance/${action}`), {
			method: 'POST',
		})
	).data!;
}
export function updateServerPackages(
	organizationId: string,
	id: string,
): Promise<ServerMaintenanceResult> {
	return runMaintenance(organizationId, id, 'update-packages');
}
export function restartServer(
	organizationId: string,
	id: string,
): Promise<ServerMaintenanceResult> {
	return runMaintenance(organizationId, id, 'restart');
}
export function pruneServerImages(
	organizationId: string,
	id: string,
): Promise<ServerMaintenanceResult> {
	return runMaintenance(organizationId, id, 'prune-images');
}
export function restartServerPodman(
	organizationId: string,
	id: string,
): Promise<ServerMaintenanceResult> {
	return runMaintenance(organizationId, id, 'restart-podman');
}
export function checkServerDisk(
	organizationId: string,
	id: string,
): Promise<ServerMaintenanceResult> {
	return runMaintenance(organizationId, id, 'check-disk');
}
export function createServerSwap(
	organizationId: string,
	id: string,
): Promise<ServerMaintenanceResult> {
	return runMaintenance(organizationId, id, 'create-swap');
}
export async function listServerApplications(
	organizationId: string,
	id: string,
): Promise<ServerAssignedApplication[]> {
	return (
		(await api<ServerAssignedApplication[]>(path(organizationId, id, '/applications'))).data ?? []
	);
}
export async function listServerPorts(organizationId: string, id: string): Promise<ServerPort[]> {
	return (await api<ServerPort[]>(path(organizationId, id, '/ports'))).data ?? [];
}
export async function testDeployServer(
	organizationId: string,
	id: string,
): Promise<ServerTestDeploy> {
	return (await api<ServerTestDeploy>(path(organizationId, id, '/test-deploy'), { method: 'POST' }))
		.data!;
}
export async function listServerJobs(organizationId: string, id: string): Promise<ServerJob[]> {
	return (await api<ServerJob[]>(path(organizationId, id, '/jobs'))).data ?? [];
}
