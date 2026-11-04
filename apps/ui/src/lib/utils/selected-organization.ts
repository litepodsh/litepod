const STORAGE_KEY = 'litepod.selected-organization-id';

export function getSelectedOrganizationId(): string | null {
	return typeof window === 'undefined' ? null : window.localStorage.getItem(STORAGE_KEY);
}

export function setSelectedOrganizationId(organizationId: string): void {
	if (typeof window !== 'undefined') {
		window.localStorage.setItem(STORAGE_KEY, organizationId);
	}
}

export function resolveSelectedOrganization<T extends { id: string }>(
	organizations: T[],
): T | null {
	return (
		organizations.find((organization) => organization.id === getSelectedOrganizationId()) ??
		organizations[0] ??
		null
	);
}

/**
 * The same section under another organization, because organization pages read
 * their id from the URL. Anything deeper (a server id) belongs to the old one.
 */
export function organizationPathFor(pathname: string, organizationId: string): string {
	return pathname.replace(
		/^\/organizations\/[^/]+(\/[^/]+)?.*$/,
		(_, section = '') => `/organizations/${encodeURIComponent(organizationId)}${section}`,
	);
}
