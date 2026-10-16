import { expect, it } from 'vitest';
import { relativeTime } from './relative-time';

const now = Date.UTC(2026, 9, 8, 12, 0);

it('picks the largest unit that fits', () => {
	expect(relativeTime(now + 5 * 60_000, now)).toBe('in 5 minutes');
	expect(relativeTime(now - 2 * 3_600_000, now)).toBe('2 hours ago');
	expect(relativeTime(now + 26 * 3_600_000, now)).toBe('tomorrow');
});

it('speaks the UI locale', () => {
	expect(relativeTime(now - 2 * 3_600_000, now, 'es')).toBe('hace 2 horas');
});
