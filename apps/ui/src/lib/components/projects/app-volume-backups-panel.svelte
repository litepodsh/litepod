<script lang="ts">
	import { Button } from '#lib/components/ui/button/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Switch } from '#lib/components/ui/switch/index.js';
	import LoaderIcon from '@lucide/svelte/icons/loader-circle';
	import {
		volumeLabel,
		type AppVolumeBackups,
		type VolumeBackupDto,
	} from '#lib/clients/project-client.js';
	import { listS3TargetOptions, type S3TargetOption } from '#lib/clients/backup-client.js';
	import { t } from '#lib/i18n/index.svelte.js';
	import { toast, toastError } from '#lib/utils/toast.js';
	import { pollWhileVisible } from '#lib/utils/poll.js';

	let {
		organizationId,
		client,
		onRestored,
	}: {
		organizationId: string;
		client: AppVolumeBackups;
		/** Volumes changed: refresh the tab. */
		onRestored: () => Promise<void>;
	} = $props();

	type Run = {
		groupId: string;
		startedAt: number;
		volumes: VolumeBackupDto[];
		status: 'running' | 'succeeded' | 'failed';
		size: number;
		beforeRestore: boolean;
	};

	let rows = $state<VolumeBackupDto[]>([]);
	let targets = $state<S3TargetOption[]>([]);
	let defaultAvailable = $state(false);
	let targetInput = $state('');
	/** Chosen per run, off unless the user turns it on for this one. */
	let stopApp = $state(false);
	let loading = $state(true);
	let restoring = $state<string | null>(null);
	let stopPoll: (() => void) | null = null;

	/** One entry per run; rows of a run share `group_id`. */
	const runs = $derived.by(() => {
		const byGroup = new Map<string, Run>();
		for (const row of rows) {
			const run = byGroup.get(row.group_id!) ?? {
				groupId: row.group_id!,
				startedAt: row.started_at,
				volumes: [] as VolumeBackupDto[],
				status: 'succeeded' as Run['status'],
				size: 0,
				beforeRestore: row.triggered_by === 'pre-restore',
			};
			run.volumes.push(row);
			run.startedAt = Math.min(run.startedAt, row.started_at);
			run.size += row.size_bytes ?? 0;
			if (row.status === 'running') run.status = 'running';
			else if (row.status === 'failed' && run.status !== 'running') run.status = 'failed';
			byGroup.set(row.group_id!, run);
		}
		return [...byGroup.values()].sort((a, b) => b.startedAt - a.startedAt);
	});
	const busy = $derived(runs.some((run) => run.status === 'running'));
	const noDestination = $derived(targets.length === 0 && !defaultAvailable);

	async function refresh() {
		const list = await client.list();
		rows = list.runs;
		defaultAvailable = list.default_available;
		if (rows.some((row) => row.status === 'running')) watch();
	}

	$effect(() => {
		void (async () => {
			try {
				await refresh();
				targets = await listS3TargetOptions(organizationId).catch(() => []);
				if (!targetInput && !defaultAvailable && targets.length) targetInput = targets[0].id;
			} catch (err) {
				toastError(err, t('projects.db_backup.load_failed'));
			} finally {
				loading = false;
			}
		})();
		return () => stopPoll?.();
	});

	function watch() {
		stopPoll?.();
		stopPoll = pollWhileVisible(async () => {
			await refresh();
			if (!rows.some((row) => row.status === 'running')) stopPoll?.();
		}, 4000);
	}

	async function runAll() {
		try {
			await client.run(targetInput || null, stopApp);
			stopApp = false;
			toast.success(t('projects.app_volume_backup.started'));
			await refresh();
		} catch (err) {
			toastError(err, t('projects.db_backup.run_failed'));
		}
	}

	async function restore(run: Run) {
		if (!window.confirm(t('projects.app_volume_backup.restore_confirm', { count: run.volumes.length })))
			return;
		restoring = run.groupId;
		try {
			await client.restore(run.groupId);
			toast.success(t('projects.app_volume_backup.restore_done'));
			await refresh();
			await onRestored();
		} catch (err) {
			toastError(err, t('projects.db_backup.restore_failed'));
			await refresh().catch(() => {});
		} finally {
			restoring = null;
		}
	}

	function formatSize(bytes: number): string {
		const units = ['B', 'KB', 'MB', 'GB', 'TB'];
		let value = bytes;
		let unit = 0;
		while (value >= 1024 && unit < units.length - 1) {
			value /= 1024;
			unit += 1;
		}
		return unit === 0 ? `${value} B` : `${value.toFixed(1)} ${units[unit]}`;
	}

	function statusLabel(status: Run['status']) {
		if (status === 'running') return t('projects.db_backup.status_running');
		if (status === 'succeeded') return t('projects.db_backup.status_succeeded');
		return t('projects.db_backup.status_failed');
	}
</script>

<section class="mt-4 rounded-xl border p-5">
	<p class="font-medium">{t('projects.app_volume_backup.title')}</p>
	<p class="mt-1 text-sm text-muted-foreground">{t('projects.app_volume_backup.description')}</p>

	{#if loading}
		<div class="mt-4 flex items-center gap-2 text-sm text-muted-foreground">
			<LoaderIcon class="size-4 animate-spin" />{t('projects.db_backup.loading')}
		</div>
	{:else}
		<div class="mt-4 grid gap-3 sm:grid-cols-[1fr_auto] sm:items-end">
			<div class="grid gap-1">
				<label class="text-sm leading-none font-medium" for="app-backup-target"
					>{t('projects.db_backup.destination')}</label>
				<select
					id="app-backup-target"
					class="flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm"
					bind:value={targetInput}>
					<option value=""
						>{defaultAvailable
							? t('projects.db_backup.destination_default')
							: t('projects.db_backup.destination_none')}</option>
					{#each targets as target (target.id)}
						<option value={target.id}>{target.name}</option>
					{/each}
				</select>
			</div>
			<Button disabled={busy || noDestination || (!targetInput && !defaultAvailable)} onclick={() => void runAll()}>
				{#if busy}<LoaderIcon class="size-4 animate-spin" />{t('projects.db_backup.running')}{:else}{t(
						'projects.app_volume_backup.run',
					)}{/if}
			</Button>
		</div>
		<div class="mt-3 flex items-center justify-between gap-4">
			<div class="grid gap-1">
				<label class="text-sm font-medium" for="app-backup-stop"
					>{t('projects.app_volume_backup.stop_app')}</label>
				<p class="text-xs text-muted-foreground">{t('projects.volume_backup.stop_app_hint')}</p>
			</div>
			<Switch id="app-backup-stop" checked={stopApp} onCheckedChange={(next) => (stopApp = next)} />
		</div>
		{#if noDestination}
			<p class="mt-3 rounded-md border border-dashed p-3 text-sm text-muted-foreground">
				{t('projects.volume_backup.no_destination')}
				<a
					class="font-medium text-foreground underline"
					href={`/organizations/${organizationId}?tab=backup-storage`}
					target="_blank"
					rel="noopener">{t('projects.volume_backup.add_target')}</a>
			</p>
		{/if}

		{#if runs.length === 0}
			<p class="mt-4 text-sm text-muted-foreground">{t('projects.app_volume_backup.empty')}</p>
		{:else}
			<ul class="mt-4 divide-y rounded-lg border">
				{#each runs as run (run.groupId)}
					<li class="flex flex-wrap items-start justify-between gap-3 p-3">
						<div class="min-w-0">
							<p class="text-sm font-medium">
								{new Date(run.startedAt).toLocaleString()}
								<Badge
									class="ml-2"
									variant={run.status === 'succeeded'
										? 'default'
										: run.status === 'running'
											? 'secondary'
											: 'destructive'}>{statusLabel(run.status)}</Badge>
							</p>
							<p class="mt-1 text-xs text-muted-foreground">
								{#if run.beforeRestore}{t('projects.volume_backup.pre_restore')} ·
								{/if}{run.volumes.map((volume) => volumeLabel(volume.volume_name)).join(', ')} · {formatSize(
									run.size,
								)}
							</p>
							{#each run.volumes.filter((volume) => volume.error_message) as volume (volume.id)}
								<p class="mt-1 text-xs break-words text-destructive">
									{volumeLabel(volume.volume_name)}: {volume.error_message}
								</p>
							{/each}
						</div>
						{#if run.status === 'succeeded'}
							<Button
								size="sm"
								variant="outline"
								disabled={restoring !== null || busy}
								onclick={() => void restore(run)}>
								{#if restoring === run.groupId}<LoaderIcon class="size-4 animate-spin" />{/if}
								{t('projects.app_volume_backup.restore')}
							</Button>
						{/if}
					</li>
				{/each}
			</ul>
		{/if}
	{/if}
</section>
