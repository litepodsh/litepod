import { expect, it } from 'vitest';
import { nextTotalLunarEclipse, nextTotalSolarEclipse, sublunarPoint } from './lunar';
import { subsolarPoint } from './solar';

/** Great-circle angle between the sun and moon ground points, in degrees. */
function separation(ms: number) {
	const [sunLng, sunLat] = subsolarPoint(ms).map((v) => (v * Math.PI) / 180);
	const [moonLng, moonLat] = sublunarPoint(ms).map((v) => (v * Math.PI) / 180);
	const cos =
		Math.sin(sunLat) * Math.sin(moonLat) +
		Math.cos(sunLat) * Math.cos(moonLat) * Math.cos(sunLng - moonLng);
	return (Math.acos(cos) * 180) / Math.PI;
}

it('puts the new moon beside the sun', () => {
	expect(separation(Date.UTC(2024, 0, 11, 11, 57))).toBeLessThan(6);
});

it('puts the full moon opposite the sun', () => {
	expect(separation(Date.UTC(2024, 0, 25, 17, 54))).toBeGreaterThan(174);
});

it('keeps the moon within its orbital band and distance', () => {
	for (let day = 0; day < 60; day++) {
		const [lng, lat, distance] = sublunarPoint(Date.UTC(2025, 0, 1) + day * 864e5);
		expect(Math.abs(lat)).toBeLessThan(29.5);
		expect(lng).toBeGreaterThanOrEqual(-180);
		expect(lng).toBeLessThan(180);
		expect(distance).toBeGreaterThan(55);
		expect(distance).toBeLessThan(64);
	}
});

it('finds the 2025-03-14 total lunar eclipse', () => {
	const peak = nextTotalLunarEclipse(Date.UTC(2025, 0, 1));
	expect(Math.abs(peak - Date.UTC(2025, 2, 14, 6, 58))).toBeLessThan(5 * 60e3);
});

it('finds the 2024-04-08 total solar eclipse over Mexico', () => {
	const { peak, lat, lng } = nextTotalSolarEclipse(Date.UTC(2024, 0, 1));
	expect(Math.abs(peak - Date.UTC(2024, 3, 8, 18, 17))).toBeLessThan(5 * 60e3);
	expect(lat).toBeCloseTo(25.3, 0);
	expect(lng).toBeCloseTo(-104.1, 0);
});
