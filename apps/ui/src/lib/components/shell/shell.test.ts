import { cleanup, render } from '@testing-library/svelte';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';

// jsdom here has no localStorage; mode-watcher and the org selection read it on import.
vi.hoisted(() => {
	const storage = new Map<string, string>();
	Object.defineProperty(window, 'localStorage', {
		configurable: true,
		value: {
			getItem: (key: string) => storage.get(key) ?? null,
			setItem: (key: string, value: string) => storage.set(key, value),
			removeItem: (key: string) => storage.delete(key),
		},
	});
});

const url = { current: new URL('http://x/projects/p1/environments/e1/applications/a1') };
vi.mock('$app/state', () => ({
	page: {
		get url() {
			return url.current;
		},
	},
}));
vi.mock('#lib/clients/auth-client.js', () => ({ getMe: () => new Promise(() => {}) }));
vi.mock('#lib/clients/system-client.js', () => ({ getSystemVersion: () => new Promise(() => {}) }));
vi.mock('#lib/clients/organization-client.js', () => ({
	listOrganizations: () => new Promise(() => {}),
	getOrganization: () => new Promise(() => {}),
}));

import ScopeHeader from './scope-header.svelte';
import ScopeTabs from './scope-tabs.svelte';
import ScopeHero from './scope-hero.svelte';
import { pageHeaderState } from '#lib/state/page-header.svelte.js';
import {
	clearScopeHero,
	clearShellTabs,
	setScopeHero,
	setShellTabs,
} from '#lib/state/shell.svelte.js';

class Noop {
	observe() {}
	disconnect() {}
}
globalThis.ResizeObserver ??= Noop as unknown as typeof ResizeObserver;
globalThis.IntersectionObserver ??= Noop as unknown as typeof IntersectionObserver;

afterEach(cleanup);

beforeEach(() => {
	clearShellTabs();
	clearScopeHero();
});

it('header drops the Projects crumb and marks the deepest switcher as current', () => {
	url.current = new URL('http://x/projects/p1/environments/e1/applications/a1');
	pageHeaderState.breadcrumbs = [
		{ label: 'Projects', href: '/projects' },
		{ label: 'tools', href: '/projects/p1' },
		{ label: 'prod', href: '/projects/p1/environments/e1' },
		{ label: 'ente' },
	];
	pageHeaderState.path = url.current.pathname;
	const { getAllByTestId } = render(ScopeHeader);
	const crumbs = getAllByTestId('scope-crumb');
	expect(crumbs.map((c) => c.textContent?.trim())).toEqual(['tools', 'prod', 'ente']);
	expect(crumbs.map((c) => c.dataset.current)).toEqual(['false', 'false', 'true']);
});

it('org tabs are links and the current section is selected', () => {
	url.current = new URL('http://x/organizations/o1/servers');
	const { getAllByRole } = render(ScopeTabs, { organizationId: 'o1' });
	const tabs = getAllByRole('tab');
	expect(tabs).toHaveLength(9);
	const selected = tabs.find((tab) => tab.getAttribute('aria-selected') === 'true')!;
	expect(selected.getAttribute('href')).toBe('/organizations/o1/servers');
});

it('a page can push its own tabs, which call back instead of navigating', async () => {
	url.current = new URL('http://x/projects/p1/environments/e1/applications/a1');
	const onSelect = vi.fn();
	setShellTabs(
		[
			{ value: 'general', label: 'General' },
			{ value: 'logs', label: 'Logs' },
		],
		'general',
		onSelect,
		url.current.pathname,
	);
	const { getAllByRole } = render(ScopeTabs, { organizationId: 'o1' });
	const tabs = getAllByRole('tab');
	expect(tabs.map((tab) => tab.textContent?.trim())).toEqual(['General', 'Logs']);
	tabs[1].click();
	expect(onSelect).toHaveBeenCalledWith('logs');
});

it('hero shows its stats only on the page that set it', () => {
	url.current = new URL('http://x/projects/p1');
	setScopeHero({ title: 'tools', stats: [{ label: 'Apps', value: '4' }] }, '/projects/p1');
	const { getByTestId, getByText } = render(ScopeHero);
	expect(getByText('tools')).toBeTruthy();
	expect(getByTestId('hero-stats').querySelectorAll('dt')).toHaveLength(1);
});
