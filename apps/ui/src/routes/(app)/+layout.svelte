<script lang="ts">
	import '../../app.css';
	import favicon from '#lib/assets/favicon.svg';
	import { page } from '$app/state';
	import CommandPalette from '#lib/components/custom/command-palette.svelte';
	import { UpdateOverlay } from '#lib/components/update/index.js';
	import OnboardingDock from '#lib/components/onboarding/onboarding-dock.svelte';
	import DeploymentsIndicator from '#lib/components/deployments/deployments-indicator.svelte';
	import ScopeHeader from '#lib/components/shell/scope-header.svelte';
	import ScopeHero from '#lib/components/shell/scope-hero.svelte';
	import ScopeTabs from '#lib/components/shell/scope-tabs.svelte';
	import AssistantRail from '#lib/components/shell/assistant-rail.svelte';
	import type { OrganizationDto } from '#lib/clients/organization-client.js';
	import { restoreAssistant } from '#lib/state/assistant.svelte.js';
	import { readStoredLayoutWidth, setLayoutWidth } from '#lib/state/shell.svelte.js';
	import {
		getSelectedOrganizationId,
		resolveSelectedOrganization,
	} from '#lib/utils/selected-organization.js';
	import { resolveBreadcrumbs } from '#lib/state/page-header.svelte.js';
	import { loadPermissions } from '#lib/state/permissions.svelte.js';
	let { children } = $props();

	setLayoutWidth(readStoredLayoutWidth());
	restoreAssistant();

	let organizations = $state<OrganizationDto[]>([]);
	let isSysadmin = $state(false);
	let isCloud = $state(false);

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

<ScopeHeader bind:organizations bind:isSysadmin bind:isCloud />
<ScopeHero />
<ScopeTabs {organizationId} {isSysadmin} {isCloud} />
<main class="shell-col flex min-w-0 flex-1 flex-col gap-4">
	{@render children()}
</main>

<div class="fixed bottom-5 left-4 z-30 max-sm:hidden">
	<DeploymentsIndicator />
</div>
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
