import { api, apiForm } from './base-client';

export type OrganizationDto = {
	id: string;
	name: string;
	slug: string;
	description: string;
	icon: string;
	icon_bg: string;
	plan: string;
	is_root: boolean;
	audit_logs_enabled: boolean;
	/** The assistant's kill switch. The panel is hidden while this is false. */
	assistant_enabled: boolean;
	/** The MCP endpoint answers only while this is true. */
	mcp_enabled: boolean;
	audit_logs_retention_days: number;
	created_at: string;
};

// Every page and the sidebar independently call listOrganizations() on mount,
// re-fetching the same list on every navigation. That's an extra serialized
// round trip in front of the org-scoped data each page actually needs — cheap
// locally, real latency against a remote API. Cache in-memory for the tab's
// lifetime; invalidate on any write below and on logout (organization-client
// invalidates itself, auth-client invalidates on logout).
let organizationsCache: Promise<OrganizationDto[]> | null = null;

export function listOrganizations(): Promise<OrganizationDto[]> {
	if (!organizationsCache) {
		organizationsCache = api<OrganizationDto[]>('/api/v1/organizations')
			.then((response) => response.data ?? [])
			.catch((err) => {
				organizationsCache = null;
				throw err;
			});
	}
	return organizationsCache;
}

export function invalidateOrganizationsCache(): void {
	organizationsCache = null;
}

export async function createOrganization(input: {
	name: string;
	slug: string;
	description?: string;
	icon?: string;
	icon_bg?: string;
	plan?: string;
}): Promise<OrganizationDto> {
	const response = await api<OrganizationDto>('/api/v1/organizations', {
		method: 'POST',
		body: JSON.stringify(input),
	});
	invalidateOrganizationsCache();
	return response.data!;
}

export async function updateOrganization(
	organizationId: string,
	updates: {
		name?: string;
		description?: string;
		icon?: string;
		icon_bg?: string;
		plan?: string;
		audit_logs_enabled?: boolean;
		assistant_enabled?: boolean;
		mcp_enabled?: boolean;
		audit_logs_retention_days?: number;
	},
): Promise<OrganizationDto> {
	const response = await api<OrganizationDto>(`/api/v1/organizations/${organizationId}`, {
		method: 'PATCH',
		body: JSON.stringify(updates),
	});
	invalidateOrganizationsCache();
	return response.data!;
}

export async function uploadOrganizationIcon(
	organizationId: string,
	icon: File,
): Promise<OrganizationDto> {
	const form = new FormData();
	form.append('icon', icon);
	const response = await apiForm<OrganizationDto>(
		`/api/v1/organizations/${organizationId}/icon`,
		form,
	);
	invalidateOrganizationsCache();
	return response.data!;
}

export async function getOrganization(organizationId: string): Promise<OrganizationDto> {
	const response = await api<OrganizationDto>(`/api/v1/organizations/${organizationId}`);
	return response.data!;
}

export async function deleteOrganization(organizationId: string): Promise<void> {
	await api<void>(`/api/v1/organizations/${organizationId}`, {
		method: 'DELETE',
	});
	invalidateOrganizationsCache();
}

export type OrganizationMemberDto = {
	user_id: string;
	email: string;
	name: string;
	role_id: number;
	role_name: string;
	joined_at: string;
	/** Disabled by a SysAdmin: cannot sign in by any method. */
	disabled: boolean;
};

export async function listMembers(organizationId: string): Promise<OrganizationMemberDto[]> {
	const response = await api<OrganizationMemberDto[]>(
		`/api/v1/organizations/${organizationId}/members`,
	);
	return response.data ?? [];
}

/** Sets a member's base role: 1 Admin, 2 Editor, 3 Viewer, 4 Member. */
export async function updateMemberRole(
	organizationId: string,
	userId: string,
	roleId: number,
): Promise<void> {
	await api<void>(`/api/v1/organizations/${organizationId}/members/${userId}`, {
		method: 'PATCH',
		body: JSON.stringify({ role_id: roleId }),
	});
}

/** SysAdmin only. Disabling also signs the user out everywhere. */
export async function setUserDisabled(userId: string, disabled: boolean): Promise<void> {
	await api<void>(`/api/v1/users/${userId}/disabled`, {
		method: 'PUT',
		body: JSON.stringify({ disabled }),
	});
}

export async function removeMember(organizationId: string, userId: string): Promise<void> {
	await api<void>(`/api/v1/organizations/${organizationId}/members/${userId}`, {
		method: 'DELETE',
	});
}

/** A domain as the organization-wide lists show it: enough to link and label a host. */
export type AppSummaryDomainDto = {
	id: string;
	host: string;
	port: number;
	https: boolean;
	is_primary: boolean;
	/** Fully resolved address, e.g. `https://ghost.example.com`. */
	url: string;
	/** Name of the saved certificate the domain uses, when it uses one. */
	certificate_name: string | null;
};

/** One application with everything the organization-wide lists render about it. */
export type AppSummaryDto = {
	application: {
		id: string;
		name: string;
		template_kind: string | null;
		catalog_icon_url: string | null;
		icon_url: string | null;
		created_at: number;
		updated_at: number;
	};
	project: { id: string; name: string };
	environment: { id: string; name: string; is_production: boolean };
	status: string;
	/** Last successful deploy, epoch ms; `null` when never deployed. */
	deployed_at: number | null;
	domains: AppSummaryDomainDto[];
	/** `null` when the application's server could not be reached. */
	containers: { running: number; total: number } | null;
};

/**
 * Every application of the organization in one request. The lists used to fan
 * out one deployment call per application, which the global rate limit turned
 * into blank rows for all but the first handful.
 */
export async function listOrganizationAppsSummary(
	organizationId: string,
): Promise<AppSummaryDto[]> {
	const response = await api<AppSummaryDto[]>(
		`/api/v1/organizations/${organizationId}/apps/summary`,
	);
	return response.data ?? [];
}
