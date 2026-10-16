import { cleanup, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';

import ApplicationMapTab from './application-map-tab.svelte';
import type { ApplicationNetworkOverviewDto } from '#lib/clients/project-client.js';

afterEach(cleanup);

// The canvas itself (SvelteFlow) needs layout APIs jsdom lacks, so these cover
// the non-canvas states. Graph construction is tested in
// src/lib/utils/graph-layout.test.ts.

function renderTab(overview: ApplicationNetworkOverviewDto | null, error: string | null = null) {
	return render(ApplicationMapTab, {
		overview,
		organizationId: 'org-1',
		serverId: 'local',
		loading: false,
		error,
		onRefresh: vi.fn().mockResolvedValue(undefined),
	});
}

describe('ApplicationMapTab', () => {
	it('shows the error state', () => {
		renderTab(null, 'Could not load network topology');
		expect(screen.getByText('Could not load network topology')).toBeTruthy();
	});

	it('shows the empty state when nothing is deployed', () => {
		renderTab({ containers: [], networks: [], findings: [], observed_at: '2026-08-31T00:00:00Z' });
		expect(screen.getByText(/No deployed containers or networks/)).toBeTruthy();
	});
});
