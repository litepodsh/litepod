import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { flushSync } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

class FakeStream extends EventTarget {
	close = vi.fn();
}
let stream: FakeStream;
const streamMock = vi.fn(() => (stream = new FakeStream()));
const historyMock = vi.fn();

vi.mock('#lib/clients/container-metrics-client.js', async (importActual) => ({
	...(await importActual<object>()),
	streamApplicationContainerMetrics: (...args: unknown[]) => streamMock(...(args as [])),
	getApplicationMetricsHistory: (...args: unknown[]) => historyMock(...args),
}));

const analyticsMock = vi.fn();
vi.mock('#lib/clients/analytics-client.js', async (importActual) => ({
	...(await importActual<object>()),
	getApplicationAnalytics: (...args: unknown[]) => analyticsMock(...args),
}));

import ApplicationMetricsTab from './application-metrics-tab.svelte';

afterEach(() => {
	cleanup();
	streamMock.mockClear();
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
	analyticsMock.mockResolvedValue({
		series: [
			{ ts: 0, requests: 1200 },
			{ ts: 1, requests: 300 },
		],
	});
});

const point = (ts: number, cpu: number, mem: number) => ({
	ts,
	cpu_avg: cpu,
	cpu_max: cpu * 2,
	mem_avg: mem,
	mem_limit: null,
});

const props = (containerNames: string[]) => ({
	organizationId: 'org-1',
	projectId: 'p-1',
	environmentId: 'e-1',
	applicationId: 'a-1',
	containerNames,
});

function emit(type: string, data?: unknown) {
	stream.dispatchEvent(
		data === undefined ? new Event(type) : new MessageEvent(type, { data: JSON.stringify(data) }),
	);
	flushSync();
}

const metric = (name: string, cpu: number, mem: number) => ({
	id: `id-${name}`,
	name,
	state: 'running',
	cpu_percent: cpu,
	memory_usage_bytes: mem,
	memory_limit_bytes: 1024 * 1024 * 1024,
	observed_at: '2026-10-08T00:00:00Z',
});

describe('ApplicationMetricsTab', () => {
	it('shows live CPU and memory summed across the running containers', async () => {
		const { container } = render(ApplicationMetricsTab, props(['web', 'worker']));
		expect(streamMock).toHaveBeenCalledWith('org-1', 'p-1', 'e-1', 'a-1');
		expect(screen.queryByTestId('metrics-cpu')).toBeNull();

		emit('metrics', {
			observed_at: '2026-10-08T00:00:00Z',
			metrics: [
				metric('web', 1.5, 256 * 1024 * 1024),
				metric('worker', 2, 256 * 1024 * 1024),
				metric('other-app', 50, 1024),
			],
		});

		expect(screen.getByTestId('metrics-cpu').textContent).toContain('3.5%');
		expect(screen.getByTestId('metrics-memory').textContent).toContain('512 MiB / 2 GiB');
		// Real 24h requests from the analytics endpoint, with the total.
		await vi.waitFor(() =>
			expect(container.querySelector('svg[aria-label="Requests"]')).toBeTruthy(),
		);
		expect(analyticsMock).toHaveBeenCalledWith('org-1', 'p-1', 'e-1', 'a-1', '24h');
		expect(screen.getByText('1.5K')).toBeTruthy();
	});

	it('shows the empty state and opens no stream without running containers', () => {
		render(ApplicationMetricsTab, props([]));
		expect(streamMock).not.toHaveBeenCalled();
		expect(screen.getByText(/No running containers/)).toBeTruthy();
	});

	it('shows a stream error and closes the stream on destroy', () => {
		const { unmount } = render(ApplicationMetricsTab, props(['web']));
		emit('error');
		expect(screen.getByRole('status').textContent).toMatch(/unavailable/);
		unmount();
		expect(stream.close).toHaveBeenCalled();
	});

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
		const { container } = render(ApplicationMetricsTab, props(['web']));
		expect(historyMock).toHaveBeenCalledWith('org-1', 'p-1', 'e-1', 'a-1', '1h');
		// Bucket 2 average, summed across containers: 3 + 2.
		await vi.waitFor(() => expect(screen.getByText('Peak 5.0%')).toBeTruthy());
		expect(container.querySelector('svg[aria-label="CPU, average"]')).toBeTruthy();

		await fireEvent.click(screen.getByRole('tab', { name: '7d' }));
		expect(historyMock).toHaveBeenLastCalledWith('org-1', 'p-1', 'e-1', 'a-1', '7d');
	});

	it('shows the empty and error states of the history', async () => {
		render(ApplicationMetricsTab, props(['web']));
		await vi.waitFor(() => expect(screen.getByText(/Nothing stored for this window/)).toBeTruthy());
		cleanup();

		historyMock.mockRejectedValue(new Error('boom'));
		render(ApplicationMetricsTab, props(['web']));
		await vi.waitFor(() =>
			expect(screen.getByRole('alert').textContent).toMatch(/Could not load the metrics history/),
		);
	});
});
