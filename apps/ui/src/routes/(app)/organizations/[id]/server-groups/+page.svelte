<script lang="ts">
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import BoxesIcon from '@lucide/svelte/icons/boxes';
	import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
	import DeleteConfirmControl from '#lib/components/custom/delete-confirm-control.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Empty from '#lib/components/ui/empty/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import ServerGroupCard from '#lib/components/server-groups/server-group-card.svelte';
	import ServerGroupDialog from '#lib/components/server-groups/server-group-dialog.svelte';
	import { getMe } from '#lib/clients/auth-client.js';
	import { listMembers } from '#lib/clients/organization-client.js';
	import { listReadyServers, type ServerSummaryDto } from '#lib/clients/project-client.js';
	import {
		deleteServerGroup,
		listServerGroups,
		type ServerGroup,
	} from '#lib/clients/server-group-client.js';
	import { t } from '#lib/i18n/index.svelte.js';
	import { toast } from '#lib/utils/toast.js';

	const organizationId = page.params.id!;

	let groups = $state<ServerGroup[]>([]);
	let servers = $state<ServerSummaryDto[]>([]);
	let canManage = $state(false);
	let loading = $state(true);
	let error = $state('');

	let dialogOpen = $state(false);
	let editingGroup = $state<ServerGroup | null>(null);
	let pendingDelete = $state<ServerGroup | null>(null);
	let busy = $state(false);

	async function refresh() {
		groups = await listServerGroups(organizationId);
	}

	onMount(() => {
		void (async () => {
			try {
				const me = await getMe();
				canManage =
					me.is_sysadmin ||
					(await listMembers(organizationId)).some(
						(member) => member.user_id === me.user_id && member.role_id <= 1,
					);
				await refresh();
				servers = (await listReadyServers(organizationId)).filter(
					(server) => server.kind === 'ssh',
				);
			} catch (caught) {
				error =
					caught instanceof Error ? caught.message : t('organizations.server_groups.load_failed');
			} finally {
				loading = false;
			}
		})();
	});

	function create() {
		editingGroup = null;
		dialogOpen = true;
	}

	function edit(group: ServerGroup) {
		editingGroup = group;
		dialogOpen = true;
	}

	function askDelete(group: ServerGroup) {
		pendingDelete = group;
	}

	async function confirmDelete() {
		const target = pendingDelete;
		if (!target) return;
		busy = true;
		try {
			await deleteServerGroup(organizationId, target.id);
			pendingDelete = null;
			await refresh();
			toast.success(t('organizations.server_groups.deleted_toast', { name: target.name }));
		} catch (caught) {
			toast.error(
				caught instanceof Error ? caught.message : t('organizations.server_groups.delete_failed'),
			);
		} finally {
			busy = false;
		}
	}
</script>

<div class="flex flex-col gap-5 px-4 py-6 sm:px-6">
	<div class="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
		<div>
			<h1 class="text-xl font-semibold tracking-tight">{t('organizations.server_groups.title')}</h1>
			<p class="text-sm text-muted-foreground">
				{t('organizations.server_groups.description')}
			</p>
		</div>
		{#if !error && canManage}
			<Button onclick={create}>
				<PlusIcon class="size-4" />
				{t('organizations.server_groups.create')}
			</Button>
		{/if}
	</div>

	{#if loading}
		<div class="grid gap-4">
			{#each [0, 1] as placeholder (placeholder)}
				<Skeleton class="h-24 w-full rounded-2xl" />
			{/each}
		</div>
	{:else if error}
		<p
			class="rounded-2xl border border-destructive/30 bg-card p-6 text-sm text-destructive">
			{error}
		</p>
	{:else if groups.length === 0}
		<Empty.Root class="rounded-2xl border border-dashed border-foreground/15 bg-card">
			<Empty.Header>
				<Empty.Media variant="icon">
					<BoxesIcon />
				</Empty.Media>
				<Empty.Title>{t('organizations.server_groups.empty')}</Empty.Title>
				<Empty.Description>{t('organizations.server_groups.description')}</Empty.Description>
			</Empty.Header>
			{#if canManage}
				<Empty.Content>
					<Button onclick={create}>
						<PlusIcon class="size-4" />
						{t('organizations.server_groups.create')}
					</Button>
				</Empty.Content>
			{/if}
		</Empty.Root>
	{:else}
		<div class="space-y-4">
			{#each groups as group, index (group.id)}
				<div
					class="animate-in fade-in slide-in-from-top-2 fill-mode-both duration-300"
					style="animation-delay: {index * 60}ms">
					<ServerGroupCard {group} {canManage} onEdit={edit} onDelete={askDelete} />
				</div>
			{/each}
		</div>
	{/if}
</div>

<ServerGroupDialog
	bind:open={dialogOpen}
	{organizationId}
	group={editingGroup}
	{servers}
	onSaved={() => void refresh()} />

<AlertDialog.Root
	open={Boolean(pendingDelete)}
	onOpenChange={(next) => {
		if (!next) pendingDelete = null;
	}}>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title
				>{t('organizations.server_groups.remove_confirm', {
					name: pendingDelete?.name ?? '',
				})}</AlertDialog.Title>
			<AlertDialog.Description
				>{t('organizations.server_groups.remove_hint')}</AlertDialog.Description>
		</AlertDialog.Header>
		<AlertDialog.Footer>
			<AlertDialog.Cancel>{t('organizations.common.cancel')}</AlertDialog.Cancel>
			<DeleteConfirmControl
				label={t('organizations.server_groups.remove')}
				pendingLabel={t('organizations.server_groups.deleting')}
				disabled={busy}
				onconfirm={confirmDelete} />
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>
