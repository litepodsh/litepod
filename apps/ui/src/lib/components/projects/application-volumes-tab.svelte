<script lang="ts">
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Tooltip from '#lib/components/ui/tooltip/index.js';
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import VolumeFileWorkspace from '#lib/components/volumes/volume-file-workspace.svelte';
	import ManagedVolumeFormDialog from '#lib/components/projects/managed-volume-form-dialog.svelte';
	import ManagedVolumeDeleteDialog from '#lib/components/projects/managed-volume-delete-dialog.svelte';
	import CopyButton from '#lib/components/custom/copy-button.svelte';
	import type {
		CreateManagedVolumeInput,
		ManagedVolumeDto,
		VolumeDto,
	} from '#lib/clients/project-client.js';
	import type { WorkspaceClient } from '#lib/clients/volume-workspace-client.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';

	let {
		volumes = [],
		managedVolumes = [],
		isCompose,
		composeServices = [],
		busy = false,
		onRefresh,
		onCreateManagedVolume,
		onAttachManagedVolume,
		onDetachManagedVolume,
		onDeleteManagedVolume,
		workspaceFor,
		managedWorkspaceFor,
	}: {
		volumes?: VolumeDto[];
		managedVolumes?: ManagedVolumeDto[];
		isCompose: boolean;
		composeServices?: string[];
		busy?: boolean;
		onRefresh: () => Promise<void>;
		onCreateManagedVolume: (input: CreateManagedVolumeInput) => Promise<void>;
		onAttachManagedVolume: (volumeId: string) => Promise<void>;
		onDetachManagedVolume: (volumeId: string) => Promise<void>;
		onDeleteManagedVolume: (volumeId: string) => Promise<void>;
		workspaceFor: (source: string) => WorkspaceClient;
		managedWorkspaceFor: (volumeId: string) => WorkspaceClient;
	} = $props();

	let selectedSource = $state<string | null>(null);
	let selectedManagedVolumeId = $state<string | null>(null);
	let addDialogOpen = $state(false);
	let deleteVolume = $state<ManagedVolumeDto | null>(null);
	let selectedManagedVolume = $derived(
		managedVolumes.find((volume) => volume.id === selectedManagedVolumeId) ?? null,
	);

	async function run(action: () => Promise<void>, failure: string) {
		try {
			await action();
		} catch (error) {
			toast.error(error instanceof Error ? error.message : failure);
		}
	}
</script>

{#snippet mountPath(path: string)}
	<Tooltip.Root delayDuration={200}>
		<Tooltip.Trigger>
			{#snippet child({ props })}
				<CopyButton
					{...props}
					value={path}
					errorMessage={t('projects.application_volumes.copy_path_failed')}
					class="flex w-full min-w-0 font-mono text-xs outline-none focus-visible:ring-2"
					iconClass="size-3" />
			{/snippet}
		</Tooltip.Trigger>
		<Tooltip.Content>{path}</Tooltip.Content>
	</Tooltip.Root>
{/snippet}

<section class="rounded-xl border p-5">
	<div class="flex flex-wrap items-start justify-between gap-3">
		<div>
			<p class="font-medium">{t('projects.application_volumes.managed_volumes')}</p>
			<p class="mt-1 text-sm text-muted-foreground">
				{t('projects.application_volumes.managed_volumes_hint')}
			</p>
		</div>
		<div class="flex gap-2">
			<Button variant="outline" disabled={busy} onclick={() => void onRefresh()}
				><RefreshCwIcon class={busy ? 'animate-spin' : ''} />{t(
					'projects.application_volumes.refresh',
				)}</Button>
			<Button onclick={() => (addDialogOpen = true)}><PlusIcon />{t('projects.volume.add')}</Button>
		</div>
	</div>

	{#if managedVolumes.length}
		<div class="mt-4 grid gap-3 md:grid-cols-2">
			{#each managedVolumes as volume (volume.id)}
				<article class="rounded-lg border p-4">
					<div class="flex items-start justify-between gap-3">
						<div class="min-w-0">
							<p class="truncate font-medium">{volume.name}</p>
							<p class="mt-1 truncate font-mono text-xs text-muted-foreground">
								{volume.mount_path}
							</p>
						</div>
						<span class="rounded-full bg-muted px-2 py-1 text-xs"
							>{volume.attached
								? t('projects.application_volumes.attached')
								: t('projects.application_volumes.detached')}</span>
					</div>
					<p class="mt-3 text-sm text-muted-foreground">
						{volume.compose_service
							? t('projects.application_volumes.compose_service_named', {
									name: volume.compose_service,
								})
							: t('projects.application_volumes.application_container')} · {volume.attached
							? t('projects.application_volumes.applies_next_deployment')
							: t('projects.application_volumes.files_retained')}
					</p>
					<div class="mt-4 flex flex-wrap gap-2">
						<Button
							size="sm"
							variant="outline"
							onclick={() => (selectedManagedVolumeId = volume.id)}
							>{t('projects.application_volumes.manage_files')}</Button>
						{#if volume.attached}
							<Button
								size="sm"
								variant="outline"
								disabled={busy}
								onclick={() =>
									void run(
										() => onDetachManagedVolume(volume.id),
										t('projects.application_volumes.detach_failed'),
									)}>{t('projects.application_volumes.detach')}</Button>
						{:else}
							<Button
								size="sm"
								variant="outline"
								disabled={busy}
								onclick={() =>
									void run(
										() => onAttachManagedVolume(volume.id),
										t('projects.application_volumes.attach_failed'),
									)}>{t('projects.application_volumes.attach')}</Button>
							<Button
								size="sm"
								variant="destructive"
								disabled={busy}
								onclick={() => (deleteVolume = volume)}>{t('projects.common.delete')}</Button>
						{/if}
					</div>
				</article>
			{/each}
		</div>
	{:else}
		<p class="mt-4 rounded-lg border border-dashed p-6 text-sm text-muted-foreground">
			{t('projects.application_volumes.empty_managed')}
		</p>
	{/if}
</section>

<Dialog.Root
	open={!!selectedManagedVolume}
	onOpenChange={(open) => {
		if (!open) selectedManagedVolumeId = null;
	}}>
	<Dialog.Content
		class="flex h-[95vh] w-[95vw] max-w-[95vw] min-w-0 flex-col overflow-hidden sm:max-w-[95vw]">
		<Dialog.Header>
			<Dialog.Title class="font-mono text-sm"
				>{t('projects.application_volumes.dialog_title', {
					name: selectedManagedVolume?.name,
				})}</Dialog.Title>
		</Dialog.Header>
		{#if selectedManagedVolume}{#key selectedManagedVolume.id}
				<div class="min-h-0 flex-1 overflow-hidden">
					<VolumeFileWorkspace
						title={t('projects.application_volumes.volume_named', {
							name: selectedManagedVolume.name,
						})}
						client={managedWorkspaceFor(selectedManagedVolume.id)} />
				</div>
			{/key}{/if}
	</Dialog.Content>
</Dialog.Root>

<section class="mt-4 rounded-xl border p-5">
	<p class="font-medium">{t('projects.application_volumes.live_mounts')}</p>
	<p class="mt-1 text-sm text-muted-foreground">
		{t('projects.application_volumes.live_mounts_hint')}
	</p>
	{#if volumes.length}<div class="mt-4 overflow-x-auto">
			<table class="w-full min-w-[500px] text-left text-sm">
				<thead class="text-muted-foreground"
					><tr
						><th class="pb-2">{t('projects.volume.name')}</th><th class="pb-2"
							>{t('projects.application_volumes.type')}</th
						><th class="pb-2">{t('projects.application_volumes.source')}</th><th class="pb-2"
							>{t('projects.application_volumes.destination')}</th
						><th class="pb-2">{t('projects.application_volumes.access')}</th><th></th></tr
					></thead
				><tbody
					>{#each volumes as volume}<tr class="border-t"
							><td class="py-2 font-mono text-xs"
								>{volume.source.replace(/\/+$/, '').split('/').pop() || volume.source}</td
							><td class="py-2">{volume.mount_type}</td><td class="max-w-0 py-2"
								>{@render mountPath(volume.source)}</td
							><td class="max-w-0 py-2">{@render mountPath(volume.destination)}</td><td class="py-2"
								>{volume.read_only
									? t('projects.application_volumes.read_only')
									: t('projects.application_volumes.read_write')}</td
							><td
								><Button
									size="sm"
									variant="outline"
									onclick={() => (selectedSource = volume.source)}
									>{t('projects.application_volumes.browse_files')}</Button
								></td
							></tr
						>{/each}</tbody>
			</table>
		</div>{:else}<p class="mt-4 text-sm text-muted-foreground">
			{t('projects.application_volumes.no_live_mounts')}
		</p>{/if}
</section>
<Dialog.Root
	open={!!selectedSource}
	onOpenChange={(open) => {
		if (!open) selectedSource = null;
	}}>
	<Dialog.Content
		class="flex h-[95vh] w-[95vw] max-w-[95vw] min-w-0 flex-col overflow-hidden sm:max-w-[95vw]">
		<Dialog.Header>
			<Dialog.Title class="font-mono text-sm"
				>{t('projects.application_volumes.mount_dialog_title', {
					name: selectedSource,
				})}</Dialog.Title>
		</Dialog.Header>
		{#if selectedSource}{#key selectedSource}
				<div class="min-h-0 flex-1 overflow-hidden">
					<VolumeFileWorkspace
						title={t('projects.application_volumes.volume_named', { name: selectedSource })}
						client={workspaceFor(selectedSource)} />
				</div>
			{/key}{/if}
	</Dialog.Content>
</Dialog.Root>

<ManagedVolumeFormDialog
	bind:open={addDialogOpen}
	{isCompose}
	services={composeServices}
	onSubmit={onCreateManagedVolume} />
{#if deleteVolume}<ManagedVolumeDeleteDialog
		open={true}
		volumeName={deleteVolume.name}
		ondelete={async () => {
			await onDeleteManagedVolume(deleteVolume!.id);
			if (selectedManagedVolumeId === deleteVolume!.id) selectedManagedVolumeId = null;
			deleteVolume = null;
		}} />{/if}
