import type { Edge, Node } from '@xyflow/svelte';
import type { ServerAssignedApplication, ServerPort } from '#lib/clients/server-client.js';
import { environmentApplicationsHref } from '#lib/utils/environment-route.js';

export type AppClusterNodeData = {
	id: string;
	name: string;
	project: string;
	env: string;
	deployType: string;
	templateKind: string | null;
	iconUrl: string | null;
	catalogIconUrl: string | null;
	ingress: boolean;
	ports: string[];
	conflict: boolean;
	conflictPorts: string[];
	href: string;
};

const COL_W = 260;
const ROW_H = 168;
const MAX_COLS = 4;

function sortKey(a: ServerAssignedApplication) {
	return `${a.project_name} ${a.environment_name} ${a.name}`;
}

function portLabel(p: ServerPort) {
	return `${p.host_port}→${p.container_port}`;
}

function collisionKey(p: ServerPort) {
	return `${p.host_ip || '0.0.0.0'}:${p.host_port}/${p.protocol}`;
}

/**
 * Pure, deterministic graph for one server: every assigned application as a
 * cluster node in a grid. Apps that publish the same host
 * ip:port are marked as a conflict and joined by a red edge — that is a Podman
 * deploy failure waiting to happen. No network inspection here; the per-app Map
 * tab carries the container/network detail.
 */
export function buildServerGraph(
	apps: ServerAssignedApplication[],
	ports: ServerPort[] = [],
): { nodes: Node[]; edges: Edge[] } {
	const sorted = [...apps].sort((a, b) => sortKey(a).localeCompare(sortKey(b)));
	if (sorted.length === 0) return { nodes: [], edges: [] };

	const portsByApp = new Map<string, string[]>();
	for (const port of [...ports].sort((a, b) => portLabel(a).localeCompare(portLabel(b)))) {
		const list = portsByApp.get(port.application_id) ?? [];
		if (!list.includes(portLabel(port))) list.push(portLabel(port));
		portsByApp.set(port.application_id, list);
	}

	// host ip:port claimed by more than one application on this server
	const byKey = new Map<string, Set<string>>();
	for (const port of ports) {
		const set = byKey.get(collisionKey(port)) ?? new Set<string>();
		set.add(port.application_id);
		byKey.set(collisionKey(port), set);
	}
	const conflictPortsByApp = new Map<string, Set<string>>();
	const conflictPairs = new Set<string>();
	for (const [key, appIds] of byKey) {
		if (appIds.size < 2) continue;
		const hostPort = key.split(':')[1].split('/')[0];
		const ids = [...appIds].sort();
		for (const id of ids) {
			const set = conflictPortsByApp.get(id) ?? new Set<string>();
			set.add(hostPort);
			conflictPortsByApp.set(id, set);
		}
		for (let i = 0; i < ids.length - 1; i++) {
			conflictPairs.add(`${ids[i]}|${ids[i + 1]}|${hostPort}`);
		}
	}

	const ingressAppIds = new Set(ports.map((p) => p.application_id));
	const cols = Math.min(MAX_COLS, Math.ceil(Math.sqrt(sorted.length)));

	const nodes: Node[] = sorted.map((app, i) => {
		const conflictPorts = [...(conflictPortsByApp.get(app.id) ?? [])].sort();
		return {
			id: `a:${app.id}`,
			type: 'appCluster',
			position: { x: (i % cols) * COL_W, y: Math.floor(i / cols) * ROW_H },
			data: {
				id: app.id,
				name: app.name,
				project: app.project_name,
				env: app.environment_name,
				deployType: app.deploy_type,
				templateKind: app.template_kind,
				iconUrl: app.icon_url,
				catalogIconUrl: app.catalog_icon_url,
				ingress: ingressAppIds.has(app.id),
				ports: portsByApp.get(app.id) ?? [],
				conflict: conflictPorts.length > 0,
				conflictPorts,
				href:
					`${environmentApplicationsHref(app.project_id, app.environment_name)}` +
					`/${encodeURIComponent(app.id)}?tab=map`,
			} satisfies AppClusterNodeData,
		};
	});

	const edges: Edge[] = [];

	for (const pair of [...conflictPairs].sort()) {
		const [a, b, hostPort] = pair.split('|');
		edges.push({
			id: `conflict:${a}|${b}|${hostPort}`,
			source: `a:${a}`,
			target: `a:${b}`,
			label: `:${hostPort}`,
			style: 'stroke: #ef4444; stroke-width: 2; stroke-dasharray: 5 4;',
			data: {
				kind: 'conflict',
				tip: `Both apps publish host port ${hostPort} — one deploy will fail`,
			},
		});
	}

	return { nodes, edges };
}
