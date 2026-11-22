<script lang="ts">
	import { untrack } from 'svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Switch } from '#lib/components/ui/switch/index.js';
	import { NativeSelect } from '#lib/components/ui/native-select/index.js';
	import * as DropdownMenu from '#lib/components/ui/dropdown-menu/index.js';
	import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
	import LoaderIcon from '@lucide/svelte/icons/loader-circle';
	import EllipsisIcon from '@lucide/svelte/icons/ellipsis';
	import UploadIcon from '@lucide/svelte/icons/upload';
	import CheckIcon from '@lucide/svelte/icons/check';
	import type {
		AppVolumeBackups,
		VolumeBackupClient,
		VolumeBackupDto,
	} from '#lib/clients/project-client.js';
	import { listS3TargetOptions, type S3TargetOption } from '#lib/clients/backup-client.js';
	import { t } from '#lib/i18n/index.svelte.js';
	import { toast, toastError } from '#lib/utils/toast.js';
	import { pollWhileVisible } from '#lib/utils/poll.js';

	let {
		organizationId,
		volumeNames,
		client,
		backupsFor,
		onImport,
		onRestored,
		labelFor,
		runningVolumes = $bindable([]),
		canRun = $bindable(false),
		needsDestination = $bindable(false),
	}: {
		organizationId: string;
		/** Podman names of the volumes that can be backed up: live named and managed. */
		volumeNames: string[];
		/** What to call a volume: a managed volume's name, else the Compose part. */
		labelFor: (volumeName: string) => string;
		client: AppVolumeBackups;
		backupsFor: (volumeName: string) => VolumeBackupClient;
		/** Open "Add volume" in import mode (a `.tar` from S3 or uploaded). */
		onImport: () => void;
		/** Volumes changed: refresh the tab. */
		onRestored: () => Promise<void>;
		/** Volumes with a backup starting or running, for their row's button. */
		runningVolumes?: string[];
		/** `backUp` has somewhere to store the copy. */
		canRun?: boolean;
		/** Loaded, and there is no S3 destination to back up to. */
		needsDestination?: boolean;
	} = $props();

	type Run = {
		/** `group_id`, or the row id for a lone backup (pre-restore copies). */
		key: string;
		grouped: boolean;
		startedAt: number;
		volumes: VolumeBackupDto[];
		status: 'running' | 'succeeded' | 'failed';
		size: number;
		triggeredBy: string;
	};
	type Settings = {
		target: string;
		enabled: boolean;
		cron: string;
		retention: number;
		stopApp: boolean;
	};

	const PRESETS = [
		{ value: '0 3 * * *', label: () => t('projects.volume_backup.daily') },
		{ value: '0 */6 * * *', label: () => t('projects.volume_backup.every_6_hours') },
		{ value: '0 3 * * 0', label: () => t('projects.volume_backup.weekly') },
	];
	const CUSTOM = 'custom';
	const addTargetUrl = $derived(`/organizations/${organizationId}?tab=backup-storage`);

	let rows = $state<VolumeBackupDto[]>([]);
	let targets = $state<S3TargetOption[]>([]);
	let defaultAvailable = $state(false);
	let loading = $state(true);
	let settings = $state<Settings>({
		target: '',
		enabled: false,
		cron: PRESETS[0].value,
		retention: 7,
		stopApp: false,
	});
	let saved = $state('');
	/** Sorted names of the volumes with a schedule, as saved. */
	let savedScheduled = $state('');
	let frequency = $state(PRESETS[0].value);
	let saving = $state(false);
	/** Volumes in the automatic schedule (edited here, saved with the settings). */
	let scheduled = $state<string[]>([]);
	/** Volumes whose "Back up" request is on its way. */
	let starting = $state<string[]>([]);
	/** Runs started from a row button: group id → loading toast to resolve. */
	const watched = new Map<string, string | number>();
	let acting = $state<string | null>(null);
	let confirm = $state<{
		title: string;
		description: string;
		action: string;
		destructive?: boolean;
		run: () => Promise<void>;
	} | null>(null);
	let stopPoll: (() => void) | null = null;
	/** Bumped per load: a slower, older load (say, while the app restarts) must not win. */
	let loadId = 0;

	const runs = $derived.by(() => {
		const byKey = new Map<string, Run>();
		for (const row of rows) {
			const key = row.group_id ?? row.id;
			const run = byKey.get(key) ?? {
				key,
				grouped: !!row.group_id,
				startedAt: row.started_at,
				volumes: [],
				status: 'succeeded',
				size: 0,
				triggeredBy: row.triggered_by,
			};
			run.volumes.push(row);
			run.startedAt = Math.min(run.startedAt, row.started_at);
			run.size += row.size_bytes ?? 0;
			if (row.status === 'running') run.status = 'running';
			else if (row.status === 'failed' && run.status !== 'running') run.status = 'failed';
			byKey.set(key, run);
		}
		return [...byKey.values()].sort((a, b) => b.startedAt - a.startedAt);
	});
	const running = $derived(runs.some((run) => run.status === 'running'));
	const noDestination = $derived(targets.length === 0 && !defaultAvailable);
	const scheduledKey = $derived([...scheduled].sort().join('\n'));
	const dirty = $derived(
		JSON.stringify(settings) !== saved || (settings.enabled && scheduledKey !== savedScheduled),
	);
	const volumeKey = $derived(volumeNames.join('\n'));

	$effect(() => {
		runningVolumes = [
			...starting,
			...rows.filter((row) => row.status === 'running').map((row) => row.volume_name),
		];
		canRun = !loading && !noDestination;
		needsDestination = !loading && noDestination;
	});

	$effect(() => {
		void volumeKey;
		untrack(() => void load());
		return () => stopPoll?.();
	});

	async function load() {
		const id = ++loadId;
		try {
			const [list, policies, options] = await Promise.all([
				client.list(),
				// A server group has no volume policies; the history still shows.
				Promise.all(volumeNames.map((name) => backupsFor(name).getPolicy())).catch(() => []),
				listS3TargetOptions(organizationId).catch(() => []),
			]);
			if (id !== loadId) return;
			rows = list.runs;
			defaultAvailable = list.default_available;
			targets = options;
			const base = policies.find((policy) => policy.enabled) ?? policies[0];
			scheduled = policies.filter((policy) => policy.enabled).map((p) => p.volume_name);
			savedScheduled = [...scheduled].sort().join('\n');
			settings = {
				target: base?.s3_target_id ?? '',
				enabled: base?.enabled ?? false,
				cron: base?.enabled ? base.cron_expression : PRESETS[0].value,
				retention: base?.retention_count ?? 7,
				stopApp: base?.stop_app ?? false,
			};
			saved = JSON.stringify(settings);
			frequency = PRESETS.some((preset) => preset.value === settings.cron) ? settings.cron : CUSTOM;
			if (!settings.target && !defaultAvailable && targets.length) settings.target = targets[0].id;
			if (running) watch();
		} catch (err) {
			toastError(err, t('projects.db_backup.load_failed'));
		} finally {
			loading = false;
		}
	}

	async function refreshHistory() {
		rows = (await client.list()).runs;
		for (const [groupId, toastId] of watched) {
			const run = runs.find((candidate) => candidate.key === groupId);
			if (!run || run.status === 'running') continue;
			watched.delete(groupId);
			const name = run.volumes.map((volume) => labelFor(volume.volume_name)).join(', ');
			const failure = run.volumes.find((volume) => volume.error_message)?.error_message;
			if (run.status === 'succeeded')
				toast.success(t('projects.volume_backup.done', { name, size: formatSize(run.size) }), {
					id: toastId,
				});
			else
				toast.error(t('projects.volume_backup.failed', { name, error: failure ?? '' }), {
					id: toastId,
				});
		}
	}

	function watch() {
		stopPoll?.();
		stopPoll = pollWhileVisible(async () => {
			await refreshHistory();
			if (!running) stopPoll?.();
		}, 3000);
	}

	/** "Add destination" opens the organization's storage in a new tab; the
	 *  list reloads when the user comes back. */
	function onTargetChange(event: Event & { currentTarget: HTMLSelectElement }) {
		if (event.currentTarget.value !== '__add__') {
			settings.target = event.currentTarget.value;
			return;
		}
		event.currentTarget.value = settings.target;
		addDestination();
	}

	/** Opens the organization's backup storage in a new tab; the destinations
	 *  reload when the user comes back, so the buttons turn on without a refresh. */
	export function addDestination() {
		window.open(addTargetUrl, '_blank', 'noopener');
		window.addEventListener(
			'focus',
			async () => {
				targets = await listS3TargetOptions(organizationId).catch(() => targets);
				if (!settings.target && !defaultAvailable && targets.length)
					settings.target = targets[0].id;
			},
			{ once: true },
		);
	}

	async function save() {
		saving = true;
		try {
			await Promise.all(
				volumeNames.map((name) =>
					backupsFor(name).updatePolicy({
						enabled: settings.enabled && scheduled.includes(name),
						cron_expression: settings.cron.trim(),
						retention_count: Number(settings.retention),
						s3_target_id: settings.target || null,
						stop_app: settings.stopApp,
					}),
				),
			);
			saved = JSON.stringify(settings);
			savedScheduled = settings.enabled ? scheduledKey : '';
			toast.success(t('projects.db_backup.saved'));
		} catch (err) {
			toastError(err, t('projects.db_backup.save_failed'));
		} finally {
			saving = false;
		}
	}

	/** Backs up one volume; called from its row in Live mounts or Managed volumes.
	 *  A loading toast follows the run until it succeeds or fails. */
	export async function backUp(volumeName: string) {
		const name = labelFor(volumeName);
		starting = [...starting, volumeName];
		const toastId = toast.loading(t('projects.volume_backup.started', { name }));
		try {
			const groupId = await client.run(settings.target || null, settings.stopApp, [volumeName]);
			watched.set(groupId, toastId);
			await refreshHistory();
			watch();
		} catch (err) {
			toast.error(err instanceof Error ? err.message : t('projects.db_backup.run_failed'), {
				id: toastId,
			});
		} finally {
			starting = starting.filter((candidate) => candidate !== volumeName);
		}
	}

	async function act(run: Run, action: () => Promise<void>, failure: string) {
		acting = run.key;
		try {
			await action();
		} catch (err) {
			toastError(err, failure);
		} finally {
			acting = null;
			await refreshHistory().catch(() => {});
		}
	}

	function askRestore(run: Run) {
		confirm = {
			title: t('projects.volume_backup.restore_title'),
			description: t('projects.volume_backup.restore_confirm', { count: run.volumes.length }),
			action: t('projects.volume_backup.restore'),
			run: () =>
				act(
					run,
					async () => {
						if (run.grouped) await client.restore(run.key);
						else
							await backupsFor(run.volumes[0].volume_name).restore(run.volumes[0].id, 'in_place');
						toast.success(t('projects.volume_backup.restore_done'));
						await onRestored();
						// shortcut: Podman may still be recreating the containers; refresh once more instead of polling.
						setTimeout(() => void onRestored(), 3000);
					},
					t('projects.db_backup.restore_failed'),
				),
		};
	}

	function restoreAsNew(run: Run, volume: VolumeBackupDto) {
		void act(
			run,
			async () => {
				const created = await backupsFor(volume.volume_name).restore(volume.id, 'new_volume');
				toast.success(t('projects.volume_backup.restore_new_done', { name: created?.name ?? '' }));
				await onRestored();
			},
			t('projects.db_backup.restore_failed'),
		);
	}

	function askDelete(run: Run) {
		confirm = {
			title: t('projects.volume_backup.delete_title'),
			description: t('projects.db_backup.delete_confirm'),
			action: t('projects.common.delete'),
			destructive: true,
			run: () =>
				act(
					run,
					async () => {
						for (const volume of run.volumes)
							await backupsFor(volume.volume_name).remove(volume.id);
						toast.success(t('projects.db_backup.deleted'));
					},
					t('projects.db_backup.delete_failed'),
				),
		};
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

	function origin(triggeredBy: string) {
		if (triggeredBy === 'scheduled') return t('projects.volume_backup.by_schedule');
		if (triggeredBy === 'pre-restore') return t('projects.volume_backup.pre_restore');
		return triggeredBy;
	}
</script>

<section class="mt-4 rounded-xl border p-5">
	<div class="flex flex-wrap items-start justify-between gap-3">
		<div class="min-w-0">
			<p class="font-medium">{t('projects.volume_backup.title')}</p>
			<p class="mt-1 text-sm text-muted-foreground">
				{volumeNames.length
					? t('projects.volume_backup.description')
					: t('projects.volume_backup.no_volumes')}
			</p>
		</div>
		<Button variant="outline" onclick={onImport}
			><UploadIcon />{t('projects.volume_backup.import')}</Button>
	</div>

	{#if loading}
		<div class="mt-4 flex items-center gap-2 text-sm text-muted-foreground">
			<LoaderIcon class="size-4 animate-spin" />{t('projects.db_backup.loading')}
		</div>
	{:else}
		{#if noDestination}
			<p class="mt-4 rounded-md border border-dashed p-3 text-sm text-muted-foreground">
				{t('projects.volume_backup.no_destination')}
				<a
					class="font-medium text-foreground underline"
					href={addTargetUrl}
					target="_blank"
					rel="noopener"
					onclick={(event) => {
						event.preventDefault();
						addDestination();
					}}>{t('projects.volume_backup.add_target')}</a>
			</p>
		{:else if volumeNames.length}
			<div class="mt-4 grid gap-4 rounded-lg bg-muted/40 p-4">
				<div class="grid gap-4 sm:grid-cols-2">
					<div class="grid gap-1.5">
						<label class="text-sm font-medium" for="volume-backup-target"
							>{t('projects.db_backup.destination')}</label>
						<NativeSelect
							id="volume-backup-target"
							class="w-full"
							value={settings.target}
							onchange={onTargetChange}>
							{#if defaultAvailable}<option value=""
									>{t('projects.db_backup.destination_default')}</option
								>{/if}
							{#each targets as target (target.id)}
								<option value={target.id}>{target.name}</option>
							{/each}
							<option value="__add__">{t('projects.volume_backup.add_target')}</option>
						</NativeSelect>
					</div>
					<div class="flex items-center justify-between gap-4 sm:pt-6">
						<div class="grid gap-0.5">
							<label class="text-sm font-medium" for="volume-backup-stop"
								>{t('projects.volume_backup.stop_app')}</label>
							<p class="text-xs text-muted-foreground">
								{t('projects.volume_backup.stop_app_hint')}
							</p>
						</div>
						<Switch
							id="volume-backup-stop"
							checked={settings.stopApp}
							onCheckedChange={(next) => (settings.stopApp = next)} />
					</div>
				</div>

				<div class="flex items-center justify-between gap-4 border-t pt-4">
					<div class="grid gap-0.5">
						<label class="text-sm font-medium" for="volume-backup-enabled"
							>{t('projects.volume_backup.automatic')}</label>
						<p class="text-xs text-muted-foreground">
							{t('projects.volume_backup.automatic_hint')}
						</p>
					</div>
					<Switch
						id="volume-backup-enabled"
						checked={settings.enabled}
						onCheckedChange={(next) => (settings.enabled = next)} />
				</div>
				{#if settings.enabled}
					<div class="grid gap-4 sm:grid-cols-[1fr_8rem]">
						<div class="grid gap-1.5">
							<label class="text-sm font-medium" for="volume-backup-frequency"
								>{t('projects.volume_backup.frequency')}</label>
							<div class="flex flex-wrap gap-2">
								<NativeSelect
									id="volume-backup-frequency"
									class="w-full sm:w-64"
									value={frequency}
									onchange={(event) => {
										frequency = event.currentTarget.value;
										if (frequency !== CUSTOM) settings.cron = frequency;
									}}>
									{#each PRESETS as preset (preset.value)}
										<option value={preset.value}>{preset.label()}</option>
									{/each}
									<option value={CUSTOM}>{t('projects.volume_backup.custom')}</option>
								</NativeSelect>
								{#if frequency === CUSTOM}
									<Input
										class="w-full font-mono sm:w-40"
										bind:value={settings.cron}
										placeholder="0 3 * * *"
										aria-label={t('projects.db_backup.cron')} />
								{/if}
							</div>
						</div>
						<div class="grid gap-1.5">
							<label class="text-sm font-medium" for="volume-backup-retention"
								>{t('projects.volume_backup.keep')}</label>
							<Input
								id="volume-backup-retention"
								type="number"
								min={1}
								bind:value={settings.retention} />
						</div>
					</div>
					<div class="grid gap-1.5">
						<p class="text-sm font-medium">{t('projects.volume_backup.scheduled_volumes')}</p>
						<div class="flex flex-wrap gap-1.5">
							{#each volumeNames as name (name)}
								{@const on = scheduled.includes(name)}
								<Button
									size="sm"
									variant={on ? 'secondary' : 'outline'}
									aria-pressed={on}
									onclick={() =>
										(scheduled = on
											? scheduled.filter((candidate) => candidate !== name)
											: [...scheduled, name])}>
									{#if on}<CheckIcon />{/if}{labelFor(name)}
								</Button>
							{/each}
						</div>
						{#if !scheduled.length}
							<p class="text-xs text-destructive">{t('projects.volume_backup.pick_volumes')}</p>
						{/if}
					</div>
				{/if}
				{#if dirty}
					<div class="flex justify-end">
						<Button
							size="sm"
							disabled={saving || (settings.enabled && !scheduled.length)}
							onclick={() => void save()}>
							{#if saving}<LoaderIcon class="animate-spin" />{/if}{t('projects.volume_backup.save')}
						</Button>
					</div>
				{/if}
			</div>
		{/if}

		<p class="mt-5 text-sm font-medium">{t('projects.db_backup.history')}</p>
		{#if runs.length === 0}
			<p class="mt-2 rounded-lg border border-dashed p-6 text-sm text-muted-foreground">
				{t('projects.volume_backup.empty')}
			</p>
		{:else}
			<ul class="mt-2 divide-y rounded-lg border">
				{#each runs as run (run.key)}
					{@const succeeded = run.volumes.filter((volume) => volume.status === 'succeeded')}
					<li class="flex flex-wrap items-center justify-between gap-3 px-3 py-2.5">
						<div class="min-w-0">
							<p class="flex flex-wrap items-center gap-2 text-sm">
								<span class="font-medium">{new Date(run.startedAt).toLocaleString()}</span>
								{#if run.status === 'running'}
									<Badge variant="secondary">{t('projects.db_backup.status_running')}</Badge>
								{:else if run.status === 'failed'}
									<Badge variant="destructive">{t('projects.db_backup.status_failed')}</Badge>
								{/if}
							</p>
							<p class="mt-0.5 text-xs text-muted-foreground">
								{run.volumes.map((volume) => labelFor(volume.volume_name)).join(', ')}
								{#if run.size}· {formatSize(run.size)}{/if} · {origin(run.triggeredBy)}
							</p>
							{#each run.volumes.filter((volume) => volume.error_message) as volume (volume.id)}
								<p class="mt-1 text-xs break-words text-destructive">
									{labelFor(volume.volume_name)}: {volume.error_message}
								</p>
							{/each}
						</div>
						{#if run.status !== 'running'}
							<div class="flex items-center gap-1">
								{#if acting === run.key}
									<LoaderIcon class="size-4 animate-spin text-muted-foreground" />
								{/if}
								{#if run.status === 'succeeded'}
									<Button
										size="sm"
										variant="outline"
										disabled={acting !== null || running}
										onclick={() => askRestore(run)}>{t('projects.volume_backup.restore')}</Button>
								{/if}
								<DropdownMenu.Root>
									<DropdownMenu.Trigger>
										{#snippet child({ props })}
											<Button
												{...props}
												size="icon-sm"
												variant="ghost"
												disabled={acting !== null}
												aria-label={t('projects.volume_backup.more')}><EllipsisIcon /></Button>
										{/snippet}
									</DropdownMenu.Trigger>
									<DropdownMenu.Content align="end" class="w-64">
										{#each succeeded as volume (volume.id)}
											<DropdownMenu.Item
												disabled={running}
												onSelect={() => restoreAsNew(run, volume)}
												>{t('projects.volume_backup.restore_new', {
													name: labelFor(volume.volume_name),
												})}</DropdownMenu.Item>
										{/each}
										{#if succeeded.length}<DropdownMenu.Separator />{/if}
										<DropdownMenu.Item variant="destructive" onSelect={() => askDelete(run)}
											>{t('projects.common.delete')}</DropdownMenu.Item>
									</DropdownMenu.Content>
								</DropdownMenu.Root>
							</div>
						{/if}
					</li>
				{/each}
			</ul>
		{/if}
	{/if}
</section>

<AlertDialog.Root open={!!confirm} onOpenChange={(open) => !open && (confirm = null)}>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title>{confirm?.title}</AlertDialog.Title>
			<AlertDialog.Description>{confirm?.description}</AlertDialog.Description>
		</AlertDialog.Header>
		<AlertDialog.Footer>
			<Button variant="outline" onclick={() => (confirm = null)}
				>{t('projects.projects.cancel')}</Button>
			<Button
				variant={confirm?.destructive ? 'destructive' : 'default'}
				onclick={() => {
					const action = confirm!.run;
					confirm = null;
					void action();
				}}>{confirm?.action}</Button>
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>
