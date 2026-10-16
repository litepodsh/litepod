<script lang="ts">
	import CheckIcon from '@lucide/svelte/icons/check';
	import HardDriveIcon from '@lucide/svelte/icons/hard-drive';
	import LoaderCircleIcon from '@lucide/svelte/icons/loader-circle';
	import PackageIcon from '@lucide/svelte/icons/package';
	import PowerIcon from '@lucide/svelte/icons/power';
	import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';
	import Trash2Icon from '@lucide/svelte/icons/trash-2';
	import XIcon from '@lucide/svelte/icons/x';
	import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
	import DeleteConfirmControl from '#lib/components/custom/delete-confirm-control.svelte';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import {
		checkServerDisk,
		pruneServerImages,
		restartServer,
		restartServerPodman,
		updateServerPackages,
		type Server,
		type ServerMaintenanceResult,
	} from '#lib/clients/server-client.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';

	type Action = 'update-packages' | 'restart' | 'prune-images' | 'restart-podman' | 'check-disk';

	let {
		open = $bindable(false),
		organizationId,
		server,
		onDone,
	}: {
		open?: boolean;
		organizationId: string;
		server: Server | null;
		onDone?: () => void;
	} = $props();

	let running = $state<Action | null>(null);
	let restartConfirmOpen = $state(false);
	let results = $state<Partial<Record<Action, ServerMaintenanceResult>>>({});

	const actions: {
		id: Action;
		label: string;
		description: string;
		icon: typeof PackageIcon;
	}[] = [
		{
			id: 'update-packages',
			label: t('servers.maintenance.action_update_packages'),
			description: t('servers.maintenance.action_update_packages_desc'),
			icon: PackageIcon,
		},
		{
			id: 'restart-podman',
			label: t('servers.maintenance.action_restart_podman'),
			description: t('servers.maintenance.action_restart_podman_desc'),
			icon: RefreshCwIcon,
		},
		{
			id: 'prune-images',
			label: t('servers.maintenance.action_prune_images'),
			description: t('servers.maintenance.action_prune_images_desc'),
			icon: Trash2Icon,
		},
		{
			id: 'check-disk',
			label: t('servers.maintenance.action_check_disk'),
			description: t('servers.maintenance.action_check_disk_desc'),
			icon: HardDriveIcon,
		},
	];

	function resultFor(action: Action) {
		return results[action];
	}

	async function execute(action: Action) {
		if (!server || running) return;
		running = action;
		try {
			const result = await (action === 'update-packages'
				? updateServerPackages(organizationId, server.id)
				: action === 'prune-images'
					? pruneServerImages(organizationId, server.id)
					: action === 'restart-podman'
						? restartServerPodman(organizationId, server.id)
						: checkServerDisk(organizationId, server.id));
			results[action] = result;
			if (result.success) {
				toast.success(
					t('servers.maintenance.completed_toast', {
						action: actionLabel(action),
						name: server.name,
					}),
				);
			} else {
				toast.error(
					t('servers.maintenance.failed_toast', {
						action: actionLabel(action),
						name: server.name,
					}),
				);
			}
			onDone?.();
		} catch (caught) {
			toast.error(
				caught instanceof Error
					? caught.message
					: t('servers.maintenance.action_failed_toast', { action: actionLabel(action) }),
			);
		} finally {
			running = null;
		}
	}

	async function executeRestart() {
		if (!server || running) return;
		running = 'restart';
		restartConfirmOpen = false;
		try {
			const result = await restartServer(organizationId, server.id);
			results.restart = result;
			if (result.success) {
				toast.success(t('servers.maintenance.restarting_toast', { name: server.name }));
				onDone?.();
			} else {
				toast.error(t('servers.maintenance.restart_failed_toast', { name: server.name }));
			}
		} catch (caught) {
			toast.error(
				caught instanceof Error ? caught.message : t('servers.maintenance.restart_failed'),
			);
		} finally {
			running = null;
		}
	}

	function actionLabel(action: Action) {
		return {
			'update-packages': t('servers.maintenance.label_package_update'),
			restart: t('servers.maintenance.label_server_restart'),
			'prune-images': t('servers.maintenance.label_image_cleanup'),
			'restart-podman': t('servers.maintenance.label_podman_restart'),
			'check-disk': t('servers.maintenance.label_disk_check'),
		}[action];
	}

	$effect(() => {
		if (open) {
			results = {};
			running = null;
			restartConfirmOpen = false;
		}
	});
</script>

<Dialog.Root bind:open>
	<Dialog.Content class="sm:max-w-2xl">
		<Dialog.Header>
			<Dialog.Title>{t('servers.maintenance.title', { name: server?.name ?? '' })}</Dialog.Title>
			<Dialog.Description>
				{t('servers.maintenance.description')}
			</Dialog.Description>
		</Dialog.Header>

		<div class="space-y-3">
			{#each actions as action}
				{@const result = resultFor(action.id)}
				<div class="rounded-lg border p-3">
					<div class="flex items-start gap-3">
						<action.icon class="mt-0.5 size-4 shrink-0 text-muted-foreground" />
						<div class="min-w-0 flex-1 space-y-1">
							<p class="text-sm font-medium">{action.label}</p>
							<p class="text-xs text-muted-foreground">{action.description}</p>
						</div>
						<Button
							size="sm"
							variant="outline"
							disabled={running !== null}
							onclick={() => void execute(action.id)}>
							{#if running === action.id}
								<LoaderCircleIcon class="size-4 animate-spin" />
								{t('servers.maintenance.running')}
							{:else}
								{t('organizations.common.run')}
							{/if}
						</Button>
					</div>
					{#if result}
						<div class="mt-3 space-y-2">
							<div class="flex items-center gap-2 text-xs">
								{#if result.success}
									<CheckIcon class="size-4 text-success" />
									<Badge variant="secondary">{t('servers.maintenance.completed')}</Badge>
								{:else}
									<XIcon class="size-4 text-destructive" />
									<Badge variant="destructive">{t('organizations.servers.failed')}</Badge>
								{/if}
								<span class="text-muted-foreground">{(result.duration_ms / 1000).toFixed(1)}s</span>
							</div>
							{#if result.stdout.trim()}
								<pre
									class="max-h-48 overflow-auto rounded-md border bg-muted/40 p-2 text-xs whitespace-pre-wrap">{result.stdout.trim()}</pre>
							{/if}
							{#if result.stderr.trim()}
								<pre
									class="max-h-32 overflow-auto rounded-md border border-destructive/30 bg-destructive/5 p-2 text-xs whitespace-pre-wrap">{result.stderr.trim()}</pre>
							{/if}
						</div>
					{/if}
				</div>
			{/each}

			<div class="rounded-lg border border-destructive/30 bg-destructive/5 p-3">
				<div class="flex items-start gap-3">
					<PowerIcon class="mt-0.5 size-4 shrink-0 text-destructive" />
					<div class="min-w-0 flex-1 space-y-1">
						<p class="text-sm font-medium">{t('servers.maintenance.restart_server')}</p>
						<p class="text-xs text-muted-foreground">
							{t('servers.maintenance.restart_description')}
						</p>
					</div>
					<Button
						size="sm"
						variant="destructive"
						disabled={running !== null}
						onclick={() => (restartConfirmOpen = true)}>
						{t('servers.maintenance.restart_server')}
					</Button>
				</div>
				{#if results.restart}
					<div class="mt-3 flex items-center gap-2 text-xs">
						{#if results.restart.success}
							<CheckIcon class="size-4 text-success" />
							<Badge variant="secondary">{t('servers.maintenance.scheduled')}</Badge>
						{:else}
							<XIcon class="size-4 text-destructive" />
							<Badge variant="destructive">{t('organizations.servers.failed')}</Badge>
						{/if}
					</div>
				{/if}
			</div>
		</div>

		<Dialog.Footer>
			<Button variant="outline" disabled={running !== null} onclick={() => (open = false)}
				>{t('organizations.common.close')}</Button>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>

<AlertDialog.Root bind:open={restartConfirmOpen}>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title
				>{t('servers.maintenance.restart_confirm', {
					name: server?.name ?? '',
				})}</AlertDialog.Title>
			<AlertDialog.Description>
				{t('servers.maintenance.restart_confirm_description')}
			</AlertDialog.Description>
		</AlertDialog.Header>
		<AlertDialog.Footer>
			<AlertDialog.Cancel disabled={running !== null}
				>{t('organizations.common.cancel')}</AlertDialog.Cancel>
			<DeleteConfirmControl
				label={t('servers.maintenance.restart_server')}
				pendingLabel={t('servers.maintenance.restarting')}
				variant="primary"
				disabled={running !== null}
				onconfirm={executeRestart} />
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>
