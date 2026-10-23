import { render, screen } from '@testing-library/svelte';
import { expect, it } from 'vitest';
import AppsTable from './apps-table.svelte';
import type { OrganizationApp } from '#lib/utils/organization-apps.js';

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
	containers: { running: 1, total: 2 },
	href: '/projects/p1/environments/e1/applications/a1',
} as unknown as OrganizationApp;

it('shows project, primary domain, extra domains and running/total containers', async () => {
	render(AppsTable, { apps: [app] });
	expect(screen.getByText('umami').closest('a')?.getAttribute('href')).toBe(app.href);
	expect(screen.getByText('tools')).toBeTruthy();
	expect(screen.getAllByText('stats.example.com').length).toBeGreaterThan(0);
	expect(screen.getByText('+1')).toBeTruthy();
	expect(screen.getByText('1/2')).toBeTruthy();
});

it('keeps the container cell empty when the application server was unreachable', () => {
	const unreachable = {
		...app,
		application: { ...app.application, id: 'a2', name: 'ghost' },
		containers: null,
		domains: [],
	} as unknown as OrganizationApp;
	render(AppsTable, { apps: [unreachable] });
	expect(screen.getByText('ghost')).toBeTruthy();
	expect(screen.queryByText('0/2')).toBeNull();
});
