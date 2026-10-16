<script lang="ts">
	import { allowedHere } from '#lib/state/permissions.svelte.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
	import DeleteConfirmControl from '#lib/components/custom/delete-confirm-control.svelte';
	import DeploymentAuthor from '#lib/components/projects/deployment-author.svelte';
	import DeploymentLogViewer, {
		LOG_BUTTON,
	} from '#lib/components/projects/deployment-log-viewer.svelte';
	import RunStatusPill from '#lib/components/deployments/run-status-pill.svelte';
	import { runSteps } from '#lib/utils/deployment-steps.js';
	import { relativeTime } from '#lib/utils/relative-time.js';
	import DeploymentReplicaLogs from '#lib/components/projects/deployment-replica-logs.svelte';
	import OsIcon from '#lib/components/os-icon.svelte';
	import DeleteResourceDialog from '#lib/components/projects/delete-resource-dialog.svelte';
	import LiveTimer from '#lib/components/live-timer.svelte';
	import { formatDuration } from '#lib/utils/format-duration.js';
	import { toneBorder } from '#lib/utils/status-tone.js';
	import GitBranchIcon from '@lucide/svelte/icons/git-branch';
	import OctagonXIcon from '@lucide/svelte/icons/octagon-x';
	import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';
	import Trash2Icon from '@lucide/svelte/icons/trash-2';
	import RocketIcon from '@lucide/svelte/icons/rocket';
	import Undo2Icon from '@lucide/svelte/icons/undo-2';
	import {
		cancelDeploymentQueueJob,
		type DeploymentRunDetailDto,
		type DeploymentRunDto,
	} from '#lib/clients/project-client.js';
	import { toast } from '#lib/utils/toast.js';
	import { i18n, t } from '#lib/i18n/index.svelte.js';
	import { deploymentActionLabel } from '#lib/utils/deployment-run-presentation.js';

	let {
		runs = [],
		selected = null,
		liveStatus = null,
		streamState = 'idle',
		applicationName = '',
		onSelect,
		onDeleteRun,
		onDeleteAll,
		onRefresh,
		onRedeploy,
		onRollback,
		refreshing = false,
		livePaused = false,
		onToggleLive,
		onRetryStream,
		onAskAssistant,
		onRetryReplica,
	}: {
		runs: DeploymentRunDto[];
		selected: DeploymentRunDetailDto | null;
		liveStatus?: string | null;
		streamState?: 'idle' | 'live' | 'reconnecting' | 'error';
		applicationName?: string;
		onSelect: (id: string) => void;
		onDeleteRun: (runId: string) => Promise<void>;
		onDeleteAll: () => Promise<void>;
		onRefresh: () => void | Promise<void>;
		onRedeploy: () => void;
		onRollback: (runId: string) => Promise<void>;
		refreshing?: boolean;
		livePaused?: boolean;
		onToggleLive: () => void | Promise<void>;
		onRetryStream?: () => void | Promise<void>;
		/** Given the rendered log text. Omitted when the assistant is off. */
		onAskAssistant?: (text: string) => void;
		/** Re-deploys one server of a group-targeted application. */
		onRetryReplica?: (serverId: string) => Promise<void>;
	} = $props();

	const replicaChipClass = (status: string) =>
		toneBorder(
			status === 'failed'
				? 'danger'
				: status === 'succeeded'
					? 'ok'
					: status === 'deploying'
						? 'warn'
						: 'neutral',
		);

	const date = (value: number) => new Date(value).toLocaleString();
	/** Row headline: the commit message when there is one, else what the run did. */
	const headline = (run: DeploymentRunDto) =>
		run.commit_message?.toLocaleLowerCase() ||
		`${deploymentActionLabel(run.triggered_by)} · ${run.build_method}`;
	/** The newest run that went live is what is running now. */
	let currentRunId = $derived(runs.find((run) => run.status === 'succeeded')?.id ?? null);

	const ACTIVE_STATUSES = ['queued', 'building', 'deploying', 'cancel_requested'];
	const isActiveStatus = (status: string) => ACTIVE_STATUSES.includes(status);
	const rowStatus = (run: DeploymentRunDto) =>
		selected?.id === run.id ? displayedStatus : run.status;
	const shortSha = (sha: string) => sha.slice(0, 7);
	let isRailpack = $derived(selected?.build_method === 'railpack');
	let displayedStatus = $derived(liveStatus ?? selected?.status ?? '');
	let isDeploying = $derived(isActiveStatus(displayedStatus));
	let stepBar = $derived(selected ? runSteps(selected, displayedStatus) : null);
	let selectedRun = $derived(runs.find((run) => run.id === selected?.id) ?? null);
	let canceling = $state(false);
	let deleteAllOpen = $state(false);
	let deleteRunDialogOpen = $state(false);
	let deleteRunTarget = $state<DeploymentRunDto | null>(null);
	let rollbackDialogOpen = $state(false);
	let rollbackTarget = $state<DeploymentRunDto | null>(null);
	let rollingBack = $state(false);

	async function handleRollback() {
		if (!rollbackTarget) return;
		rollingBack = true;
		try {
			await onRollback(rollbackTarget.id);
		} catch (error) {
			toast.error(
				error instanceof Error
					? error.message
					: t('projects.application_deployment.rollback_failed_toast'),
			);
		} finally {
			rollingBack = false;
			rollbackDialogOpen = false;
			rollbackTarget = null;
		}
	}

	async function stopRun() {
		if (!selected?.job_id) return;
		canceling = true;
		try {
			await cancelDeploymentQueueJob(selected.job_id);
			toast.success(t('projects.application_deployment.cancel_requested_toast'));
		} catch (error) {
			toast.error(
				error instanceof Error
					? error.message
					: t('projects.application_deployment.cancel_failed_toast'),
			);
		} finally {
			canceling = false;
		}
	}

	async function handleDeleteRun() {
		if (!deleteRunTarget) return;
		await onDeleteRun(deleteRunTarget.id);
		toast.success(t('projects.application_deployment.run_deleted_toast'));
	}

	async function handleDeleteAll() {
		await onDeleteAll();
		toast.success(t('projects.application_deployment.history_cleared_toast'));
	}
</script>

{#snippet runActions(dark: boolean)}
	{#if selected && selectedRun}
		{@const ghost = dark ? LOG_BUTTON : 'gap-1.5'}
		{@const danger = dark
			? `${LOG_BUTTON} text-[#ff6d64] hover:text-[#ff6d64]`
			: 'text-destructive hover:text-destructive'}
		{#if isDeploying && selected.job_id && allowedHere('app.deploy')}
			<Button
				size="sm"
				variant="destructive"
				class="gap-1.5 {dark ? 'h-7 px-2 text-xs' : ''}"
				onclick={stopRun}
				disabled={canceling}
				><OctagonXIcon class="size-4" />{canceling
					? t('projects.application_deployment.killing_build')
					: t('projects.application_deployment.kill_build')}</Button>
		{:else}
			{#if selectedRun.id === runs[0]?.id}
				{#if allowedHere('app.deploy')}
					<Button size="sm" variant="ghost" class={ghost} onclick={onRedeploy}
						><RocketIcon class="size-4" />{t('projects.environment.redeploy')}</Button>
				{/if}
			{:else}
				<Button
					size="sm"
					variant="ghost"
					class={ghost}
					onclick={() => {
						rollbackTarget = selectedRun;
						rollbackDialogOpen = true;
					}}
					><Undo2Icon class="size-4" />{t('projects.application_deployment.roll_back_to')}</Button>
			{/if}
			<Button
				size="sm"
				variant="ghost"
				class={danger}
				onclick={() => {
					deleteRunTarget = selectedRun;
					deleteRunDialogOpen = true;
				}}
				aria-label={t('projects.application_deployment.delete_run_aria')}
				><Trash2Icon class="size-4" /></Button>
		{/if}
	{/if}
{/snippet}

{#snippet waiting()}
	{#if selected?.last_error}
		<p class="whitespace-pre-wrap">{selected.last_error}</p>
	{:else if streamState === 'error'}
		<div class="flex flex-wrap items-center gap-3">
			<p>{t('projects.application_deployment.lost_connection')}</p>
			<Button size="sm" variant="ghost" class={LOG_BUTTON} onclick={() => void onRetryStream?.()}>
				<RefreshCwIcon class="size-3.5" />{t('projects.application_deployment.reconnect')}
			</Button>
		</div>
	{:else}
		<p>
			{streamState === 'reconnecting'
				? t('projects.application_deployment.reconnecting_output')
				: t('projects.application_deployment.waiting_output')}
		</p>
	{/if}
{/snippet}

<div class="grid max-w-full gap-4">
	<div class="flex flex-wrap items-center justify-between gap-3">
		<div class="min-w-0">
			<p class="font-medium">{t('projects.application_deployment.title')}</p>
			<p class="text-sm text-muted-foreground">
				{t('projects.application_deployment.description')}
			</p>
		</div>
		{#if runs.length > 0}
			<div class="flex flex-wrap gap-2">
				<Button size="sm" variant="outline" disabled={refreshing} onclick={() => void onRefresh()}>
					<RefreshCwIcon class={refreshing ? 'size-4 animate-spin' : 'size-4'} />
					{refreshing
						? t('projects.application_deployment.refreshing')
						: t('projects.application_general.refresh')}
				</Button>
				<Button
					size="sm"
					variant="ghost"
					class="text-destructive hover:text-destructive"
					onclick={() => (deleteAllOpen = true)}
					>{t('projects.application_deployment.clear_history')}</Button>
			</div>
		{/if}
	</div>

	{#if runs.length === 0}
		<div
			class="flex flex-col items-center gap-2 rounded-xl border border-dashed p-8 text-center text-sm text-muted-foreground">
			<RocketIcon class="size-5" aria-hidden="true" />
			<p class="font-medium text-foreground">{t('projects.application_deployment.empty')}</p>
			<p>{t('projects.application_deployment.empty_hint')}</p>
		</div>
	{:else}
		<div class="grid min-w-0 items-start gap-4 lg:grid-cols-[minmax(0,1fr)_minmax(0,1.05fr)]">
			<div class="min-w-0 overflow-hidden rounded-xl border bg-card lp-raised">
				<ol class="p-1.5">
					{#each runs as run (run.id)}
						{@const status = rowStatus(run)}
						<li class="not-first:lp-groove-t">
							<button
								class="grid w-full min-w-0 grid-cols-[minmax(0,1fr)_auto] items-center gap-x-4 gap-y-1.5 rounded-lg px-3 py-2.5 text-left transition-colors {selected?.id ===
								run.id
									? 'bg-primary/10'
									: 'hover:bg-muted'}"
								aria-current={selected?.id === run.id}
								onclick={() => onSelect(run.id)}>
								<span class="flex min-w-0 flex-col gap-1.5">
									<span class="flex min-w-0 items-center gap-2">
										<span class="truncate text-sm font-medium" title={headline(run)}
											>{headline(run)}</span>
										{#if run.id === currentRunId}
											<span
												class="shrink-0 rounded-full bg-primary/12 px-1.5 py-px text-[10.5px] font-medium text-primary"
												>{t('projects.application_deployment.current')}</span>
										{/if}
									</span>
									<small
										class="flex min-w-0 items-center gap-1.5 overflow-hidden text-xs whitespace-nowrap text-muted-foreground">
										<DeploymentAuthor
											actor={run.actor}
											triggeredBy={run.triggered_by}
											commitAuthorName={run.commit_author_name}
											commitAuthorEmail={run.commit_author_email} />
										<span class="tag shrink-0 font-mono"
											>{run.commit_sha ? shortSha(run.commit_sha) : run.source_type}</span>
										{#if run.branch}
											<span class="tag min-w-0 truncate"
												><GitBranchIcon class="size-3 shrink-0" />{run.branch}</span>
										{/if}
									</small>
								</span>
								<span class="flex flex-col items-end gap-1.5">
									<RunStatusPill {status} />
									<span class="text-xs whitespace-nowrap text-muted-foreground">
										<b class="font-mono font-medium text-foreground/80"
											>{#if run.duration_ms}{formatDuration(
													run.duration_ms,
												)}{:else if isActiveStatus(status)}<LiveTimer
													startedAt={run.started_at} />{:else}-{/if}</b>
										·
										<time
											datetime={new Date(run.started_at).toISOString()}
											title={date(run.started_at)}
											>{relativeTime(run.started_at, Date.now(), i18n.locale)}</time>
									</span>
								</span>
								{#if run.replicas.length > 0}
									{@const shown = run.replicas.slice(0, 6)}
									<span class="col-span-full flex flex-wrap items-center gap-1.5">
										{#each shown as replica (replica.server_id)}
											<span
												class="inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-[11px] {replicaChipClass(
													replica.status,
												)}"
												title={replica.last_error ?? replica.status}>
												<OsIcon
													os={replica.os}
													distribution={replica.distribution}
													class="size-3" />
												{replica.server_name}
											</span>
										{/each}
										{#if run.replicas.length > shown.length}
											<span class="text-[11px] text-muted-foreground"
												>+{run.replicas.length - shown.length}</span>
										{/if}
									</span>
								{/if}
							</button>
						</li>
					{/each}
				</ol>
			</div>

			<div class="min-w-0">
				{#if !selected}
					<div
						class="rounded-xl border border-dashed p-8 text-center text-sm text-muted-foreground">
						{t('projects.application_deployment.select_run')}
					</div>
				{:else}
					{#key selected.id}
						{#if selected.replicas.length > 0}
							<div class="mb-3 flex flex-wrap items-center justify-between gap-3">
								<div class="min-w-0">
									<p class="truncate text-sm">{selectedRun ? headline(selectedRun) : ''}</p>
									<p class="truncate font-mono text-xs text-muted-foreground">{selected.id}</p>
								</div>
								<div class="flex flex-wrap items-center gap-2">{@render runActions(false)}</div>
							</div>
							<DeploymentReplicaLogs
								replicas={selected.replicas}
								logs={selected.logs}
								onRetry={isDeploying ? undefined : onRetryReplica}
								{onRefresh}
								{livePaused}
								{onToggleLive}
								{onAskAssistant} />
						{:else}
							<DeploymentLogViewer
								logs={selected.logs}
								label={t('projects.application_deployment.deployment_output_label')}
								title={selectedRun ? headline(selectedRun) : undefined}
								subtitle="{selected.id} · {selected.build_method}{selected.branch
									? ` · ${selected.branch}`
									: ''}"
								steps={stepBar?.steps}
								stepsPreview={stepBar?.preview}
								showLineNumbers={false}
								{onRefresh}
								{livePaused}
								{onToggleLive}
								{onAskAssistant}>
								{#snippet actions()}{@render runActions(true)}{/snippet}
								{#snippet empty()}{@render waiting()}{/snippet}
							</DeploymentLogViewer>
						{/if}
						{#if selected.truncated}
							<p class="mt-2 text-xs text-muted-foreground">
								{t('projects.application_deployment.truncated_note')}
							</p>
						{/if}
					{/key}
				{/if}
			</div>
		</div>
	{/if}
</div>

<DeleteResourceDialog
	bind:open={deleteAllOpen}
	resourceName={t('projects.application_deployment.resource_type_history')}
	resourceType={t('projects.application_deployment.resource_type_history')}
	cascadeDescription={t('projects.application_deployment.delete_all_cascade', {
		count: runs.length,
	})}
	ondelete={handleDeleteAll} />
<DeleteResourceDialog
	bind:open={deleteRunDialogOpen}
	resourceName={deleteRunTarget?.id ?? ''}
	resourceType={t('projects.application_deployment.resource_type_run')}
	ondelete={handleDeleteRun} />

<AlertDialog.Root bind:open={rollbackDialogOpen}>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title>{t('projects.application_deployment.rollback_title')}</AlertDialog.Title>
			<AlertDialog.Description>
				{t('projects.application_deployment.rollback_desc_1')}
				<span class="font-mono">{rollbackTarget?.id.slice(0, 8)}</span
				>{#if rollbackTarget?.commit_sha}
					({t('projects.application_deployment.commit')}
					<span class="font-mono">{shortSha(rollbackTarget.commit_sha)}</span>){/if}{t(
					'projects.application_deployment.rollback_desc_2',
					{ name: applicationName },
				)}
			</AlertDialog.Description>
		</AlertDialog.Header>
		<AlertDialog.Footer>
			<AlertDialog.Cancel disabled={rollingBack}
				>{t('projects.projects.cancel')}</AlertDialog.Cancel>
			<DeleteConfirmControl
				label={t('projects.application_deployment.rollback_confirm')}
				pendingLabel={t('projects.application_deployment.rollback_pending')}
				variant="primary"
				icon={Undo2Icon}
				disabled={rollingBack}
				onconfirm={handleRollback} />
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>

<style>
	.tag {
		display: inline-flex;
		align-items: center;
		gap: 4px;
		height: 18px;
		padding: 0 6px;
		border-radius: 6px;
		background: color-mix(in srgb, var(--foreground) 6%, transparent);
		font-size: 11px;
	}
</style>
