import { render, screen } from '@testing-library/svelte';
import { expect, it, vi } from 'vitest';
import AppsTable from './apps-table.svelte';
import type { OrganizationApp } from '#lib/utils/organization-apps.js';

vi.mock('#lib/clients/project-client.js', () => ({
	listApplicationLogTargets: vi.fn(async () => [
		{ container_name: 'web', service: 'web', status: 'running' },
		{ container_name: 'db', service: 'db', status: 'exited' },
	]),
}));

const app = {
	application: {
		id: 'a1',
		name: 'umami',
		template_kind: null,
		catalog_icon_url: null,
		icon_url: null,
		updated_at: 1,
	},
	project: { id: 'p1', name: 'tools' },
	environment: { id: 'e1', name: 'production', is_production: true },
	status: 'running',
	domains: [
		{
			id: 'd1',
			host: 'other.example.com',
			is_primary: false,
			url: 'https://other.example.com',
			port: 80,
		},
		{
			id: 'd2',
			host: 'stats.example.com',
			is_primary: true,
			url: 'https://stats.example.com',
			port: 80,
		},
	],
	deployedAt: Date.now() - 60_000,
	href: '/projects/p1/environments/e1/applications/a1',
} as unknown as OrganizationApp;

it('shows project, primary domain, extra domains and running/total containers', async () => {
	render(AppsTable, { apps: [app], organizationId: 'o1' });
	expect(screen.getByText('umami').closest('a')?.getAttribute('href')).toBe(app.href);
	expect(screen.getByText('tools')).toBeTruthy();
	expect(screen.getAllByText('stats.example.com').length).toBeGreaterThan(0);
	expect(screen.getByText('+1')).toBeTruthy();
	expect(await screen.findByText('1/2')).toBeTruthy();
});
