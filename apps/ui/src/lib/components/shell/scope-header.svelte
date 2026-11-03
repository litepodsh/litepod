<script lang="ts">
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import SearchableDropdown from '#lib/components/custom/searchable-dropdown.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import Kbd from '#lib/components/ui/kbd/kbd.svelte';
	import { Separator } from '#lib/components/ui/separator/index.js';
	import * as Sidebar from '#lib/components/ui/sidebar/index.js';
	import SearchIcon from '@lucide/svelte/icons/search';
	import SparklesIcon from '@lucide/svelte/icons/sparkles';
	import UpdateButton from './update-button.svelte';
	import RuntimePill from './runtime-pill.svelte';
	import { getMe } from '#lib/clients/auth-client.js';
	import { listOrganizations, type OrganizationDto } from '#lib/clients/organization-client.js';
	import { getSystemVersion, type SystemVersion } from '#lib/clients/system-client.js';
	import { commandPaletteState } from '#lib/state/command-palette.svelte.js';
	import { assistantState, setAssistantMode } from '#lib/state/assistant.svelte.js';
	import {
		organizationIdFromPath,
		pageHeaderState,
		primeOrganizationName,
		resolveBreadcrumbs,
		type HeaderBreadcrumb,
	} from '#lib/state/page-header.svelte.js';
	import { activeTab, orgTabs, scopeOf } from '#lib/navigation/scope-tabs.js';
	import {
		getSelectedOrganizationId,
		resolveSelectedOrganization,
		setSelectedOrganizationId,
	} from '#lib/utils/selected-organization.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let {
		organizations = $bindable<OrganizationDto[]>([]),
		isSysadmin = $bindable(false),
		isCloud = $bindable(false),
		user = $bindable({ name: '', email: '', is_sysadmin: false }),
		systemVersion = $bindable(null),
		organizationId = null,
		organizationsLoaded = $bindable(false),
	}: {
		organizations?: OrganizationDto[];
		isSysadmin?: boolean;
		isCloud?: boolean;
		user?: { name: string; email: string; is_sysadmin: boolean };
		systemVersion?: SystemVersion | null;
		organizationId?: string | null;
		organizationsLoaded?: boolean;
	} = $props();

	let activeOrganization = $derived(resolveSelectedOrganization(organizations));

	$effect(() => {
		// Hidden, not disabled: the API refuses these routes on its own.
		assistantState.available = activeOrganization?.assistant_enabled ?? false;
		if (activeOrganization && getSelectedOrganizationId() !== activeOrganization.id)
			setSelectedOrganizationId(activeOrganization.id);
	});

	// Settings flips `assistant_enabled` on its own copy of the organization;
	// this list is the one the header icon reads, so it has to hear about it.
	$effect(() => {
		function sync(event: Event) {
			const { id, assistant_enabled } = (
				event as CustomEvent<{ id: string; assistant_enabled: boolean }>
			).detail;
			organizations = organizations.map((organization) =>
				organization.id === id ? { ...organization, assistant_enabled } : organization,
			);
		}
		window.addEventListener('organization-assistant-updated', sync);
		return () => window.removeEventListener('organization-assistant-updated', sync);
	});

	$effect(() => primeOrganizationName(page.url.pathname));

	/**
	 * At the org level the trail starts at the sidebar's section, not at the org:
	 * the switcher above it already names that. Pages a section owns hang from it.
	 */
	let crumbs = $derived.by((): HeaderBreadcrumb[] => {
		const path = page.url.pathname;
		const all = resolveBreadcrumbs(path);
		const scope = scopeOf(path);
		if (scope === 'project' || scope === 'app') {
			// Until the page pushes its switchers, show only one placeholder per id
			// (project, environment, app) so the trail keeps its final shape.
			const pushed = pageHeaderState.path === path && !!pageHeaderState.breadcrumbs?.length;
			return pushed ? all : all.filter((c) => c.pending || c.href === '/projects');
		}
		if (scope !== 'org') return all;

		const rest = organizationIdFromPath(path) ? all.slice(2) : all;
		const tabs = organizationId ? orgTabs(organizationId, isSysadmin, isCloud) : [];
		const currentTab = activeTab(path, tabs);
		const tab = tabs.find((candidate) => candidate.id === currentTab);
		if (!tab) return rest;
		const section = { label: t(tab.labelKey), icon: tab.icon };
		if (path === tab.href) return [section];
		const below = rest[0]?.href === tab.href ? rest.slice(1) : rest;
		return [{ ...section, href: tab.href }, ...below];
	});

	onMount(async () => {
		try {
			const [me, version] = await Promise.all([getMe(), getSystemVersion()]);
			user = { name: me.name, email: me.email, is_sysadmin: me.is_sysadmin };
			isSysadmin = me.is_sysadmin;
			systemVersion = version;
			isCloud = version.is_cloud;
			organizations = await listOrganizations();
		} catch {
			// The shell can render before an authenticated session exists.
		} finally {
			organizationsLoaded = true;
		}
	});
</script>

{#snippet crumbIcon(crumb: HeaderBreadcrumb)}
	{#if crumb.iconSnippet}
		<span class="flex size-5.5 shrink-0 items-center justify-center overflow-hidden rounded-md">
			{@render crumb.iconSnippet()}
		</span>
	{:else if crumb.icon}
		<crumb.icon
			class="size-3.5 shrink-0 text-muted-foreground transition-colors group-hover/crumb:text-foreground" />
	{/if}
{/snippet}

<header class="lp-groove-b sticky top-0 z-30 h-(--app-header-height) bg-background">
	<div class="flex h-full min-w-0 items-center gap-1.5 px-3 sm:px-4">
		<Sidebar.Trigger class="-ms-1" aria-label={t('menu.sidebar.toggle')} />
		<Separator orientation="vertical" class="me-1 data-[orientation=vertical]:h-4" />

		<nav aria-label={t('menu.header.scope')} class="min-w-0">
			<ol class="flex min-w-0 items-center">
				{#each crumbs as crumb, index (index)}
					{@const current = index === crumbs.length - 1}
					<li
						data-testid="scope-crumb"
						data-current={current}
						aria-current={current ? 'page' : undefined}
						class="flex min-w-0 items-center gap-1 max-sm:data-[current=false]:hidden">
						{#if index > 0}<span class="slash max-sm:hidden" aria-hidden="true"></span>{/if}
						{#if crumb.pending}
							<!-- Same box as a switcher trigger (h-8 + px-2) so nothing moves when it resolves. -->
							<span class="flex h-8 items-center gap-1.5 px-2" aria-hidden="true">
								{@render crumbIcon(crumb)}
								<span class="h-3 w-20 animate-pulse rounded-sm bg-muted-foreground/20"></span>
							</span>
						{:else if crumb.href}
							<a
								href={crumb.href}
								class="group/crumb flex h-8 max-w-44 min-w-0 items-center gap-1.5 rounded-lg px-2 text-sm text-muted-foreground transition-colors hover:bg-muted hover:text-foreground">
								{@render crumbIcon(crumb)}
								<span class="truncate">{crumb.label}</span>
							</a>
						{:else if !crumb.switcher}
							<span class="flex max-w-64 min-w-0 items-center gap-1.5 px-2 text-sm font-medium">
								{@render crumbIcon(crumb)}
								<span class="truncate">{crumb.label}</span>
							</span>
						{/if}
						{#if crumb.switcher}
							<SearchableDropdown
								items={crumb.switcher.items}
								selectedValue={crumb.switcher.selectedValue}
								onSelect={crumb.switcher.onSelect}
								icon={crumb.switcher.itemIcon}
								label={crumb.switcher.label ?? crumb.label}
								searchPlaceholder={crumb.switcher.searchPlaceholder ?? t('common.common.search')}
								emptyText={crumb.switcher.emptyText ?? t('common.page_header.no_options')}
								variant="ghost"
								size="sm"
								triggerClass={crumb.href
									? 'size-6 min-w-0 justify-center rounded-md p-0 shadow-none [&>*:not(:last-child)]:hidden'
									: 'group/crumb h-8 w-fit min-w-0 max-w-48 justify-start gap-1.5 rounded-lg px-2 font-medium shadow-none'}>
								{#snippet leading()}
									<!-- An itemIcon already draws on the trigger; a link's icon sits in the link. -->
									{#if !crumb.href && !crumb.switcher?.itemIcon}{@render crumbIcon(crumb)}{/if}
								{/snippet}
							</SearchableDropdown>
						{/if}
					</li>
				{/each}
			</ol>
		</nav>

		<div class="ms-auto flex shrink-0 items-center gap-1">
			<RuntimePill />
			<UpdateButton isSysadmin={user.is_sysadmin} />
			<Button
				variant="outline"
				size="sm"
				class="gap-2 text-muted-foreground max-md:size-8 max-md:p-0 md:min-w-44 md:justify-start"
				onclick={() => (commandPaletteState.open = true)}>
				<SearchIcon />
				<span class="max-md:sr-only">{t('menu.header.search')}</span>
				<span class="ms-auto hidden gap-0.5 md:flex"><Kbd>⌘</Kbd><Kbd>K</Kbd></span>
			</Button>

			{#if assistantState.available}
				<Button
					variant="ghost"
					size="icon-sm"
					class="relative"
					aria-pressed={assistantState.mode !== 'nav'}
					onclick={() => setAssistantMode(assistantState.mode === 'nav' ? 'chat' : 'nav')}>
					<SparklesIcon />
					{#if assistantState.unread}
						<span class="absolute top-1 right-1 size-1.5 rounded-full bg-primary"></span>
					{/if}
					<span class="sr-only">{t('menu.header.assistant')}</span>
				</Button>
			{/if}
		</div>
	</div>
</header>

<style>
	/* The rotated hairline between scope levels. */
	.slash {
		position: relative;
		flex: none;
		width: 12px;
		height: 28px;
		margin: 0 2px;
	}
	.slash::after {
		position: absolute;
		top: 4px;
		bottom: 4px;
		left: 50%;
		width: 1px;
		background: color-mix(in srgb, var(--foreground) 22%, transparent);
		content: '';
		transform: rotate(18deg);
	}
</style>
