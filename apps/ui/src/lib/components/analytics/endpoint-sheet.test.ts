import { render, screen, waitFor } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import EndpointSheet from './endpoint-sheet.svelte';
import type { PathAnalytics } from '#lib/clients/analytics-client.js';

const detail: PathAnalytics = {
	path: '/version.json',
	bucket_seconds: 1800,
	series: [{ ts: 1, requests: 48, ips: 2 }],
	requests: 48,
	errors: 0,
	avg_ms: 5,
	p95_ms: 10,
	ips: 2,
	countries: [{ key: 'DE', requests: 48, ips: 2 }],
	regions: [],
	cities: [{ key: 'Berlin, DE', requests: 24, ips: 1 }],
	clients: [{ key: 'unknown', requests: 48, ips: 2 }],
	agents: [],
	visitors: [
		{
			ip: '1.2.3.4',
			country: 'DE',
			region: null,
			city: 'Berlin',
			client: 'unknown',
			os: 'unknown',
			agent: '',
			requests: 24,
			errors: 0,
			bytes_out: 100,
			first_seen: 1,
			last_seen: 2,
		},
	],
};

describe('EndpointSheet', () => {
	it('loads the endpoint for the range and shows its groupings and IPs', async () => {
		const load = vi.fn().mockResolvedValue(detail);
		render(EndpointSheet, { path: '/version.json', range: '7d', load });
		expect(load).toHaveBeenCalledWith('/version.json', '7d');
		await waitFor(() => expect(screen.getByText('Berlin, DE')).toBeTruthy());
		expect(screen.getByText('1.2.3.4')).toBeTruthy();
		expect(screen.queryByText(/before IP breakdown/)).toBeNull();
	});

	it('says why there are no IPs for requests recorded before the breakdown', async () => {
		const load = vi.fn().mockResolvedValue({
			...detail,
			ips: 0,
			countries: [],
			cities: [],
			clients: [],
			visitors: [],
		});
		render(EndpointSheet, { path: '/version.json', range: '24h', load });
		await waitFor(() => expect(screen.getByText(/before IP breakdown/)).toBeTruthy());
	});

	it('stays closed and loads nothing without an endpoint', () => {
		const load = vi.fn();
		render(EndpointSheet, { path: null, range: '24h', load });
		expect(load).not.toHaveBeenCalled();
	});
});
