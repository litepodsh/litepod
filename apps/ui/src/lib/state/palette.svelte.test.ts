import { beforeEach, expect, it } from 'vitest';
import {
	PALETTE_STORAGE_KEY,
	paletteState,
	readStoredPalette,
	setPalette,
} from './palette.svelte.js';

const storage = new Map<string, string>();
Object.defineProperty(window, 'localStorage', {
	configurable: true,
	value: {
		getItem: (key: string) => storage.get(key) ?? null,
		setItem: (key: string, value: string) => storage.set(key, value),
		removeItem: (key: string) => storage.delete(key),
	},
});

beforeEach(() => {
	storage.clear();
	delete document.documentElement.dataset.palette;
	paletteState.current = 'warm';
});

it('defaults to warm when nothing is stored', () => {
	expect(readStoredPalette()).toBe('warm');
});

it('ignores unknown stored values', () => {
	localStorage.setItem(PALETTE_STORAGE_KEY, 'neon');
	expect(readStoredPalette()).toBe('warm');
});

it('applies cool to <html>, the state and storage', () => {
	setPalette('cool');
	expect(document.documentElement.dataset.palette).toBe('cool');
	expect(paletteState.current).toBe('cool');
	expect(localStorage.getItem(PALETTE_STORAGE_KEY)).toBe('cool');
});

it('warm removes the attribute so the default tokens apply', () => {
	setPalette('cool');
	setPalette('warm');
	expect(document.documentElement.dataset.palette).toBeUndefined();
});
