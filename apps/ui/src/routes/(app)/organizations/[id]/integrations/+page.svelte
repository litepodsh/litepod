<script lang="ts">
	import { onDestroy, onMount } from 'svelte';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import AssistantSettingsPanel from '#lib/components/organizations/assistant-settings-panel.svelte';
	import GitProvidersPanel from '#lib/components/organizations/git-providers-panel.svelte';
	import McpSettingsPanel from '#lib/components/organizations/mcp-settings-panel.svelte';
	import NotificationsPanel from '#lib/components/organizations/notifications/notifications-panel.svelte';
	import { getMe } from '#lib/clients/auth-client.js';
	import {
		getOrganization,
		listMembers,
		type OrganizationDto,
		type OrganizationMemberDto,
	} from '#lib/clients/organization-client.js';
	import {
		clearScopeHero,
		clearShellTabs,
		setScopeHero,
		setShellTabs,
	} from '#lib/state/shell.svelte.js';
	import { toast } from '#lib/utils/toast.js';
	import { tabIndexFromKeydown } from '#lib/utils/tab-shortcut.js';
	import BellIcon from '@lucide/svelte/icons/bell';
	import GitBranchIcon from '@lucide/svelte/icons/git-branch';
	import PlugIcon from '@lucide/svelte/icons/plug';
	import SparklesIcon from '@lucide/svelte/icons/sparkles';
	import { t } from '#lib/i18n/index.svelte.js';

	const organizationId = page.params.id!;

	let activeTab = $state(page.url.searchParams.get('tab') ?? 'git-providers');
	let organization = $state<OrganizationDto | null>(null);
	let members = $state<OrganizationMemberDto[]>([]);
	let myUserId = $state<string | null>(null);
	let isSysadmin = $state(false);
	let loading = $state(true);

	/** A SysAdmin manages any organization without being a member, which is what
	 *  the API enforces; anything else takes the Admin role (1) inside it. */
	const canManage = $derived(
		isSysadmin || (members.find((member) => member.user_id === myUserId)?.role_id ?? 99) <= 1,
	);

	const tabs = $derived([
		{ value: 'git-providers', label: t('menu.navigation.git_providers'), icon: GitBranchIcon },
		...(canManage
			? [
					{ value: 'assistant', label: t('organizations.detail.assistant'), icon: SparklesIcon },
					{ value: 'mcp', label: t('organizations.detail.mcp'), icon: PlugIcon },
				]
			: []),
		{ value: 'notifications', label: t('organizations.detail.notifications'), icon: BellIcon },
	]);

	/** A URL naming a tab this user cannot see falls back to the first one. */
	const visibleTab = $derived(
		tabs.some((tab) => tab.value === activeTab) ? activeTab : 'git-providers',
	);

	function setTab(value: string) {
		activeTab = value;
		const url = new URL(page.url.href);
		url.searchParams.set('tab', value);
		goto(url, { shallow: true, replace: true });
	}

	function handleTabShortcut(event: KeyboardEvent) {
		const index = tabIndexFromKeydown(event, tabs.length);
		if (index === null) return;
		event.preventDefault();
		setTab(tabs[index].value);
	}

	// The shell draws the title card and the tab capsule.
	$effect(() => {
		setScopeHero({ title: t('menu.tabs.integrations') });
		// The admin-only tabs depend on the members, so wait for them instead of shifting the row.
		if (!loading) setShellTabs(tabs, visibleTab, setTab);
	});
	onDestroy(() => {
		clearScopeHero();
		clearShellTabs();
	});

	onMount(async () => {
		try {
			const [me, loaded, loadedMembers] = await Promise.all([
				getMe(),
				getOrganization(organizationId),
				listMembers(organizationId),
			]);
			myUserId = me.user_id;
			isSysadmin = me.is_sysadmin;
			organization = loaded;
			members = loadedMembers;
		} catch (err) {
			toast.error(err instanceof Error ? err.message : t('organizations.detail.load_failed'));
		} finally {
			loading = false;
		}
	});
</script>

<svelte:window onkeydown={handleTabShortcut} />

<div class="flex w-full min-w-0 flex-col gap-5 px-4 py-6 sm:px-6">
	{#if loading}
		<div class="flex flex-col gap-3" aria-label={t('organizations.detail.loading')}>
			<Skeleton class="h-6 w-40" />
			<Skeleton class="h-24 w-full rounded-2xl" />
			<Skeleton class="h-24 w-full rounded-2xl" />
		</div>
	{:else if !organization}
		<p class="text-sm text-muted-foreground">{t('organizations.detail.not_found')}</p>
	{:else}
		<!-- Only the visible tab is mounted: each panel loads its data on mount. -->
		<div class="flex max-w-[960px] min-w-0 flex-col gap-4">
			{#if visibleTab === 'assistant'}
				<AssistantSettingsPanel {organizationId} bind:organization />
			{:else if visibleTab === 'mcp'}
				<McpSettingsPanel {organizationId} bind:organization />
			{:else if visibleTab === 'notifications'}
				<NotificationsPanel {organizationId} {canManage} />
			{:else}
				<GitProvidersPanel {organizationId} />
			{/if}
		</div>
	{/if}
</div>
