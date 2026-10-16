<script lang="ts">
	import { onMount } from 'svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
	import DeleteConfirmControl from '#lib/components/custom/delete-confirm-control.svelte';
	import SoftCheckbox from '#lib/components/custom/soft-checkbox.svelte';
	import SkeletonRows from '#lib/components/custom/skeleton-rows.svelte';
	import UsersIcon from '@lucide/svelte/icons/users';
	import Trash2Icon from '@lucide/svelte/icons/trash-2';
	import {
		createGroup,
		deleteGroup,
		listGroups,
		setGroupMembers,
		type Group,
	} from '#lib/clients/access-client.js';
	import type { OrganizationMemberDto } from '#lib/clients/organization-client.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let {
		organizationId,
		members,
		canManage,
	}: { organizationId: string; members: OrganizationMemberDto[]; canManage: boolean } = $props();

	let groups = $state<Group[]>([]);
	let loading = $state(true);
	let newName = $state('');
	let busy = $state(false);
	let editing = $state<Group | null>(null);
	let picked = $state<Set<string>>(new Set());
	let toDelete = $state<Group | null>(null);

	onMount(async () => {
		try {
			groups = await listGroups(organizationId);
		} catch (err) {
			toast.error(err instanceof Error ? err.message : t('organizations.access.load_failed'));
		} finally {
			loading = false;
		}
	});

	function fail(err: unknown) {
		toast.error(err instanceof Error ? err.message : t('organizations.access.save_failed'));
	}

	async function create(event: SubmitEvent) {
		event.preventDefault();
		busy = true;
		try {
			groups = [...groups, await createGroup(organizationId, newName)];
			newName = '';
		} catch (err) {
			fail(err);
		} finally {
			busy = false;
		}
	}

	function openMembers(group: Group) {
		editing = group;
		picked = new Set(group.members.map((m) => m.user_id));
	}

	function toggle(userId: string, on: boolean) {
		const next = new Set(picked);
		if (on) next.add(userId);
		else next.delete(userId);
		picked = next;
	}

	async function saveMembers() {
		if (!editing) return;
		busy = true;
		try {
			const updated = await setGroupMembers(organizationId, editing.id, [...picked]);
			groups = groups.map((g) => (g.id === updated.id ? updated : g));
			toast.success(t('organizations.access.group_saved'));
			editing = null;
		} catch (err) {
			fail(err);
		} finally {
			busy = false;
		}
	}

	async function remove(group: Group) {
		try {
			await deleteGroup(organizationId, group.id);
			groups = groups.filter((g) => g.id !== group.id);
		} catch (err) {
			fail(err);
		} finally {
			toDelete = null;
		}
	}
</script>

<section class="space-y-3" aria-labelledby="groups-heading">
	<div>
		<h3 id="groups-heading" class="text-[15px] font-semibold">
			{t('organizations.access.groups')}
		</h3>
		<p class="mt-1 text-sm text-muted-foreground">{t('organizations.access.groups_hint')}</p>
	</div>

	{#if canManage}
		<form class="flex flex-wrap gap-2" onsubmit={create}>
			<Input
				bind:value={newName}
				placeholder={t('organizations.access.group_name')}
				maxlength={64}
				class="w-full sm:w-64" />
			<Button type="submit" disabled={busy || !newName.trim()}>
				{t('organizations.access.new_group')}
			</Button>
		</form>
	{/if}

	{#if loading}
		<SkeletonRows rows={3} height="h-14" label={t('organizations.access.loading')} />
	{:else if groups.length === 0}
		<p class="text-sm text-muted-foreground">{t('organizations.access.no_groups')}</p>
	{:else}
		<ul class="lp-raised overflow-hidden rounded-2xl border bg-card">
			{#each groups as group (group.id)}
				{@const local = group.source === 'local'}
				<li
					class="flex flex-col gap-3 p-4 not-last:border-b not-last:border-border/60 sm:flex-row sm:items-center">
					<div class="flex min-w-0 flex-1 flex-col">
						<span class="flex items-center gap-2 font-medium">
							{group.name}
							<Badge variant={local ? 'secondary' : 'outline'}
								>{t(`organizations.access.source_${group.source}`)}</Badge>
						</span>
						<span class="truncate text-sm text-muted-foreground">
							{group.members.length === 0
								? t('organizations.access.no_members')
								: group.members.map((m) => m.name || m.email).join(', ')}
						</span>
					</div>
					{#if canManage && local}
						<div class="flex gap-1">
							<Button variant="ghost" size="sm" onclick={() => openMembers(group)}>
								<UsersIcon class="size-4" />
								{t('organizations.access.members')}
							</Button>
							<Button
								variant="ghost"
								size="sm"
								class="text-destructive hover:text-destructive"
								onclick={() => (toDelete = group)}>
								<Trash2Icon class="size-4" />
								{t('organizations.access.delete')}
							</Button>
						</div>
					{/if}
				</li>
			{/each}
		</ul>
	{/if}
</section>

<Dialog.Root
	open={editing !== null}
	onOpenChange={(o) => {
		if (!o) editing = null;
	}}>
	<Dialog.Content class="flex flex-col gap-0 overflow-y-hidden p-0 sm:max-w-lg">
		<Dialog.Header class="border-b p-6 pr-14">
			<Dialog.Title>{editing?.name}</Dialog.Title>
			<Dialog.Description>{t('organizations.access.group_members_hint')}</Dialog.Description>
		</Dialog.Header>
		<ul class="grid min-h-0 flex-1 overflow-y-auto p-6 content-start gap-2">
			{#each members as member (member.user_id)}
				<li>
					<label class="flex items-center gap-2 text-sm">
						<SoftCheckbox
							checked={picked.has(member.user_id)}
							onCheckedChange={(on) => toggle(member.user_id, on === true)} />
						<span class="min-w-0 truncate">{member.name || member.email}</span>
						<span class="min-w-0 truncate text-muted-foreground">{member.email}</span>
					</label>
				</li>
			{/each}
		</ul>
		<Dialog.Footer class="border-t p-4">
			<Button variant="outline" onclick={() => (editing = null)}>
				{t('organizations.common.cancel')}
			</Button>
			<Button disabled={busy} onclick={saveMembers}>{t('organizations.access.save')}</Button>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>

<AlertDialog.Root
	open={toDelete !== null}
	onOpenChange={(o) => {
		if (!o) toDelete = null;
	}}>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title>{t('organizations.access.delete_group_confirm')}</AlertDialog.Title>
			<AlertDialog.Description>
				{toDelete ? t('organizations.access.delete_group_hint', { name: toDelete.name }) : ''}
			</AlertDialog.Description>
		</AlertDialog.Header>
		<AlertDialog.Footer>
			<AlertDialog.Cancel>{t('organizations.common.cancel')}</AlertDialog.Cancel>
			<DeleteConfirmControl
				disabled={!toDelete}
				onconfirm={async () => {
					if (toDelete) await remove(toDelete);
				}} />
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>
