import { describe, expect, it } from 'vitest';

import { buildTopologyGraph } from './graph-layout';
import type { ApplicationNetworkOverviewDto } from '#lib/clients/project-client.js';

function overview(
	patch: Partial<ApplicationNetworkOverviewDto> = {},
): ApplicationNetworkOverviewDto {
	return {
		containers: [
			{
				name: 'app-web',
				service: 'web',
				networks: [{ name: 'app_default', ip_address: '10.89.0.2', aliases: ['web'] }],
			},
			{
				name: 'app-worker',
				service: 'worker',
				networks: [{ name: 'app_default', ip_address: '10.89.0.3', aliases: [] }],
			},
			{
				name: 'app-db',
				service: 'db',
				networks: [{ name: 'app_default', ip_address: '10.89.0.4', aliases: ['db'] }],
			},
		],
		networks: [
			{
				name: 'app_default',
				driver: 'bridge',
				scope: 'private',
				application_member_count: 3,
				external_member_count: 0,
			},
		],
		findings: [],
		observed_at: '2026-08-31T00:00:00Z',
		...patch,
	};
}

describe('buildTopologyGraph', () => {
	it('returns an empty graph for no overview', () => {
		expect(buildTopologyGraph(null)).toEqual({ nodes: [], edges: [] });
	});

	it('builds one node per container and network, one edge per attachment', () => {
		const { nodes, edges } = buildTopologyGraph(overview());
		expect(nodes.filter((n) => n.type === 'containerNode')).toHaveLength(3);
		expect(nodes.filter((n) => n.type === 'networkNode')).toHaveLength(1);
		const attach = edges.filter((e) => (e.data as { kind?: string })?.kind === 'attach');
		expect(attach).toHaveLength(3);
		expect(attach.every((e) => e.source.startsWith('c:') && e.target.startsWith('n:'))).toBe(true);
	});

	it('is deterministic and sorted by name', () => {
		const a = buildTopologyGraph(overview());
		const b = buildTopologyGraph(overview());
		expect(a).toEqual(b);
		expect(a.nodes.filter((n) => n.type === 'containerNode').map((n) => n.id)).toEqual([
			'c:app-db',
			'c:app-web',
			'c:app-worker',
		]);
	});

	it('leaves private-network attachment edges neutral', () => {
		const { edges } = buildTopologyGraph(overview());
		const attach = edges.filter((e) => (e.data as { kind?: string })?.kind === 'attach');
		expect(attach.every((e) => (e.data as { tone: string }).tone === 'neutral')).toBe(true);
		expect(attach[0].style).toContain('#8a8f98');
	});

	it('tints shared-network edges amber when a proxy-membership finding is present', () => {
		const data = overview({
			containers: [
				{
					name: 'app-web',
					service: 'web',
					networks: [{ name: 'litepod-proxy', ip_address: '10.89.1.2', aliases: [] }],
				},
			],
			networks: [
				{
					name: 'litepod-proxy',
					driver: 'bridge',
					scope: 'shared',
					application_member_count: 1,
					external_member_count: 7,
				},
			],
			findings: [
				{
					code: 'unexpected_proxy_membership',
					severity: 'warning',
					message: 'db is on the shared network without a route.',
					container: 'app-db',
					network: 'litepod-proxy',
				},
			],
		});
		const { edges } = buildTopologyGraph(data);
		expect(edges).toHaveLength(1);
		expect((edges[0].data as { tone: string }).tone).toBe('warn');
		expect(edges[0].style).toContain('#f59e0b');
	});

	it('tints shared-network edges red for a remote_shared_network finding', () => {
		const data = overview({
			containers: [
				{
					name: 'fern-cron',
					service: 'cron',
					networks: [{ name: 'litepod-proxy', ip_address: '10.89.1.9', aliases: [] }],
				},
			],
			networks: [
				{
					name: 'litepod-proxy',
					driver: 'bridge',
					scope: 'shared',
					application_member_count: 1,
					external_member_count: 8,
				},
			],
			findings: [
				{
					code: 'remote_shared_network',
					severity: 'warning',
					message: 'Remote app on the shared ingress network.',
					container: 'fern-cron',
					network: 'litepod-proxy',
				},
			],
		});
		const { edges } = buildTopologyGraph(data);
		expect(edges[0].style).toContain('#ef4444');
	});

	it('links adjacent containers that share a private network', () => {
		const { edges } = buildTopologyGraph(overview());
		const peer = edges.filter((e) => (e.data as { tone?: string })?.tone === 'peer');
		// app-db <-> app-web <-> app-worker, all on app_default
		expect(peer).toHaveLength(2);
		expect(peer.every((e) => e.source.startsWith('c:') && e.target.startsWith('c:'))).toBe(true);
		expect(peer[0].label).toBe('resolves by hostname');
		expect((peer[0].data as { via: string[] }).via).toEqual(['app_default']);
	});

	it('overrides a peer edge with a measured probe result', () => {
		const graph = buildTopologyGraph(overview(), {}, [
			{
				from: 'app-db',
				to: 'app-web',
				from_service: 'db',
				to_service: 'web',
				port: 5432,
				reason: 'name_not_resolved',
			},
		]);
		const peer = graph.edges.find((e) => (e.data as { tone?: string })?.tone === 'peer');
		expect(peer?.label).toBe('✗ no DNS');
		expect(peer?.style).toContain('#ef4444');
		expect((peer?.data as { probe: string }).probe).toBe('name_not_resolved');
	});

	it('leaves peer edges inferred when no probe is given', () => {
		const peer = buildTopologyGraph(overview()).edges.find(
			(e) => (e.data as { tone?: string })?.tone === 'peer',
		);
		expect(peer?.label).toBe('resolves by hostname');
		expect((peer?.data as { probe: unknown }).probe).toBeNull();
	});

	it('does not link containers with no common private network', () => {
		const data = overview({
			containers: [
				{ name: 'a', service: 'a', networks: [{ name: 'net-a', ip_address: null, aliases: [] }] },
				{ name: 'b', service: 'b', networks: [{ name: 'net-b', ip_address: null, aliases: [] }] },
			],
			networks: [
				{
					name: 'net-a',
					driver: 'bridge',
					scope: 'private',
					application_member_count: 1,
					external_member_count: 0,
				},
				{
					name: 'net-b',
					driver: 'bridge',
					scope: 'private',
					application_member_count: 1,
					external_member_count: 0,
				},
			],
		});
		const peer = buildTopologyGraph(data).edges.filter(
			(e) => (e.data as { tone?: string })?.tone === 'peer',
		);
		expect(peer).toHaveLength(0);
	});

	it('maps container status to an LED state', () => {
		const graph = buildTopologyGraph(overview(), {
			'app-web': 'running',
			'app-worker': 'exited (0)',
			'app-db': 'restarting',
		});
		const led = (id: string) => (graph.nodes.find((n) => n.id === id)?.data as { led: string }).led;
		expect(led('c:app-web')).toBe('running');
		expect(led('c:app-worker')).toBe('stopped');
		expect(led('c:app-db')).toBe('degraded');
	});

	it('defaults the LED to unknown when no status is given', () => {
		const graph = buildTopologyGraph(overview());
		expect((graph.nodes[0].data as { led: string }).led).toBe('unknown');
	});

	it('skips an attachment whose network is not in the network list', () => {
		const data = overview({
			containers: [
				{
					name: 'app-web',
					service: 'web',
					networks: [{ name: 'ghost_net', ip_address: null, aliases: [] }],
				},
			],
			networks: [],
		});
		const { nodes, edges } = buildTopologyGraph(data);
		expect(nodes.filter((n) => n.type === 'networkNode')).toHaveLength(0);
		expect(edges).toHaveLength(0);
	});
});
