import { describe, expect, it } from 'vitest';
import { endpointPath, formatLatency, sumSparklines } from './analytics-client';

it('sums the sparklines of the given apps only', () => {
	const sparklines = {
		bucket_seconds: 1800,
		since: 0,
		applications: { a: [1, 2, 3], b: [10, 0, 1], c: [100, 100, 100] },
	};
	expect(sumSparklines(sparklines, ['a', 'b', 'missing'])).toEqual([11, 2, 4]);
	expect(sumSparklines(sparklines, ['missing'])).toEqual([]);
	expect(sumSparklines(null, ['a'])).toEqual([]);
});

it('formats latency in ms or seconds', () => {
	expect(formatLatency(null)).toBe('-');
	expect(formatLatency(42.4)).toBe('42 ms');
	expect(formatLatency(2500)).toBe('2.5 s');
	expect(formatLatency(12_000)).toBe('12 s');
});

describe('endpointPath', () => {
	it('turns what was typed or pasted into the stored path', () => {
		expect(endpointPath(' /version.json ')).toBe('/version.json');
		expect(endpointPath('version.json')).toBe('/version.json');
		expect(endpointPath('/api/items?page=2#top')).toBe('/api/items');
		expect(endpointPath('https://litepod.sh/version.json')).toBe('/version.json');
	});

	it('returns null when there is no path', () => {
		expect(endpointPath('   ')).toBeNull();
		expect(endpointPath('http://')).toBeNull();
	});
});
