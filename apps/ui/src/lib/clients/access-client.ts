import { api } from './base-client';
import type { PermissionSet } from './permission-client';

/** Access-control management (roles, groups, assignments). */

export type CatalogEntry = { name: string; org_only: boolean };

/** `id` is `'1'`..`'4'` for system roles, a UUID for custom ones. */
export type Role = {
	id: string;
	name: string;
	description: string;
	system: boolean;
	permissions: string[];
};

export type RoleInput = { name: string; description: string; permissions: string[] };

export type GroupMember = { user_id: string; name: string; email: string };

export type Group = {
	id: string;
	name: string;
	/** `local`, or `idp` / `scim` when an identity provider owns it. */
	source: string;
	members: GroupMember[];
};

export type ScopeType = 'org' | 'project' | 'environment';

export type Assignment = {
	id: string;
	principal_type: 'user' | 'group';
	principal_id: string;
	principal_name: string;
	role_id: string;
	role_name: string;
	scope_type: ScopeType;
	scope_id: string;
	/** Organization, project, or "project / environment" name. */
	scope_name: string;
	project_id: string | null;
	via_group?: string;
};

export type AssignmentInput = {
	principal_type: 'user' | 'group';
	principal_id: string;
	role_id: string;
	scope_type: ScopeType;
	scope_id: string;
};

export type EffectivePermissions = {
	base_role_id: number;
	base_role_name: string;
	permissions: PermissionSet;
	assignments: Assignment[];
};

const org = (organizationId: string) =>
	`/api/v1/organizations/${encodeURIComponent(organizationId)}`;

const json = (method: string, body: unknown): RequestInit => ({
	method,
	body: JSON.stringify(body),
});

export async function getCatalog(organizationId: string): Promise<CatalogEntry[]> {
	return (await api<CatalogEntry[]>(`${org(organizationId)}/permissions/catalog`)).data!;
}

export async function listRoles(organizationId: string): Promise<Role[]> {
	return (await api<Role[]>(`${org(organizationId)}/roles`)).data!;
}

export async function createRole(organizationId: string, input: RoleInput): Promise<Role> {
	return (await api<Role>(`${org(organizationId)}/roles`, json('POST', input))).data!;
}

export async function updateRole(
	organizationId: string,
	roleId: string,
	input: RoleInput,
): Promise<Role> {
	return (
		await api<Role>(
			`${org(organizationId)}/roles/${encodeURIComponent(roleId)}`,
			json('PATCH', input),
		)
	).data!;
}

export async function deleteRole(organizationId: string, roleId: string, force = false) {
	await api(
		`${org(organizationId)}/roles/${encodeURIComponent(roleId)}${force ? '?force=true' : ''}`,
		{
			method: 'DELETE',
		},
	);
}

export async function listGroups(organizationId: string): Promise<Group[]> {
	return (await api<Group[]>(`${org(organizationId)}/groups`)).data!;
}

export async function createGroup(organizationId: string, name: string): Promise<Group> {
	return (await api<Group>(`${org(organizationId)}/groups`, json('POST', { name }))).data!;
}

export async function renameGroup(
	organizationId: string,
	groupId: string,
	name: string,
): Promise<Group> {
	return (
		await api<Group>(
			`${org(organizationId)}/groups/${encodeURIComponent(groupId)}`,
			json('PATCH', { name }),
		)
	).data!;
}

export async function deleteGroup(organizationId: string, groupId: string) {
	await api(`${org(organizationId)}/groups/${encodeURIComponent(groupId)}`, { method: 'DELETE' });
}

export async function setGroupMembers(
	organizationId: string,
	groupId: string,
	userIds: string[],
): Promise<Group> {
	return (
		await api<Group>(
			`${org(organizationId)}/groups/${encodeURIComponent(groupId)}/members`,
			json('PUT', { user_ids: userIds }),
		)
	).data!;
}

/** All assignments, or only those made at one scope. */
export async function listAssignments(
	organizationId: string,
	scope?: { scopeType: ScopeType; scopeId: string },
): Promise<Assignment[]> {
	const query = scope
		? `?${new URLSearchParams({ scope_type: scope.scopeType, scope_id: scope.scopeId })}`
		: '';
	return (await api<Assignment[]>(`${org(organizationId)}/assignments${query}`)).data!;
}

export async function createAssignment(
	organizationId: string,
	input: AssignmentInput,
): Promise<Assignment> {
	return (await api<Assignment>(`${org(organizationId)}/assignments`, json('POST', input))).data!;
}

export async function deleteAssignment(organizationId: string, assignmentId: string) {
	await api(`${org(organizationId)}/assignments/${encodeURIComponent(assignmentId)}`, {
		method: 'DELETE',
	});
}

export async function getEffectivePermissions(
	organizationId: string,
	userId: string,
): Promise<EffectivePermissions> {
	return (
		await api<EffectivePermissions>(
			`${org(organizationId)}/members/${encodeURIComponent(userId)}/effective-permissions`,
		)
	).data!;
}

/** Catalog entries grouped by resource prefix (`app.env.read` → `app`), in
 *  catalog order, for the role editor. */
export function groupCatalog(catalog: CatalogEntry[]): [string, CatalogEntry[]][] {
	const groups = new Map<string, CatalogEntry[]>();
	for (const entry of catalog) {
		const resource = entry.name.split('.')[0];
		groups.set(resource, [...(groups.get(resource) ?? []), entry]);
	}
	return [...groups];
}

/** Assignments that reach `scope` from above: org-wide ones everywhere, and
 *  project ones on an environment of that project. */
export function inheritedAssignments(
	all: Assignment[],
	scope: { scopeType: ScopeType; scopeId: string; projectId?: string },
): Assignment[] {
	if (scope.scopeType === 'org') return [];
	return all.filter(
		(a) =>
			a.scope_type === 'org' ||
			(scope.scopeType === 'environment' &&
				a.scope_type === 'project' &&
				a.scope_id === scope.projectId),
	);
}
