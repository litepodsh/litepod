/**
 * Where the moon is, and when the next eclipses are, from astronomy-engine
 * (offline, arcsecond-grade). Kept apart from `solar.ts` so the library only
 * loads with the globe chunk, not with every page that asks if it is daytime.
 */
import {
	Body,
	EquatorFromVector,
	GeoVector,
	KM_PER_AU,
	NextGlobalSolarEclipse,
	NextLunarEclipse,
	RotateVector,
	Rotation_EQJ_EQD,
	SearchGlobalSolarEclipse,
	SearchLunarEclipse,
	SiderealTime,
} from 'astronomy-engine';

const EARTH_RADIUS_KM = 6371;

/**
 * The point the moon is directly overhead, as `[longitude, latitude]` in
 * degrees, plus its distance from the earth's centre in earth radii.
 */
export function sublunarPoint(ms: number = Date.now()): [number, number, number] {
	const date = new Date(ms);
	const geocentric = GeoVector(Body.Moon, date, true);
	// True equator of date, so Greenwich sidereal time turns it into a ground point.
	const { ra, dec } = EquatorFromVector(RotateVector(Rotation_EQJ_EQD(date), geocentric));
	const longitude = ((((ra - SiderealTime(date)) * 15) % 360) + 540) % 360 - 180;
	return [longitude, dec, (geocentric.Length() * KM_PER_AU) / EARTH_RADIUS_KM];
}

/** Peak of the next total lunar eclipse after `ms`, in epoch millis. */
export function nextTotalLunarEclipse(ms: number = Date.now()): number {
	let eclipse = SearchLunarEclipse(new Date(ms));
	while (eclipse.kind !== 'total') eclipse = NextLunarEclipse(eclipse.peak);
	return eclipse.peak.date.getTime();
}

/** Peak of the next total solar eclipse after `ms`, with where on earth it peaks. */
export function nextTotalSolarEclipse(ms: number = Date.now()) {
	let eclipse = SearchGlobalSolarEclipse(new Date(ms));
	while (eclipse.kind !== 'total') eclipse = NextGlobalSolarEclipse(eclipse.peak);
	return { peak: eclipse.peak.date.getTime(), lat: eclipse.latitude!, lng: eclipse.longitude! };
}
