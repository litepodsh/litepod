import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('$app/state', () => ({ page: { url: { pathname: '/' } } }));
vi.mock('#lib/clients/organization-client.js', () => ({ getOrganization: vi.fn() }));

const { pageHeaderState, resolveBreadcrumbs, routeBreadcrumbs, cacheOrganizationName } =
	await import('./page-header.svelte');

beforeEach(() => {
	pageHeaderState.breadcrumbs = null;
	pageHeaderState.path = null;
});

describe('routeBreadcrumbs', () => {
	it('labels a static route from the shared map', () => {
		expect(routeBreadcrumbs('/git-providers')).toMatchObject([
			{ label: 'Git providers', pending: false },
		]);
	});

	it('names the organization and links back to it', () => {
		cacheOrganizationName('abc', 'Acme Corp');
		expect(routeBreadcrumbs('/organizations/abc/containers')).toMatchObject([
			{ label: 'Organizations', pending: false, href: '/organizations' },
			{ label: 'Acme Corp', pending: false, href: '/organizations/abc' },
			{ label: 'Containers', pending: false },
		]);
	});

	it('marks the organization crumb pending until its name arrives', () => {
		const [, organization] = routeBreadcrumbs('/organizations/unfetched/servers');
		expect(organization).toMatchObject({ pending: true, href: '/organizations/unfetched' });
	});

	it('reads topology as Map', () => {
		cacheOrganizationName('abc', 'Acme Corp');
		expect(routeBreadcrumbs('/organizations/abc/topology').at(-1)?.label).toBe('Map');
	});
});

describe('resolveBreadcrumbs', () => {
	it('uses a trail a page pushed for this exact path', () => {
		pageHeaderState.breadcrumbs = [{ label: 'Pushed' }];
		pageHeaderState.path = '/organizations/abc/containers';
		expect(resolveBreadcrumbs('/organizations/abc/containers')).toEqual([{ label: 'Pushed' }]);
	});

	// The reported bug: a trail left behind by the previous page stayed on screen.
	it('ignores a trail pushed for a different path', () => {
		pageHeaderState.breadcrumbs = [{ label: 'Git providers' }];
		pageHeaderState.path = '/git-providers';
		cacheOrganizationName('abc', 'Acme Corp');
		expect(resolveBreadcrumbs('/organizations/abc/containers').at(-1)?.label).toBe('Containers');
	});
});
