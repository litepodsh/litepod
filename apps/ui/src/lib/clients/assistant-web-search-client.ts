import { api } from './base-client';

/**
 * The web search an organization lets its assistant and MCP agents use: a
 * SearXNG instance it runs, or a Brave Search API key. The key is write-only —
 * nothing here returns it, not even a hint. All that can be done with a saved
 * setting is turning it off and deleting it.
 */

export type WebSearchProvider = 'searxng' | 'brave';

export type WebSearchDto = {
	provider: WebSearchProvider;
	base_url: string;
	has_key: boolean;
	enabled: boolean;
	created_at: number;
	updated_at: number;
};

export type SaveWebSearchInput = {
	provider: WebSearchProvider;
	/** SearXNG only. */
	base_url?: string;
	/** Brave only. Stored encrypted. */
	api_key?: string;
};

const path = (organizationId: string) =>
	`/api/v1/organizations/${encodeURIComponent(organizationId)}/assistant/web-search`;

/** `null` when the organization has not set one up. */
export async function getWebSearch(organizationId: string): Promise<WebSearchDto | null> {
	return (await api<WebSearchDto | null>(path(organizationId))).data ?? null;
}

/** Saving runs a real search first, so a wrong address or key fails here. */
export async function saveWebSearch(
	organizationId: string,
	input: SaveWebSearchInput,
): Promise<WebSearchDto> {
	return (
		await api<WebSearchDto>(path(organizationId), {
			method: 'POST',
			body: JSON.stringify(input),
		})
	).data!;
}

export async function setWebSearchEnabled(
	organizationId: string,
	enabled: boolean,
): Promise<WebSearchDto> {
	return (
		await api<WebSearchDto>(path(organizationId), {
			method: 'PATCH',
			body: JSON.stringify({ enabled }),
		})
	).data!;
}

export async function deleteWebSearch(organizationId: string): Promise<WebSearchDto> {
	return (await api<WebSearchDto>(path(organizationId), { method: 'DELETE' })).data!;
}
