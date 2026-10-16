import { page } from '$app/state';
import {
	can,
	getMyPermissions,
	type PermissionSet,
	type PermissionTarget,
} from '#lib/clients/permission-client.js';

/** The signed-in user's permissions in the selected organization. Loaded by the
 *  `(app)` layout whenever the organization changes. Cosmetic only — it hides
 *  actions the API would refuse; the API stays the authority. */
export const permissionsState = $state<{
	organizationId: string | null;
	set: PermissionSet | null;
}>({ organizationId: null, set: null });

let requestVersion = 0;

export async function loadPermissions(organizationId: string | null) {
	const version = ++requestVersion;
	permissionsState.organizationId = organizationId;
	permissionsState.set = null;
	if (!organizationId) return;
	try {
		const set = await getMyPermissions(organizationId);
		if (version === requestVersion) permissionsState.set = set;
	} catch {
		// No set: `allowed` falls back to showing actions; the API still decides.
	}
}

/** `can()` against the loaded set. Without a set (loading, or the request
 *  failed) it answers `true`, so actions don't flash away for the common case;
 *  the API refuses if that was wrong. */
export function allowed(permission: string, target: PermissionTarget = {}): boolean {
	if (!permissionsState.set) return true;
	return can(permissionsState.set, permission, target);
}

/** `allowed()` against the project and environment named in the current route,
 *  for components deep under `/projects/[project_id]/environments/[environment_id]`. */
export function allowedHere(permission: string): boolean {
	return allowed(permission, {
		projectId: page.params.project_id,
		environmentId: page.params.environment_id,
	});
}
