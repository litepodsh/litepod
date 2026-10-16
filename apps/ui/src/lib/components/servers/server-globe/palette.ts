/** Globe colors, read from the theme's own tokens so both engines agree. */

import type { ServerGeo } from '#lib/clients/server-client.js';

export type GlobePalette = {
	dark: boolean;
	atmosphere: string;
	/** Strength of the sunrise line, 0 to 1. */
	terminator: number;
	accent: string;
	ok: string;
	warn: string;
	muted: string;
};

function token(name: string) {
	return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

export function readPalette(): GlobePalette {
	return {
		dark: document.documentElement.classList.contains('dark'),
		atmosphere: token('--lp-earth-atmosphere'),
		terminator: Number.parseFloat(token('--lp-earth-terminator')) || 0.3,
		accent: token('--lp-accent'),
		ok: token('--lp-ok'),
		warn: token('--lp-warn'),
		muted: token('--lp-muted'),
	};
}

/** `#rgb` or `#rrggbb` as three 0..1 channels. */
export function channels(hex: string): [number, number, number] {
	const body = hex.replace('#', '');
	const full = body.length === 3 ? [...body].map((c) => c + c).join('') : body;
	const value = Number.parseInt(full, 16);
	return [((value >> 16) & 255) / 255, ((value >> 8) & 255) / 255, (value & 255) / 255];
}

export function withAlpha(hex: string, alpha: number) {
	const [r, g, b] = channels(hex).map((c) => Math.round(c * 255));
	return `rgba(${r},${g},${b},${alpha})`;
}

/** Status color, ignoring whether the server is the main one. */
export function stateColor(server: ServerGeo, palette: GlobePalette) {
	if (server.state === 'ready') return palette.ok;
	if (server.state === 'degraded' || server.state === 'setting_up') return palette.warn;
	return palette.muted;
}

/** Marker color: the main server always wears the accent, remotes their status. */
export function markerColor(server: ServerGeo, palette: GlobePalette) {
	return server.is_main ? palette.accent : stateColor(server, palette);
}

export const reducedMotion = () =>
	typeof window !== 'undefined' && window.matchMedia('(prefers-reduced-motion: reduce)').matches;
