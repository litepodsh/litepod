import { api } from './base-client';
import type { ServerGroupSummaryDto, ServerSummaryDto } from './project-client';

export type ServerGroup = {
	id: string;
	name: string;
	description: string;
	member_count: number;
	member_servers: ServerSummaryDto[];
	created_at: number;
	updated_at: number;
};

/** The compact shape an application card or picker carries. */
export type ServerGroupSummary = ServerGroupSummaryDto;

export type CreateServerGroupInput = {
	name: string;
	description?: string;
};

export type UpdateServerGroupInput = {
	name?: string;
	description?: string;
};

/** Server groups are owned by an organization, so every call is scoped to one. */
const base = (organizationId: string) =>
	`/api/v1/organizations/${encodeURIComponent(organizationId)}/server-groups`;
const path = (organizationId: string, id: string, suffix = '') =>
	`${base(organizationId)}/${encodeURIComponent(id)}${suffix}`;

export async function listServerGroups(organizationId: string): Promise<ServerGroup[]> {
	return (await api<ServerGroup[]>(base(organizationId))).data ?? [];
}

export async function getServerGroup(organizationId: string, id: string): Promise<ServerGroup> {
	return (await api<ServerGroup>(path(organizationId, id))).data!;
}

export async function createServerGroup(
	organizationId: string,
	input: CreateServerGroupInput,
): Promise<ServerGroup> {
	return (
		await api<ServerGroup>(base(organizationId), {
			method: 'POST',
			body: JSON.stringify(input),
		})
	).data!;
}

export async function updateServerGroup(
	organizationId: string,
	id: string,
	input: UpdateServerGroupInput,
): Promise<ServerGroup> {
	return (
		await api<ServerGroup>(path(organizationId, id), {
			method: 'PATCH',
			body: JSON.stringify(input),
		})
	).data!;
}

export async function deleteServerGroup(organizationId: string, id: string): Promise<void> {
	await api(path(organizationId, id), { method: 'DELETE' });
}

export async function replaceServerGroupServers(
	organizationId: string,
	id: string,
	serverIds: string[],
): Promise<ServerGroup> {
	return (
		await api<ServerGroup>(path(organizationId, id, '/servers'), {
			method: 'PUT',
			body: JSON.stringify({ server_ids: serverIds }),
		})
	).data!;
}
