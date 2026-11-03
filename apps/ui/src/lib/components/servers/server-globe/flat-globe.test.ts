import { expect, it } from 'vitest';
import { fleetCenter, isOnNearSide } from './flat-globe';

it('points at the only server', () => {
	const [lng, lat] = fleetCenter([[-94.58, 39.1]]);
	expect(lng).toBeCloseTo(-94.58);
	expect(lat).toBeCloseTo(39.1);
});

it('keeps a fleet across the antimeridian on its own side of the world', () => {
	const [lng] = fleetCenter([
		[179, 0],
		[-179, 0],
	]);
	expect(Math.abs(lng)).toBeCloseTo(180);
});

it('falls back to a default when there is nothing to point at, or the fleet cancels out', () => {
	expect(fleetCenter([])).toEqual([-30, 20]);
	expect(
		fleetCenter([
			[0, 0],
			[180, 0],
		]),
	).toEqual([-30, 20]);
});

it('knows which servers the globe is hiding', () => {
	expect(isOnNearSide([8.68, 50.11], [0, 20])).toBe(true);
	expect(isOnNearSide([151.2, -33.87], [0, 20])).toBe(false);
});
