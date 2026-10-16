import { expect, it } from 'vitest';
import { resourceLimitGuidance } from './resource-limit-guidance';

it('reserves 1.8 times sampled service usage without blocking the physical maximum', () => {
	expect(
		resourceLimitGuidance(
			{ cpu_cores: 8, memory_bytes: 8 * 1024 ** 3, disk_free_bytes: null },
			{ cpu_cores: 1, memory_bytes: 1024 ** 3 },
		),
	).toEqual({ maxCpu: 8, recommendedCpu: 6.2, maxMemoryMib: 8192, recommendedMemoryMib: 6336 });
	expect(
		resourceLimitGuidance({ cpu_cores: 1, memory_bytes: 1024 ** 3, disk_free_bytes: null }),
	).toEqual({ maxCpu: 1, recommendedCpu: null, maxMemoryMib: 1024, recommendedMemoryMib: null });
	expect(
		resourceLimitGuidance({ cpu_cores: null, memory_bytes: null, disk_free_bytes: null }),
	).toEqual({ maxCpu: null, recommendedCpu: null, maxMemoryMib: null, recommendedMemoryMib: null });
});
