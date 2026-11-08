<script lang="ts">
	import { onDestroy, onMount } from 'svelte';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import { AnimatedTabs } from '#lib/components/custom/animated-tabs/index.js';
	import { groupSections } from '#lib/navigation/section-groups.js';
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
	import TwoFactorSettingsPanel from '#lib/components/organizations/two-factor-settings-panel.svelte';
	import BackupTargetsPanel from '#lib/components/organizations/backup-targets-panel.svelte';
	import InvitationsPanel from '#lib/components/users/invitations-panel.svelte';
	import RolesPanel from '#lib/components/access/roles-panel.svelte';
	import GroupsPanel from '#lib/components/access/groups-panel.svelte';
	import AccessPanel from '#lib/components/access/access-panel.svelte';
	import { allowed } from '#lib/state/permissions.svelte.js';
	import SsoPanel from '#lib/ee/sso/sso-panel.svelte';
	import { getRegistrationStatus } from '#lib/clients/auth-client.js';
	import {
		clearScopeHero,
		clearShellTabs,
		setScopeHero,
		setShellTabs,
	} from '#lib/state/shell.svelte.js';
	import PencilIcon from '@lucide/svelte/icons/pencil';
	import Trash2Icon from '@lucide/svelte/icons/trash-2';
	import UsersIcon from '@lucide/svelte/icons/users';
	import DatabaseBackupIcon from '@lucide/svelte/icons/database-backup';
	import ShieldCheckIcon from '@lucide/svelte/icons/shield-check';
	import { t } from '#lib/i18n/index.svelte.js';

	const organizationId = page.params.id!;
	/** Every tab `?tab=` may name. A value missing here silently reloads onto
	 *  People, which is what used to happen to `assistant`. */
	const TABS = ['members', 'access', 'authentication', 'backup-storage', 'audit-logs'];

	function tabFromUrl() {
		const t = page.url.searchParams.get('tab');
		if (t === 'invitations') return 'members';
		if (t === 'security') return 'authentication';
		return t && TABS.includes(t) ? t : 'members';
	}

	function setTab(value: string) {
		activeTab = value;
		const url = new URL(page.url.href);
		url.searchParams.set('tab', value);
		goto(url, { shallow: true, replace: true });
	}

	function handleTabShortcut(event: KeyboardEvent) {
		const index = tabIndexFromKeydown(event, sections.groups.length);
		if (index === null) return;
		event.preventDefault();
		selectGroup(sections.groups[index].value);
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

	const showSso = $derived(ssoAvailable && allowed('org.sso.manage'));

	const tabItems = $derived([
		{ value: 'members', label: t('organizations.access.members'), group: 'people' },
		{ value: 'access', label: t('organizations.access.access'), group: 'people' },
		...(canManage || showSso
			? [
					{
						value: 'authentication',
						label: t('organizations.sso.authentication'),
						group: 'security',
					},
				]
			: []),
		...(allowed('backup.targets.manage')
			? [
					{
						value: 'backup-storage',
						label: t('organizations.detail.backup_storage'),
						group: 'data',
					},
				]
			: []),
		...(canManage
			? [{ value: 'audit-logs', label: t('organizations.detail.audit_logs'), group: 'data' }]
			: []),
	]);

	/** `canManage` only resolves once members load, and the admin-only tabs do
	 *  not exist before it does. A URL naming a tab this user cannot see would
	 *  otherwise leave the page on a tab with nothing rendered under it, so the
	 *  view falls back to People while `activeTab` keeps the asked-for value. */
	const visibleTab = $derived(
		tabItems.some((item) => item.value === activeTab) ? activeTab : 'members',
	);

	const sections = $derived(
		groupSections(
			[
				{ value: 'people', label: t('organizations.detail.people'), icon: UsersIcon },
				{ value: 'security', label: t('organizations.detail.security'), icon: ShieldCheckIcon },
				{ value: 'data', label: t('organizations.detail.group_data'), icon: DatabaseBackupIcon },
			],
			tabItems,
			visibleTab,
		),
	);

	// Clicking the open group stays put instead of jumping back to its first section.
	function selectGroup(group: string) {
		if (group !== sections.group) setTab(tabItems.find((item) => item.group === group)!.value);
	}

	// The shell draws the title card and the tab capsule.
	$effect(() => {
		setScopeHero({
			title: t('organizations.detail.settings'),
			meta: heroMeta,
			actions: heroActions,
		});
		// The admin-only tabs depend on the members, so wait for them instead of shifting the row.
		if (organization) setShellTabs(sections.groups, sections.group, selectGroup);
	});
	onDestroy(() => {
		clearScopeHero();
		clearShellTabs();
	});

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

{#snippet heroActions()}
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

{#snippet heroMeta()}
	<!-- Reserved height, so nothing shifts when the organization loads. -->
	<span class="flex min-h-5 flex-wrap items-center gap-2">
		{#if organization}
			<span class="font-mono text-xs">{organization.slug}</span>
			{#if organization.plan}<span aria-hidden="true">·</span><span>{organization.plan}</span>{/if}
			{#if organization.is_root}
				<span
					class="lp-raised inline-flex h-5 items-center rounded-md bg-primary/10 px-1.5 text-[11px] font-medium text-primary"
					>{t('organizations.common.root')}</span>
			{/if}
		{/if}
	</span>
{/snippet}

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
		<div class="flex max-w-[960px] min-w-0 flex-col gap-4">
			{#if sections.items.length > 1}
				<AnimatedTabs
					value={visibleTab}
					onValueChange={setTab}
					items={sections.items}
					shortcuts={false} />
			{/if}
			<!-- Only the visible tab is mounted: each panel loads its data on mount, and
			     mounting them all at once fired ~35 requests, enough to trip the API's
			     per-IP burst limit on every reload. -->
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
			{:else if visibleTab === 'authentication'}
				<div class="flex flex-col gap-10">
					{#if canManage}
						<TwoFactorSettingsPanel
							{organizationId}
							bind:organization
							{members}
							{myUserId}
							{isSysadmin} />
					{/if}
					{#if showSso}
						<SsoPanel {organizationId} />
					{/if}
				</div>
			{:else if visibleTab === 'backup-storage'}
				<BackupTargetsPanel {organizationId} />
			{:else if visibleTab === 'audit-logs' && canManage}
				<MetricsRetentionSetting {organizationId} />
				<AuditLogsPanel {organizationId} bind:organization />
			{/if}
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
