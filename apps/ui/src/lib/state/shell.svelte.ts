import type { Component, Snippet } from 'svelte';
import { page } from '$app/state';

/** What the console shell shows around a page: column width, the tab capsule, the info card. */

export type LayoutWidth = 'contained' | 'full';
export const LAYOUT_STORAGE_KEY = 'lp-layout';

export const layoutWidth = $state<{ current: LayoutWidth }>({ current: 'contained' });

export function readStoredLayoutWidth(): LayoutWidth {
	try {
		return localStorage.getItem(LAYOUT_STORAGE_KEY) === 'full' ? 'full' : 'contained';
	} catch {
		return 'contained';
	}
}

export function setLayoutWidth(width: LayoutWidth) {
	layoutWidth.current = width;
	document.documentElement.dataset.layout = width;
	try {
		localStorage.setItem(LAYOUT_STORAGE_KEY, width);
	} catch {
		// Private mode: the choice lasts for this tab only.
	}
}

export type ShellTab = {
	value: string;
	label: string;
	icon?: Component;
	disabled?: boolean;
	disabledTooltip?: string;
};

/**
 * Tabs a project or application page pushes into the capsule. The page keeps
 * owning which tabs exist and which one is active; the shell only draws them.
 * Like the breadcrumbs, a set of tabs only counts on the path it was set for.
 */
export const shellTabs = $state<{
	items: ShellTab[];
	active: string;
	onSelect: ((value: string) => void) | null;
	path: string | null;
}>({ items: [], active: '', onSelect: null, path: null });

export function setShellTabs(
	items: ShellTab[],
	active: string,
	onSelect: (value: string) => void,
	path: string = page.url.pathname,
) {
	shellTabs.items = items;
	shellTabs.active = active;
	shellTabs.onSelect = onSelect;
	shellTabs.path = path;
}

export function clearShellTabs() {
	shellTabs.items = [];
	shellTabs.onSelect = null;
	shellTabs.path = null;
}

export type HeroStat = { label: string; value: string; href?: string };
export type HeroSibling = {
	id: string;
	label: string;
	href: string;
	/** Omitted when the status is not known; no dot is drawn then. */
	tone?: 'ok' | 'warn' | 'off';
	current: boolean;
};
export type ScopeHero = {
	title: string;
	avatar?: Snippet;
	status?: Snippet;
	meta?: Snippet;
	actions?: Snippet;
	stats?: HeroStat[];
	siblings?: HeroSibling[];
	siblingsLabel?: string;
	path: string;
};

/** The info card above the tabs; pages set it in an `$effect` and clear it on destroy. */
export const scopeHero = $state<{ current: ScopeHero | null }>({ current: null });

export function setScopeHero(hero: Omit<ScopeHero, 'path'>, path: string = page.url.pathname) {
	scopeHero.current = { ...hero, path };
}

export function clearScopeHero() {
	scopeHero.current = null;
}
