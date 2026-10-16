import { describe, expect, it } from 'vitest';

import { containerSizeBytes, resourcePercent, sortContainerUsage } from './podman-monitoring';

describe('podman monitoring helpers', () => {
	it('normalizes resource percentages and keeps invalid capacity unavailable', () => {
		expect(resourcePercent(512, 1024)).toBe(50);
		expect(resourcePercent(1, 0)).toBeNull();
	});

	it('parses Podman container sizes', () => {
		expect(containerSizeBytes('1.5 MiB')).toBe(1.5 * 1024 ** 2);
		expect(containerSizeBytes(42)).toBe(42);
		expect(containerSizeBytes('unknown')).toBeNull();
	});

	it('orders unavailable metrics after measured consumers', () => {
		const result = sortContainerUsage(
			[
				{ id: 'idle', name: 'idle', cpu: null, memory: null, disk: null },
				{ id: 'busy', name: 'busy', cpu: 12, memory: 1, disk: 2 },
			],
			'cpu',
		);
		expect(result.map((item) => item.name)).toEqual(['busy', 'idle']);
	});
});
