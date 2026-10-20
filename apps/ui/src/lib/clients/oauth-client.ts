import { api } from './base-client';

export type ConsentInfo = {
	client_name: string;
	organization: { id: string; name: string };
	redirect_host: string;
	/** The code leaves the user's device for a hosted client. */
	remote: boolean;
};

/** The parts of the original authorize query that consent must echo back. */
export type ConsentParams = {
	response_type: string;
	client_id: string;
	redirect_uri: string;
	code_challenge: string;
	code_challenge_method: string;
	state: string;
};

/** The organization id inside the MCP `resource` URL the client was given. */
export function organizationFromResource(resource: string): string | null {
	const match = resource.match(/\/api\/v1\/organizations\/([^/]+)\/mcp/);
	return match ? decodeURIComponent(match[1]) : null;
}

export async function getConsent(
	organizationId: string,
	clientId: string,
	redirectUri: string,
): Promise<ConsentInfo> {
	const query = new URLSearchParams({ client_id: clientId, redirect_uri: redirectUri });
	const response = await api<ConsentInfo>(
		`/api/v1/organizations/${organizationId}/oauth/consent?${query.toString()}`,
	);
	return response.data!;
}

export async function submitConsent(
	organizationId: string,
	params: ConsentParams,
	approve: boolean,
): Promise<string> {
	const response = await api<{ redirect_to: string }>(
		`/api/v1/organizations/${organizationId}/oauth/consent`,
		{
			method: 'POST',
			body: JSON.stringify({ ...params, approve }),
		},
	);
	return response.data!.redirect_to;
}
