<script lang="ts">
	import * as Button from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import * as Input from '#lib/components/ui/input/index.js';
	import * as Table from '#lib/components/ui/table/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Switch } from '#lib/components/ui/switch/index.js';
	import BackupIcon from '@lucide/svelte/icons/database-backup';
	import LoaderIcon from '@lucide/svelte/icons/loader-circle';
	import RefreshIcon from '@lucide/svelte/icons/refresh-cw';
	import TrashIcon from '@lucide/svelte/icons/trash-2';
	import {
		deleteDatabaseBackup,
		getDatabaseBackupPolicy,
		listDatabaseBackups,
		restoreDatabaseBackup,
		runDatabaseBackup,
		updateDatabaseBackupPolicy,
		type DatabaseBackupDto,
		type DatabaseBackupPolicyDto,
	} from '#lib/clients/project-client.js';
	import { listS3TargetOptions, type S3TargetOption } from '#lib/clients/backup-client.js';
	import { getSystemVersion } from '#lib/clients/system-client.js';
	import { t } from '#lib/i18n/index.svelte.js';
	import { toast, toastError } from '#lib/utils/toast.js';

	let {
		organizationId,
		projectId,
		environmentId,
		applicationId,
		engine,
	}: {
		organizationId: string;
		projectId: string;
		environmentId: string;
		applicationId: string;
		engine: string;
	} = $props();

	const restoreSupported = $derived(['postgres', 'mariadb', 'mongodb'].includes(engine));

	let policy = $state<DatabaseBackupPolicyDto | null>(null);
	let history = $state<DatabaseBackupDto[]>([]);
	let targets = $state<S3TargetOption[]>([]);
	/** Cloud has no shared default storage: a database must name its own target. */
	let isCloud = $state(false);
	let loading = $state(true);
	let saving = $state(false);
	let running = $state(false);
	let cronInput = $state('');
	let retentionInput = $state('');
	let enabledInput = $state(false);
	let targetInput = $state<string>('');

	async function load() {
		loading = true;
		try {
			const [nextPolicy, runs] = await Promise.all([
				getDatabaseBackupPolicy(organizationId, projectId, environmentId, applicationId),
				listDatabaseBackups(organizationId, projectId, environmentId, applicationId),
			]);
			policy = nextPolicy;
			cronInput = nextPolicy.cron_expression;
			retentionInput = String(nextPolicy.retention_count);
			enabledInput = nextPolicy.enabled;
			targetInput = nextPolicy.s3_target_id ?? '';
			history = runs;
			getSystemVersion()
				.then((version) => (isCloud = version.is_cloud))
				.catch(() => {});
			try {
				targets = await listS3TargetOptions(organizationId);
			} catch {
				// Target list is a nicety for the selector; failure must not
				// block the rest of the backup tab.
				targets = [];
			}
		} catch (err) {
			toastError(err, t('projects.db_backup.load_failed'));
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		void load();
	});

	async function savePolicy() {
		if (!policy) return;
		saving = true;
		try {
			policy = await updateDatabaseBackupPolicy(
				organizationId,
				projectId,
				environmentId,
				applicationId,
				{
					enabled: enabledInput,
					cron_expression: cronInput,
					retention_count: Number(retentionInput),
					s3_target_id: targetInput || null,
				},
			);
			toast.success(t('projects.db_backup.saved'));
		} catch (err) {
			toastError(err, t('projects.db_backup.save_failed'));
		} finally {
			saving = false;
		}
	}

	async function runNow() {
		running = true;
		try {
			const record = await runDatabaseBackup(
				organizationId,
				projectId,
				environmentId,
				applicationId,
			);
			history = [record, ...history];
			toast.success(t('projects.db_backup.run_started'));
			// Poll until the background run settles.
			const deadline = Date.now() + 30 * 60_000;
			const poll = setInterval(async () => {
				const runs = await listDatabaseBackups(
					organizationId,
					projectId,
					environmentId,
					applicationId,
				);
				history = runs;
				const current = runs.find((r) => r.id === record.id);
				if ((current && current.status !== 'running') || Date.now() > deadline) {
					clearInterval(poll);
					running = false;
				}
			}, 4000);
		} catch (err) {
			running = false;
			toastError(err, t('projects.db_backup.run_failed'));
		}
	}

	async function restoreRun(backupId: string) {
		if (!window.confirm(t('projects.db_backup.restore_confirm'))) return;
		try {
			await restoreDatabaseBackup(
				organizationId,
				projectId,
				environmentId,
				applicationId,
				backupId,
			);
			toast.success(t('projects.db_backup.restore_done'));
		} catch (err) {
			toastError(err, t('projects.db_backup.restore_failed'));
		}
	}

	async function removeRun(backupId: string) {
		if (!window.confirm(t('projects.db_backup.delete_confirm'))) return;
		try {
			await deleteDatabaseBackup(organizationId, projectId, environmentId, applicationId, backupId);
			history = history.filter((r) => r.id !== backupId);
			toast.success(t('projects.db_backup.deleted'));
		} catch (err) {
			toastError(err, t('projects.db_backup.delete_failed'));
		}
	}

	function stateLabel(status: string): string {
		if (status === 'running') return 'projects.db_backup.status_running';
		if (status === 'succeeded') return 'projects.db_backup.status_succeeded';
		return 'projects.db_backup.status_failed';
	}

	function formatTime(millis: number | null): string {
		if (millis == null) return '—';
		return new Date(millis).toLocaleString();
	}

	function formatSize(bytes: number | null): string {
		if (bytes == null) return '—';
		if (bytes < 1024) return `${bytes} B`;
		if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
		if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
		return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
	}
</script>

<div class="grid gap-6">
	<Card.Root class="max-w-3xl overflow-hidden">
		<Card.Header>
			<div class="flex items-center gap-2">
				<BackupIcon class="size-4 text-muted-foreground" />
				<Card.Title>{t('projects.db_backup.title')}</Card.Title>
			</div>
			<Card.Description>{t('projects.db_backup.description')}</Card.Description>
		</Card.Header>
		<Card.Content class="grid gap-4 pt-2">
			{#if !loading && policy}
				<div class="flex items-center justify-between gap-4">
					<div class="grid gap-1">
						<label class="text-sm font-medium leading-none" for="backup-enabled">
							{t('projects.db_backup.enabled')}
						</label>
						<p class="text-sm text-muted-foreground">{t('projects.db_backup.enabled_hint')}</p>
					</div>
					<Switch
						id="backup-enabled"
						checked={enabledInput}
						disabled={saving}
						onCheckedChange={(next) => (enabledInput = next)} />
				</div>

				<div class="grid grid-cols-1 gap-2 sm:grid-cols-2">
					<div class="grid gap-1">
						<label class="text-sm font-medium leading-none" for="backup-cron"
							>{t('projects.db_backup.cron')}</label>
						<Input.Root
							id="backup-cron"
							value={cronInput}
							disabled={saving}
							oninput={(e) => (cronInput = e.currentTarget.value)}
							placeholder="0 3 * * *" />
						<p class="text-xs text-muted-foreground">{t('projects.db_backup.cron_hint')}</p>
					</div>
					<div class="grid gap-1">
						<label class="text-sm font-medium leading-none" for="backup-retention"
							>{t('projects.db_backup.retention')}</label>
						<Input.Root
							id="backup-retention"
							type="number"
							min={1}
							value={retentionInput}
							disabled={saving}
							oninput={(e) => (retentionInput = e.currentTarget.value)} />
						<p class="text-xs text-muted-foreground">{t('projects.db_backup.retention_hint')}</p>
					</div>
					<div class="grid gap-1">
						<label class="text-sm font-medium leading-none" for="backup-target"
							>{t('projects.db_backup.destination')}</label>
						<select
							id="backup-target"
							class="flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm disabled:cursor-not-allowed disabled:opacity-50"
							bind:value={targetInput}
							disabled={saving}>
							<option value=""
								>{isCloud
									? t('projects.db_backup.destination_none')
									: t('projects.db_backup.destination_default')}</option>
							{#each targets as target}
								<option value={target.id}>{target.name}</option>
							{/each}
						</select>
						<p class="text-xs text-muted-foreground">
							{isCloud
								? t('projects.db_backup.destination_hint_cloud')
								: t('projects.db_backup.destination_hint')}
						</p>
					</div>
				</div>

				<div class="flex flex-wrap items-center gap-2">
					<Button.Root
						variant="default"
						size="sm"
						disabled={saving}
						onclick={() => void savePolicy()}>
						{t('projects.db_backup.save')}
					</Button.Root>
					<Button.Root
						variant="outline"
						size="sm"
						disabled={running || saving}
						onclick={() => void runNow()}>
						{#if running}
							<LoaderIcon class="size-4 animate-spin" />
							{t('projects.db_backup.running')}
						{:else}
							<RefreshIcon class="size-4" />
							{t('projects.db_backup.run_now')}
						{/if}
					</Button.Root>
				</div>
			{/if}

			{@const notSupported = engine === 'sqlserver'}
			{#if notSupported}
				<p class="text-sm text-muted-foreground">{t('projects.db_backup.not_supported')}</p>
			{/if}
		</Card.Content>
	</Card.Root>

	<Card.Root class="max-w-3xl overflow-hidden">
		<Card.Header>
			<Card.Title>{t('projects.db_backup.history')}</Card.Title>
		</Card.Header>
		<Card.Content class="pt-2">
			{#if loading}
				<div class="flex items-center gap-2 text-sm text-muted-foreground">
					<LoaderIcon class="size-4 animate-spin" />
					{t('projects.db_backup.loading')}
				</div>
			{:else if history.length === 0}
				<p class="text-sm text-muted-foreground">{t('projects.db_backup.history_empty')}</p>
			{:else}
				<Table.Root>
					<Table.Header>
						<Table.Row>
							<Table.Head>{t('projects.db_backup.started')}</Table.Head>
							<Table.Head>{t('projects.db_backup.finished')}</Table.Head>
							<Table.Head>{t('projects.db_backup.status')}</Table.Head>
							<Table.Head>{t('projects.db_backup.size')}</Table.Head>
							<Table.Head>{t('projects.db_backup.destination')}</Table.Head>
							<Table.Head class="text-right"></Table.Head>
						</Table.Row>
					</Table.Header>
					<Table.Body>
						{#each history as run (run.id)}
							<Table.Row>
								<Table.Cell>{formatTime(run.started_at)}</Table.Cell>
								<Table.Cell>{formatTime(run.finished_at)}</Table.Cell>
								<Table.Cell>
									<Badge
										variant={run.status === 'succeeded'
											? 'default'
											: run.status === 'running'
												? 'secondary'
												: 'destructive'}>
										{t(stateLabel(run.status))}
									</Badge>
									{#if run.error_message}
										<p
											class="mt-1 max-w-xs truncate text-xs text-muted-foreground"
											title={run.error_message}>
											{run.error_message}
										</p>
									{/if}
								</Table.Cell>
								<Table.Cell>{formatSize(run.size_bytes)}</Table.Cell>
								<Table.Cell class="text-sm text-muted-foreground">
									{#if run.s3_target_id}
										{targets.find((target) => target.id === run.s3_target_id)?.name ??
											run.s3_target_id.slice(0, 8)}
									{:else}
										{t('projects.db_backup.destination_default')}
									{/if}
								</Table.Cell>
								<Table.Cell class="text-right">
									<div class="flex justify-end gap-1">
										{#if run.status === 'succeeded'}
											<Button.Root
												variant="outline"
												size="sm"
												disabled={!restoreSupported || running}
												title={restoreSupported ? '' : t('projects.db_backup.restore_unsupported')}
												onclick={() => void restoreRun(run.id)}>
												{t('projects.db_backup.restore')}
											</Button.Root>
											<Button.Root
												variant="destructive"
												size="sm"
												disabled={running}
												onclick={() => void removeRun(run.id)}>
												<TrashIcon class="size-4" />
											</Button.Root>
										{/if}
									</div>
								</Table.Cell>
							</Table.Row>
						{/each}
					</Table.Body>
				</Table.Root>
			{/if}
		</Card.Content>
	</Card.Root>
</div>
