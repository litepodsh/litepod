import { api } from './base-client';

/**
 * An organization's model connections: the keys and endpoints the assistant
 * talks to. The key itself never crosses this boundary — `api_key_hint` is a
 * redacted string that identifies one without being one.
 */

/** One kind of provider the "add connection" dialog offers. */
export type ProviderDto = {
	id: string;
	name: string;
	/** Prefilled in the dialog. Empty for the bring-your-own-endpoint kinds. */
	base_url: string;
	protocol: 'openai' | 'anthropic';
	key: 'required' | 'optional' | 'none';
	/** True when the user is expected to supply the URL themselves. */
	custom_url: boolean;
	opencode: boolean;
};

export type ModelEntry = {
	id: string;
	label: string;
	context: number | null;
	tools: boolean;
	/** `null` when usable; `endpoint` or `no-tools` when it is listed but not selectable. */
	blocked: string | null;
};

export type ConnectionDto = {
	id: string;
	organization_id: string;
	name: string;
	provider: string;
	protocol: 'openai' | 'anthropic';
	base_url: string;
	has_key: boolean;
	api_key_hint: string | null;
	is_default: boolean;
	status: 'unverified' | 'ok' | 'failed';
	/** The provider's own words when a probe failed. */
	status_detail: string | null;
	models: ModelEntry[];
	models_fetched_at: number | null;
	created_at: number;
	updated_at: number;
};

export type CreateConnectionInput = {
	name: string;
	provider: string;
	base_url?: string;
	api_key?: string;
	is_default?: boolean;
};

export type UpdateConnectionInput = {
	name?: string;
	base_url?: string;
	/** Omit to keep the stored key; send an empty string to clear it. */
	api_key?: string;
	is_default?: boolean;
};

const base = (organizationId: string) =>
	`/api/v1/organizations/${encodeURIComponent(organizationId)}/assistant/connections`;

/** Not organization-scoped: the provider list is a constant, and holds no secrets. */
export async function listProviders(): Promise<ProviderDto[]> {
	return (await api<ProviderDto[]>('/api/v1/assistant/providers')).data!;
}

export async function listConnections(organizationId: string): Promise<ConnectionDto[]> {
	return (await api<ConnectionDto[]>(base(organizationId))).data!;
}

/**
 * Saving probes the provider first: the connection is only stored once its key
 * has listed models. So a rejected key surfaces here, in the dialog that set it.
 */
export async function createConnection(
	organizationId: string,
	input: CreateConnectionInput,
): Promise<ConnectionDto> {
	return (
		await api<ConnectionDto>(base(organizationId), {
			method: 'POST',
			body: JSON.stringify(input),
		})
	).data!;
}

export async function updateConnection(
	organizationId: string,
	connectionId: string,
	input: UpdateConnectionInput,
): Promise<ConnectionDto> {
	return (
		await api<ConnectionDto>(`${base(organizationId)}/${encodeURIComponent(connectionId)}`, {
			method: 'PATCH',
			body: JSON.stringify(input),
		})
	).data!;
}

/**
 * Re-reads the catalog. Unlike a save, a failure comes back on the connection
 * rather than as an error: the row still exists, and its `status_detail` is what
 * tells the user how to fix it.
 */
export async function refreshConnection(
	organizationId: string,
	connectionId: string,
): Promise<ConnectionDto> {
	return (
		await api<ConnectionDto>(
			`${base(organizationId)}/${encodeURIComponent(connectionId)}/refresh`,
			{ method: 'POST' },
		)
	).data!;
}

export async function deleteConnection(
	organizationId: string,
	connectionId: string,
): Promise<ConnectionDto> {
	return (
		await api<ConnectionDto>(`${base(organizationId)}/${encodeURIComponent(connectionId)}`, {
			method: 'DELETE',
		})
	).data!;
}
