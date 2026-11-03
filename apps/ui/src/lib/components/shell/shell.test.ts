import { cleanup, render } from '@testing-library/svelte';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import LayersIcon from '@lucide/svelte/icons/layers';

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

it('header keeps the whole project trail and marks the deepest crumb as current', () => {
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
	expect(crumbs.map((c) => c.textContent?.trim())).toEqual(['Projects', 'tools', 'prod', 'ente']);
	expect(crumbs.map((c) => c.dataset.current)).toEqual(['false', 'false', 'false', 'true']);
});

it('header links a switcher crumb to its level and keeps the chevron for switching', () => {
	url.current = new URL('http://x/projects/p1/environments/e1/applications/a1');
	const switcher = { items: [{ value: 'p1', label: 'tools' }], selectedValue: 'p1', onSelect() {} };
	pageHeaderState.breadcrumbs = [
		{ label: 'tools', href: '/projects/p1', switcher },
		{ label: 'ente', switcher },
	];
	pageHeaderState.path = url.current.pathname;
	const [ancestor, currentCrumb] = render(ScopeHeader).getAllByTestId('scope-crumb');
	expect(ancestor.querySelector('a')?.getAttribute('href')).toBe('/projects/p1');
	expect(ancestor.querySelector('button')).not.toBeNull();
	expect(currentCrumb.querySelector('a')).toBeNull();
	expect(currentCrumb.querySelector('button')).not.toBeNull();
});

it('header draws each crumb icon inside its clickable', () => {
	url.current = new URL('http://x/projects/p1/environments/e1');
	const switcher = { items: [{ value: 'e1', label: 'prod' }], selectedValue: 'e1', onSelect() {} };
	pageHeaderState.breadcrumbs = [
		{ label: 'tools', href: '/projects/p1', icon: LayersIcon },
		{ label: 'prod', icon: LayersIcon, switcher },
	];
	pageHeaderState.path = url.current.pathname;
	const [link, current] = render(ScopeHeader).getAllByTestId('scope-crumb');
	expect(link.querySelector('a svg')).not.toBeNull();
	// The environment's icon plus the chevron.
	expect(current.querySelectorAll('button svg')).toHaveLength(2);
	expect(current.querySelectorAll('svg')).toHaveLength(2);
});

it.each([
	['/home', [['Home', null]]],
	['/organizations/o1/servers', [['Servers', null]]],
	['/organizations/o1', [['Settings', null]]],
	['/organizations/o1/integrations', [['Integrations', null]]],
	[
		'/organizations/o1/server-groups',
		[
			['Servers', '/organizations/o1/servers'],
			['Server Groups', null],
		],
	],
	['/organizations', [['Organizations', null]]],
])('header shows the org-level trail of %s', (path, expected) => {
	url.current = new URL(`http://x${path}`);
	const crumbs = render(ScopeHeader, { organizationId: 'o1' }).getAllByTestId('scope-crumb');
	expect(
		crumbs.map((c) => [c.textContent?.trim(), c.querySelector('a')?.getAttribute('href') ?? null]),
	).toEqual(expected);
});

it('org tabs are links and the current section is selected', () => {
	url.current = new URL('http://x/organizations/o1/servers');
	const { getAllByRole } = render(ScopeTabs, { organizationId: 'o1' });
	const tabs = getAllByRole('tab');
	expect(tabs).toHaveLength(10);
	const selected = tabs.find((tab) => tab.getAttribute('aria-selected') === 'true')!;
	expect(selected.getAttribute('href')).toBe('/organizations/o1/servers');
	// The stylesheet hides these from tablet up, where the sidebar carries them.
	expect(tabs[0].closest('.scope-tabs')?.getAttribute('data-org')).toBe('true');
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
	expect(tabs[0].closest('.scope-tabs')?.getAttribute('data-org')).toBe('false');
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
