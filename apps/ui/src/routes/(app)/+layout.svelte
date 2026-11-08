<script lang="ts">
	import '../../app.css';
	import { animate } from 'motion';
	import favicon from '#lib/assets/favicon.svg';
	import { page } from '$app/state';
	import CommandPalette from '#lib/components/custom/command-palette.svelte';
	import { UpdateOverlay } from '#lib/components/update/index.js';
	import OnboardingDock from '#lib/components/onboarding/onboarding-dock.svelte';
	import AppSidebar from '#lib/components/shell/app-sidebar.svelte';
	import { sidebarSide } from '#lib/state/sidebar-side.svelte.js';
	import * as Sidebar from '#lib/components/ui/sidebar/index.js';
	import { SIDEBAR_COOKIE_NAME } from '#lib/components/ui/sidebar/constants.js';
	import ScopeHeader from '#lib/components/shell/scope-header.svelte';
	import ScopeHero from '#lib/components/shell/scope-hero.svelte';
	import ScopeTabs from '#lib/components/shell/scope-tabs.svelte';
	import AssistantRail from '#lib/components/shell/assistant-rail.svelte';
	import type { OrganizationDto } from '#lib/clients/organization-client.js';
	import type { SystemVersion } from '#lib/clients/system-client.js';
	import { restoreAssistant } from '#lib/state/assistant.svelte.js';
	import {
		getSelectedOrganizationId,
		resolveSelectedOrganization,
	} from '#lib/utils/selected-organization.js';
	import { resolveBreadcrumbs } from '#lib/state/page-header.svelte.js';
	import { loadPermissions } from '#lib/state/permissions.svelte.js';
	let { children } = $props();

	restoreAssistant();

	let organizations = $state<OrganizationDto[]>([]);
	let isSysadmin = $state(false);
	let isCloud = $state(false);
	let user = $state({ name: '', email: '', is_sysadmin: false });
	let systemVersion = $state<SystemVersion | null>(null);
	let organizationsLoaded = $state(false);
	// The primitive writes this cookie but never reads it back.
	let sidebarOpen = $state(!document.cookie.includes(`${SIDEBAR_COOKIE_NAME}=false`));

	// Side swap, FLIP style: the layout jumps, then each piece springs from where it was.
	let content = $state<HTMLElement>();
	let before = new Map<Element, DOMRect>();
	const flipped = () =>
		[document.querySelector('[data-slot="sidebar-container"]'), content].filter(
			(el): el is Element => !!el,
		);
	$effect.pre(() => {
		void sidebarSide.current;
		before = new Map(flipped().map((el) => [el, el.getBoundingClientRect()]));
	});
	$effect(() => {
		void sidebarSide.current;
		if (matchMedia('(prefers-reduced-motion: reduce)').matches) return;
		for (const el of flipped()) {
			const from = before.get(el);
			const dx = from ? from.left - el.getBoundingClientRect().left : 0;
			if (dx) animate(el, { x: [dx, 0] }, { type: 'spring', stiffness: 260, damping: 30 });
		}
	});

	function pageTitle(): string {
		const labels = resolveBreadcrumbs(page.url.pathname)
			.map((crumb) => crumb.label)
			.filter(Boolean);
		if (labels.length === 0) return 'litepod';
		return `${labels.reverse().join(' · ')} - litepod`;
	}

	// The organization the tabs, the assistant and permissions are about: the one
	// in the URL when a route names it, otherwise the one the switcher last selected.
	let organizationId = $derived.by(() => {
		const fromRoute = page.url.pathname.match(/^\/organizations\/([^/]+)/);
		if (fromRoute && fromRoute[1] !== 'add') return decodeURIComponent(fromRoute[1]);
		return resolveSelectedOrganization(organizations)?.id ?? getSelectedOrganizationId();
	});

	let organizationName = $derived(organizations.find((o) => o.id === organizationId)?.name);
	let activeOrganization = $derived(resolveSelectedOrganization(organizations));

	$effect(() => {
		void loadPermissions(organizationId);
	});
</script>

<svelte:head>
	<link rel="icon" href={favicon} />
	<title>{pageTitle()}</title>
</svelte:head>

<CommandPalette />
<UpdateOverlay />
<Sidebar.Provider
	bind:open={sidebarOpen}
	class={sidebarSide.current === 'right' ? 'flex-row-reverse' : ''}>
	<AppSidebar
		bind:organizations
		active={activeOrganization}
		{organizationsLoaded}
		{organizationId}
		{isSysadmin}
		{isCloud}
		{user}
		{systemVersion} />
	<!-- Not Sidebar.Inset: it renders a <main>, and the page's own <main> sits inside. -->
	<div bind:this={content} class="flex min-h-svh min-w-0 flex-1 flex-col">
		<ScopeHeader
			bind:organizations
			bind:isSysadmin
			bind:isCloud
			bind:user
			bind:systemVersion
			bind:organizationsLoaded
			{organizationId} />
		<ScopeHero />
		<ScopeTabs {organizationId} {isSysadmin} {isCloud} />
		<main class="shell-col flex min-w-0 flex-1 flex-col gap-4">
			{@render children()}
		</main>
	</div>
</Sidebar.Provider>
<AssistantRail {organizationId} scope={organizationName} />
<OnboardingDock {organizationId} />

<style>
	@media (max-width: 640px) {
		/* Room for the bottom tab bar. */
		:global(body:has(.scope-tabs)) main {
			padding-bottom: calc(104px + env(safe-area-inset-bottom));
		}
	}
</style>
