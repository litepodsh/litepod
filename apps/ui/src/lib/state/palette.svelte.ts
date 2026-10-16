export type Palette = 'warm' | 'cool';
export const PALETTE_STORAGE_KEY = 'lp-palette';

export const paletteState = $state<{ current: Palette }>({ current: 'warm' });

export function readStoredPalette(): Palette {
	try {
		return localStorage.getItem(PALETTE_STORAGE_KEY) === 'cool' ? 'cool' : 'warm';
	} catch {
		return 'warm';
	}
}

/** Warm is the default token set, so it is the absence of the attribute. */
export function setPalette(palette: Palette) {
	paletteState.current = palette;
	if (palette === 'cool') document.documentElement.dataset.palette = 'cool';
	else delete document.documentElement.dataset.palette;
	try {
		localStorage.setItem(PALETTE_STORAGE_KEY, palette);
	} catch {
		// Private mode: the choice lasts for this tab only.
	}
}
