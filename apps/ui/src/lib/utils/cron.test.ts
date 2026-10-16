import { expect, it } from 'vitest';
import { describeCron, nextRuns, parseCron } from './cron';
import { i18n } from '#lib/i18n/index.svelte.js';

it('rejects malformed expressions', () => {
	expect(parseCron('0 3 * *')).toBeNull();
	expect(parseCron('61 * * * *')).toBeNull();
	expect(parseCron('*/0 * * * *')).toBeNull();
});

it('describes common schedules', () => {
	expect(describeCron('*/15 * * * *')).toBe('Every 15 minutes');
	expect(describeCron('0 * * * *')).toBe('Hourly at :00');
	expect(describeCron('0 3 * * *')).toBe('Every day at 03:00 UTC');
	expect(describeCron('0 4 * * 1')).toBe('Every Monday at 04:00 UTC');
	expect(describeCron('bad')).toBeNull();
});

it('describes schedules in the UI locale', () => {
	i18n.locale = 'es';
	try {
		expect(describeCron('0 4 * * 1')).toBe('Cada lunes a las 04:00 UTC');
	} finally {
		i18n.locale = 'en';
	}
});

it('computes the next runs in UTC', () => {
	const from = new Date(Date.UTC(2026, 9, 8, 17, 26));
	const runs = nextRuns('0 3 * * *', 2, from).map((d) => d.toISOString());
	expect(runs).toEqual(['2026-10-09T03:00:00.000Z', '2026-10-10T03:00:00.000Z']);
});

it('honours day-of-week', () => {
	const from = new Date(Date.UTC(2026, 9, 8, 0, 0)); // Thursday
	expect(nextRuns('0 4 * * 1', 1, from)[0].toISOString()).toBe('2026-10-12T04:00:00.000Z');
});
