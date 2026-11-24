import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const historyMock = vi.fn();

vi.mock('#lib/clients/container-metrics-client.js', async (importActual) => ({
	...(await importActual<object>()),
	getApplicationMetricsHistory: (...args: unknown[]) => historyMock(...args),
}));

import ApplicationMetricsTab from './application-metrics-tab.svelte';

afterEach(() => {
	cleanup();
	historyMock.mockReset();
});

beforeEach(() => {
	historyMock.mockResolvedValue({
		range: '1h',
		from: 0,
		to: 60_000,
		bucket_seconds: 30,
		containers: [],
		host: [],
	});
});

const point = (ts: number, cpu: number, mem: number) => ({
	ts,
	cpu_avg: cpu,
	cpu_max: cpu * 2,
	mem_avg: mem,
	mem_limit: null,
});

const props = {
	organizationId: 'org-1',
	projectId: 'p-1',
	environmentId: 'e-1',
	applicationId: 'a-1',
};

describe('ApplicationMetricsTab', () => {
	it('loads the stored history, sums containers and reloads on range change', async () => {
		historyMock.mockResolvedValue({
			range: '1h',
			from: 0,
			to: 60_000,
			bucket_seconds: 30,
			containers: [
				{ container_name: 'web', points: [point(0, 1, 100), point(30_000, 3, 100)] },
				{ container_name: 'worker', points: [point(30_000, 2, 50)] },
			],
			host: [],
		});
		const { container } = render(ApplicationMetricsTab, props);
		expect(historyMock).toHaveBeenCalledWith('org-1', 'p-1', 'e-1', 'a-1', '1h');
		// Bucket 2 average, summed across containers: 3 + 2.
		await vi.waitFor(() => expect(screen.getByText('Peak 5.0%')).toBeTruthy());
		expect(container.querySelector('svg[aria-label="CPU, average"]')).toBeTruthy();

		await fireEvent.click(screen.getByRole('tab', { name: '7d' }));
		expect(historyMock).toHaveBeenLastCalledWith('org-1', 'p-1', 'e-1', 'a-1', '7d');
	});

	it('shows the empty and error states of the history', async () => {
		render(ApplicationMetricsTab, props);
		await vi.waitFor(() => expect(screen.getByText(/Nothing stored for this window/)).toBeTruthy());
		cleanup();

		historyMock.mockRejectedValue(new Error('boom'));
		render(ApplicationMetricsTab, props);
		await vi.waitFor(() =>
			expect(screen.getByRole('alert').textContent).toMatch(/Could not load the metrics history/),
		);
	});
});
