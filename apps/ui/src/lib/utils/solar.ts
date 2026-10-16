/**
 * Where the sun is over the earth, right now.
 *
 * NOAA's solar position algorithm, the same one `solar-calculator` implements.
 * It is ~40 lines, so the fleet map carries it rather than a dependency.
 */

const RAD = Math.PI / 180;
const DEG = 180 / Math.PI;

/** Julian centuries since J2000.0, from an epoch-millis timestamp. */
function century(ms: number) {
	return (ms - Date.UTC(2000, 0, 1, 12)) / 315576e7;
}

/** Obliquity of the ecliptic, corrected for nutation, in degrees. */
function obliquity(t: number) {
	const seconds = 21.448 - t * (46.815 + t * (0.00059 - t * 0.001813));
	return 23 + (26 + seconds / 60) / 60 + 0.00256 * Math.cos(RAD * (125.04 - 1934.136 * t));
}

/** Difference between apparent and mean solar time, in minutes. */
function equationOfTime(t: number) {
	const meanLongitude = RAD * ((280.46646 + t * (36000.76983 + t * 0.0003032)) % 360);
	const meanAnomaly = RAD * (357.52911 + t * (35999.05029 - 0.0001537 * t));
	const eccentricity = 0.016708634 - t * (0.000042037 + 0.0000001267 * t);
	const y = Math.tan((RAD * obliquity(t)) / 2) ** 2;
	return (
		4 *
		DEG *
		(y * Math.sin(2 * meanLongitude) -
			2 * eccentricity * Math.sin(meanAnomaly) +
			4 * eccentricity * y * Math.sin(meanAnomaly) * Math.cos(2 * meanLongitude) -
			0.5 * y * y * Math.sin(4 * meanLongitude) -
			1.25 * eccentricity * eccentricity * Math.sin(2 * meanAnomaly))
	);
}

/** Solar declination in degrees: how far north or south the sun stands today. */
function declination(t: number) {
	const meanLongitude = (280.46646 + t * (36000.76983 + t * 0.0003032)) % 360;
	const meanAnomaly = 357.52911 + t * (35999.05029 - 0.0001537 * t);
	const center =
		Math.sin(RAD * meanAnomaly) * (1.914602 - t * (0.004817 + 0.000014 * t)) +
		Math.sin(RAD * 2 * meanAnomaly) * (0.019993 - 0.000101 * t) +
		Math.sin(RAD * 3 * meanAnomaly) * 0.000289;
	const apparent =
		meanLongitude + center - 0.00569 - 0.00478 * Math.sin(RAD * (125.04 - 1934.136 * t));
	return DEG * Math.asin(Math.sin(RAD * obliquity(t)) * Math.sin(RAD * apparent));
}

/** The point the sun is directly overhead, as `[longitude, latitude]` in degrees. */
export function subsolarPoint(ms: number = Date.now()): [number, number] {
	const midnight = new Date(ms).setUTCHours(0, 0, 0, 0);
	const t = century(ms);
	const longitude = ((midnight - ms) / 864e5) * 360 - 180;
	return [longitude - equationOfTime(t) / 4, declination(t)];
}

/**
 * Whether a point is in daylight. The threshold is civil twilight (sun 6 degrees
 * below the horizon), so a server at dusk is not abruptly reported as night.
 */
export function isDaylight(latitude: number, longitude: number, ms: number = Date.now()) {
	const [sunLongitude, sunLatitude] = subsolarPoint(ms);
	const a = RAD * sunLatitude;
	const b = RAD * latitude;
	const delta = RAD * (longitude - sunLongitude);
	return Math.sin(a) * Math.sin(b) + Math.cos(a) * Math.cos(b) * Math.cos(delta) > -0.1045;
}
