import { api } from './base-client';

export type PersonalAccessToken = {
	id: string;
	name: string;
	token_prefix: string;
	scopes: string[];
	expires_at: string | null;
	last_used_at: string | null;
	created_at: string;
};

export type CreatedPersonalAccessToken = {
	token_info: PersonalAccessToken;
	token: string;
};

export async function listTokens(): Promise<PersonalAccessToken[]> {
	const resp = await api<PersonalAccessToken[]>('/api/v1/account/tokens');
	return resp.data!;
}

export async function createToken(input: {
	name: string;
	scopes: string[];
	expires_at: string | null;
}): Promise<CreatedPersonalAccessToken> {
	const resp = await api<CreatedPersonalAccessToken>('/api/v1/account/tokens', {
		method: 'POST',
		body: JSON.stringify(input),
	});
	return resp.data!;
}

export async function revokeToken(id: string): Promise<void> {
	await api(`/api/v1/account/tokens/${id}`, {
		method: 'DELETE',
	});
}

export async function revokeAllTokens(): Promise<void> {
	await api('/api/v1/account/tokens', {
		method: 'DELETE',
	});
}

/** Catalog permission names a token may take as scopes, in catalog order. */
export async function listGrantablePermissions(): Promise<string[]> {
	const resp = await api<string[]>('/api/v1/account/tokens/permissions');
	return resp.data!;
}
