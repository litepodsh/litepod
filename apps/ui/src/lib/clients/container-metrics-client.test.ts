import { afterEach, describe, expect, it, vi } from 'vitest';

import {
	formatCpu,
	formatMemory,
	metricContainerIds,
	metricIndex,
	streamApplicationContainerMetrics,
} from './container-metrics-client';

class MockEventSource {
	static instances: MockEventSource[] = [];
	readonly url: string;
	readonly withCredentials: boolean;

	constructor(url: string, options?: EventSourceInit) {
		this.url = url;
		this.withCredentials = options?.withCredentials ?? false;
		MockEventSource.instances.push(this);
	}

	addEventListener = vi.fn();
	removeEventListener = vi.fn();
	close = vi.fn();
}

describe('container metrics client', () => {
	afterEach(() => {
		MockEventSource.instances = [];
		vi.unstubAllGlobals();
	});

	it('creates a credentialed application-scoped metrics stream', () => {
		vi.stubGlobal('EventSource', MockEventSource);
		const stream = streamApplicationContainerMetrics(
			'org 1',
			'project/1',
			'environment',
			'app',
		) as unknown as MockEventSource;

		expect(stream.url).toContain(
			'/organizations/org%201/projects/project%2F1/environments/environment/applications/app/containers/metrics/stream',
		);
		expect(stream.withCredentials).toBe(true);
	});

	it('formats CPU and RAM values for compact status chips', () => {
		expect(formatCpu(12.345)).toBe('12.3%');
		expect(formatMemory(67_108_864, 268_435_456)).toBe('64 MiB / 256 MiB');
		expect(formatMemory(null, null)).toBe('Unavailable');
	});

	it('indexes metrics by both Podman ID and name', () => {
		const metrics = metricIndex({
			observed_at: '2026-07-20T12:00:00Z',
			metrics: [
				{
					id: 'abc123',
					name: 'demo-api',
					state: 'running',
					cpu_percent: 12.5,
					memory_usage_bytes: 67_108_864,
					memory_limit_bytes: null,
					observed_at: '2026-07-20T12:00:00Z',
				},
			],
		});

		expect(metrics.get('abc123')?.name).toBe('demo-api');
		expect(metrics.get('demo-api')?.cpu_percent).toBe(12.5);
	});

	it('selects running standard and Compose container names for stream filters', () => {
		expect(metricContainerIds({ container_name: 'single-api' }, [])).toEqual(['single-api']);
		expect(
			metricContainerIds(null, [
				{ container_name: 'demo-web-1', status: 'running' },
				{ container_name: 'demo-worker-1', status: 'up 2 minutes' },
				{ container_name: 'demo-old-1', status: 'exited' },
			]),
		).toEqual(['demo-web-1', 'demo-worker-1']);
	});
});
