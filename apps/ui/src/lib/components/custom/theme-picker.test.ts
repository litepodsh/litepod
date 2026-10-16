import { fireEvent, render } from '@testing-library/svelte';
import { beforeEach, expect, it, vi } from 'vitest';

const setMode = vi.fn();
vi.mock('mode-watcher', () => ({
	setMode: (m: string) => setMode(m),
	userPrefersMode: { current: 'system' },
	mode: { current: 'light' },
}));

import ThemePicker, { themeId } from './theme-picker.svelte';
import { paletteState } from '#lib/state/palette.svelte.js';

beforeEach(() => {
	setMode.mockClear();
	paletteState.current = 'warm';
});

it('names the four themes', () => {
	expect(themeId('cool', 'dark')).toBe('cool-dark');
});

it('renders four theme radios and a system switch', () => {
	const { getAllByRole, getByRole } = render(ThemePicker);
	expect(getAllByRole('radio')).toHaveLength(4);
	expect(getByRole('switch')).toBeTruthy();
});

it('picking a card while system is on changes only the palette', async () => {
	const { getAllByRole } = render(ThemePicker);
	await fireEvent.click(getAllByRole('radio')[2]); // cool light
	expect(paletteState.current).toBe('cool');
	expect(setMode).not.toHaveBeenCalled();
});
