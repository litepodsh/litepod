<script lang="ts">
	import { goto } from '$app/navigation';
	import type { Component } from 'svelte';
	import Fuse, { type FuseResult } from 'fuse.js';
	import * as Command from '#lib/components/ui/command/index.js';
	import * as Kbd from '#lib/components/ui/kbd/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import ProjectIcon from '#lib/components/projects/project-icon.svelte';
	import ApplicationIcon from '#lib/components/projects/application-icon.svelte';
	import { getMe } from '#lib/clients/auth-client.js';
	import { getSystemVersion } from '#lib/clients/system-client.js';
	import { listOrganizations } from '#lib/clients/organization-client.js';
	import { listProjects, getProject } from '#lib/clients/project-client.js';
	import {
		getSelectedOrganizationId,
		resolveSelectedOrganization,
	} from '#lib/utils/selected-organization.js';
	import { userPrefersMode } from 'mode-watcher';
	import { switchThemeMode } from '#lib/utils/theme-transition.js';
	import MoonIcon from '@lucide/svelte/icons/moon';
	import SunIcon from '@lucide/svelte/icons/sun';
	import {
		buildProjectSearchIndex,
		type ProjectSearchEntry,
	} from '#lib/utils/project-search-index.js';
	import { highlightMatches } from '#lib/utils/highlight-matches.js';
	import { environmentApplicationsHref } from '#lib/utils/environment-route.js';
	import { commandPaletteState } from '#lib/state/command-palette.svelte.js';
	import { setAssistantMode } from '#lib/state/assistant.svelte.js';
	import HouseIcon from '@lucide/svelte/icons/house';
	import FrameIcon from '@lucide/svelte/icons/frame';
	import GitBranchIcon from '@lucide/svelte/icons/git-branch';
	import ListTodoIcon from '@lucide/svelte/icons/list-todo';
	import BugIcon from '@lucide/svelte/icons/bug';
	import ServerCogIcon from '@lucide/svelte/icons/server-cog';
	import ServerIcon from '@lucide/svelte/icons/server';
	import SettingsIcon from '@lucide/svelte/icons/settings';
	import BadgeCheckIcon from '@lucide/svelte/icons/badge-check';
	import CreditCardIcon from '@lucide/svelte/icons/credit-card';
	import LogOutIcon from '@lucide/svelte/icons/log-out';
	import BoxesIcon from '@lucide/svelte/icons/boxes';
	import SparklesIcon from '@lucide/svelte/icons/sparkles';
	import BellIcon from '@lucide/svelte/icons/bell';
	import Building2Icon from '@lucide/svelte/icons/building-2';
	import ContainerIcon from '@lucide/svelte/icons/container';
	import DatabaseBackupIcon from '@lucide/svelte/icons/database-backup';
	import ShieldCheckIcon from '@lucide/svelte/icons/shield-check';
	import GlobeIcon from '@lucide/svelte/icons/globe';
	import ChartColumnIcon from '@lucide/svelte/icons/chart-column';
	import WaypointsIcon from '@lucide/svelte/icons/waypoints';
	import CalendarClockIcon from '@lucide/svelte/icons/calendar-clock';
	import ScrollTextIcon from '@lucide/svelte/icons/scroll-text';
	import NetworkIcon from '@lucide/svelte/icons/network';
	import { t } from '#lib/i18n/index.svelte.js';

	let query = $state('');
	let isSysadmin = $state(false);
	let isCloud = $state(false);
	let selectedOrganizationId = $state<string | null>(null);
	let assistantAvailable = $state(false);
	let projectEntries = $state<ProjectSearchEntry[]>([]);
	let failedIconApplicationIds = $state(new Set<string>());
	let lastOrgId: string | null = null;
	let loadedProjectOrgId: string | null = null;
	let meAndSystemLoaded = false;

	$effect(() => {
		if (!commandPaletteState.open) {
			query = '';
			return;
		}
		void loadMeAndSystem();
		const currentOrgId = getSelectedOrganizationId();

		if (currentOrgId !== lastOrgId) {
			assistantAvailable = false;
			projectEntries = [];
			loadedProjectOrgId = null;
			lastOrgId = currentOrgId;
			void refreshOrganizationContext().then(() => {
				if (currentOrgId && currentOrgId === lastOrgId && selectedOrganizationId === currentOrgId) {
					void loadProjectsForCurrentOrg(currentOrgId);
				}
			});
			return;
		}

		if (currentOrgId && currentOrgId !== loadedProjectOrgId) {
			loadedProjectOrgId = currentOrgId;
			void loadProjectsForCurrentOrg(currentOrgId);
		}
	});

	async function refreshOrganizationContext() {
		try {
			const organizations = await listOrganizations();
			const organization = resolveSelectedOrganization(organizations);
			if (!organization) {
				selectedOrganizationId = null;
				assistantAvailable = false;
				return;
			}
			selectedOrganizationId = organization.id;
			assistantAvailable = organization.assistant_enabled ?? false;
		} catch {
			assistantAvailable = false;
		}
	}

	async function loadProjectsForCurrentOrg(orgId: string) {
		try {
			const projects = await listProjects(orgId);
			const details = await Promise.all(projects.map((project) => getProject(orgId, project.id)));
			projectEntries = buildProjectSearchIndex(details);
		} catch {
			projectEntries = [];
		}
	}

	async function loadMeAndSystem() {
		if (meAndSystemLoaded) return;
		meAndSystemLoaded = true;
		try {
			const [me, systemVersion] = await Promise.all([getMe(), getSystemVersion()]);
			isSysadmin = me.is_sysadmin;
			isCloud = systemVersion.is_cloud;
		} catch {
			meAndSystemLoaded = false;
			// Palette can open before an authenticated session is available.
		}
	}

	function handleKeydown(event: KeyboardEvent) {
		if (event.key.toLowerCase() !== 'k' || !(event.metaKey || event.ctrlKey)) return;
		event.preventDefault();
		commandPaletteState.open = !commandPaletteState.open;
	}

	function closeAndReset() {
		commandPaletteState.open = false;
		query = '';
	}

	function handleAssistantSelect() {
		setAssistantMode('chat');
		closeAndReset();
	}

	function handleThemeChange(nextMode: 'light' | 'dark', event?: MouseEvent) {
		if (userPrefersMode.current === nextMode) return;

		const iconRect = document
			.querySelector(`.command-palette [data-theme-icon='${nextMode}']`)
			?.getBoundingClientRect();
		const paletteRect = document.querySelector('.command-palette')?.getBoundingClientRect();
		const originRect = iconRect ?? paletteRect;
		const x = originRect
			? originRect.left + originRect.width / 2
			: (event?.clientX ?? window.innerWidth / 2);
		const y = originRect
			? originRect.top + originRect.height / 2
			: (event?.clientY ?? window.innerHeight / 2);

		switchThemeMode(nextMode, { x, y }, closeAndReset);
	}

	// One logout path: /logout also ends the identity provider's session.
	function handleLogout() {
		closeAndReset();
		goto('/logout');
	}

	// Every org page the header tabs and the user menu link to, so search reaches all of them.
	function organizationItems(organizationId: string | null) {
		if (!organizationId) return [];
		const base = `/organizations/${encodeURIComponent(organizationId)}`;
		return [
			{ title: t('menu.tabs.apps'), url: `${base}/apps`, icon: BoxesIcon },
			{ title: t('menu.navigation.deployment_queue'), url: `${base}/deployment-queue`, icon: ListTodoIcon },
			{ title: t('menu.tabs.domains'), url: `${base}/domains`, icon: GlobeIcon },
			{ title: t('menu.tabs.analytics'), url: `${base}/analytics`, icon: ChartColumnIcon },
			{ title: t('menu.navigation.servers'), url: `${base}/servers`, icon: ServerIcon },
			{ title: t('menu.navigation.server_groups'), url: `${base}/server-groups`, icon: BoxesIcon },
			{ title: t('menu.navigation.map'), url: `${base}/topology`, icon: WaypointsIcon },
			{ title: t('menu.navigation.scheduled_tasks'), url: `${base}/scheduled-tasks`, icon: CalendarClockIcon },
			{ title: t('menu.navigation.organization'), url: base, icon: Building2Icon },
			{ title: t('menu.navigation.audit_logs'), url: `${base}?tab=audit-logs`, icon: ScrollTextIcon },
			{ title: t('menu.navigation.git_providers'), url: `${base}?tab=git-providers`, icon: GitBranchIcon },
			{ title: t('menu.navigation.notifications'), url: `${base}?tab=notifications`, icon: BellIcon },
		];
	}

	type NavItem = { title: string; icon: Component; url?: string; onSelect?: () => void };

	const navItems = $derived<NavItem[]>([
		{ title: t('menu.navigation.home'), url: '/home', icon: HouseIcon },
		...(assistantAvailable
			? [
					{
						title: t('menu.command_palette.ai_assistant'),
						icon: SparklesIcon,
						onSelect: handleAssistantSelect,
					},
				]
			: []),
		{ title: t('menu.navigation.projects'), url: '/projects', icon: FrameIcon },
		...organizationItems(selectedOrganizationId),
		{ title: t('menu.navigation.organizations'), url: '/organizations', icon: Building2Icon },
		...(isSysadmin
			? [
					{ title: t('menu.navigation.system'), url: '/system', icon: ContainerIcon },
					{
						title: t('menu.navigation.containers'),
						url: '/system?tab=containers',
						icon: ContainerIcon,
					},
					{ title: t('menu.navigation.caddy'), url: '/system?tab=caddy', icon: NetworkIcon },
					{
						title: t('menu.user_menu.maintenance'),
						url: '/system?tab=maintenance',
						icon: ServerCogIcon,
					},
					{ title: t('menu.user_menu.debug'), url: '/system?tab=debug', icon: BugIcon },
					{
						title: t('menu.navigation.backups'),
						url: '/system?tab=backups',
						icon: DatabaseBackupIcon,
					},
					...(isCloud
						? [{ title: t('menu.navigation.admin'), url: '/admin', icon: ShieldCheckIcon }]
						: []),
				]
			: []),
		...(!isSysadmin && isCloud
			? [{ title: t('menu.navigation.subscription'), url: '/billing', icon: CreditCardIcon }]
			: []),
		{ title: t('menu.user_menu.settings'), url: '/settings', icon: SettingsIcon },
		{ title: t('menu.user_menu.account'), url: '/account', icon: BadgeCheckIcon },
	]);

	const navFuse = $derived(
		new Fuse(navItems, { threshold: 0.4, ignoreLocation: true, keys: ['title'] }),
	);
	const navResults = $derived(
		query.trim() ? navFuse.search(query.trim()).map((result) => result.item) : navItems,
	);

	const actionItems = $derived([
		{ title: t('menu.user_menu.log_out'), icon: LogOutIcon, onSelect: handleLogout },
	]);
	const actionResults = $derived(
		query.trim()
			? actionItems.filter((item) => item.title.toLowerCase().includes(query.trim().toLowerCase()))
			: actionItems,
	);

	const appearanceItems = $derived([
		{
			title: t('menu.command_palette.switch_light'),
			mode: 'light' as const,
			icon: SunIcon,
			disabled: userPrefersMode.current === 'light',
		},
		{
			title: t('menu.command_palette.switch_dark'),
			mode: 'dark' as const,
			icon: MoonIcon,
			disabled: userPrefersMode.current === 'dark',
		},
	]);
	const appearanceResults = $derived(
		query.trim()
			? appearanceItems.filter((item) =>
					item.title.toLowerCase().includes(query.trim().toLowerCase()),
				)
			: appearanceItems,
	);

	const projectFuse = $derived(
		new Fuse(projectEntries, {
			includeMatches: true,
			threshold: 0.35,
			ignoreLocation: true,
			keys: [
				{ name: 'project.name', weight: 2 },
				{ name: 'project.description', weight: 1 },
				{ name: 'application.name', weight: 2 },
				{ name: 'application.description', weight: 1 },
				{ name: 'environment.name', weight: 0.5 },
			],
		}),
	);
	const projectResults = $derived(query.trim() ? projectFuse.search(query.trim()).slice(0, 8) : []);

	function matchIndices(result: FuseResult<ProjectSearchEntry>, key: string) {
		return result.matches?.find((match) => match.key === key)?.indices;
	}

	function hrefFor(entry: ProjectSearchEntry) {
		return entry.type === 'project'
			? `/projects/${encodeURIComponent(entry.project.id)}`
			: `${environmentApplicationsHref(entry.project.id, entry.environment.name)}/${encodeURIComponent(entry.application.id)}`;
	}
</script>

<svelte:window onkeydown={handleKeydown} />

<Command.Dialog
	bind:open={commandPaletteState.open}
	shouldFilter={false}
	class="command-palette top-[16vh] w-[min(42rem,calc(100%-2rem))] max-w-none rounded-[20px]! p-0 shadow-2xl"
	title={t('menu.command_palette.title')}
	description={t('menu.command_palette.description')}>
	<div class="command-palette-header">
		<Command.Input bind:value={query} placeholder={t('menu.command_palette.search')} />
	</div>
	<Command.List>
		<Command.Empty>{t('menu.command_palette.no_results')}</Command.Empty>

		{#if navResults.length}
			<Command.Group heading={t('menu.command_palette.navigation')}>
				{#each navResults as item (item.onSelect ? item.title : item.url)}
					{#if item.onSelect}
						<Command.Item onSelect={item.onSelect}>
							<item.icon class="size-4 text-muted-foreground" />
							{item.title}
						</Command.Item>
					{:else}
						<Command.LinkItem href={item.url} onSelect={closeAndReset}>
							<item.icon class="size-4 text-muted-foreground" />
							{item.title}
						</Command.LinkItem>
					{/if}
				{/each}
			</Command.Group>
		{/if}

		{#if projectResults.length}
			<Command.Group heading={t('menu.command_palette.projects')}>
				{#each projectResults as result (result.item.id)}
					{@const entry = result.item}
					<Command.LinkItem href={hrefFor(entry)} onSelect={closeAndReset}>
						{#if entry.type === 'project'}
							<ProjectIcon iconUrl={entry.project.icon_url} />
							<span class="min-w-0 flex-1 truncate">
								{#each highlightMatches(entry.project.name, matchIndices(result, 'project.name')) as segment}
									{#if segment.highlighted}<mark class="bg-primary/20 text-foreground rounded-sm"
											>{segment.text}</mark
										>{:else}{segment.text}{/if}
								{/each}
							</span>
						{:else}
							<ApplicationIcon
								templateKind={entry.application.template_kind}
								catalogIconUrl={entry.application.catalog_icon_url}
								iconUrl={entry.application.icon_url}
								applicationId={entry.application.id}
								bind:failedApplicationIds={failedIconApplicationIds}
								class="size-4 shrink-0 rounded-[4px]" />
							<span class="min-w-0 flex-1 truncate">
								{#each highlightMatches(entry.application.name, matchIndices(result, 'application.name')) as segment}
									{#if segment.highlighted}<mark class="bg-primary/20 text-foreground rounded-sm"
											>{segment.text}</mark
										>{:else}{segment.text}{/if}
								{/each}
								<span class="text-muted-foreground"> · {entry.project.name}</span>
							</span>
							<Badge variant={entry.environment.is_production ? 'default' : 'secondary'}>
								{entry.environment.name}
							</Badge>
						{/if}
					</Command.LinkItem>
				{/each}
			</Command.Group>
		{/if}

		{#if appearanceResults.length}
			<Command.Group heading={t('menu.command_palette.appearance')}>
				{#each appearanceResults as item (item.mode)}
					<Command.Item
						disabled={item.disabled}
						onclick={(event: MouseEvent) => {
							event.stopPropagation();
							handleThemeChange(item.mode, event);
						}}
						onSelect={() => handleThemeChange(item.mode)}>
						<item.icon data-theme-icon={item.mode} class="size-4 text-muted-foreground" />
						<span class="min-w-0 flex-1 truncate">{item.title}</span>
						{#if item.disabled}
							<Command.Shortcut>{t('menu.command_palette.current')}</Command.Shortcut>
						{/if}
					</Command.Item>
				{/each}
			</Command.Group>
		{/if}

		{#if actionResults.length}
			<Command.Group heading={t('menu.command_palette.actions')}>
				{#each actionResults as action (action.title)}
					<Command.Item onSelect={action.onSelect}>
						<action.icon class="size-4 text-muted-foreground" />
						{action.title}
					</Command.Item>
				{/each}
			</Command.Group>
		{/if}
	</Command.List>
	<footer class="command-palette-footer" aria-label={t('menu.command_palette.title')}>
		<span><Kbd.Kbd>↑</Kbd.Kbd><Kbd.Kbd>↓</Kbd.Kbd> {t('menu.command_palette.navigation')}</span>
		<span><Kbd.Kbd>↵</Kbd.Kbd> {t('menu.command_palette.select')}</span>
		<span><Kbd.Kbd>Esc</Kbd.Kbd> {t('menu.command_palette.close')}</span>
	</footer>
</Command.Dialog>

<style>
	:global(.command-palette [data-slot='command']) {
		border: 1px solid color-mix(in oklch, var(--border) 82%, transparent);
		border-radius: 20px;
		background: var(--popover);
		padding: 0;
	}

	:global(.command-palette [data-slot='command-input-wrapper']) {
		border-bottom: 1px solid var(--border);
		padding: 0.75rem;
	}

	:global(.command-palette .command-palette-header) {
		display: flex;
		align-items: center;
		gap: 0.5rem;
	}

	:global(.command-palette .command-palette-header [data-slot='command-input-wrapper']) {
		flex: 1 1 auto;
		min-width: 0;
	}

	:global(.command-palette [data-slot='input-group']) {
		height: 2.75rem !important;
		border-radius: 0.75rem;
		background: color-mix(in oklch, var(--muted) 72%, transparent);
	}

	:global(.command-palette [data-slot='command-list']) {
		max-height: min(24rem, 55vh);
		padding: 0.35rem;
	}

	:global(.command-palette [data-slot='command-group']) {
		padding: 0.25rem;
	}

	:global(.command-palette [data-command-group-heading]) {
		padding: 0.6rem 0.65rem 0.35rem;
		color: var(--muted-foreground);
		font-size: 0.6875rem;
		font-weight: 700;
		letter-spacing: 0.08em;
		text-transform: uppercase;
	}

	:global(.command-palette [data-slot='command-item']) {
		min-height: 1.9rem;
		border-radius: 0.625rem;
		padding: 0.25rem 0.6rem;
	}

	:global(.command-palette [data-slot='command-item'][aria-selected='true']),
	:global(.command-palette [data-slot='command-item'][data-selected='true']) {
		background: var(--primary);
		color: var(--primary-foreground);
	}

	:global(.command-palette [data-slot='command-item'][aria-selected='true'] svg),
	:global(.command-palette [data-slot='command-item'][data-selected='true'] svg) {
		color: var(--primary-foreground);
	}

	:global(.command-palette [data-slot='command-item'][aria-selected='true'] mark),
	:global(.command-palette [data-slot='command-item'][data-selected='true'] mark) {
		background: oklch(1 0 0 / 25%);
		color: inherit;
	}

	:global(.command-palette .command-palette-footer) {
		display: flex;
		align-items: center;
		justify-content: flex-end;
		gap: 1rem;
		border-top: 1px solid var(--border);
		padding: 0.65rem 0.9rem;
		color: var(--muted-foreground);
		font-size: 0.75rem;
	}

	:global(.command-palette .command-palette-footer span) {
		display: inline-flex;
		align-items: center;
		gap: 0.25rem;
	}
</style>
