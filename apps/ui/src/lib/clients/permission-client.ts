import { api } from './base-client';

export type PermissionSet = {
	org: string[];
	projects: Record<string, string[]>;
	environments: Record<string, string[]>;
};

export type PermissionTarget = { projectId?: string; environmentId?: string };

/** Cosmetic only: hides actions the API would refuse. The API stays the authority.
 *  The server only lists an environment under the project it belongs to, so a
 *  matching `environmentId` needs no project check here. */
export function can(
	set: PermissionSet | null,
	permission: string,
	target: PermissionTarget = {},
): boolean {
	if (!set) return false;
	if (set.org.includes(permission)) return true;
	if (target.projectId && set.projects[target.projectId]?.includes(permission)) return true;
	if (target.environmentId && set.environments[target.environmentId]?.includes(permission))
		return true;
	return false;
}

export async function getMyPermissions(organizationId: string): Promise<PermissionSet> {
	const response = await api<PermissionSet>(
		`/api/v1/organizations/${encodeURIComponent(organizationId)}/me/permissions`,
	);
	return response.data!;
}
