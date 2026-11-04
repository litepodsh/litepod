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
