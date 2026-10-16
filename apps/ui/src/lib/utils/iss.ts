/**
 * The ISS, propagated in the browser with SGP4 from CelesTrak's TLE. Only the
 * globe chunk imports this, so satellite.js stays out of every other page.
 */
import {
	degreesLat,
	degreesLong,
	eciToGeodetic,
	gstime,
	propagate,
	twoline2satrec,
	type SatRec,
} from 'satellite.js';

const TLE_URL = 'https://celestrak.org/NORAD/elements/gp.php?CATNR=25544&FORMAT=tle';
const CACHE_KEY = 'lp.iss-tle';
// CelesTrak refreshes elements every ~2 hours and blocks clients that ask more often.
const CACHE_MS = 2 * 3600e3;

export type IssState = { lat: number; lng: number; altitudeKm: number; speedKmH: number };

function readCache(): { at: number; lines: string[] } | null {
	try {
		return JSON.parse(localStorage.getItem(CACHE_KEY) ?? 'null');
	} catch {
		return null;
	}
}

/** Two TLE lines parsed into SGP4 elements, or `null` when they are malformed. */
export function parseTle(text: string): SatRec | null {
	const lines = text.split('\n').map((line) => line.trim());
	const one = lines.find((line) => line.startsWith('1 '));
	const two = lines.find((line) => line.startsWith('2 '));
	if (!one || !two) return null;
	try {
		const satrec = twoline2satrec(one, two);
		return satrec.error ? null : satrec;
	} catch {
		return null;
	}
}

/** The ISS's elements: cached for two hours, `null` when CelesTrak is unreachable and nothing is cached. */
export async function loadIss(): Promise<SatRec | null> {
	const cached = readCache();
	if (cached && Date.now() - cached.at < CACHE_MS) return parseTle(cached.lines.join('\n'));
	try {
		const response = await fetch(TLE_URL);
		if (!response.ok) throw new Error(`CelesTrak ${response.status}`);
		const text = await response.text();
		const satrec = parseTle(text);
		if (!satrec) throw new Error('CelesTrak sent no TLE');
		try {
			localStorage.setItem(CACHE_KEY, JSON.stringify({ at: Date.now(), lines: text.split('\n') }));
		} catch {
			// Storage blocked: the next visit just fetches again.
		}
		return satrec;
	} catch {
		// A stale TLE still places the ISS within a few km for days; beyond that, hide it.
		return cached ? parseTle(cached.lines.join('\n')) : null;
	}
}

/** Where the ISS is at `ms`, or `null` once SGP4 gives up on the elements. */
export function issState(satrec: SatRec, ms: number = Date.now()): IssState | null {
	const date = new Date(ms);
	try {
		const { position, velocity } = propagate(satrec, date);
		// SGP4 answers `false` instead of vectors once the elements decay.
		if (typeof position !== 'object' || typeof velocity !== 'object') return null;
		const geo = eciToGeodetic(position, gstime(date));
		const { x, y, z } = velocity;
		const state = {
			lat: degreesLat(geo.latitude),
			lng: degreesLong(geo.longitude),
			altitudeKm: geo.height,
			speedKmH: Math.hypot(x, y, z) * 3600,
		};
		return Object.values(state).every(Number.isFinite) ? state : null;
	} catch {
		return null;
	}
}
