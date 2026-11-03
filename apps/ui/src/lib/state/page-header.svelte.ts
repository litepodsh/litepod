import type { Component, Snippet } from 'svelte';
import { page } from '$app/state';
import { getOrganization } from '#lib/clients/organization-client.js';
import { t } from '#lib/i18n/index.svelte.js';
import { ROUTE_ICONS, ROUTE_LABELS } from '#lib/utils/route-labels.js';

export type CrumbSwitcherItem = {
	value: string;
	label: string;
	description?: string;
	/** Key handed to the switcher's `itemIcon` snippet (usually the item id). */
	icon?: string;
};

export type CrumbSwitcher = {
	items: CrumbSwitcherItem[];
	selectedValue: string;
	onSelect: (value: string) => void;
	label?: string;
	searchPlaceholder?: string;
	emptyText?: string;
	/** Renders each item's avatar in the list and on the trigger. */
	itemIcon?: Snippet<[string]>;
};

export type HeaderBreadcrumb = {
	label: string;
	href?: string;
	switcher?: CrumbSwitcher;
	/** The label is still being fetched; render a placeholder instead of empty text. */
	pending?: boolean;
	/** Static leading glyph (a Lucide icon). Set automatically for known routes. */
	icon?: Component;
	/** Dynamic leading glyph supplied by a page (e.g. an app or project avatar). */
	iconSnippet?: Snippet;
};

export const pageHeaderState = $state<{
	breadcrumbs: HeaderBreadcrumb[] | null;
	/** Path the breadcrumbs above were set for, so another page's trail is never reused. */
	path: string | null;
	/** Rendered inside the bottom header on mobile only (e.g. the projects search input). */
	mobileSlot: Snippet | null;
}>({
	breadcrumbs: null,
	path: null,
	mobileSlot: null,
});

/**
 * Only pages that need something the URL can't express (a switcher dropdown) call
 * this; everything else is derived from the path by `routeBreadcrumbs`, or named
 * with `nameCurrentCrumb`.
 */
export function setPageBreadcrumbs(
	breadcrumbs: HeaderBreadcrumb[],
	path: string = page.url.pathname,
) {
	pageHeaderState.breadcrumbs = breadcrumbs;
	pageHeaderState.path = path;
}

/** A detail page: the URL gives the whole trail except the name of the last id. */
export function nameCurrentCrumb(name: string, path: string = page.url.pathname) {
	const trail = routeBreadcrumbs(path);
	trail[trail.length - 1] = { ...trail[trail.length - 1], label: name, pending: false };
	setPageBreadcrumbs(trail, path);
}

/** A project page that failed to load has no names to show: leave the way back, not a placeholder. */
export function setProjectsOnlyCrumb() {
	const [projects] = routeBreadcrumbs('/projects');
	setPageBreadcrumbs([{ ...projects, href: '/projects' }]);
}

export function setMobileHeaderSlot(slot: Snippet) {
	pageHeaderState.mobileSlot = slot;
}

export function clearMobileHeaderSlot() {
	pageHeaderState.mobileSlot = null;
}

// ponytail: names cached for the whole session. Renames go through
// cacheOrganizationName; nothing else invalidates.
const organizationNames = $state<Record<string, string>>({});

export function cacheOrganizationName(organizationId: string, name: string) {
	organizationNames[organizationId] = name;
}

/**
 * Fetches the organization name a path needs. Kept out of `routeBreadcrumbs` so
 * that stays a pure read — Svelte forbids mutating state inside a `$derived`.
 */
export function primeOrganizationName(pathname: string) {
	const organizationId = organizationIdFromPath(pathname);
	if (!organizationId || organizationId in organizationNames) return;
	organizationNames[organizationId] = '';
	void getOrganization(organizationId)
		.then((organization) => (organizationNames[organizationId] = organization.name))
		.catch(() => (organizationNames[organizationId] = 'Organization'));
}

export function organizationIdFromPath(pathname: string) {
	const segments = pathname.split('/').filter(Boolean);
	if (segments[0] !== 'organizations' || !segments[1]) return null;
	const id = decodeURIComponent(segments[1]);
	return id in ROUTE_LABELS ? null : id;
}

export function routeBreadcrumbs(pathname: string): HeaderBreadcrumb[] {
	const segments = pathname.split('/').filter(Boolean);
	if (segments.length === 0) return [{ label: t('menu.navigation.home'), icon: ROUTE_ICONS.home }];

	let href = '';
	return segments.map((segment, index) => {
		href += `/${segment}`;
		const known = ROUTE_LABELS[segment];
		const isOrganization = known === undefined && segments[index - 1] === 'organizations';
		const name = isOrganization
			? organizationNames[decodeURIComponent(segment)]
			: known && t(known);
		// An unknown segment is an id whose page pushes its own trail once loaded;
		// until then it is a placeholder, never a generic "Details" that then jumps.
		return {
			label: name || (isOrganization ? t('menu.navigation.organization') : ''),
			pending: !name,
			href: index === segments.length - 1 ? undefined : href,
			icon: ROUTE_ICONS[segment],
		};
	});
}

/**
 * The single source both the breadcrumb bar and the document title read. A trail
 * pushed by a page wins only on the path it was pushed for.
 */
export function resolveBreadcrumbs(pathname: string): HeaderBreadcrumb[] {
	if (pageHeaderState.path === pathname && pageHeaderState.breadcrumbs?.length)
		return pageHeaderState.breadcrumbs;
	return routeBreadcrumbs(pathname);
}
