<script lang="ts">
	import * as Empty from '#lib/components/ui/empty/index.js';
	import DatabaseBackupIcon from '@lucide/svelte/icons/database-backup';
	import SkeletonRows from '#lib/components/custom/skeleton-rows.svelte';
	import SettingsSection from '#lib/components/projects/settings-section.svelte';
	import { onMount } from 'svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import * as Select from '#lib/components/ui/select/index.js';
	import * as Table from '#lib/components/ui/table/index.js';
	import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
	import DeleteConfirmControl from '#lib/components/custom/delete-confirm-control.svelte';
	import SoftSwitch from '#lib/components/custom/soft-switch.svelte';
	import EyeIcon from '@lucide/svelte/icons/eye';
	import EyeOffIcon from '@lucide/svelte/icons/eye-off';
	import CheckIcon from '@lucide/svelte/icons/check';
	import XIcon from '@lucide/svelte/icons/x';
	import UploadIcon from '@lucide/svelte/icons/upload';
	import RotateCcwIcon from '@lucide/svelte/icons/rotate-ccw';
	import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';
	import DownloadIcon from '@lucide/svelte/icons/download';
	import {
		getS3Config,
		updateS3Config,
		listBackupHistory,
		triggerBackup,
		triggerRestore,
		downloadBackupUrl,
		type S3ConfigDto,
		type BackupRecordDto,
		type DownloadFormat,
	} from '#lib/clients/backup-client.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';

	const INTERVALS: { value: 'daily' | 'weekly'; label: string }[] = [
		{ value: 'daily', label: t('account.backups.daily') },
		{ value: 'weekly', label: t('account.backups.weekly') },
	];

	let loading = $state(true);
	let saving = $state(false);
	let backingUp = $state(false);
	let restoringId = $state<string | null>(null);
	let hasConfig = $state(false);
	let hasSecretKey = $state(false);

	let endpointUrl = $state('');
	let bucket = $state('');
	let region = $state('us-east-1');
	let accessKeyId = $state('');
	let secretAccessKey = $state('');
	let showSecret = $state(false);
	let scheduleEnabled = $state(false);
	let scheduleInterval = $state<'daily' | 'weekly'>('daily');
	let scheduleTime = $state('03:00');
	let backupBeforeUpdate = $state(false);

	let history = $state<BackupRecordDto[]>([]);
	let confirmBackupId = $state<string | null>(null);
	let downloadFormat = $state<DownloadFormat>('tar');

	onMount(() => {
		load();
		// Refetch history when the tab regains focus if any backup/restore is
		// still in flight — the background task writes its terminal status to
		// the DB, and the UI is otherwise unaware of it until reload.
		const onVisibility = () => {
			if (document.visibilityState === 'visible' && history.some((r) => r.status === 'running')) {
				load();
			}
		};
		document.addEventListener('visibilitychange', onVisibility);
		// Live status while the page is open: poll every 1.5s only when at
		// least one row is still `running`. The interval self-cancels when
		// the last in-flight backup lands, and the teardown below cancels
		// it for good when the user navigates away — so scheduled backups
		// running in the background don't pay for polling no one is watching.
		const POLL_MS = 1500;
		let pollTimer: ReturnType<typeof setInterval> | null = null;
		$effect(() => {
			const hasRunning = history.some((r) => r.status === 'running');
			if (hasRunning && pollTimer === null) {
				pollTimer = setInterval(() => {
					if (document.visibilityState === 'visible') refreshHistory();
				}, POLL_MS);
			} else if (!hasRunning && pollTimer !== null) {
				clearInterval(pollTimer);
				pollTimer = null;
			}
		});
		return () => {
			document.removeEventListener('visibilitychange', onVisibility);
			if (pollTimer !== null) {
				clearInterval(pollTimer);
				pollTimer = null;
			}
		};
	});

	async function load() {
		loading = true;
		try {
			const [config, backups] = await Promise.all([getS3Config(), listBackupHistory()]);
			applyConfig(config);
			history = backups;
		} catch (e) {
			toast.error(t('account.backups.load_failed', { message: (e as Error).message }));
		} finally {
			loading = false;
		}
	}

	/// Polling-friendly refresh: refetches only the history rows and never
	/// touches the S3 config or the `loading` flag. Keeps the whole page from
	/// flashing the Loading… placeholder on every poll tick.
	async function refreshHistory() {
		try {
			history = await listBackupHistory();
		} catch (e) {
			toast.error(t('account.backups.refresh_failed', { message: (e as Error).message }));
		}
	}

	function applyConfig(config: S3ConfigDto | null) {
		hasConfig = config !== null;
		hasSecretKey = config?.has_secret_key ?? false;
		endpointUrl = config?.endpoint_url ?? '';
		bucket = config?.bucket ?? '';
		region = config?.region ?? 'us-east-1';
		accessKeyId = config?.access_key_id ?? '';
		scheduleEnabled = config?.schedule_enabled ?? false;
		scheduleInterval = config?.schedule_interval ?? 'daily';
		scheduleTime = config?.schedule_time ?? '03:00';
		backupBeforeUpdate = config?.backup_before_update ?? false;
	}

	async function saveConfig() {
		saving = true;
		try {
			const saved = await updateS3Config({
				endpoint_url: endpointUrl,
				bucket,
				region,
				access_key_id: accessKeyId,
				secret_access_key: secretAccessKey || undefined,
				schedule_enabled: scheduleEnabled,
				schedule_interval: scheduleEnabled ? scheduleInterval : null,
				schedule_time: scheduleEnabled ? scheduleTime : null,
				backup_before_update: backupBeforeUpdate,
			});
			applyConfig(saved);
			secretAccessKey = '';
			toast.success(t('account.backups.storage_saved'));
		} catch (e) {
			toast.error(t('account.backups.save_failed', { message: (e as Error).message }));
		} finally {
			saving = false;
		}
	}

	async function runBackupNow() {
		backingUp = true;
		try {
			const record = await triggerBackup();
			// Surface the new row immediately — the polling loop will pick up
			// the terminal status in <= 1.5s. Without this prepend, the row
			// only appears on the next tick, which feels laggy.
			history = [record, ...history];
			toast.success(t('account.backups.started'));
		} catch (e) {
			toast.error(t('account.backups.start_failed', { message: (e as Error).message }));
		} finally {
			backingUp = false;
		}
	}

	async function confirmRestore() {
		const id = confirmBackupId;
		if (!id) return;
		restoringId = id;
		try {
			await triggerRestore(id);
			toast.success(t('account.backups.restore_started'));
		} catch (e) {
			toast.error(t('account.backups.restore_failed', { message: (e as Error).message }));
		} finally {
			restoringId = null;
			confirmBackupId = null;
		}
	}

	function closeConfirm() {
		confirmBackupId = null;
	}

	function downloadBackup(backupId: string) {
		const a = document.createElement('a');
		a.href = downloadBackupUrl(backupId, downloadFormat);
		a.download = `backup-${backupId.slice(0, 8)}.${downloadFormat === '7z' ? '7z' : downloadFormat}`;
		document.body.appendChild(a);
		a.click();
		document.body.removeChild(a);
	}

	function formatDate(ms: number | null): string {
		return ms ? new Date(ms).toLocaleString() : '—';
	}

	function formatSize(bytes: number | null): string {
		if (!bytes) return '—';
		const units = ['B', 'KB', 'MB', 'GB', 'TB'];
		let value = bytes;
		let unit = 0;
		while (value >= 1024 && unit < units.length - 1) {
			value /= 1024;
			unit += 1;
		}
		return `${value.toFixed(1)} ${units[unit]}`;
	}

	function statusClass(status: string): string {
		switch (status) {
			case 'success':
				return 'bg-success/10 text-success';
			case 'failed':
				return 'bg-destructive/10 text-destructive';
			default:
				return 'bg-warning/10 text-warning';
		}
	}
</script>

<div class="flex flex-col gap-4">
	<div class="flex flex-wrap items-end justify-between gap-3">
		<div class="min-w-0">
			<h2 class="text-lg font-semibold tracking-tight">{t('account.backups.title')}</h2>
			<p class="mt-1 max-w-2xl text-sm text-muted-foreground">
				{t('account.backups.description')}
			</p>
		</div>
		<Button size="sm" onclick={runBackupNow} disabled={loading || !hasSecretKey || backingUp}>
			<UploadIcon class={backingUp ? 'animate-pulse motion-reduce:animate-none' : ''} />
			{backingUp ? t('account.backups.starting') : t('account.backups.backup_now')}
		</Button>
	</div>

	{#if loading}
		<SkeletonRows rows={4} height="h-14" label={t('account.backups.loading')} />
	{:else}
		<div class="flex flex-col gap-4">
			<SettingsSection
				title={t('account.backups.storage_destinations')}
				description={t('account.backups.storage_destinations_hint')}>
				{#snippet aside()}
					<span
						class="mt-2.5 inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-xs font-medium {hasSecretKey
							? 'border-success/30 bg-success/10 text-success'
							: 'text-muted-foreground'}">
						{#if hasSecretKey}<CheckIcon class="size-3" />{:else}<XIcon class="size-3" />{/if}
						{hasSecretKey ? t('account.backups.configured') : t('account.backups.not_configured')}
					</span>
				{/snippet}
				<div>
					<h3 class="text-[13px] font-semibold">{t('account.backups.default_destination')}</h3>
					<p class="mt-0.5 text-xs text-muted-foreground">
						{t('account.backups.default_destination_hint')}
					</p>
				</div>
				<form
					class="flex flex-col gap-4"
					onsubmit={(event) => {
						event.preventDefault();
						saveConfig();
					}}>
					<div class="grid gap-4 sm:grid-cols-2">
						<div class="grid content-start gap-1.5">
							<Label for="s3-endpoint">{t('account.backups.endpoint_url')}</Label>
							<Input
								id="s3-endpoint"
								bind:value={endpointUrl}
								placeholder="https://s3.example.com"
								class="font-mono"
								required />
						</div>
						<div class="grid content-start gap-1.5">
							<Label for="s3-bucket">{t('account.backups.bucket')}</Label>
							<Input
								id="s3-bucket"
								bind:value={bucket}
								placeholder="litepod-backups"
								class="font-mono"
								required />
						</div>
						<div class="grid content-start gap-1.5">
							<Label for="s3-region">{t('account.backups.region')}</Label>
							<Input
								id="s3-region"
								bind:value={region}
								placeholder="us-east-1"
								class="font-mono"
								required />
						</div>
						<div class="grid content-start gap-1.5">
							<Label for="s3-access-key">{t('account.backups.access_key_id')}</Label>
							<Input
								id="s3-access-key"
								bind:value={accessKeyId}
								autocomplete="off"
								class="font-mono"
								required />
						</div>
						<div class="grid content-start gap-1.5 sm:col-span-2">
							<Label for="s3-secret-key">{t('account.backups.secret_access_key')}</Label>
							<div class="relative">
								<Input
									id="s3-secret-key"
									type={showSecret ? 'text' : 'password'}
									bind:value={secretAccessKey}
									placeholder={hasSecretKey
										? t('account.backups.secret_keep_placeholder')
										: t('account.backups.secret_required_placeholder')}
									autocomplete="off"
									class="pr-10 font-mono" />
								<Button
									variant="ghost"
									size="icon"
									class="absolute right-1 top-1/2 size-7 -translate-y-1/2"
									onclick={() => (showSecret = !showSecret)}
									aria-label={t('account.backups.toggle_visibility')}
									type="button">
									{#if showSecret}
										<EyeOffIcon class="size-4" />
									{:else}
										<EyeIcon class="size-4" />
									{/if}
								</Button>
							</div>
						</div>
					</div>
					<div class="flex justify-end">
						<Button type="submit" disabled={saving}
							>{saving ? t('account.common.saving') : t('account.common.save')}</Button>
					</div>
				</form>
			</SettingsSection>

			<SettingsSection
				title={t('account.backups.schedule')}
				description={t('account.backups.schedule_description')}>
				<div class="flex items-center justify-between gap-4">
					<div class="min-w-0">
						<Label for="schedule-enabled">{t('account.backups.enable_schedule')}</Label>
						<p class="mt-1 text-xs text-muted-foreground">
							{t('account.backups.enable_schedule_hint')}
						</p>
					</div>
					<SoftSwitch id="schedule-enabled" bind:checked={scheduleEnabled} />
				</div>
				<div class="flex items-center justify-between gap-4">
					<div class="min-w-0">
						<Label for="backup-before-update">{t('account.backups.backup_before_update')}</Label>
						<p class="mt-1 text-xs text-muted-foreground">
							{t('account.backups.backup_before_update_hint')}
						</p>
					</div>
					<SoftSwitch id="backup-before-update" bind:checked={backupBeforeUpdate} />
				</div>
				<div
					class="grid gap-4 transition-opacity duration-200 motion-reduce:transition-none sm:grid-cols-2"
					class:opacity-60={!scheduleEnabled}>
					<div class="grid content-start gap-1.5">
						<Label for="schedule-interval">{t('account.backups.interval')}</Label>
						<Select.Root
							type="single"
							value={scheduleInterval}
							onValueChange={(v) => (scheduleInterval = v as 'daily' | 'weekly')}
							disabled={!scheduleEnabled}>
							<Select.Trigger id="schedule-interval" class="w-full">
								{INTERVALS.find((i) => i.value === scheduleInterval)?.label}
							</Select.Trigger>
							<Select.Content>
								{#each INTERVALS as i}
									<Select.Item value={i.value}>{i.label}</Select.Item>
								{/each}
							</Select.Content>
						</Select.Root>
					</div>
					<div class="grid content-start gap-1.5">
						<Label for="schedule-time">{t('account.backups.time_utc')}</Label>
						<Input
							id="schedule-time"
							type="time"
							class="font-mono tabular-nums"
							bind:value={scheduleTime}
							disabled={!scheduleEnabled} />
					</div>
				</div>
				<div class="flex justify-end">
					<Button onclick={saveConfig} disabled={saving}
						>{saving ? t('account.common.saving') : t('account.backups.save_schedule')}</Button>
				</div>
			</SettingsSection>
		</div>

		<section
			class="lp-panel flex flex-col gap-4 rounded-[15px] border border-foreground/9 bg-card p-4 text-sm text-card-foreground sm:p-5">
			<div class="flex flex-wrap items-start justify-between gap-3">
				<div class="min-w-0">
					<h2 class="text-[15px] font-semibold">{t('account.backups.history_title')}</h2>
					<p class="mt-1 text-[12.5px] text-muted-foreground">
						{t('account.backups.history_description')}
					</p>
				</div>
				<div class="flex flex-wrap items-center gap-2">
					{#if history.length > 0}
						<Label for="download-format" class="text-xs text-muted-foreground"
							>{t('account.backups.download_as')}</Label>
						<Select.Root
							type="single"
							value={downloadFormat}
							onValueChange={(v) => (downloadFormat = v as DownloadFormat)}>
							<Select.Trigger id="download-format" size="sm" class="w-20">
								{downloadFormat === '7z' ? '7z' : downloadFormat.toUpperCase()}
							</Select.Trigger>
							<Select.Content>
								<Select.Item value="tar">TAR</Select.Item>
								<Select.Item value="zip">ZIP</Select.Item>
								<Select.Item value="7z">7Z</Select.Item>
							</Select.Content>
						</Select.Root>
					{/if}
					<Button
						variant="ghost"
						size="icon-sm"
						onclick={load}
						disabled={loading}
						aria-label={t('account.backups.refresh_history')}>
						<RefreshCwIcon class={loading ? 'animate-spin motion-reduce:animate-none' : ''} />
					</Button>
				</div>
			</div>
			{#if history.length === 0}
				<Empty.Root class="rounded-xl border border-dashed border-foreground/15">
					<Empty.Header>
						<Empty.Media variant="icon"><DatabaseBackupIcon /></Empty.Media>
						<Empty.Title>{t('account.backups.empty_title')}</Empty.Title>
						<Empty.Description>
							{hasSecretKey
								? t('account.backups.empty_description')
								: t('account.backups.empty_description_needs_config')}
						</Empty.Description>
					</Empty.Header>
					<Empty.Content>
						<Button onclick={runBackupNow} disabled={!hasSecretKey || backingUp}>
							{backingUp ? t('account.backups.running') : t('account.backups.run_backup_now')}
						</Button>
					</Empty.Content>
				</Empty.Root>
			{:else}
				<div class="lp-inset overflow-hidden rounded-[10px] border border-foreground/9">
					<Table.Root>
						<Table.Header>
							<Table.Row>
								<Table.Head>{t('account.backups.col_started')}</Table.Head>
								<Table.Head>{t('account.backups.col_finished')}</Table.Head>
								<Table.Head>{t('account.backups.col_status')}</Table.Head>
								<Table.Head>{t('account.backups.col_size')}</Table.Head>
								<Table.Head>{t('account.backups.col_triggered_by')}</Table.Head>
								<Table.Head class="text-right">{t('account.common.actions')}</Table.Head>
							</Table.Row>
						</Table.Header>
						<Table.Body>
							{#each history as backup (backup.id)}
								<Table.Row>
									<Table.Cell class="font-mono text-xs tabular-nums"
										>{formatDate(backup.started_at)}</Table.Cell>
									<Table.Cell class="font-mono text-xs tabular-nums"
										>{formatDate(backup.finished_at)}</Table.Cell>
									<Table.Cell>
										<span
											class="inline-flex items-center gap-1.5 rounded-full border border-current/25 px-2 py-0.5 text-xs font-medium {statusClass(
												backup.status,
											)}">
											<span
												class="size-1.5 rounded-full bg-current {backup.status === 'running'
													? 'animate-pulse motion-reduce:animate-none'
													: ''}"></span>
											{backup.status}
										</span>
									</Table.Cell>
									<Table.Cell class="font-mono text-xs tabular-nums"
										>{formatSize(backup.size_bytes)}</Table.Cell>
									<Table.Cell class="text-sm capitalize">{backup.triggered_by}</Table.Cell>
									<Table.Cell class="text-right">
										<div class="flex items-center justify-end gap-1">
											<Button
												variant="ghost"
												size="icon-sm"
												aria-label={t('account.common.download')}
												disabled={backup.status !== 'success'}
												onclick={() => downloadBackup(backup.id)}>
												<DownloadIcon />
											</Button>
											<Button
												variant="ghost"
												size="sm"
												disabled={backup.status !== 'success' || restoringId !== null}
												onclick={() => (confirmBackupId = backup.id)}>
												<RotateCcwIcon />
												{t('account.backups.restore')}
											</Button>
										</div>
									</Table.Cell>
								</Table.Row>
							{/each}
						</Table.Body>
					</Table.Root>
				</div>
			{/if}
		</section>
	{/if}
</div>

<AlertDialog.Root open={confirmBackupId !== null}>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title>{t('account.backups.restore_confirm_title')}</AlertDialog.Title>
			<AlertDialog.Description>
				{t('account.backups.restore_confirm_description')}
			</AlertDialog.Description>
		</AlertDialog.Header>
		<AlertDialog.Footer>
			<AlertDialog.Cancel disabled={restoringId !== null} onclick={closeConfirm}>
				{t('account.common.cancel')}
			</AlertDialog.Cancel>
			<DeleteConfirmControl
				label={t('account.backups.restore_backup')}
				pendingLabel={t('account.backups.restoring')}
				variant="primary"
				icon={RotateCcwIcon}
				disabled={restoringId !== null}
				onconfirm={confirmRestore} />
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>
