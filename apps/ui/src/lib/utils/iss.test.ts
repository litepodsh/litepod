import { expect, it } from 'vitest';
import { issState, parseTle } from './iss';

const TLE = `ISS (ZARYA)
1 25544U 98067A   26286.82178372  .00006592  00000+0  12831-3 0  9991
2 25544  51.6325  73.0089 0006710 259.3258 100.6975 15.48855783590140`;

it('rejects text without TLE lines', () => {
	expect(parseTle('<html>rate limited</html>')).toBeNull();
});

it('propagates the ISS to a plausible low orbit', () => {
	const satrec = parseTle(TLE)!;
	// 2026-10-13, about the TLE epoch, then a day either side.
	for (const ms of [Date.UTC(2026, 9, 13, 19, 43), Date.UTC(2026, 9, 14, 19), Date.UTC(2026, 9, 12)]) {
		const state = issState(satrec, ms)!;
		expect(state.altitudeKm).toBeGreaterThan(380);
		expect(state.altitudeKm).toBeLessThan(450);
		expect(state.speedKmH).toBeGreaterThan(27000);
		expect(state.speedKmH).toBeLessThan(28000);
		expect(Math.abs(state.lat)).toBeLessThanOrEqual(51.7);
	}
});
