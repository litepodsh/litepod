import { api } from './base-client';
import type { UserWithOrgs } from './auth-client';

export type { UserWithOrgs };

/**
 * The Admin portal's own account list. It is a different route from the
 * generic `/api/v1/users` on purpose: the portal exists only in the cloud
 * edition, and the API answers 404 there otherwise — so a self-hosted
 * instance cannot reach this surface even by calling the endpoint directly.
 */
export async function listAdminUsers(): Promise<UserWithOrgs[]> {
	const resp = await api<UserWithOrgs[]>('/api/v1/admin/users');
	return resp.data ?? [];
}

/** The portal's own disable toggle — same cloud-only rule as the list. */
export async function setAdminUserDisabled(userId: string, disabled: boolean): Promise<void> {
	await api<void>(`/api/v1/admin/users/${userId}/disabled`, {
		method: 'PUT',
		body: JSON.stringify({ disabled }),
	});
}

/** One agent a user approved over MCP OAuth. Times are UTC, without a zone suffix. */
export type McpConnection = {
	id: string;
	client_name: string;
	/** Detected agent: claude | chatgpt | hermes | other. */
	agent: string;
	organization_name: string;
	created_at: string;
	expires_at: string;
	closed_at: string | null;
	last_used_at: string | null;
};

export async function listUserMcpConnections(userId: string): Promise<McpConnection[]> {
	const resp = await api<McpConnection[]>(`/api/v1/admin/users/${userId}/mcp-connections`);
	return resp.data ?? [];
}

export async function revokeUserMcpConnection(userId: string, connectionId: string): Promise<void> {
	await api<void>(`/api/v1/admin/users/${userId}/mcp-connections/${connectionId}`, {
		method: 'DELETE',
	});
}
