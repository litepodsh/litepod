import type { Edge, Node } from '@xyflow/svelte';
import type {
	ApplicationNetworkOverviewDto,
	NetworkProbePairDto,
} from '#lib/clients/project-client.js';

export type EdgeTone = 'neutral' | 'ok' | 'warn' | 'crit';

// Literal hex rather than CSS vars: Svelte Flow renders edges into its own SVG
// layer where our token cascade is awkward to reach, and these read fine on the
// dark canvas.
const TONE_STROKE: Record<EdgeTone, string> = {
	neutral: '#8a8f98',
	ok: '#10b981',
	warn: '#f59e0b',
	crit: '#ef4444',
};
const PEER_STROKE = '#4cc38a';

export function formatRate(bytesPerSec: number): string {
	if (bytesPerSec < 1000) return `${Math.round(bytesPerSec)} B/s`;
	if (bytesPerSec < 1_000_000) return `${(bytesPerSec / 1000).toFixed(1)} kB/s`;
	return `${(bytesPerSec / 1_000_000).toFixed(1)} MB/s`;
}

// A measured reachability result overrides the inferred green peer edge.
function probeStyle(
	reason: string,
	port: number | null,
): {
	label: string;
	stroke: string;
	dashed: boolean;
} {
	const p = port ? ` :${port}` : '';
	switch (reason) {
		case 'reachable':
			return { label: `✓ reachable${p}`, stroke: '#10b981', dashed: false };
		case 'resolved':
			return { label: '✓ resolves', stroke: '#10b981', dashed: false };
		case 'connection_refused':
			return { label: `✗ refused${p}`, stroke: '#ef4444', dashed: true };
		case 'unreachable':
			return { label: `✗ unreachable${p}`, stroke: '#ef4444', dashed: true };
		case 'name_not_resolved':
			return { label: '✗ no DNS', stroke: '#ef4444', dashed: true };
		case 'no_tools':
			return { label: '— cannot probe', stroke: '#8a8f98', dashed: true };
		default:
			return { label: '— probe failed', stroke: '#8a8f98', dashed: true };
	}
}

export type ContainerLed = 'running' | 'degraded' | 'error' | 'stopped' | 'unknown';

export type ContainerNodeData = {
	label: string;
	service: string | null;
	ips: string[];
	aliases: string[];
	led: ContainerLed;
};

function ledFor(status: string | undefined): ContainerLed {
	if (!status) return 'unknown';
	const s = status.toLowerCase();
	if (s === 'running' || s === 'up' || s.startsWith('running') || s.startsWith('up'))
		return 'running';
	if (s.includes('dead') || s.includes('oom')) return 'error';
	if (
		s.includes('restart') ||
		s.includes('paus') ||
		s.includes('created') ||
		s.includes('starting')
	)
		return 'degraded';
	if (s.includes('exit') || s.includes('stop') || s.includes('down')) return 'stopped';
	return 'unknown';
}

export type NetworkNodeData = {
	label: string;
	driver: string | null;
	scope: 'private' | 'shared';
	appCount: number;
	extCount: number;
};

const COL_GAP = 500;
const ROW_GAP = 190;

function byName<T extends { name: string }>(a: T, b: T) {
	return a.name.localeCompare(b.name);
}

/**
 * Pure, deterministic graph model for the application topology canvas:
 * - container nodes in a left column, the networks they touch in a right column;
 * - one edge per attachment (container -> network);
 * - a "peer" edge between two adjacent containers that share a private network,
 *   i.e. they can resolve each other by hostname.
 * Sorted throughout so the layout and its test never flicker.
 */
export function buildTopologyGraph(
	overview: ApplicationNetworkOverviewDto | null,
	statuses: Record<string, string> = {},
	probes: NetworkProbePairDto[] = [],
	rates: Record<string, number> = {},
): {
	nodes: Node[];
	edges: Edge[];
} {
	if (!overview) return { nodes: [], edges: [] };

	const probeByPair = new Map<string, NetworkProbePairDto>();
	for (const probe of probes) {
		probeByPair.set(`${probe.from} ${probe.to}`, probe);
		probeByPair.set(`${probe.to} ${probe.from}`, probe);
	}

	const containers = [...overview.containers].sort(byName);
	const networks = [...overview.networks].sort(byName);
	const networkByName = new Map(networks.map((n) => [n.name, n]));

	// ponytail: findings carry no container/service reference, so a shared-network
	// warning tints every edge into that network, not just the offending one.
	// Upgrade path: add a `subject` field to ApplicationNetworkFindingDto.
	const codes = new Set(overview.findings.map((f) => f.code));
	const sharedTone: EdgeTone = codes.has('remote_shared_network')
		? 'crit'
		: codes.has('unexpected_proxy_membership')
			? 'warn'
			: 'ok';

	const nodes: Node[] = [];

	containers.forEach((container, i) => {
		const ips = container.networks.map((a) => a.ip_address).filter((ip): ip is string => !!ip);
		const aliases = [...new Set(container.networks.flatMap((a) => a.aliases))].sort();
		nodes.push({
			id: `c:${container.name}`,
			type: 'containerNode',
			position: { x: 0, y: i * ROW_GAP },
			data: {
				label: container.name,
				service: container.service,
				ips,
				aliases,
				led: ledFor(statuses[container.name]),
			} satisfies ContainerNodeData,
		});
	});

	networks.forEach((network, i) => {
		nodes.push({
			id: `n:${network.name}`,
			type: 'networkNode',
			position: { x: COL_GAP, y: i * ROW_GAP + ROW_GAP / 2 },
			data: {
				label: network.name,
				driver: network.driver,
				scope: network.scope,
				appCount: network.application_member_count,
				extCount: network.external_member_count,
			} satisfies NetworkNodeData,
		});
	});

	const edges: Edge[] = [];

	// container -> network attachments
	containers.forEach((container) => {
		[...container.networks].sort(byName).forEach((attachment) => {
			const network = networkByName.get(attachment.name);
			if (!network) return;
			const tone: EdgeTone = network.scope === 'shared' ? sharedTone : 'neutral';
			const stroke = TONE_STROKE[tone];
			// Always a number and always shown: an idle link reads "0 B/s", it
			// never blanks out. Live updates patch this in place (see the tab).
			const rate = typeof rates[container.name] === 'number' ? rates[container.name] : 0;
			const tip =
				`${container.name} on ${attachment.name}` +
				(attachment.ip_address ? ` · ${attachment.ip_address}` : '') +
				(network.scope === 'shared' ? ' · shared network' : '') +
				` · ${formatRate(rate)}`;
			edges.push({
				id: `c:${container.name}->n:${attachment.name}`,
				type: 'packet',
				source: `c:${container.name}`,
				sourceHandle: 'net',
				target: `n:${attachment.name}`,
				style: `stroke: ${stroke}; stroke-width: ${tone === 'neutral' ? 1.5 : 2};`,
				label: formatRate(rate),
				data: { kind: 'attach', tone, stroke, tip, rate, container: container.name },
			});
		});
	});

	// peer reachability: adjacent containers sharing a private network can call
	// each other by hostname. A chain (n-1 edges), not a mesh, keeps it legible.
	const privateNames = (c: (typeof containers)[number]) =>
		new Set(
			c.networks.map((a) => a.name).filter((name) => networkByName.get(name)?.scope === 'private'),
		);
	for (let i = 0; i < containers.length - 1; i++) {
		const a = privateNames(containers[i]);
		const shared = [...privateNames(containers[i + 1])].filter((name) => a.has(name)).sort();
		if (shared.length === 0) continue;
		const probe = probeByPair.get(`${containers[i].name} ${containers[i + 1].name}`);
		const measured = probe ? probeStyle(probe.reason, probe.port) : null;
		edges.push({
			id: `c:${containers[i].name}<->c:${containers[i + 1].name}`,
			type: 'packet',
			source: `c:${containers[i].name}`,
			sourceHandle: 'peer-out',
			target: `c:${containers[i + 1].name}`,
			targetHandle: 'peer-in',
			style: measured
				? `stroke: ${measured.stroke}; stroke-width: 2;${measured.dashed ? ' stroke-dasharray: 5 4;' : ''}`
				: `stroke: ${PEER_STROKE}; stroke-width: 2;`,
			label: measured ? measured.label : 'resolves by hostname',
			data: {
				kind: 'peer',
				tone: 'peer',
				stroke: measured ? measured.stroke : PEER_STROKE,
				via: shared,
				probe: probe?.reason ?? null,
				tip: probe
					? `${containers[i].name} → ${containers[i + 1].name}: ${probe.reason.replace(/_/g, ' ')}`
					: `${containers[i].name} and ${containers[i + 1].name} share ${shared[0]} — each reaches the other at its service name`,
			},
		});
	}

	return { nodes, edges };
}
