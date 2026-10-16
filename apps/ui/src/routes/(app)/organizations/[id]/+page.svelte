<script lang="ts">
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
	import DeleteConfirmControl from '#lib/components/custom/delete-confirm-control.svelte';
	import { getMe } from '#lib/clients/auth-client.js';
	import {
		getOrganization,
		listMembers,
		deleteOrganization,
		type OrganizationDto,
		type OrganizationMemberDto,
	} from '#lib/clients/organization-client.js';
	import { toast } from '#lib/utils/toast.js';
	import { tabIndexFromKeydown } from '#lib/utils/tab-shortcut.js';
	import { cacheOrganizationName } from '#lib/state/page-header.svelte.js';
	import OrganizationEditDialog from '#lib/components/organizations/organization-edit-dialog.svelte';
	import MembersPanel from '#lib/components/organizations/members-panel.svelte';
	import AuditLogsPanel from '#lib/components/organizations/audit-logs-panel.svelte';
	import MetricsRetentionSetting from '#lib/components/settings/metrics-retention-setting.svelte';
	import AssistantSettingsPanel from '#lib/components/organizations/assistant-settings-panel.svelte';
	import McpSettingsPanel from '#lib/components/organizations/mcp-settings-panel.svelte';
	import BackupTargetsPanel from '#lib/components/organizations/backup-targets-panel.svelte';
	import NotificationsPanel from '#lib/components/organizations/notifications/notifications-panel.svelte';
	import InvitationsPanel from '#lib/components/users/invitations-panel.svelte';
	import RolesPanel from '#lib/components/access/roles-panel.svelte';
	import GroupsPanel from '#lib/components/access/groups-panel.svelte';
	import AccessPanel from '#lib/components/access/access-panel.svelte';
	import { allowed } from '#lib/state/permissions.svelte.js';
	import SsoPanel from '#lib/ee/sso/sso-panel.svelte';
	import { getRegistrationStatus } from '#lib/clients/auth-client.js';
	import SectionNav from '#lib/components/custom/section-nav.svelte';
	import GitProvidersPanel from '#lib/components/organizations/git-providers-panel.svelte';
	import PencilIcon from '@lucide/svelte/icons/pencil';
	import Trash2Icon from '@lucide/svelte/icons/trash-2';
	import UsersIcon from '@lucide/svelte/icons/users';
	import KeyRoundIcon from '@lucide/svelte/icons/key-round';
	import LockIcon from '@lucide/svelte/icons/lock';
	import BellIcon from '@lucide/svelte/icons/bell';
	import DatabaseBackupIcon from '@lucide/svelte/icons/database-backup';
	import SparklesIcon from '@lucide/svelte/icons/sparkles';
	import ScrollTextIcon from '@lucide/svelte/icons/scroll-text';
	import GitBranchIcon from '@lucide/svelte/icons/git-branch';
	import { t } from '#lib/i18n/index.svelte.js';

	const organizationId = page.params.id!;
	/** Every tab `?tab=` may name. A value missing here silently reloads onto
	 *  People, which is what used to happen to `assistant`. */
	const TABS = [
		'members',
		'access',
		'authentication',
		'notifications',
		'backup-storage',
		'assistant',
		'audit-logs',
		'git-providers',
	];

	function tabFromUrl() {
		const t = page.url.searchParams.get('tab');
		if (t === 'invitations') return 'members';
		return t && TABS.includes(t) ? t : 'members';
	}

	function setTab(value: string) {
		activeTab = value;
		const url = new URL(page.url.href);
		url.searchParams.set('tab', value);
		goto(url, { shallow: true, replace: true });
	}

	function handleTabShortcut(event: KeyboardEvent) {
		const index = tabIndexFromKeydown(event, tabItems.length);
		if (index === null) return;
		event.preventDefault();
		setTab(tabItems[index].value);
	}

	let activeTab = $state(tabFromUrl());
	let organization = $state<OrganizationDto | null>(null);
	let members = $state<OrganizationMemberDto[]>([]);
	let myUserId = $state<string | null>(null);
	let isSysadmin = $state(false);
	/** Single sign-on is self-hosted only; unknown until the edition loads. */
	let ssoAvailable = $state(false);
	let loading = $state(true);
	let confirmDeleteOrganization = $state(false);
	let deletingOrganization = $state(false);
	let editOpen = $state(false);

	/** A SysAdmin manages any organization without being a member, which is what
	 *  the API enforces; anything else takes the Admin role (1) inside it. */
	const canManage = $derived(
		isSysadmin || (members.find((member) => member.user_id === myUserId)?.role_id ?? 99) <= 1,
	);

	const tabItems = $derived([
		{ value: 'members', label: t('organizations.detail.people'), icon: UsersIcon },
		{ value: 'access', label: t('organizations.access.access'), icon: KeyRoundIcon },
		...(ssoAvailable && allowed('org.sso.manage')
			? [{ value: 'authentication', label: t('organizations.sso.authentication'), icon: LockIcon }]
			: []),
		{ value: 'notifications', label: t('organizations.detail.notifications'), icon: BellIcon },
		...(allowed('backup.targets.manage')
			? [
					{
						value: 'backup-storage',
						label: t('organizations.detail.backup_storage'),
						icon: DatabaseBackupIcon,
					},
				]
			: []),
		...(canManage
			? [
					{ value: 'assistant', label: t('organizations.detail.assistant'), icon: SparklesIcon },
					{
						value: 'audit-logs',
						label: t('organizations.detail.audit_logs'),
						icon: ScrollTextIcon,
					},
				]
			: []),
		{
			value: 'git-providers',
			label: t('menu.navigation.git_providers'),
			icon: GitBranchIcon,
			separated: true,
		},
	]);

	/** `canManage` only resolves once members load, and the admin-only tabs do
	 *  not exist before it does. A URL naming a tab this user cannot see would
	 *  otherwise leave the page on a tab with nothing rendered under it, so the
	 *  view falls back to People while `activeTab` keeps the asked-for value. */
	const visibleTab = $derived(
		tabItems.some((item) => item.value === activeTab) ? activeTab : 'members',
	);

	onMount(async () => {
		try {
			const [me, t, m] = await Promise.all([
				getMe(),
				getOrganization(organizationId),
				listMembers(organizationId),
			]);
			myUserId = me.user_id;
			isSysadmin = me.is_sysadmin;
			organization = t;
			cacheOrganizationName(t.id, t.name);
			members = m;
			getRegistrationStatus()
				.then((status) => (ssoAvailable = !status.is_cloud))
				.catch(() => {});
		} catch (err) {
			toast.error(err instanceof Error ? err.message : t('organizations.detail.load_failed'));
		} finally {
			loading = false;
		}
	});

	function applyUpdate(updated: OrganizationDto) {
		organization = updated;
		cacheOrganizationName(updated.id, updated.name);
	}

	async function handleDeleteOrganization() {
		if (!organization) return;
		deletingOrganization = true;
		try {
			await deleteOrganization(organization.id);
			toast.success(t('organizations.detail.deleted'));
			// Full-load nav so the persistent sidebar refetches its organization list.
			window.location.assign('/organizations');
		} catch (err) {
			toast.error(err instanceof Error ? err.message : t('organizations.detail.delete_failed'));
			deletingOrganization = false;
			confirmDeleteOrganization = false;
		}
	}
</script>

<svelte:window onkeydown={handleTabShortcut} />

{#snippet headerActions()}
	{#if canManage}
		<Button variant="outline" size="sm" onclick={() => (editOpen = true)}>
			<PencilIcon class="size-4" />
			{t('organizations.common.edit')}
		</Button>
		<!-- The root organization is the instance itself; the API refuses to delete it. -->
		{#if organization && !organization.is_root}
			<Button
				variant="outline"
				size="sm"
				class="text-destructive hover:text-destructive"
				disabled={deletingOrganization}
				onclick={() => (confirmDeleteOrganization = true)}>
				<Trash2Icon class="size-4" />
				{t('organizations.detail.delete')}
			</Button>
		{/if}
	{/if}
{/snippet}

<div class="flex w-full min-w-0 flex-col gap-5 px-4 py-6 sm:px-6">
	<!-- Static title, so nothing shifts when the organization loads. -->
	<div class="flex min-h-9 flex-wrap items-end justify-between gap-4">
		<div class="min-w-0">
			<h1 class="text-xl font-semibold tracking-tight">{t('organizations.detail.settings')}</h1>
			<p class="flex min-h-5 flex-wrap items-center gap-2 text-sm text-muted-foreground">
				{#if organization}
					<span class="font-mono text-xs">{organization.slug}</span>
					{#if organization.plan}<span aria-hidden="true">·</span><span>{organization.plan}</span
						>{/if}
					{#if organization.is_root}
						<span
							class="lp-raised inline-flex h-5 items-center rounded-md bg-primary/10 px-1.5 text-[11px] font-medium text-primary"
							>{t('organizations.common.root')}</span>
					{/if}
				{/if}
			</p>
		</div>
		<div class="flex flex-wrap items-center gap-2">{@render headerActions()}</div>
	</div>

	{#if loading}
		<div class="flex flex-col gap-6 lg:flex-row" aria-label={t('organizations.detail.loading')}>
			<div class="flex gap-2 lg:w-52 lg:flex-col">
				{#each { length: 5 } as _, index (index)}
					<Skeleton class="h-8 w-24 rounded-lg lg:w-full" />
				{/each}
			</div>
			<div class="flex flex-1 flex-col gap-3">
				<Skeleton class="h-6 w-40" />
				<Skeleton class="h-24 w-full rounded-2xl" />
				<Skeleton class="h-24 w-full rounded-2xl" />
			</div>
		</div>
	{:else if !organization}
		<p class="text-sm text-muted-foreground">{t('organizations.detail.not_found')}</p>
	{:else}
		<div class="flex flex-col gap-6 lg:flex-row lg:items-start">
			<SectionNav
				items={tabItems}
				current={visibleTab}
				label={t('organizations.detail.settings')}
				onselect={setTab} />

			<!-- Only the visible tab is mounted: each panel loads its data on mount, and
			     mounting them all at once fired ~35 requests, enough to trip the API's
			     per-IP burst limit on every reload. -->
			<div class="flex max-w-[960px] min-w-0 flex-1 flex-col gap-4">
				{#if visibleTab === 'members'}
					<div class="flex flex-col gap-8">
						<div>
							<h2 class="text-lg font-semibold tracking-tight">
								{t('organizations.detail.people')}
							</h2>
							<p class="mt-1 text-sm text-muted-foreground">
								{t('organizations.detail.people_hint')}
							</p>
						</div>
						<MembersPanel
							{organizationId}
							bind:members
							{isSysadmin}
							{myUserId}
							canManageMembers={allowed('org.members.manage')} />
						{#if canManage}
							<InvitationsPanel {organizationId} organizationName={organization.name} />
						{/if}
					</div>
				{:else if visibleTab === 'access'}
					<div class="flex flex-col gap-10">
						<AccessPanel
							{organizationId}
							scopeType="org"
							scopeId={organizationId}
							knownMembers={members} />
						<GroupsPanel {organizationId} {members} canManage={allowed('org.members.manage')} />
						<RolesPanel {organizationId} canManage={allowed('org.roles.manage')} />
					</div>
				{:else if visibleTab === 'authentication' && ssoAvailable}
					<SsoPanel {organizationId} />
				{:else if visibleTab === 'notifications'}
					<NotificationsPanel {organizationId} {canManage} />
				{:else if visibleTab === 'backup-storage'}
					<BackupTargetsPanel {organizationId} />
				{:else if visibleTab === 'assistant' && canManage}
					<AssistantSettingsPanel {organizationId} bind:organization />
					<McpSettingsPanel {organizationId} bind:organization />
				{:else if visibleTab === 'audit-logs' && canManage}
					<MetricsRetentionSetting {organizationId} />
					<AuditLogsPanel {organizationId} bind:organization />
				{:else if visibleTab === 'git-providers'}
					<GitProvidersPanel {organizationId} />
				{/if}
			</div>
		</div>
	{/if}
</div>

<OrganizationEditDialog bind:open={editOpen} {organization} onSaved={applyUpdate} />

<AlertDialog.Root
	open={confirmDeleteOrganization}
	onOpenChange={(o) => (confirmDeleteOrganization = o)}>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title>{t('organizations.detail.delete_confirm')}</AlertDialog.Title>
			<AlertDialog.Description>
				{organization ? t('organizations.detail.delete_hint', { name: organization.name }) : ''}
			</AlertDialog.Description>
		</AlertDialog.Header>
		<AlertDialog.Footer>
			<AlertDialog.Cancel>{t('organizations.common.cancel')}</AlertDialog.Cancel>
			<DeleteConfirmControl disabled={deletingOrganization} onconfirm={handleDeleteOrganization} />
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>
