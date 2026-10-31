<script lang="ts">
	import { brandedTitle } from '#lib/ee/branding/branding.svelte.js';
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import SearchIcon from '@lucide/svelte/icons/search';
	import RocketIcon from '@lucide/svelte/icons/rocket';
	import LayersIcon from '@lucide/svelte/icons/layers';
	import GitBranchIcon from '@lucide/svelte/icons/git-branch';
	import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import SegmentedSwitch from '#lib/components/custom/segmented-switch.svelte';
	import SkeletonRows from '#lib/components/custom/skeleton-rows.svelte';
	import {
		cancelDeploymentQueueJob,
		getDeploymentRun,
		listDeploymentQueue,
		streamDeploymentQueueEvents,
		type DeploymentJobDto,
		type DeploymentQueueEvent,
		type DeploymentRunDetailDto,
		type DeploymentRunStepDto,
	} from '#lib/clients/project-client.js';
	import DeploymentAuthor from '#lib/components/projects/deployment-author.svelte';
	import DeploymentLogViewer, {
		LOG_BUTTON,
	} from '#lib/components/projects/deployment-log-viewer.svelte';
	import RunStatusPill from '#lib/components/deployments/run-status-pill.svelte';
	import { runSteps } from '#lib/utils/deployment-steps.js';
	import { relativeTime } from '#lib/utils/relative-time.js';
	import LiveTimer from '#lib/components/live-timer.svelte';
	import { formatDuration } from '#lib/utils/format-duration.js';
	import { toast } from '#lib/utils/toast.js';
	import { i18n, t } from '#lib/i18n/index.svelte.js';

	let organizationId = $derived(page.params.id!);

	let jobs = $state<DeploymentJobDto[]>([]);
	let loading = $state(true);
	let selected = $state<string | null>(null);
	let canceling = $state<string | null>(null);
	let selectedRun = $state<DeploymentRunDetailDto | null>(null);
	let streamState = $state<'live' | 'reconnecting' | 'error'>('live');
	let streamRetryCount = 0;
	let queueEvents: EventSource | null = null;
	let queueGeneration = 0;
	type Filter = 'all' | 'active' | 'failed' | 'succeeded';
	let query = $state('');
	let filter = $state<Filter>('all');

	async function loadQueue(requestedOrganizationId: string) {
		const generation = ++queueGeneration;
		try {
			const nextJobs = await listDeploymentQueue(requestedOrganizationId);
			if (generation !== queueGeneration) return;
			jobs = nextJobs;
		} catch (error) {
			if (generation !== queueGeneration) return;
			toast.error(error instanceof Error ? error.message : t('organizations.queue.load_failed'));
		} finally {
			if (generation === queueGeneration) loading = false;
		}
	}

	function refresh() {
		loading = true;
		void loadQueue(organizationId);
	}

	$effect(() => {
		const requestedOrganizationId = organizationId;
		jobs = [];
		loading = true;
		selected = null;
		canceling = null;
		selectedRun = null;
		void loadQueue(requestedOrganizationId);
	});

	async function cancel(job: DeploymentJobDto) {
		canceling = job.id;
		try {
			const updated = await cancelDeploymentQueueJob(job.id);
			jobs = jobs.map((item) => (item.id === updated.id ? updated : item));
			toast.success(t('organizations.queue.cancel_requested'));
		} catch (error) {
			toast.error(error instanceof Error ? error.message : t('organizations.queue.cancel_failed'));
		} finally {
			canceling = null;
		}
	}

	async function selectJob(job: DeploymentJobDto) {
		if (selected === job.id) {
			selected = null;
			selectedRun = null;
			return;
		}
		selected = job.id;
		selectedRun = null;
		try {
			const detail = await getDeploymentRun(
				job.organization_id,
				job.project_id,
				job.environment_id,
				job.application_id,
				job.run_id,
			);
			if (selected !== job.id) return;
			selectedRun = detail;
		} catch (error) {
			if (selected !== job.id) return;
			selectedRun = null;
			toast.error(
				error instanceof Error ? error.message : t('organizations.queue.output_load_failed'),
			);
		}
	}

	function appendLog(event: DeploymentQueueEvent) {
		if (
			!selectedRun ||
			event.sequence === null ||
			selectedRun.logs.some((log) => log.sequence === event.sequence)
		)
			return;
		selectedRun = {
			...selectedRun,
			logs: [
				...selectedRun.logs,
				{
					sequence: event.sequence,
					level: event.level ?? 'info',
					message: event.payload,
					created_at: Date.now(),
				},
			],
		};
	}

	function connectStream() {
		const events = streamDeploymentQueueEvents();
		queueEvents = events;
		const update = () => refresh();
		const refreshSelected = (message: Event) => {
			update();
			const event = JSON.parse((message as MessageEvent).data) as DeploymentQueueEvent;
			const job = jobs.find((item) => item.id === event.job_id);
			if (job && job.id === selected) void selectJob(job);
		};
		events.addEventListener('status', update);
		events.addEventListener('terminal', refreshSelected);
		events.addEventListener('steps', (event) => {
			const payload = JSON.parse((event as MessageEvent).data) as DeploymentQueueEvent;
			if (payload.job_id !== selected || !selectedRun) return;
			try {
				selectedRun = {
					...selectedRun,
					steps: JSON.parse(payload.payload) as DeploymentRunStepDto[],
				};
			} catch {}
		});
		events.addEventListener('log', (event) => {
			const payload = JSON.parse((event as MessageEvent).data) as DeploymentQueueEvent;
			if (payload.job_id === selected) appendLog(payload);
		});
		events.onopen = () => {
			streamRetryCount = 0;
			streamState = 'live';
			const job = jobs.find((item) => item.id === selected);
			if (job) void selectJob(job);
		};
		events.onerror = () => {
			if (events.readyState === EventSource.CLOSED) return;
			streamRetryCount += 1;
			if (streamRetryCount >= 3) {
				events.close();
				queueEvents = null;
				streamState = 'error';
			} else {
				streamState = 'reconnecting';
			}
		};
		return () => {
			events.removeEventListener('status', update);
			events.removeEventListener('terminal', refreshSelected);
			events.close();
		};
	}

	function retryStream() {
		queueEvents?.close();
		streamRetryCount = 0;
		connectStream();
		refresh();
	}

	onMount(() => connectStream());

	const activeCount = $derived(
		jobs.filter((job) => job.status === 'building' || job.status === 'deploying').length,
	);
	const selectedJob = $derived(jobs.find((job) => job.id === selected) ?? null);

	const ACTIVE_STATUSES = ['queued', 'building', 'deploying', 'cancel_requested'];
	const isActiveStatus = (status: string) => ACTIVE_STATUSES.includes(status);
	const jobStartedAt = (job: DeploymentJobDto) => job.started_at ?? job.queued_at;
	const filters = $derived<{ value: Filter; label: string }[]>([
		{ value: 'all', label: t('organizations.queue.filter_all') },
		{ value: 'active', label: t('organizations.queue.filter_active') },
		{ value: 'failed', label: t('organizations.queue.filter_failed') },
		{ value: 'succeeded', label: t('organizations.queue.filter_succeeded') },
	]);
	const visibleJobs = $derived.by(() => {
		const needle = query.trim().toLocaleLowerCase();
		return jobs.filter((job) => {
			if (filter === 'active' && !isActiveStatus(job.status)) return false;
			if (filter === 'failed' && job.status !== 'failed' && job.status !== 'canceled') return false;
			if (filter === 'succeeded' && job.status !== 'succeeded') return false;
			if (!needle) return true;
			return [
				job.application_name,
				job.project_name,
				job.branch,
				job.commit_sha,
				job.commit_message,
			].some((value) => value?.toLocaleLowerCase().includes(needle));
		});
	});
</script>

<svelte:head>
	<title>{brandedTitle(t('organizations.queue.title'))}</title>
</svelte:head>

<div class="flex flex-col gap-5 px-4 py-6 sm:px-6">
	<div class="flex flex-wrap items-start justify-between gap-3">
		<div class="min-w-0">
			<h1 class="flex flex-wrap items-center gap-2 text-xl font-semibold tracking-tight">
				{t('organizations.queue.title')}
				{#if activeCount > 0}<span
						class="rounded-full bg-warning/10 px-2 py-0.5 text-xs font-medium text-warning lp-raised"
						>{t('organizations.queue.active_count', { count: activeCount })}</span
					>{/if}
			</h1>
			<p class="text-sm text-muted-foreground">{t('organizations.queue.description')}</p>
		</div>
		<Button variant="outline" size="sm" onclick={refresh} disabled={loading}>
			<RefreshCwIcon class={loading ? 'size-4 animate-spin' : 'size-4'} />
			{t('organizations.common.refresh')}
		</Button>
	</div>

	<div class="flex flex-wrap items-center gap-2">
		<label
			class="lp-well relative flex min-w-0 flex-1 basis-full items-center rounded-full bg-muted/60 sm:max-w-sm sm:basis-auto">
			<SearchIcon class="pointer-events-none absolute left-3 size-4 text-muted-foreground" />
			<Input
				bind:value={query}
				placeholder={t('organizations.queue.search')}
				aria-label={t('organizations.queue.search')}
				class="h-9 rounded-full border-0 bg-transparent pl-9 shadow-none" />
		</label>
		<SegmentedSwitch
			items={filters}
			bind:value={filter}
			label={t('organizations.queue.filter_label')}
			class="max-w-full" />
	</div>

	{#if loading && jobs.length === 0}
		<section class="rounded-2xl border bg-card p-4 lp-raised">
			<SkeletonRows rows={5} height="h-12" label={t('organizations.queue.loading')} />
		</section>
	{:else if jobs.length === 0}
		<div
			class="flex flex-col items-center gap-2 rounded-2xl border border-dashed p-10 text-center text-sm text-muted-foreground">
			<RocketIcon class="size-5" aria-hidden="true" />
			<p class="font-medium text-foreground">{t('organizations.queue.empty')}</p>
			<p>{t('organizations.queue.empty_hint')}</p>
		</div>
	{:else}
		<section class="overflow-hidden rounded-2xl border bg-card lp-raised">
			<div
				class="dep-row grid gap-4 px-[18px] py-2.5 text-xs text-muted-foreground lp-groove-b max-[860px]:hidden">
				<span>{t('organizations.queue.col_app')}</span>
				<span>{t('organizations.queue.col_status')}</span>
				<span>{t('organizations.queue.col_source')}</span>
				<span class="text-right">{t('organizations.queue.col_when')}</span>
			</div>
			{#if visibleJobs.length === 0}
				<p class="p-6 text-center text-sm text-muted-foreground">
					{t('organizations.queue.no_matches')}
				</p>
			{/if}
			{#each visibleJobs as job (job.id)}
				<div class="not-first:lp-groove-t">
					<button
						class="dep-row grid w-full min-w-0 items-center gap-4 px-[18px] py-3.5 text-left transition-colors max-sm:px-3.5 max-sm:py-3 {selected ===
						job.id
							? 'bg-primary/10'
							: 'hover:bg-muted'}"
						aria-expanded={selected === job.id}
						onclick={() => void selectJob(job)}>
						<span class="min-w-0">
							<b class="block truncate font-mono text-[13px] font-medium">{job.application_name}</b>
							<small class="mt-0.5 block truncate text-xs text-muted-foreground"
								>{t('organizations.queue.project', { name: job.project_name })}</small>
						</span>
						<span class="flex min-w-0 flex-col items-start gap-0.5 max-sm:items-end">
							<RunStatusPill status={job.status} />
							<small class="font-mono text-xs text-muted-foreground"
								>{#if job.duration_ms}{formatDuration(
										job.duration_ms,
									)}{:else if isActiveStatus(job.status)}<LiveTimer
										startedAt={jobStartedAt(job)} />{:else}{job.build_method}{/if}</small>
						</span>
						<span class="col-src flex min-w-0 flex-col">
							<span class="flex min-w-0 items-center gap-1.5 truncate text-sm">
								{#if job.branch}<GitBranchIcon class="size-3.5 shrink-0" /><span class="truncate"
										>{job.branch}</span
									>{:else}<LayersIcon class="size-3.5 shrink-0" />{job.build_method}{/if}
								{#if job.commit_sha}<span class="shrink-0 font-mono text-xs text-muted-foreground"
										>{job.commit_sha.slice(0, 7)}</span
									>{/if}
							</span>
							{#if job.commit_message}<small
									class="truncate text-xs text-muted-foreground"
									title={job.commit_message.toLocaleLowerCase()}
									>{job.commit_message.toLocaleLowerCase()}</small
								>{/if}
						</span>
						<span
							class="col-who flex min-w-0 items-center justify-end gap-2 text-[12.5px] whitespace-nowrap text-muted-foreground">
							<time
								class="shrink-0"
								datetime={new Date(job.queued_at).toISOString()}
								title={new Date(job.queued_at).toLocaleString()}
								>{relativeTime(job.queued_at, Date.now(), i18n.locale)}</time>
							<DeploymentAuthor
								actor={job.actor}
								triggeredBy={job.triggered_by}
								commitAuthorName={job.commit_author_name}
								commitAuthorEmail={job.commit_author_email} />
						</span>
					</button>
					{#if selected === job.id && selectedRun}
						<div class="px-3 pb-3 sm:px-4 sm:pb-4">
							{#key selectedRun.id}
								{@const stepBar = runSteps(selectedRun, job.status)}
								<DeploymentLogViewer
									logs={selectedRun.logs}
									label={t('organizations.queue.output')}
									title={job.application_name}
									subtitle="{t(
										'organizations.queue.run',
									)}: {selectedRun.id}{job.builder_container_name
										? ` · ${t('organizations.queue.build_container')}: ${job.builder_container_name}`
										: ''}"
									steps={stepBar.steps}
									stepsPreview={stepBar.preview}
									showLineNumbers={false}>
									{#snippet actions()}
										{#if job.status === 'queued' || job.status === 'building' || job.status === 'deploying'}
											<Button
												size="sm"
												variant="destructive"
												class="h-7 px-2 text-xs"
												onclick={() => cancel(job)}
												disabled={canceling === job.id}
												>{canceling === job.id
													? t('organizations.queue.canceling')
													: t('organizations.common.cancel')}</Button>
										{/if}
									{/snippet}
									{#snippet empty()}
										{#if job.sanitized_error}
											<p class="whitespace-pre-wrap">{job.sanitized_error}</p>
										{:else if job.status === 'queued'}
											<p>{t('organizations.queue.waiting_slot')}</p>
										{:else if streamState === 'error'}
											<div class="flex flex-wrap items-center gap-3">
												<p>{t('organizations.queue.connection_lost')}</p>
												<Button size="sm" variant="ghost" class={LOG_BUTTON} onclick={retryStream}>
													<RefreshCwIcon class="size-3.5" />{t('organizations.queue.reconnect')}
												</Button>
											</div>
										{:else}
											<p>
												{streamState === 'reconnecting'
													? t('organizations.queue.reconnecting')
													: t('organizations.queue.waiting_output')}
											</p>
										{/if}
									{/snippet}
								</DeploymentLogViewer>
							{/key}
						</div>
					{/if}
				</div>
			{/each}
		</section>
	{/if}
</div>

<style>
	.dep-row {
		grid-template-columns: minmax(0, 1.3fr) minmax(0, 0.9fr) minmax(0, 1.3fr) minmax(0, 1.3fr);
	}
	@media (max-width: 860px) {
		.dep-row {
			grid-template-columns: minmax(0, 1fr) auto;
		}
		.col-src,
		.col-who {
			display: none;
		}
	}
</style>
