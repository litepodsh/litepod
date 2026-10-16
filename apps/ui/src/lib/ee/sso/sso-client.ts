import { api } from '#lib/clients/base-client.js';

/** Enterprise identity, self-hosted only. */

export type SsoConnection = {
	id: string;
	kind: 'oidc' | 'saml';
	name: string;
	enabled: boolean;
	issuer: string | null;
	client_id: string | null;
	/** The secret itself is never returned. */
	has_client_secret: boolean;
	groups_claim: string;
	jit_enabled: boolean;
	default_base_role_id: number;
	/** SAML: the IdP entity id read from its metadata. */
	idp_entity_id: string | null;
	/** Paste into the IdP: OIDC redirect URI, or SAML ACS URL. */
	redirect_uri: string;
	/** SAML: SP entity id, which also serves the SP metadata. */
	sp_entity_id: string | null;
	/** Minutes a Litepod session from this connection lasts. */
	session_ttl_minutes: number;
	/** OIDC: paste into the IdP's allowed post-logout redirect URIs. */
	post_logout_redirect_uri: string | null;
	/** OIDC: paste into the IdP's back-channel logout URL. */
	backchannel_logout_url: string | null;
};

export type SsoConnectionInput = {
	/** Fixed once created. */
	kind: 'oidc' | 'saml';
	name: string;
	issuer: string;
	client_id: string;
	/** Omit to keep the stored secret. */
	client_secret?: string;
	/** SAML: the IdP federation metadata XML. Omit to keep it. */
	idp_metadata_xml?: string;
	groups_claim: string;
	jit_enabled: boolean;
	default_base_role_id: number;
	session_ttl_minutes: number;
	enabled: boolean;
};

export type SsoDomain = {
	id: string;
	domain: string;
	verified: boolean;
	txt_host: string;
	txt_name: string;
	txt_value: string;
	/** Connection serving this domain; null = the oldest enabled one. */
	connection_id: string | null;
};

export type GroupMapping = { external_group: string; group_id: string };

export type SsoSettings = {
	enforced: boolean;
	scim_enabled: boolean;
	/** Whether this build includes SAML (cargo feature `saml`). */
	saml_supported: boolean;
};

export type ScimToken = { token: string; base_url: string };

const sso = (organizationId: string) =>
	`/api/v1/organizations/${encodeURIComponent(organizationId)}/sso`;

const json = (method: string, body: unknown): RequestInit => ({
	method,
	body: JSON.stringify(body),
});

/** Returns the identity provider URL to send the browser to. */
export async function startSso(email: string): Promise<string> {
	return (await api<{ authorize_url: string }>('/api/v1/auth/sso/start', json('POST', { email })))
		.data!.authorize_url;
}

export async function listConnections(organizationId: string): Promise<SsoConnection[]> {
	return (await api<SsoConnection[]>(`${sso(organizationId)}/connections`)).data!;
}

export async function saveConnection(
	organizationId: string,
	input: SsoConnectionInput,
	connectionId?: string,
): Promise<SsoConnection> {
	const path = connectionId
		? `${sso(organizationId)}/connections/${encodeURIComponent(connectionId)}`
		: `${sso(organizationId)}/connections`;
	return (await api<SsoConnection>(path, json(connectionId ? 'PATCH' : 'POST', input))).data!;
}

export async function deleteConnection(organizationId: string, connectionId: string) {
	await api(`${sso(organizationId)}/connections/${encodeURIComponent(connectionId)}`, {
		method: 'DELETE',
	});
}

export async function listMappings(
	organizationId: string,
	connectionId: string,
): Promise<GroupMapping[]> {
	return (
		await api<GroupMapping[]>(
			`${sso(organizationId)}/connections/${encodeURIComponent(connectionId)}/group-mappings`,
		)
	).data!;
}

export async function setMappings(
	organizationId: string,
	connectionId: string,
	mappings: GroupMapping[],
): Promise<GroupMapping[]> {
	return (
		await api<GroupMapping[]>(
			`${sso(organizationId)}/connections/${encodeURIComponent(connectionId)}/group-mappings`,
			json('PUT', mappings),
		)
	).data!;
}

export async function listDomains(organizationId: string): Promise<SsoDomain[]> {
	return (await api<SsoDomain[]>(`${sso(organizationId)}/domains`)).data!;
}

export async function addDomain(organizationId: string, domain: string): Promise<SsoDomain> {
	return (await api<SsoDomain>(`${sso(organizationId)}/domains`, json('POST', { domain }))).data!;
}

export async function verifyDomain(organizationId: string, domainId: string): Promise<SsoDomain> {
	return (
		await api<SsoDomain>(`${sso(organizationId)}/domains/${encodeURIComponent(domainId)}/verify`, {
			method: 'POST',
		})
	).data!;
}

export async function assignDomain(
	organizationId: string,
	domainId: string,
	connectionId: string | null,
): Promise<SsoDomain> {
	return (
		await api<SsoDomain>(
			`${sso(organizationId)}/domains/${encodeURIComponent(domainId)}`,
			json('PATCH', { connection_id: connectionId }),
		)
	).data!;
}

export async function deleteDomain(organizationId: string, domainId: string) {
	await api(`${sso(organizationId)}/domains/${encodeURIComponent(domainId)}`, {
		method: 'DELETE',
	});
}

export async function getSsoSettings(organizationId: string): Promise<SsoSettings> {
	return (await api<SsoSettings>(`${sso(organizationId)}/settings`)).data!;
}

export async function setEnforcement(
	organizationId: string,
	enforced: boolean,
): Promise<SsoSettings> {
	return (await api<SsoSettings>(`${sso(organizationId)}/enforcement`, json('PUT', { enforced })))
		.data!;
}

export async function rotateScimToken(organizationId: string): Promise<ScimToken> {
	return (await api<ScimToken>(`${sso(organizationId)}/scim-token`, { method: 'POST' })).data!;
}

export async function disableScim(organizationId: string) {
	await api(`${sso(organizationId)}/scim-token`, { method: 'DELETE' });
}
