import { API_URL, api } from './base-client';
import { workspaceClient } from './volume-workspace-client';
import { TerminalSocket } from './terminal-client';

/** Resource families the generic list/inspect/delete/prune routes accept. */
export type PodmanResource = 'containers' | 'pods' | 'images' | 'volumes' | 'networks';
export type ContainerAction = 'start' | 'stop' | 'restart' | 'kill' | 'pause' | 'unpause';
export type PodAction = 'start' | 'stop' | 'restart' | 'kill';

export type PodmanRecord = Record<string, unknown>;

const base = (organizationId: string, serverId: string) =>
	`/api/v1/organizations/${encodeURIComponent(organizationId)}/servers/${encodeURIComponent(
		serverId,
	)}/podman`;

export async function getPodmanOverview(organizationId: string, serverId: string) {
	return (await api<PodmanRecord>(`${base(organizationId, serverId)}/overview`)).data!;
}

export async function getPodmanSystemDf(organizationId: string, serverId: string) {
	return (await api<PodmanRecord>(`${base(organizationId, serverId)}/system/df`)).data!;
}

export async function pruneBuildCache(
	organizationId: string,
	serverId: string,
	confirmation: string,
) {
	return api(`${base(organizationId, serverId)}/system/prune-build-cache`, {
		method: 'POST',
		body: JSON.stringify({ confirmation }),
	});
}

export async function listPodmanResources(
	organizationId: string,
	serverId: string,
	resource: PodmanResource,
): Promise<PodmanRecord[]> {
	const data = (
		await api<PodmanRecord[]>(`${base(organizationId, serverId)}/resources/${resource}`)
	).data;
	return Array.isArray(data) ? data : [];
}

export async function getPodmanResource(
	organizationId: string,
	serverId: string,
	resource: PodmanResource,
	id: string,
) {
	return (
		await api<PodmanRecord>(
			`${base(organizationId, serverId)}/resources/${resource}/${encodeURIComponent(id)}`,
		)
	).data!;
}

export async function deletePodmanResource(
	organizationId: string,
	serverId: string,
	resource: PodmanResource,
	id: string,
	confirmation: string,
) {
	return api(`${base(organizationId, serverId)}/resources/${resource}/${encodeURIComponent(id)}`, {
		method: 'DELETE',
		body: JSON.stringify({ confirmation }),
	});
}

export async function prunePodmanResource(
	organizationId: string,
	serverId: string,
	resource: PodmanResource,
	confirmation: string,
) {
	return api(`${base(organizationId, serverId)}/resources/${resource}/prune`, {
		method: 'POST',
		body: JSON.stringify({ confirmation }),
	});
}

export async function containerAction(
	organizationId: string,
	serverId: string,
	id: string,
	action: ContainerAction,
	confirmation: string,
) {
	return api(`${base(organizationId, serverId)}/container/${encodeURIComponent(id)}/${action}`, {
		method: 'POST',
		body: JSON.stringify({ confirmation }),
	});
}

export async function podAction(
	organizationId: string,
	serverId: string,
	id: string,
	action: PodAction,
	confirmation: string,
) {
	return api(`${base(organizationId, serverId)}/pod/${encodeURIComponent(id)}/${action}`, {
		method: 'POST',
		body: JSON.stringify({ confirmation }),
	});
}

export async function getContainerLogs(organizationId: string, serverId: string, id: string) {
	return (
		await api<{ logs: string }>(`${base(organizationId, serverId)}/logs/${encodeURIComponent(id)}`)
	).data!;
}

export async function getContainerTop(organizationId: string, serverId: string, id: string) {
	return (
		await api<{ processes: string }>(
			`${base(organizationId, serverId)}/top/${encodeURIComponent(id)}`,
		)
	).data!;
}

export async function pullImage(
	organizationId: string,
	serverId: string,
	reference: string,
	confirmation: string,
) {
	return api(`${base(organizationId, serverId)}/image/pull`, {
		method: 'POST',
		body: JSON.stringify({ reference, confirmation }),
	});
}

export async function createVolume(
	organizationId: string,
	serverId: string,
	body: { name: string; driver?: string; labels?: string[] },
) {
	return api(`${base(organizationId, serverId)}/volume/create`, {
		method: 'POST',
		body: JSON.stringify(body),
	});
}

export async function createNetwork(
	organizationId: string,
	serverId: string,
	body: {
		name: string;
		driver?: string;
		subnet?: string;
		gateway?: string;
		internal?: boolean;
	},
) {
	return api(`${base(organizationId, serverId)}/network/create`, {
		method: 'POST',
		body: JSON.stringify(body),
	});
}

export async function networkMember(
	organizationId: string,
	serverId: string,
	id: string,
	action: 'connect' | 'disconnect',
	container: string,
) {
	return api(`${base(organizationId, serverId)}/network/${encodeURIComponent(id)}/${action}`, {
		method: 'POST',
		body: JSON.stringify({ container }),
	});
}

export async function getPodmanEvents(
	organizationId: string,
	serverId: string,
	since = '1h',
): Promise<PodmanRecord[]> {
	const data = (
		await api<PodmanRecord[]>(
			`${base(organizationId, serverId)}/events?since=${encodeURIComponent(since)}`,
		)
	).data;
	return Array.isArray(data) ? data : [];
}

export function streamPodmanEvents(organizationId: string, serverId: string): EventSource {
	return new EventSource(`${API_URL}${base(organizationId, serverId)}/events/stream`, {
		withCredentials: true,
	});
}

export function streamPodmanMetrics(organizationId: string, serverId: string): EventSource {
	return new EventSource(`${API_URL}${base(organizationId, serverId)}/metrics/stream`, {
		withCredentials: true,
	});
}

export function streamContainerLogs(
	organizationId: string,
	serverId: string,
	id: string,
): EventSource {
	return new EventSource(
		`${API_URL}${base(organizationId, serverId)}/logs/${encodeURIComponent(id)}/stream`,
		{ withCredentials: true },
	);
}

/** File browser for a named volume — same shape as the app Volumes tab. */
export function volumeWorkspace(organizationId: string, serverId: string, volume: string) {
	return workspaceClient(
		`${base(organizationId, serverId)}/volume/${encodeURIComponent(volume)}/workspace`,
	);
}

/** Interactive shell into a container on the selected server (OrgAdmin). */
export async function createContainerExec(
	organizationId: string,
	serverId: string,
	containerId: string,
	command: string[],
	tty = true,
): Promise<string> {
	const res = await fetch(
		`${API_URL}${base(organizationId, serverId)}/container/${encodeURIComponent(containerId)}/exec`,
		{
			method: 'POST',
			credentials: 'include',
			headers: { 'content-type': 'application/json' },
			body: JSON.stringify({ command, tty }),
		},
	);
	if (!res.ok) throw new Error(`exec create failed: ${res.status} ${await res.text()}`);
	return ((await res.json()) as { exec_id: string }).exec_id;
}

export function openContainerExecSocket(
	organizationId: string,
	serverId: string,
	containerId: string,
	execId: string,
): TerminalSocket {
	const wsBase = API_URL.replace(/^http/, 'ws');
	const url = `${wsBase}${base(organizationId, serverId)}/container/${encodeURIComponent(
		containerId,
	)}/exec/${encodeURIComponent(execId)}/ws`;
	return new TerminalSocket(new WebSocket(url));
}
