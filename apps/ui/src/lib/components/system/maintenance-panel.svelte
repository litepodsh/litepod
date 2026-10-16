<script lang="ts">
	import { Button } from '#lib/components/ui/button/index.js';
	import SoftCheckbox from '#lib/components/custom/soft-checkbox.svelte';
	import DeleteConfirmControl from '#lib/components/custom/delete-confirm-control.svelte';
	import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
	import { toast } from '#lib/utils/toast.js';
	import { api } from '#lib/clients/base-client.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let pruning = $state(false);
	let containers = $state(true);
	let images = $state(true);
	let volumes = $state(true);
	let result = $state<PruneResult | null>(null);
	let historyDialogOpen = $state(false);
	let deletingHistory = $state(false);
	let historyResult = $state<DeploymentHistoryResult | null>(null);

	interface PruneResult {
		containers_freed: number;
		containers_space: number;
		images_freed: number;
		images_space: number;
		volumes_freed: number;
		volumes_space: number;
	}

	interface DeploymentHistoryResult {
		deployment_runs_deleted: number;
		deployment_log_lines_deleted: number;
	}

	function formatBytes(bytes: number): string {
		if (bytes === 0) return '0 B';
		const units = ['B', 'KB', 'MB', 'GB', 'TB'];
		const i = Math.floor(Math.log(bytes) / Math.log(1024));
		return `${(bytes / Math.pow(1024, i)).toFixed(1)} ${units[i]}`;
	}

	async function runPrune() {
		pruning = true;
		result = null;
		try {
			const response = await api<PruneResult>('/api/v1/maintenance/prune', {
				method: 'POST',
				body: JSON.stringify({ containers, images, volumes }),
			});
			result = response.data;
			toast.success(t('operations.maintenance.pruning_completed'));
		} catch (error) {
			toast.error(
				error instanceof Error ? error.message : t('operations.maintenance.prune_failed'),
			);
		} finally {
			pruning = false;
		}
	}

	function openHistoryDialog() {
		historyDialogOpen = true;
	}

	async function deleteDeploymentHistory() {
		if (deletingHistory) return;
		deletingHistory = true;
		try {
			const response = await api<DeploymentHistoryResult>(
				'/api/v1/maintenance/deployment-history',
				{ method: 'POST' },
			);
			historyResult = response.data ?? null;
			historyDialogOpen = false;
			toast.success(t('operations.maintenance.history_deleted'));
		} catch (error) {
			toast.error(
				error instanceof Error ? error.message : t('operations.maintenance.history_delete_failed'),
			);
		} finally {
			deletingHistory = false;
		}
	}
</script>

<div class="flex flex-col gap-4">
	<div>
		<h2 class="text-lg font-semibold tracking-tight">{t('operations.maintenance.title')}</h2>
		<p class="mt-1 text-sm text-muted-foreground">
			{t('operations.maintenance.title_description')}
		</p>
	</div>

	<div class="lp-raised rounded-2xl border bg-card p-4 sm:p-5">
		<h3 class="text-[15px] font-semibold">{t('operations.maintenance.select_resources')}</h3>
		<p class="mt-1 mb-4 text-sm text-muted-foreground">
			{t('operations.maintenance.prune_description')}
		</p>
		<div class="space-y-3">
			<label class="flex items-center gap-3">
				<SoftCheckbox bind:checked={containers} />
				<div>
					<p class="font-medium">{t('operations.maintenance.containers')}</p>
					<p class="text-sm text-muted-foreground">
						{t('operations.maintenance.containers_description')}
					</p>
				</div>
			</label>
			<label class="flex items-center gap-3">
				<SoftCheckbox bind:checked={images} />
				<div>
					<p class="font-medium">{t('operations.maintenance.images')}</p>
					<p class="text-sm text-muted-foreground">
						{t('operations.maintenance.images_description')}
					</p>
				</div>
			</label>
			<label class="flex items-center gap-3">
				<SoftCheckbox bind:checked={volumes} />
				<div>
					<p class="font-medium">{t('operations.maintenance.volumes')}</p>
					<p class="text-sm text-muted-foreground">
						{t('operations.maintenance.volumes_description')}
					</p>
				</div>
			</label>
		</div>

		<div class="lp-groove-t -mx-4 mt-5 px-4 pt-4 sm:-mx-5 sm:px-5">
			<Button onclick={runPrune} disabled={pruning || (!containers && !images && !volumes)}>
				{pruning ? t('operations.maintenance.pruning') : t('operations.maintenance.run_prune')}
			</Button>
		</div>
	</div>

	{#if result}
		<div class="lp-raised rounded-2xl border bg-card p-4 sm:p-5">
			<h3 class="mb-4 text-[15px] font-semibold">{t('operations.maintenance.results')}</h3>
			<div class="grid gap-4 sm:grid-cols-3">
				{#if result.containers_freed > 0 || result.containers_space > 0}
					<div class="lp-inset rounded-xl bg-muted/50 p-4">
						<p class="text-sm text-muted-foreground">{t('operations.maintenance.containers')}</p>
						<p class="text-2xl font-semibold">
							{t('operations.maintenance.removed', { count: result.containers_freed })}
						</p>
						<p class="text-sm text-muted-foreground">
							{t('operations.maintenance.freed', {
								amount: formatBytes(result.containers_space),
							})}
						</p>
					</div>
				{/if}
				{#if result.images_freed > 0 || result.images_space > 0}
					<div class="lp-inset rounded-xl bg-muted/50 p-4">
						<p class="text-sm text-muted-foreground">{t('operations.maintenance.images')}</p>
						<p class="text-2xl font-semibold">
							{t('operations.maintenance.removed', { count: result.images_freed })}
						</p>
						<p class="text-sm text-muted-foreground">
							{t('operations.maintenance.freed', { amount: formatBytes(result.images_space) })}
						</p>
					</div>
				{/if}
				{#if result.volumes_freed > 0 || result.volumes_space > 0}
					<div class="lp-inset rounded-xl bg-muted/50 p-4">
						<p class="text-sm text-muted-foreground">{t('operations.maintenance.volumes')}</p>
						<p class="text-2xl font-semibold">
							{t('operations.maintenance.removed', { count: result.volumes_freed })}
						</p>
						<p class="text-sm text-muted-foreground">
							{t('operations.maintenance.freed', { amount: formatBytes(result.volumes_space) })}
						</p>
					</div>
				{/if}
				{#if result.containers_freed === 0 && result.images_freed === 0 && result.volumes_freed === 0 && result.containers_space === 0 && result.images_space === 0 && result.volumes_space === 0}
					<p class="text-sm text-muted-foreground">{t('operations.maintenance.no_resources')}</p>
				{/if}
			</div>
		</div>
	{/if}

	<div class="rounded-2xl border border-destructive/30 bg-destructive/5 p-4 sm:p-5">
		<div class="flex flex-wrap items-center justify-between gap-4">
			<div>
				<h3 class="text-[15px] font-semibold text-destructive">
					{t('operations.maintenance.danger_zone')}
				</h3>
				<p class="mt-1 text-sm text-muted-foreground">
					{t('operations.maintenance.delete_history_description')}
					{t('operations.maintenance.active_deployments_preserved')}
				</p>
			</div>
			<Button variant="destructive" onclick={openHistoryDialog}
				>{t('operations.maintenance.delete_history_button')}</Button>
		</div>
		{#if historyResult}
			<div class="mt-4 grid gap-3 sm:grid-cols-2">
				<div class="rounded-xl border border-destructive/20 bg-card p-4">
					<p class="text-sm text-muted-foreground">
						{t('operations.maintenance.deployment_runs_deleted')}
					</p>
					<p class="text-2xl font-semibold">{historyResult.deployment_runs_deleted}</p>
				</div>
				<div class="rounded-xl border border-destructive/20 bg-card p-4">
					<p class="text-sm text-muted-foreground">
						{t('operations.maintenance.log_lines_deleted')}
					</p>
					<p class="text-2xl font-semibold">{historyResult.deployment_log_lines_deleted}</p>
				</div>
			</div>
		{/if}
	</div>
</div>

<AlertDialog.Root
	open={historyDialogOpen}
	onOpenChange={(open) => {
		if (!open && !deletingHistory) {
			historyDialogOpen = false;
		}
	}}>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title
				>{t('operations.maintenance.delete_history_dialog_title')}</AlertDialog.Title>
			<AlertDialog.Description
				>{t('operations.maintenance.delete_history_dialog_description')}</AlertDialog.Description>
		</AlertDialog.Header>
		<AlertDialog.Footer>
			<AlertDialog.Cancel disabled={deletingHistory}
				>{t('operations.maintenance.cancel')}</AlertDialog.Cancel>
			<DeleteConfirmControl
				label={t('operations.maintenance.delete_history')}
				pendingLabel={t('operations.maintenance.deleting')}
				disabled={deletingHistory}
				onconfirm={deleteDeploymentHistory} />
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>
