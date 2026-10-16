import { describe, expect, it } from 'vitest';
import { formatDuration, formatRelative } from './duration';

describe('formatDuration', () => {
	it('renders human units and keeps the two largest', () => {
		expect(formatDuration(0)).toBe('0s');
		expect(formatDuration(45)).toBe('45s');
		expect(formatDuration(90)).toBe('1m 30s');
		expect(formatDuration(3600)).toBe('1h');
		expect(formatDuration(3661)).toBe('1h 1m');
		expect(formatDuration(90_061)).toBe('1d 1h');
	});
});

describe('formatRelative', () => {
	const base = Date.parse('2026-07-26T12:00:00Z');

	it('counts up from the timestamp', () => {
		expect(formatRelative(null, base)).toBe('never');
		expect(formatRelative('2026-07-26T12:00:00Z', base)).toBe('just now');
		expect(formatRelative('2026-07-26T11:59:48Z', base)).toBe('12s ago');
		expect(formatRelative('2026-07-26T11:56:40Z', base)).toBe('3m 20s ago');
		expect(formatRelative('2026-07-26T09:30:00Z', base)).toBe('2h 30m ago');
	});
});
