import { api } from './base-client';

/** `ok` when Caddy serves what the database expects; anything else needs attention. */
export type RouteDiffStatus = 'ok' | 'missing' | 'mismatched' | 'unroutable' | 'orphaned';

export type RouteSource = 'app' | 'manual';

export type RouteDiffEntry = {
	host: string;
	route_id: string;
	application_id: string | null;
	application_name: string | null;
	source: RouteSource;
	status: RouteDiffStatus;
	expected_dial: string | null;
	live_dial: string | null;
};

export type RouteDiff = {
	entries: RouteDiffEntry[];
	expected: {
		route_id: string;
		application_id: string;
		application_name: string;
		domain_id: string;
		host: string;
		https: boolean;
		dial: string | null;
	}[];
	live: { route_id: string | null; hosts: string[]; dial: string | null }[];
	caddy_admin_url: string;
	needs_reconcile: number;
};

export type ManualRouteKind = 'reverse_proxy' | 'redirect';

export type ManualRoute = {
	id: string;
	server_id: string;
	host: string;
	kind: ManualRouteKind;
	target: string;
	https: boolean;
	redirect_status: number;
	created_at: number;
	updated_at: number;
};

export type ManualRouteInput = {
	host: string;
	kind: ManualRouteKind;
	target: string;
	https: boolean;
	redirect_status?: number;
};

/** `ok` routable and on disk; `missing` routable but no fragment; `pending` no
 * endpoint yet; `orphaned` a fragment with no current domain. */
export type EdgeRouteStatus = 'ok' | 'missing' | 'pending' | 'orphaned';

export type EdgeRouteRow = {
	/** Also the fragment's route_id. Empty host means an orphaned fragment. */
	domain_id: string;
	host: string;
	application_name: string;
	status: EdgeRouteStatus;
};

/** Disk-vs-database route diff for a remote server. */
export type EdgeRoutes = {
	rows: EdgeRouteRow[];
	/** True only for a public-edge server (owns its Caddy, has a route helper to
	 * reconcile/prune). False for private-relay / private-only: rows are
	 * informational, routes live on the central proxy. */
	manages_routes: boolean;
	/** A public-edge server's helper predates `list-routes`; prune is
	 * unavailable until the server is repaired. Always false when
	 * `manages_routes` is false. */
	stale_helper: boolean;
};

export type CaddyStatus = { admin_url: string; active_config: unknown };
export type CaddyfileContent = { path: string; content: string };
export type CaddyHealth = {
	server_id: string;
	kind: 'central' | 'edge';
	healthy: boolean;
	detail: string;
	managed?: boolean;
};

const base = (organizationId: string, serverId: string) =>
	`/api/v1/organizations/${encodeURIComponent(organizationId)}/servers/${encodeURIComponent(
		serverId,
	)}/caddy`;

export async function getRouteDiff(organizationId: string, serverId: string) {
	return (await api<RouteDiff>(`${base(organizationId, serverId)}/routes`)).data!;
}

export async function reconcileRoutes(organizationId: string, serverId: string) {
	return (
		await api<{ pushed: number; failed: number }>(
			`${base(organizationId, serverId)}/routes/reconcile`,
			{ method: 'POST', body: JSON.stringify({}) },
		)
	).data!;
}

export async function pruneRoutes(organizationId: string, serverId: string) {
	return (
		await api<{ dropped: number }>(`${base(organizationId, serverId)}/routes/prune`, {
			method: 'POST',
			body: JSON.stringify({}),
		})
	).data!;
}

export async function listManualRoutes(organizationId: string, serverId: string) {
	return (await api<ManualRoute[]>(`${base(organizationId, serverId)}/manual`)).data ?? [];
}

export async function createManualRoute(
	organizationId: string,
	serverId: string,
	input: ManualRouteInput,
) {
	return (
		await api<ManualRoute>(`${base(organizationId, serverId)}/manual`, {
			method: 'POST',
			body: JSON.stringify(input),
		})
	).data!;
}

export async function updateManualRoute(
	organizationId: string,
	serverId: string,
	id: string,
	input: ManualRouteInput,
) {
	return (
		await api<ManualRoute>(`${base(organizationId, serverId)}/manual/${encodeURIComponent(id)}`, {
			method: 'PUT',
			body: JSON.stringify(input),
		})
	).data!;
}

export async function deleteManualRoute(
	organizationId: string,
	serverId: string,
	id: string,
	confirmation: string,
) {
	return api(`${base(organizationId, serverId)}/manual/${encodeURIComponent(id)}`, {
		method: 'DELETE',
		body: JSON.stringify({ confirmation }),
	});
}

export async function getCaddyfile(organizationId: string, serverId: string) {
	return (await api<CaddyfileContent>(`${base(organizationId, serverId)}/config`)).data!;
}

export async function getCaddyStatus(organizationId: string, serverId: string) {
	return (await api<CaddyStatus>(`${base(organizationId, serverId)}/status`)).data!;
}

export async function getCaddyHealth(organizationId: string, serverId: string) {
	return (await api<CaddyHealth>(`${base(organizationId, serverId)}/health`)).data!;
}

export async function getEdgeRoutes(organizationId: string, serverId: string): Promise<EdgeRoutes> {
	return (
		(await api<EdgeRoutes>(`${base(organizationId, serverId)}/edge-routes`)).data ?? {
			rows: [],
			manages_routes: false,
			stale_helper: false,
		}
	);
}
