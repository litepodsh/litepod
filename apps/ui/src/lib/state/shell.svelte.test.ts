import { beforeEach, expect, it, vi } from 'vitest';

vi.mock('$app/state', () => ({ page: { url: new URL('http://x/projects/p1') } }));

import {
	LAYOUT_STORAGE_KEY,
	clearShellTabs,
	layoutWidth,
	readStoredLayoutWidth,
	setLayoutWidth,
	setShellTabs,
	shellTabs,
} from './shell.svelte.js';

const storage = new Map<string, string>();
Object.defineProperty(window, 'localStorage', {
	configurable: true,
	value: {
		getItem: (key: string) => storage.get(key) ?? null,
		setItem: (key: string, value: string) => storage.set(key, value),
		removeItem: (key: string) => storage.delete(key),
	},
});

beforeEach(() => storage.clear());

it('defaults to the contained column', () => {
	expect(readStoredLayoutWidth()).toBe('contained');
});

it('remembers full width on <html> and in storage', () => {
	setLayoutWidth('full');
	expect(layoutWidth.current).toBe('full');
	expect(document.documentElement.dataset.layout).toBe('full');
	expect(storage.get(LAYOUT_STORAGE_KEY)).toBe('full');
});

it('ties pushed tabs to the page path', () => {
	setShellTabs([{ value: 'logs', label: 'Logs' }], 'logs', () => {});
	expect(shellTabs.path).toBe('/projects/p1');
	clearShellTabs();
	expect(shellTabs.items).toEqual([]);
});
