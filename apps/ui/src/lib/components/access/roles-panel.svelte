<script lang="ts">
	import { onMount } from 'svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
	import DeleteConfirmControl from '#lib/components/custom/delete-confirm-control.svelte';
	import SoftCheckbox from '#lib/components/custom/soft-checkbox.svelte';
	import SkeletonRows from '#lib/components/custom/skeleton-rows.svelte';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import PencilIcon from '@lucide/svelte/icons/pencil';
	import Trash2Icon from '@lucide/svelte/icons/trash-2';
	import {
		createRole,
		deleteRole,
		getCatalog,
		groupCatalog,
		listRoles,
		updateRole,
		type CatalogEntry,
		type Role,
	} from '#lib/clients/access-client.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let { organizationId, canManage }: { organizationId: string; canManage: boolean } = $props();

	let roles = $state<Role[]>([]);
	let catalog = $state<CatalogEntry[]>([]);
	let loading = $state(true);
	let saving = $state(false);
	let editorOpen = $state(false);
	let editing = $state<Role | null>(null);
	let name = $state('');
	let description = $state('');
	let selected = $state<Set<string>>(new Set());
	let toDelete = $state<Role | null>(null);

	const groups = $derived(groupCatalog(catalog));

	onMount(async () => {
		try {
			[roles, catalog] = await Promise.all([listRoles(organizationId), getCatalog(organizationId)]);
		} catch (err) {
			toast.error(err instanceof Error ? err.message : t('organizations.access.load_failed'));
		} finally {
			loading = false;
		}
	});

	function openEditor(role: Role | null) {
		editing = role;
		name = role?.name ?? '';
		description = role?.description ?? '';
		selected = new Set(role?.permissions ?? []);
		editorOpen = true;
	}

	function toggle(permission: string, on: boolean) {
		const next = new Set(selected);
		if (on) next.add(permission);
		else next.delete(permission);
		selected = next;
	}

	async function save() {
		saving = true;
		const input = { name, description, permissions: [...selected] };
		try {
			if (editing) {
				const updated = await updateRole(organizationId, editing.id, input);
				roles = roles.map((r) => (r.id === updated.id ? updated : r));
			} else {
				roles = [...roles, await createRole(organizationId, input)];
			}
			toast.success(t('organizations.access.role_saved'));
			editorOpen = false;
		} catch (err) {
			toast.error(err instanceof Error ? err.message : t('organizations.access.save_failed'));
		} finally {
			saving = false;
		}
	}

	async function remove(role: Role) {
		try {
			// The dialog already warned that assignments go with it.
			await deleteRole(organizationId, role.id, true);
			roles = roles.filter((r) => r.id !== role.id);
			toast.success(t('organizations.access.role_deleted'));
		} catch (err) {
			toast.error(err instanceof Error ? err.message : t('organizations.access.save_failed'));
		} finally {
			toDelete = null;
		}
	}
</script>

<section class="space-y-3" aria-labelledby="roles-heading">
	<div class="flex flex-wrap items-start justify-between gap-3">
		<div>
			<h3 id="roles-heading" class="text-[15px] font-semibold">{t('organizations.access.roles')}</h3>
			<p class="mt-1 text-sm text-muted-foreground">{t('organizations.access.roles_hint')}</p>
		</div>
		{#if canManage}
			<Button size="sm" onclick={() => openEditor(null)}>
				<PlusIcon class="size-4" />
				{t('organizations.access.new_role')}
			</Button>
		{/if}
	</div>

	{#if loading}
		<SkeletonRows rows={4} height="h-14" label={t('organizations.access.loading')} />
	{:else}
		<ul class="lp-raised overflow-hidden rounded-2xl border bg-card">
			{#each roles as role (role.id)}
				<li
					class="flex flex-col gap-3 p-4 not-last:border-b not-last:border-border/60 sm:flex-row sm:items-center">
					<div class="flex min-w-0 flex-1 flex-col">
						<span class="flex items-center gap-2 font-medium">
							{role.name}
							{#if role.system}<Badge variant="secondary">{t('organizations.access.system')}</Badge
								>{/if}
						</span>
						<span class="text-sm text-muted-foreground">
							{role.description ||
								t('organizations.access.permission_count', { count: role.permissions.length })}
						</span>
					</div>
					{#if role.system}
						<Button variant="ghost" size="sm" onclick={() => openEditor(role)}>
							{t('organizations.access.view')}
						</Button>
					{:else if canManage}
						<div class="flex gap-1">
							<Button variant="ghost" size="sm" onclick={() => openEditor(role)}>
								<PencilIcon class="size-4" />
								{t('organizations.access.edit')}
							</Button>
							<Button
								variant="ghost"
								size="sm"
								class="text-destructive hover:text-destructive"
								onclick={() => (toDelete = role)}>
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

<Dialog.Root bind:open={editorOpen}>
	<Dialog.Content class="flex flex-col gap-0 overflow-y-hidden p-0 sm:max-w-2xl">
		<Dialog.Header class="border-b p-6 pr-14">
			<Dialog.Title>
				{editing?.system
					? editing.name
					: editing
						? t('organizations.access.edit_role')
						: t('organizations.access.new_role')}
			</Dialog.Title>
			<Dialog.Description>
				{editing?.system
					? t('organizations.access.system_read_only')
					: t('organizations.access.editor_hint')}
			</Dialog.Description>
		</Dialog.Header>
		{@const readOnly = editing?.system || !canManage}
		<div class="grid min-h-0 flex-1 overflow-y-auto p-6 gap-4">
			{#if !editing?.system}
				<div class="grid gap-2">
					<Label for="role-name">{t('organizations.access.name')}</Label>
					<Input id="role-name" bind:value={name} disabled={readOnly} maxlength={64} />
				</div>
				<div class="grid gap-2">
					<Label for="role-description">{t('organizations.access.description')}</Label>
					<Input id="role-description" bind:value={description} disabled={readOnly} />
				</div>
			{/if}
			{#each groups as [resource, entries] (resource)}
				<fieldset class="grid gap-2 rounded-xl p-3 ring-1 ring-border/60">
					<legend class="px-1 text-sm font-medium">{resource}</legend>
					<div class="grid gap-2 sm:grid-cols-2">
						{#each entries as entry (entry.name)}
							<label class="flex items-center gap-2 text-sm">
								<SoftCheckbox
									checked={selected.has(entry.name)}
									disabled={readOnly}
									onCheckedChange={(on) => toggle(entry.name, on === true)} />
								<span class="min-w-0 font-mono text-xs break-all">{entry.name}</span>
								{#if entry.org_only}<Badge variant="outline" class="text-[10px]"
										>{t('organizations.access.org_only')}</Badge
									>{/if}
							</label>
						{/each}
					</div>
				</fieldset>
			{/each}
		</div>
		{#if !readOnly}
			<Dialog.Footer class="border-t p-4">
				<Button variant="outline" onclick={() => (editorOpen = false)}>
					{t('organizations.common.cancel')}
				</Button>
				<Button disabled={saving || !name.trim()} onclick={save}>
					{t('organizations.access.save')}
				</Button>
			</Dialog.Footer>
		{/if}
	</Dialog.Content>
</Dialog.Root>

<AlertDialog.Root
	open={toDelete !== null}
	onOpenChange={(o) => {
		if (!o) toDelete = null;
	}}>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title>{t('organizations.access.delete_role_confirm')}</AlertDialog.Title>
			<AlertDialog.Description>
				{toDelete ? t('organizations.access.delete_role_hint', { name: toDelete.name }) : ''}
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
