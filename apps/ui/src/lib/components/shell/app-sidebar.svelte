<script lang="ts">
	import { page } from '$app/state';
	import type { ComponentProps } from 'svelte';
	import EllipsisIcon from '@lucide/svelte/icons/ellipsis';
	import ChevronRightIcon from '@lucide/svelte/icons/chevron-right';
	import DeploymentsIndicator from '#lib/components/deployments/deployments-indicator.svelte';
	import * as Collapsible from '#lib/components/ui/collapsible/index.js';
	import * as Sidebar from '#lib/components/ui/sidebar/index.js';
	import { useSidebar } from '#lib/components/ui/sidebar/index.js';
	import { listProjects, type ProjectDto } from '#lib/clients/project-client.js';
	import type { OrganizationDto } from '#lib/clients/organization-client.js';
	import type { SystemVersion } from '#lib/clients/system-client.js';
	import { branding } from '#lib/ee/branding/branding.svelte.js';
	import { t } from '#lib/i18n/index.svelte.js';
	import { activeTab, orgTabs } from '#lib/navigation/scope-tabs.js';
	import ProjectTreeItem from './project-tree-item.svelte';
	import OrgSwitcher from './org-switcher.svelte';
	import UserMenu from './user-menu.svelte';

	/** The console's navigation, built on the shadcn `sidebar-07` block. */
	const PROJECT_LIMIT = 6;

	let {
		organizations = $bindable<OrganizationDto[]>([]),
		active,
		organizationsLoaded = true,
		organizationId,
		isSysadmin = false,
		isCloud = false,
		user,
		systemVersion,
	}: {
		organizations?: OrganizationDto[];
		active: OrganizationDto | null;
		organizationsLoaded?: boolean;
		organizationId: string | null;
		isSysadmin?: boolean;
		isCloud?: boolean;
		user: ComponentProps<typeof UserMenu>['user'];
		systemVersion: SystemVersion | null;
	} = $props();

	const sidebar = useSidebar();

	let path = $derived(page.url.pathname);
	let tabs = $derived(organizationId ? orgTabs(organizationId, isSysadmin, isCloud) : []);
	let current = $derived(activeTab(path, tabs));

	let projects = $state<ProjectDto[]>([]);
	// shortcut: refreshes on the projects list only, so a project renamed elsewhere waits until then.
	let onProjectsList = $derived(path === '/projects');
	$effect(() => {
		void onProjectsList;
		if (!organizationId) {
			projects = [];
			return;
		}
		let stale = false;
		listProjects(organizationId)
			.then((list) => {
				if (!stale) projects = list;
			})
			.catch(() => {
				// A shortcut list: without it the Projects link above still works.
				if (!stale) projects = [];
			});
		return () => {
			stale = true;
		};
	});

	const closeOnPhone = () => sidebar.setOpenMobile(false);

	// Tree keys on the projects list: up/down walk the visible links, right/left open, close or climb.
	function treeKeys(event: KeyboardEvent) {
		if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
		const link = (event.target as HTMLElement).closest<HTMLAnchorElement>('a[href]');
		const item = link?.closest('li');
		if (!link || !item) return;
		const links = [...(event.currentTarget as HTMLElement).querySelectorAll<HTMLAnchorElement>('a[href]')].filter(
			(a) => a.getClientRects().length,
		);
		const trigger = item.querySelector<HTMLElement>(':scope > [data-slot="collapsible-trigger"]');
		const expanded = trigger?.getAttribute('aria-expanded') === 'true';
		let next: HTMLElement | null | undefined;
		if (event.key === 'ArrowDown') next = links[links.indexOf(link) + 1];
		else if (event.key === 'ArrowUp') next = links[links.indexOf(link) - 1];
		else if (event.key === 'ArrowRight' && trigger) {
			if (!expanded) trigger.click();
			else next = item.querySelector<HTMLAnchorElement>(':scope > [data-slot="collapsible-content"] a[href]');
		} else if (event.key === 'ArrowLeft') {
			if (trigger && expanded) trigger.click();
			else next = item.parentElement?.closest('li')?.querySelector<HTMLAnchorElement>(':scope > a[href]');
		} else return;
		event.preventDefault();
		next?.focus();
	}
</script>

<Sidebar.Root collapsible="icon">
	<Sidebar.Header class="h-(--app-header-height) shrink-0 justify-center border-b border-transparent py-0">
		<Sidebar.Menu>
			<Sidebar.MenuItem>
				<OrgSwitcher bind:organizations {active} loading={!organizationsLoaded} />
			</Sidebar.MenuItem>
		</Sidebar.Menu>
	</Sidebar.Header>

	<Sidebar.Content>
		{#if tabs.length}
			<Sidebar.Group>
				<Sidebar.GroupLabel>{branding.name ?? 'Litepod'}</Sidebar.GroupLabel>
				<Sidebar.Menu>
					{#each tabs as tab (tab.id)}
						<Sidebar.MenuItem>
							<Sidebar.MenuButton isActive={tab.id === current} tooltipContent={t(tab.labelKey)}>
								{#snippet child({ props })}
									<a
										href={tab.href}
										aria-current={tab.id === current ? 'page' : undefined}
										onclick={closeOnPhone}
										{...props}>
										<tab.icon />
										<span>{t(tab.labelKey)}</span>
									</a>
								{/snippet}
							</Sidebar.MenuButton>
						</Sidebar.MenuItem>
					{/each}
				</Sidebar.Menu>
			</Sidebar.Group>
		{/if}

		{#if projects.length}
			<Collapsible.Root open class="group/collapsible group-data-[collapsible=icon]:hidden">
				<Sidebar.Group>
					<Sidebar.GroupLabel>
						{#snippet child({ props })}
							<Collapsible.Trigger {...props} class="{props.class} w-full cursor-pointer">
								{t('menu.tabs.projects')}
								<ChevronRightIcon
									class="ms-auto size-4 transition-transform group-data-[state=open]/collapsible:rotate-90" />
							</Collapsible.Trigger>
						{/snippet}
					</Sidebar.GroupLabel>
					<Collapsible.Content class="lp-collapsible">
				<Sidebar.Menu onkeydown={treeKeys}>
					{#each projects.slice(0, PROJECT_LIMIT) as project (project.id)}
							<ProjectTreeItem {project} {path} onNavigate={closeOnPhone} />
						{/each}
						{#if projects.length > PROJECT_LIMIT}
						<Sidebar.MenuItem>
							<Sidebar.MenuButton class="text-sidebar-foreground/70">
								{#snippet child({ props })}
									<a href="/projects" onclick={closeOnPhone} {...props}>
										<EllipsisIcon />
										<span>{t('menu.sidebar.all_projects')}</span>
									</a>
								{/snippet}
							</Sidebar.MenuButton>
						</Sidebar.MenuItem>
					{/if}
				</Sidebar.Menu>
					</Collapsible.Content>
				</Sidebar.Group>
			</Collapsible.Root>
		{/if}
	</Sidebar.Content>

	<Sidebar.Footer>
		<DeploymentsIndicator />
		<UserMenu {user} {systemVersion} />
	</Sidebar.Footer>
	<Sidebar.Rail />
</Sidebar.Root>
