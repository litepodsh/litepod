import { geoDistance } from 'd3-geo';

export type LngLat = [number, number];

/** Where the globe starts with no servers to point at: the Atlantic, both sides in view. */
const DEFAULT_CENTER: LngLat = [-30, 20];

/** The spherical mean of the fleet, so a fleet across the antimeridian does not average to Europe. */
export function fleetCenter(points: LngLat[]): LngLat {
	if (points.length === 0) return DEFAULT_CENTER;
	let x = 0;
	let y = 0;
	let z = 0;
	for (const [lng, lat] of points) {
		const lambda = (lng * Math.PI) / 180;
		const phi = (lat * Math.PI) / 180;
		x += Math.cos(phi) * Math.cos(lambda);
		y += Math.cos(phi) * Math.sin(lambda);
		z += Math.sin(phi);
	}
	if (Math.hypot(x, y, z) < 1e-9) return DEFAULT_CENTER;
	return [(Math.atan2(y, x) * 180) / Math.PI, (Math.atan2(z, Math.hypot(x, y)) * 180) / Math.PI];
}

/** An orthographic globe only shows one hemisphere: anything past 90 degrees is behind it. */
export function isOnNearSide(point: LngLat, center: LngLat): boolean {
	return geoDistance(point, center) < Math.PI / 2;
}
