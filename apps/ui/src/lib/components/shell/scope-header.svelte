<script lang="ts">
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import SearchableDropdown from '#lib/components/custom/searchable-dropdown.svelte';
	import LitepodMark from '#lib/components/custom/litepod-mark.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import Kbd from '#lib/components/ui/kbd/kbd.svelte';
	import * as Tooltip from '#lib/components/ui/tooltip/index.js';
	import SearchIcon from '@lucide/svelte/icons/search';
	import SparklesIcon from '@lucide/svelte/icons/sparkles';
	import MaximizeIcon from '@lucide/svelte/icons/move-horizontal';
	import MinimizeIcon from '@lucide/svelte/icons/fold-horizontal';
	import OrgSwitcher from './org-switcher.svelte';
	import UserMenu from './user-menu.svelte';
	import UpdateButton from './update-button.svelte';
	import RuntimePill from './runtime-pill.svelte';
	import { getMe } from '#lib/clients/auth-client.js';
	import { listOrganizations, type OrganizationDto } from '#lib/clients/organization-client.js';
	import { getSystemVersion, type SystemVersion } from '#lib/clients/system-client.js';
	import { commandPaletteState } from '#lib/state/command-palette.svelte.js';
	import { assistantState, setAssistantMode } from '#lib/state/assistant.svelte.js';
	import { layoutWidth, setLayoutWidth } from '#lib/state/shell.svelte.js';
	import {
		pageHeaderState,
		primeOrganizationName,
		resolveBreadcrumbs,
		type HeaderBreadcrumb,
	} from '#lib/state/page-header.svelte.js';
	import { scopeOf } from '#lib/navigation/scope-tabs.js';
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
	}: { organizations?: OrganizationDto[]; isSysadmin?: boolean; isCloud?: boolean } = $props();

	let user = $state({ name: '', email: '', is_sysadmin: false });
	let systemVersion = $state<SystemVersion | null>(null);
	let activeOrganization = $derived(resolveSelectedOrganization(organizations));

	$effect(() => {
		// Hidden, not disabled: the API refuses these routes on its own.
		assistantState.available = activeOrganization?.assistant_enabled ?? false;
		if (activeOrganization && getSelectedOrganizationId() !== activeOrganization.id)
			setSelectedOrganizationId(activeOrganization.id);
	});

	$effect(() => primeOrganizationName(page.url.pathname));

	/**
	 * The org switcher and the tabs already say where you are at the org level,
	 * so the trail only carries what they cannot: the project, environment and
	 * app switchers, a detail page under an org section, or a page outside any
	 * scope (account, instance settings).
	 */
	let crumbs = $derived.by((): HeaderBreadcrumb[] => {
		const path = page.url.pathname;
		const all = resolveBreadcrumbs(path);
		const scope = scopeOf(path);
		if (scope === 'org') return path.startsWith('/organizations/') ? all.slice(3) : [];
		if (scope === 'project' || scope === 'app') {
			// Until the page pushes its switchers, show only one placeholder per id
			// (project, environment, app) so the trail keeps its final shape.
			const pushed = pageHeaderState.path === path && !!pageHeaderState.breadcrumbs?.length;
			return pushed ? all.filter((c) => c.href !== '/projects') : all.filter((c) => c.pending);
		}
		return all;
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
		}
	});
</script>

{#snippet crumbIcon(crumb: HeaderBreadcrumb)}
	{#if crumb.iconSnippet}
		<span class="flex size-5.5 shrink-0 items-center justify-center overflow-hidden rounded-md">
			{@render crumb.iconSnippet()}
		</span>
	{:else if crumb.icon}
		<crumb.icon class="size-3.5 shrink-0 text-muted-foreground" />
	{/if}
{/snippet}

<header
	class="lp-groove-b sticky top-0 z-30 h-(--app-header-height) bg-card/88 backdrop-blur-md supports-[backdrop-filter]:bg-card/80">
	<div class="shell-col flex h-full min-w-0 items-center gap-1.5 px-3 sm:px-5">
		<a
			href="/home"
			aria-label="Litepod"
			class="grid size-8 shrink-0 place-items-center rounded-lg transition-colors hover:bg-muted">
			<LitepodMark class="size-6" />
		</a>

		<nav aria-label={t('menu.header.scope')} class="flex min-w-0 items-center">
			<span class="slash" aria-hidden="true"></span>
			<OrgSwitcher bind:organizations active={activeOrganization} />
			{#each crumbs as crumb, index (index)}
				{@const current = index === crumbs.length - 1}
				<span class="slash" class:max-sm:hidden={!current} aria-hidden="true"></span>
				<div
					data-testid="scope-crumb"
					data-current={current}
					class="flex min-w-0 items-center gap-1 max-sm:data-[current=false]:hidden">
					{#if !crumb.switcher?.itemIcon}{@render crumbIcon(crumb)}{/if}
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
							triggerClass="h-8 w-fit min-w-0 max-w-48 justify-start gap-1.5 rounded-lg px-2 font-medium shadow-none max-sm:[&>.truncate]:hidden" />
					{:else if crumb.pending}
						<!-- Same box as a switcher trigger (h-8 + px-2) so nothing moves when it resolves. -->
						<span class="flex h-8 items-center px-2" aria-hidden="true">
							<span class="h-3 w-20 animate-pulse rounded-sm bg-muted-foreground/20"></span>
						</span>
					{:else if crumb.href}
						<a
							href={crumb.href}
							class:max-sm:hidden={crumb.icon || crumb.iconSnippet}
							class="block h-8 max-w-44 truncate rounded-lg px-2 text-sm leading-8 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
							>{crumb.label}</a>
					{:else}
						<span
							aria-current="page"
							class:max-sm:sr-only={crumb.icon || crumb.iconSnippet}
							class="block max-w-64 truncate px-2 text-sm font-medium"
							>{crumb.label}</span>
					{/if}
				</div>
			{/each}
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

			<Tooltip.Provider>
				<Tooltip.Root delayDuration={300}>
					<Tooltip.Trigger>
						{#snippet child({ props })}
							<Button
								{...props}
								variant="ghost"
								size="icon-sm"
								class="max-sm:hidden"
								aria-pressed={layoutWidth.current === 'full'}
								onclick={() =>
									setLayoutWidth(layoutWidth.current === 'full' ? 'contained' : 'full')}>
								{#if layoutWidth.current === 'full'}<MinimizeIcon />{:else}<MaximizeIcon />{/if}
								<span class="sr-only"
									>{layoutWidth.current === 'full'
										? t('menu.header.contained_width')
										: t('menu.header.full_width')}</span>
							</Button>
						{/snippet}
					</Tooltip.Trigger>
					<Tooltip.Content
						>{layoutWidth.current === 'full'
							? t('menu.header.contained_width')
							: t('menu.header.full_width')}</Tooltip.Content>
				</Tooltip.Root>
			</Tooltip.Provider>

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

			<UserMenu {user} organizationId={activeOrganization?.id ?? null} {systemVersion} />
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
