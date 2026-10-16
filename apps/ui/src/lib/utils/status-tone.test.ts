import { describe, expect, it } from 'vitest';

import {
	toneBorder,
	toneClass,
	toneDot,
	toneFill,
	utilizationTone,
	type StatusTone,
} from './status-tone';

const TONES: StatusTone[] = ['ok', 'warn', 'danger', 'info', 'neutral'];

/** The palette names Phase 1 is migrating away from. */
const RAW_PALETTE = /\b(emerald|green|amber|yellow|red|sky|blue|purple|indigo|teal|rose)-\d{3}\b/;

describe('status tone helpers', () => {
	it('returns a class for every tone and never a raw Tailwind palette colour', () => {
		for (const tone of TONES) {
			for (const value of [toneClass(tone), toneDot(tone), toneBorder(tone), toneFill(tone)]) {
				expect(value).not.toBe('');
				expect(value).not.toMatch(RAW_PALETTE);
			}
		}
	});

	it('gives each tone a distinct dot colour so states stay tellable apart', () => {
		expect(new Set(TONES.map(toneDot)).size).toBe(TONES.length);
	});

	it('escalates utilization at the shared 75/90 thresholds', () => {
		expect(utilizationTone(0)).toBe('ok');
		expect(utilizationTone(74.9)).toBe('ok');
		expect(utilizationTone(75)).toBe('warn');
		expect(utilizationTone(89.9)).toBe('warn');
		expect(utilizationTone(90)).toBe('danger');
	});

	it('treats missing or non-finite utilization as neutral, not zero', () => {
		expect(utilizationTone(null)).toBe('neutral');
		expect(utilizationTone(Number.NaN)).toBe('neutral');
		expect(utilizationTone(Number.POSITIVE_INFINITY)).toBe('neutral');
	});
});
