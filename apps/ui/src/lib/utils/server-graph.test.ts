import { describe, expect, it } from 'vitest';

import { buildServerGraph } from './server-graph';
import type { ServerAssignedApplication, ServerPort } from '#lib/clients/server-client.js';

function app(
	patch: Partial<ServerAssignedApplication> & { id: string; name: string },
): ServerAssignedApplication {
	return {
		environment_id: 'env-1',
		environment_name: 'production',
		project_id: 'proj-1',
		project_name: 'acme',
		deploy_type: 'compose',
		template_kind: null,
		icon_url: null,
		catalog_icon_url: null,
		...patch,
	};
}

function port(
	patch: Partial<ServerPort> & { application_id: string; host_port: string },
): ServerPort {
	return {
		application_name: 'x',
		service: null,
		host_ip: null,
		container_port: '80',
		protocol: 'tcp',
		source: 'compose',
		...patch,
	};
}

describe('buildServerGraph', () => {
	it('returns an empty graph for no apps', () => {
		expect(buildServerGraph([])).toEqual({ nodes: [], edges: [] });
	});

	it('builds one app-cluster node per application, sorted, with a Map link', () => {
		const { nodes, edges } = buildServerGraph([
			app({ id: 'b', name: 'beta' }),
			app({ id: 'a', name: 'alpha' }),
		]);
		expect(nodes.map((n) => n.id)).toEqual(['a:a', 'a:b']);
		expect(nodes.every((n) => n.type === 'appCluster')).toBe(true);
		expect((nodes[0].data as { href: string }).href).toContain('?tab=map');
		expect(edges).toHaveLength(0);
	});

	it('marks apps that publish a host port without a proxy node', () => {
		const { nodes, edges } = buildServerGraph(
			[app({ id: 'web', name: 'web' }), app({ id: 'db', name: 'db' })],
			[port({ application_id: 'web', host_port: '8080', container_port: '80' })],
		);
		expect(nodes.some((n) => n.id === 'proxy')).toBe(false);
		expect(edges).toHaveLength(0);
		expect((nodes.find((n) => n.id === 'a:web')!.data as { ingress: boolean }).ingress).toBe(true);
		expect((nodes.find((n) => n.id === 'a:web')!.data as { ports: string[] }).ports).toEqual([
			'8080→80',
		]);
		expect((nodes.find((n) => n.id === 'a:db')!.data as { ingress: boolean }).ingress).toBe(false);
	});

	it('marks a host-port collision on both apps and joins them with an edge', () => {
		const { nodes, edges } = buildServerGraph(
			[app({ id: 'a', name: 'a' }), app({ id: 'b', name: 'b' })],
			[
				port({ application_id: 'a', host_port: '8080' }),
				port({ application_id: 'b', host_port: '8080' }),
			],
		);
		expect((nodes.find((n) => n.id === 'a:a')!.data as { conflict: boolean }).conflict).toBe(true);
		expect((nodes.find((n) => n.id === 'a:b')!.data as { conflict: boolean }).conflict).toBe(true);
		const conflictEdge = edges.find((e) => (e.data as { kind?: string })?.kind === 'conflict');
		expect(conflictEdge).toMatchObject({ source: 'a:a', target: 'a:b', label: ':8080' });
	});

	it('does not flag a collision when the host ip differs', () => {
		const { nodes, edges } = buildServerGraph(
			[app({ id: 'a', name: 'a' }), app({ id: 'b', name: 'b' })],
			[
				port({ application_id: 'a', host_port: '8080', host_ip: '127.0.0.1' }),
				port({ application_id: 'b', host_port: '8080', host_ip: '10.0.0.5' }),
			],
		);
		expect(nodes.every((n) => !(n.data as { conflict?: boolean }).conflict)).toBe(true);
		expect(edges.some((e) => (e.data as { kind?: string })?.kind === 'conflict')).toBe(false);
	});

	it('omits the proxy node when no app has ingress', () => {
		const { nodes, edges } = buildServerGraph([app({ id: 'a', name: 'a' })], []);
		expect(nodes.some((n) => n.id === 'proxy')).toBe(false);
		expect(edges).toHaveLength(0);
	});
});
