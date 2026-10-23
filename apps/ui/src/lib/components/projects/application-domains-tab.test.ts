import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';

import ApplicationDomainsTab from './application-domains-tab.svelte';
import type { DomainDto } from '#lib/clients/project-client.js';
import type { Server } from '#lib/clients/server-client.js';

const publicEdgeDomain: DomainDto = {
	id: 'domain-1',
	application_id: 'application-1',
	host: 'app.example.com',
	port: 80,
	https: true,
	is_primary: true,
	url: 'https://app.example.com',
	is_local: false,
	required_dns_target: '203.0.113.10',
	ingress_mode: 'public_edge',
	route_health: 'ready',
	route_health_reason: null,
	server_id: null,
	created_at: 1,
	updated_at: 1,
};

const publicEdgeServer: Server = {
	id: 'server-1',
	kind: 'ssh',
	name: 'Edge server',
	ingress_mode: 'public_edge',
	podman_mode: 'rootless',
	state: 'ready',
	public_address: '203.0.113.10',
	ssh_host: 'edge.example.com',
	ssh_port: 22,
	ssh_user: 'litepod',
	host_key_fingerprint: null,
	assigned_application_count: 1,
	organization_id: 'organization-1',
	facts: {
		os: 'linux',
		version: '24.04',
		architecture: 'x86_64',
		podman_version: '5.0',
		caddy_version: '2.8',
		swap_bytes: null,
		swap_recommended: false,
			helper: null,
		capacity: { cpu_cores: 2, memory_bytes: 1, disk_free_bytes: 1 },
	},
	last_error: null,
};

afterEach(() => {
	cleanup();
});

function renderDomain(
	domain: DomainDto,
	onValidateDns = vi.fn().mockResolvedValue({
		status: 'valid' as const,
		resolved_addresses: ['203.0.113.10'],
		expected_addresses: ['203.0.113.10'],
		cdn_provider: null,
		message: null,
	}),
	server: Server = publicEdgeServer,
) {
	const { container } = render(ApplicationDomainsTab, {
		domains: [domain],
		server,
		busy: false,
		refreshing: false,
		configStale: false,
		constraints: { reserved_ports: [], taken_ports: [] },
		onValidateDns,
		onCreate: async () => domain,
		onUpdate: async () => domain,
		onDelete: async () => {},
		onGenerate: async () => {},
		onRefresh: async () => {},
		onDeploy: async () => {},
	});
	return { target: container, onValidateDns };
}

describe('ApplicationDomainsTab DNS validation', () => {
	it('shows remote DNS guidance with root and wildcard records', async () => {
		renderDomain(publicEdgeDomain);
		await fireEvent.click(screen.getByRole('button', { name: 'DNS setup help' }));
		expect(screen.getByText('Connect your domain')).toBeTruthy();
		expect(screen.getAllByText('@')).not.toHaveLength(0);
		expect(screen.getAllByText('*')).not.toHaveLength(0);
		expect(screen.getAllByText('203.0.113.10')).not.toHaveLength(0);
	});

	it('explains private relay DNS routing', async () => {
		renderDomain(
			{ ...publicEdgeDomain, ingress_mode: 'private_relay', required_dns_target: '198.51.100.10' },
			undefined,
			{ ...publicEdgeServer, ingress_mode: 'private_relay', public_address: null },
		);
		await fireEvent.click(screen.getByRole('button', { name: 'DNS setup help' }));
		expect(screen.getAllByText('198.51.100.10')).not.toHaveLength(0);
		expect(screen.getByText(/traffic is relayed privately/i)).toBeTruthy();
	});

	it('waits for a public domain before showing a private relay target', async () => {
		renderDomain(
			{ ...publicEdgeDomain, host: 'localhost', is_local: true, required_dns_target: null },
			undefined,
			{ ...publicEdgeServer, ingress_mode: 'private_relay', public_address: null },
		);
		await fireEvent.click(screen.getByRole('button', { name: 'DNS setup help' }));
		expect(screen.getByText(/Add a public domain first/i)).toBeTruthy();
	});

	it('uses a CNAME record when the public edge target is a hostname', async () => {
		renderDomain(publicEdgeDomain, undefined, {
			...publicEdgeServer,
			public_address: 'edge.example.com',
		});
		await fireEvent.click(screen.getByRole('button', { name: 'DNS setup help' }));
		expect(screen.getAllByText('CNAME')).not.toHaveLength(0);
		expect(screen.getAllByText('edge.example.com')).not.toHaveLength(0);
	});

	it('explains private-only domain limitations', async () => {
		renderDomain(
			{ ...publicEdgeDomain, ingress_mode: 'private_only', required_dns_target: null },
			undefined,
			{ ...publicEdgeServer, ingress_mode: 'private_only', public_address: null },
		);
		await fireEvent.click(screen.getByRole('button', { name: 'DNS setup help' }));
		expect(screen.getByText(/does not permit public domains/i)).toBeTruthy();
	});

	it('hides DNS guidance for the local server', () => {
		renderDomain(publicEdgeDomain, undefined, { ...publicEdgeServer, kind: 'local' });
		expect(screen.queryByRole('button', { name: 'DNS setup help' })).toBeNull();
	});

	it('validates an eligible domain and retains the result in its card', async () => {
		const { target, onValidateDns } = renderDomain(publicEdgeDomain);
		const button = [...target.querySelectorAll('button')].find((candidate) =>
			candidate.textContent?.includes('Check DNS'),
		);
		expect(button).toBeDefined();
		button!.click();
		await waitFor(() => expect(target.textContent).toContain('DNS valid'));
		expect(onValidateDns).toHaveBeenCalledWith(publicEdgeDomain.id);
	});

	it('hides validation for local and generated sslip.io domains', () => {
		const local = renderDomain({ ...publicEdgeDomain, host: 'localhost', is_local: true });
		expect(local.target.textContent).not.toContain('Check DNS');
		const generated = renderDomain({ ...publicEdgeDomain, host: 'app.203-0-113-10.sslip.io' });
		expect(generated.target.textContent).not.toContain('Check DNS');
	});

	it('displays CDN and failure results', async () => {
		const cdn = renderDomain(
			publicEdgeDomain,
			vi.fn().mockResolvedValue({
				status: 'cdn',
				resolved_addresses: ['173.245.48.5'],
				expected_addresses: ['203.0.113.10'],
				cdn_provider: 'Cloudflare',
				message: 'Domain resolves through Cloudflare.',
			}),
		);
		[...cdn.target.querySelectorAll('button')]
			.find((candidate) => candidate.textContent?.includes('Check DNS'))!
			.click();
		await waitFor(() => expect(cdn.target.textContent).toContain('DNS valid · Cloudflare'));

		const invalid = renderDomain(
			publicEdgeDomain,
			vi.fn().mockResolvedValue({
				status: 'invalid',
				resolved_addresses: ['203.0.113.11'],
				expected_addresses: ['203.0.113.10'],
				cdn_provider: null,
				message: 'Domain resolves to 203.0.113.11 but should point to 203.0.113.10.',
			}),
		);
		[...invalid.target.querySelectorAll('button')]
			.find((candidate) => candidate.textContent?.includes('Check DNS'))!
			.click();
		await waitFor(() => expect(invalid.target.textContent).toContain('DNS mismatch'));
		expect(invalid.target.textContent).toContain('should point to 203.0.113.10');
	});
});
