import { render, screen } from '@testing-library/svelte';
import { expect, it } from 'vitest';
import AppRow from './app-row.svelte';
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
		{ host: 'other.example.com', is_primary: false },
		{ host: 'stats.example.com', is_primary: true },
	],
	href: '/projects/p1/environments/e1/applications/a1',
} as unknown as OrganizationApp;

it('renders name, project / env, primary domain and links to the app', () => {
	render(AppRow, { app });
	expect(screen.getByText('umami')).toBeTruthy();
	expect(screen.getByText('tools / production')).toBeTruthy();
	expect(screen.getByText('stats.example.com', { exact: false })).toBeTruthy();
	expect(screen.getByTestId('app-row').getAttribute('href')).toBe(app.href);
});
