import { cleanup, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';

import ApplicationNetworksTab from './application-networks-tab.svelte';
import type { ApplicationNetworkOverviewDto } from '#lib/clients/project-client.js';

afterEach(cleanup);

const overview: ApplicationNetworkOverviewDto = {
	containers: [
		{
			name: 'app-web',
			service: 'web',
			networks: [{ name: 'app_default', ip_address: '10.89.0.2', aliases: ['web'] }],
		},
	],
	networks: [
		{
			name: 'app_default',
			driver: 'bridge',
			scope: 'private',
			application_member_count: 1,
			external_member_count: 0,
		},
	],
	findings: [],
	observed_at: '2026-08-31T00:00:00Z',
};

function renderTab(
	data: ApplicationNetworkOverviewDto | null = overview,
	error: string | null = null,
) {
	return render(ApplicationNetworksTab, {
		overview: data,
		error,
		onRefresh: vi.fn().mockResolvedValue(undefined),
	});
}

describe('ApplicationNetworksTab', () => {
	it('shows application network membership', () => {
		renderTab();
		expect(screen.getAllByText('app_default')).not.toHaveLength(0);
		expect(screen.getByText(/10\.89\.0\.2/)).toBeTruthy();
		expect(screen.getByText(/Service: web/)).toBeTruthy();
	});

	it('shows aggregate external members without an identity', () => {
		renderTab({
			...overview,
			networks: [{ ...overview.networks[0], scope: 'shared', external_member_count: 3 }],
			findings: [
				{
					code: 'shared_network_external_members',
					severity: 'info',
					message: "Litepod's shared proxy network has 3 container(s) outside this application.",
					container: null,
					network: null,
				},
			],
		});
		expect(screen.getByText(/3 container\(s\) outside this application/)).toBeTruthy();
		expect(screen.queryByText('other-app-db')).toBeNull();
	});

	it('shows an empty or error state', () => {
		renderTab(null, 'Could not load network topology');
		expect(screen.getByText('Could not load network topology')).toBeTruthy();
	});
});
