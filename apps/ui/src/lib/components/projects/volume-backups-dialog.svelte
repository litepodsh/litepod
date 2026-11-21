<script lang="ts">
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import * as Input from '#lib/components/ui/input/index.js';
	import * as Table from '#lib/components/ui/table/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Switch } from '#lib/components/ui/switch/index.js';
	import * as Alert from '#lib/components/ui/alert/index.js';
	import InfoIcon from '@lucide/svelte/icons/info';
	import LoaderIcon from '@lucide/svelte/icons/loader-circle';
	import RefreshIcon from '@lucide/svelte/icons/refresh-cw';
	import TrashIcon from '@lucide/svelte/icons/trash-2';
	import {
		volumeLabel,
		type VolumeBackupClient,
		type VolumeBackupDto,
		type VolumeRestoreTarget,
	} from '#lib/clients/project-client.js';
	import { listS3TargetOptions, type S3TargetOption } from '#lib/clients/backup-client.js';
	import { t } from '#lib/i18n/index.svelte.js';
	import { toast, toastError } from '#lib/utils/toast.js';
	import { pollWhileVisible } from '#lib/utils/poll.js';

	let {
		open = $bindable(false),
		organizationId,
		volumeName,
		client,
		onRestored,
	}: {
		open?: boolean;
		organizationId: string;
		/** Podman name of the volume. */
		volumeName: string;
		client: VolumeBackupClient;
		/** A restore changed the volumes: refresh the lists. */
		onRestored: () => Promise<void>;
	} = $props();

	let history = $state<VolumeBackupDto[]>([]);
	let targets = $state<S3TargetOption[]>([]);
	/** The install-wide default S3 storage exists (never on cloud). */
	let defaultAvailable = $state(false);
	/** Neither an organization target nor a default storage to back up to. */
	const noDestination = $derived(targets.length === 0 && !defaultAvailable);
	const ADD_TARGET = '__add_s3_target__';
	const addTargetUrl = $derived(`/organizations/${organizationId}?tab=backup-storage`);
	let loading = $state(true);
	let saving = $state(false);
	let restoring = $state<string | null>(null);
	let enabledInput = $state(false);
	let cronInput = $state('');
	let retentionInput = $state('');
	let targetInput = $state('');
	let stopAppInput = $state(false);
	let stopPoll: (() => void) | null = null;

	const label = $derived(volumeLabel(volumeName));
	const running = $derived(history.some((run) => run.status === 'running'));
	const lastSucceeded = $derived(history.find((run) => run.status === 'succeeded'));

	async function load() {
		loading = true;
		try {
			const [policy, runs] = await Promise.all([client.getPolicy(), client.list()]);
			enabledInput = policy.enabled;
			cronInput = policy.cron_expression;
			retentionInput = String(policy.retention_count);
			targetInput = policy.s3_target_id ?? '';
			stopAppInput = policy.stop_app;
			history = runs;
			if (runs.some((run) => run.status === 'running')) watch();
			defaultAvailable = policy.default_available;
			await loadTargets();
		} catch (err) {
			toastError(err, t('projects.db_backup.load_failed'));
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		if (open) void load();
		return () => stopPoll?.();
	});

	async function loadTargets() {
		targets = await listS3TargetOptions(organizationId).catch(() => []);
	}

	/** "Add S3 target" opens the organization's backup storage in a new tab; the
	 *  list reloads when the user comes back to this one. */
	function onTargetChange(event: Event & { currentTarget: HTMLSelectElement }) {
		if (event.currentTarget.value !== ADD_TARGET) {
			targetInput = event.currentTarget.value;
			return;
		}
		event.currentTarget.value = targetInput;
		window.open(addTargetUrl, '_blank', 'noopener');
		window.addEventListener('focus', () => void loadTargets(), { once: true });
	}

	/** Polls until no run is in progress (scheduled ones included). */
	function watch() {
		stopPoll?.();
		stopPoll = pollWhileVisible(async () => {
			history = await client.list();
			if (!history.some((run) => run.status === 'running')) stopPoll?.();
		}, 4000);
	}

	async function savePolicy() {
		saving = true;
		try {
			await client.updatePolicy({
				enabled: enabledInput,
				cron_expression: cronInput,
				retention_count: Number(retentionInput),
				s3_target_id: targetInput || null,
				stop_app: stopAppInput,
			});
			toast.success(t('projects.db_backup.saved'));
		} catch (err) {
			toastError(err, t('projects.db_backup.save_failed'));
		} finally {
			saving = false;
		}
	}

	async function runNow() {
		try {
			history = [await client.run(targetInput || null, stopAppInput), ...history];
			toast.success(t('projects.db_backup.run_started'));
			watch();
		} catch (err) {
			toastError(err, t('projects.db_backup.run_failed'));
		}
	}

	async function restore(run: VolumeBackupDto, target: VolumeRestoreTarget) {
		let name: string | null = null;
		if (target === 'new_volume') {
			name = window.prompt(t('projects.volume_backup.restore_prompt'), '');
			if (name === null) return;
		} else if (!window.confirm(t('projects.volume_backup.restore_in_place_confirm'))) {
			return;
		}
		restoring = run.id;
		try {
			const restored = await client.restore(run.id, target, name?.trim() || null);
			toast.success(
				restored
					? t('projects.volume_backup.restore_done', { name: restored.name })
					: t('projects.volume_backup.restore_in_place_done'),
			);
			history = await client.list();
			await onRestored();
		} catch (err) {
			toastError(err, t('projects.db_backup.restore_failed'));
		} finally {
			restoring = null;
		}
	}

	async function remove(run: VolumeBackupDto) {
		if (!window.confirm(t('projects.db_backup.delete_confirm'))) return;
		try {
			await client.remove(run.id);
			history = history.filter((r) => r.id !== run.id);
			toast.success(t('projects.db_backup.deleted'));
		} catch (err) {
			toastError(err, t('projects.db_backup.delete_failed'));
		}
	}

	function statusLabel(status: string): string {
		if (status === 'running') return t('projects.db_backup.status_running');
		if (status === 'succeeded') return t('projects.db_backup.status_succeeded');
		return t('projects.db_backup.status_failed');
	}

	function formatTime(millis: number | null): string {
		return millis == null ? '—' : new Date(millis).toLocaleString();
	}

	function formatSize(bytes: number | null): string {
		if (bytes == null) return '—';
		const units = ['B', 'KB', 'MB', 'GB', 'TB'];
		let value = bytes;
		let unit = 0;
		while (value >= 1024 && unit < units.length - 1) {
			value /= 1024;
			unit += 1;
		}
		return unit === 0 ? `${value} B` : `${value.toFixed(1)} ${units[unit]}`;
	}
</script>

<Dialog.Root bind:open>
	<Dialog.Content class="max-h-[90vh] overflow-y-auto sm:max-w-3xl">
		<Dialog.Header>
			<Dialog.Title>{t('projects.volume_backup.title', { name: label })}</Dialog.Title>
			<Dialog.Description>{t('projects.volume_backup.description')}</Dialog.Description>
		</Dialog.Header>

		{#if loading}
			<div class="flex items-center gap-2 text-sm text-muted-foreground">
				<LoaderIcon class="size-4 animate-spin" />{t('projects.db_backup.loading')}
			</div>
		{:else}
			<div class="grid gap-4">
				<p class="text-sm text-muted-foreground">
					{lastSucceeded
						? t('projects.volume_backup.last_succeeded', {
								time: formatTime(lastSucceeded.finished_at),
							})
						: t('projects.volume_backup.never')}
				</p>

				<div class="flex items-center justify-between gap-4">
					<div class="grid gap-1">
						<label class="text-sm leading-none font-medium" for="volume-backup-enabled"
							>{t('projects.db_backup.enabled')}</label>
						<p class="text-sm text-muted-foreground">{t('projects.db_backup.enabled_hint')}</p>
					</div>
					<Switch
						id="volume-backup-enabled"
						checked={enabledInput}
						disabled={saving}
						onCheckedChange={(next) => (enabledInput = next)} />
				</div>

				<div class="flex items-center justify-between gap-4">
					<div class="grid gap-1">
						<label class="text-sm leading-none font-medium" for="volume-backup-stop-app"
							>{t('projects.volume_backup.stop_app')}</label>
						<p class="text-sm text-muted-foreground">{t('projects.volume_backup.stop_app_hint')}</p>
					</div>
					<Switch
						id="volume-backup-stop-app"
						checked={stopAppInput}
						disabled={saving}
						onCheckedChange={(next) => (stopAppInput = next)} />
				</div>

				<div class="grid grid-cols-1 gap-3 sm:grid-cols-3">
					<div class="grid gap-1">
						<label class="text-sm leading-none font-medium" for="volume-backup-cron"
							>{t('projects.db_backup.cron')}</label>
						<Input.Root
							id="volume-backup-cron"
							bind:value={cronInput}
							disabled={saving}
							placeholder="0 3 * * *" />
					</div>
					<div class="grid gap-1">
						<label class="text-sm leading-none font-medium" for="volume-backup-retention"
							>{t('projects.db_backup.retention')}</label>
						<Input.Root
							id="volume-backup-retention"
							type="number"
							min={1}
							bind:value={retentionInput}
							disabled={saving} />
					</div>
					<div class="grid gap-1">
						<label class="text-sm leading-none font-medium" for="volume-backup-target"
							>{t('projects.db_backup.destination')}</label>
						<select
							id="volume-backup-target"
							class="flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm disabled:cursor-not-allowed disabled:opacity-50"
							value={targetInput}
							onchange={onTargetChange}
							disabled={saving}>
							<option value=""
								>{defaultAvailable
									? t('projects.db_backup.destination_default')
									: t('projects.db_backup.destination_none')}</option>
							{#each targets as target (target.id)}
								<option value={target.id}>{target.name}</option>
							{/each}
							<option value={ADD_TARGET}>{t('projects.volume_backup.add_target')}</option>
						</select>
					</div>
				</div>
				{#if noDestination}
					<p class="rounded-md border border-dashed p-3 text-sm text-muted-foreground">
						{t('projects.volume_backup.no_destination')}
						<a class="font-medium text-foreground underline" href={addTargetUrl} target="_blank" rel="noopener"
							>{t('projects.volume_backup.add_target')}</a>
					</p>
				{/if}
				<p class="text-xs text-muted-foreground">
					{t('projects.db_backup.cron_hint')}
					{t('projects.db_backup.retention_hint')}
				</p>

				<div class="flex flex-wrap gap-2">
					<Button size="sm" disabled={saving} onclick={() => void savePolicy()}
						>{t('projects.db_backup.save')}</Button>
					<Button
						size="sm"
						variant="outline"
						disabled={running || noDestination}
						onclick={() => void runNow()}>
						{#if running}
							<LoaderIcon class="size-4 animate-spin" />{t('projects.db_backup.running')}
						{:else}
							<RefreshIcon class="size-4" />{t('projects.db_backup.run_now')}
						{/if}
					</Button>
				</div>

				<p class="mt-2 font-medium">{t('projects.db_backup.history')}</p>
				<Alert.Root>
					<InfoIcon />
					<Alert.Title>{t('projects.volume_backup.how_title')}</Alert.Title>
					<Alert.Description>
						<ul class="list-disc space-y-1 pl-4">
							<li>{t('projects.volume_backup.how_in_place')}</li>
							<li>{t('projects.volume_backup.how_new_volume')}</li>
							<li>{t('projects.volume_backup.how_managed')}</li>
							<li>{t('projects.volume_backup.how_compose')}</li>
						</ul>
					</Alert.Description>
				</Alert.Root>
				{#if history.length === 0}
					<p class="text-sm text-muted-foreground">{t('projects.db_backup.history_empty')}</p>
				{:else}
					<Table.Root>
						<Table.Header>
							<Table.Row>
								<Table.Head>{t('projects.db_backup.started')}</Table.Head>
								<Table.Head>{t('projects.db_backup.status')}</Table.Head>
								<Table.Head>{t('projects.db_backup.size')}</Table.Head>
								<Table.Head></Table.Head>
							</Table.Row>
						</Table.Header>
						<Table.Body>
							{#each history as run (run.id)}
								<Table.Row>
									<Table.Cell>
										{formatTime(run.started_at)}
										{#if run.triggered_by === 'pre-restore'}
											<p class="text-xs text-muted-foreground">
												{t('projects.volume_backup.pre_restore')}
											</p>
										{/if}
									</Table.Cell>
									<Table.Cell>
										<Badge
											variant={run.status === 'succeeded'
												? 'default'
												: run.status === 'running'
													? 'secondary'
													: 'destructive'}>{statusLabel(run.status)}</Badge>
										{#if run.error_message}
											<p
												class="mt-1 max-w-xs text-xs break-words whitespace-normal text-muted-foreground">
												{run.error_message}
											</p>
										{/if}
									</Table.Cell>
									<Table.Cell>{formatSize(run.size_bytes)}</Table.Cell>
									<Table.Cell class="text-right">
										{#if run.status !== 'running'}
											<div class="flex justify-end gap-1">
												{#if run.status === 'succeeded'}
													<Button
														variant="outline"
														size="sm"
														disabled={restoring !== null || running}
														onclick={() => void restore(run, 'new_volume')}>
														{#if restoring === run.id}<LoaderIcon
																class="size-4 animate-spin" />{/if}
														{t('projects.volume_backup.restore')}
													</Button>
													<Button
														variant="outline"
														size="sm"
														disabled={restoring !== null || running}
														onclick={() => void restore(run, 'in_place')}>
														{t('projects.volume_backup.restore_in_place')}
													</Button>
												{/if}
												<Button
													variant="destructive"
													size="sm"
													aria-label={t('projects.common.delete')}
													onclick={() => void remove(run)}><TrashIcon class="size-4" /></Button>
											</div>
										{/if}
									</Table.Cell>
								</Table.Row>
							{/each}
						</Table.Body>
					</Table.Root>
				{/if}
			</div>
		{/if}
	</Dialog.Content>
</Dialog.Root>
