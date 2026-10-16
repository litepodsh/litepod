<script lang="ts">
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import ActivityIcon from '@lucide/svelte/icons/activity';
	import CalendarClockIcon from '@lucide/svelte/icons/calendar-clock';
	import CircleXIcon from '@lucide/svelte/icons/circle-x';
	import LayersIcon from '@lucide/svelte/icons/layers';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import SearchIcon from '@lucide/svelte/icons/search';
	import SkeletonRows from '#lib/components/custom/skeleton-rows.svelte';
	import CellChart from '#lib/components/custom/cell-chart.svelte';
	import SegmentedSwitch from '#lib/components/custom/segmented-switch.svelte';
	import DeleteConfirmControl from '#lib/components/custom/delete-confirm-control.svelte';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
	import { getRuntimeStatus } from '#lib/clients/runtime-client.js';
	import { listServers, type Server } from '#lib/clients/server-client.js';
	import { getProject, listProjects } from '#lib/clients/project-client.js';
	import {
		deleteScheduledTask,
		getScheduledTask,
		listScheduledTaskExecutions,
		listScheduledTasks,
		updateScheduledTask,
		type ScheduledTask,
		type ScheduledTaskDetail,
		type ScheduledTaskExecution,
	} from '#lib/clients/scheduled-tasks-client.js';
	import TaskDialog from '#lib/components/scheduled-tasks/task-dialog.svelte';
	import TaskSheet from '#lib/components/scheduled-tasks/execution-detail-dialog.svelte';
	import TaskTable from '#lib/components/scheduled-tasks/task-table.svelte';
	import {
		detailToInput,
		runOutcome,
		taskHealth,
		type TaskHealth,
	} from '#lib/components/scheduled-tasks/task-utils.js';
	import { nextRuns } from '#lib/utils/cron.js';
	import { relativeTime } from '#lib/utils/relative-time.js';
	import { toast } from '#lib/utils/toast.js';
	import { i18n, t } from '#lib/i18n/index.svelte.js';

	const organizationId = page.params.id!;

	let tasks = $state<ScheduledTask[]>([]);
	let executions = $state<Record<string, ScheduledTaskExecution[]>>({});
	let servers = $state<Server[]>([]);
	let appNames = $state<Record<string, string>>({});
	let loading = $state(true);
	let clientNow = $state(new Date());
	let serverOffsetSeconds = $state(0);
	// difference between the server clock and this browser at the moment the page loaded
	let serverSkewMs = $state<number | null>(null);

	type Filter = 'all' | 'healthy' | 'failing' | 'paused';
	let filter = $state<Filter>('all');
	let query = $state('');

	let dialogOpen = $state(false);
	let editing = $state<ScheduledTaskDetail | null>(null);
	let cloneFrom = $state<ScheduledTaskDetail | null>(null);
	let sheetOpen = $state(false);
	let sheetTaskId = $state<string | null>(null);
	let runOnOpen = $state(false);
	let deleteTask = $state<ScheduledTask | null>(null);

	// the server timestamp arrives as RFC3339: render it with the same locale format as the local
	// clock, but keep it in the server's own UTC offset instead of converting to the browser's zone
	const serverNow = $derived.by(() => {
		if (serverSkewMs === null) return null;
		const shifted = clientNow.getTime() + serverSkewMs + serverOffsetSeconds * 1000;
		return new Date(shifted).toLocaleString(undefined, { timeZone: 'UTC' });
	});

	const health = $derived(
		Object.fromEntries(
			tasks.map((task) => [task.id, taskHealth(task, executions[task.id] ?? [])]),
		) as Record<string, TaskHealth>,
	);
	const matches = (task: ScheduledTask, kind: Filter) =>
		kind === 'all' ||
		(kind === 'paused' && !task.enabled) ||
		(kind === 'failing' && health[task.id] === 'failing') ||
		(kind === 'healthy' && task.enabled && health[task.id] !== 'failing');
	const counts = $derived({
		all: tasks.length,
		healthy: tasks.filter((task) => matches(task, 'healthy')).length,
		failing: tasks.filter((task) => matches(task, 'failing')).length,
		paused: tasks.filter((task) => matches(task, 'paused')).length,
	});
	const visible = $derived.by(() => {
		const needle = query.trim().toLowerCase();
		return tasks.filter(
			(task) =>
				matches(task, filter) &&
				(!needle ||
					[task.name, task.description, task.cron_expression, task.runner].some((value) =>
						value.toLowerCase().includes(needle),
					)),
		);
	});

	// KPI numbers come from the runs each task's history returns
	const allRuns = $derived(Object.values(executions).flat());
	const finishedRuns = $derived(allRuns.filter((run) => runOutcome(run) !== 'live'));
	const okRuns = $derived(finishedRuns.filter((run) => runOutcome(run) === 'ok').length);
	const failedRuns = $derived(finishedRuns.length - okRuns);
	const successRate = $derived(
		finishedRuns.length ? ((okRuns / finishedRuns.length) * 100).toFixed(1) : '100.0',
	);
	// successful runs per hour over the last 40 hours, oldest first
	const hourly = $derived.by(() => {
		const hour = 3_600_000;
		const end = clientNow.getTime();
		const buckets = Array<number>(40).fill(0);
		for (const run of finishedRuns) {
			const slot = 39 - Math.floor((end - run.queued_at) / hour);
			if (slot >= 0 && slot < 40 && runOutcome(run) === 'ok') buckets[slot]++;
		}
		return buckets;
	});
	const worst = $derived.by(() => {
		let best: { task: ScheduledTask; failed: number; total: number } | null = null;
		for (const task of tasks) {
			const runs = (executions[task.id] ?? []).filter((run) => runOutcome(run) !== 'live');
			const failed = runs.filter((run) => runOutcome(run) === 'err').length;
			if (failed && (!best || failed > best.failed)) best = { task, failed, total: runs.length };
		}
		return best;
	});
	const nextTask = $derived(
		tasks
			.filter((task) => task.enabled && task.next_run_at)
			.sort((left, right) => left.next_run_at! - right.next_run_at!)[0] ?? null,
	);
	const nextHourCount = $derived(
		tasks.filter(
			(task) =>
				task.enabled && task.next_run_at && task.next_run_at - clientNow.getTime() <= 3_600_000,
		).length,
	);
	// how far along the wait for the next run is, using the cron's own interval
	const nextProgress = $derived.by(() => {
		if (!nextTask?.next_run_at) return 0;
		const [first, second] = nextRuns(
			nextTask.cron_expression,
			2,
			new Date(nextTask.next_run_at - 1),
		);
		const interval = first && second ? second.getTime() - first.getTime() : 0;
		if (!interval) return 0;
		const left = nextTask.next_run_at - clientNow.getTime();
		return Math.min(100, Math.max(0, (1 - left / interval) * 100));
	});

	const sheetTask = $derived(tasks.find((task) => task.id === sheetTaskId) ?? null);
	const sheetIndex = $derived(visible.findIndex((task) => task.id === sheetTaskId));

	function serverLabel(id: string) {
		if (id === 'local') return t('organizations.tasks.local_server');
		return servers.find((candidate) => candidate.id === id)?.name ?? id;
	}

	function target(task: ScheduledTask) {
		if (task.runner === 'ephemeral')
			return task.image ?? t('organizations.tasks.ephemeral_container');
		if (task.runner === 'application_exec')
			return (
				[appNames[task.application_id ?? ''], task.compose_service].filter(Boolean).join(' / ') ||
				t('organizations.tasks.application_exec')
			);
		if (task.runner === 'remote_shell') return serverLabel(task.server_id);
		return t('organizations.tasks.litepod_api');
	}

	onMount(() => {
		void load();
		const timer = setInterval(() => (clientNow = new Date()), 1000);
		// keeps each task's live status in sync with runs the scheduler starts on its own
		const taskRefresh = setInterval(() => {
			if (document.visibilityState === 'visible') void refreshTasks();
		}, 5000);
		return () => {
			clearInterval(timer);
			clearInterval(taskRefresh);
		};
	});

	async function load() {
		try {
			const [data, runtime, serverList] = await Promise.all([
				listScheduledTasks(organizationId),
				getRuntimeStatus(),
				listServers(organizationId),
			]);
			tasks = data;
			servers = serverList;
			serverOffsetSeconds = runtime.utc_offset_seconds ?? 0;
			const parsed = new Date(runtime.server_time);
			serverSkewMs = Number.isNaN(parsed.getTime()) ? null : parsed.getTime() - Date.now();
			void loadRuns(data);
			void loadAppNames();
		} catch (error) {
			toast.error(error instanceof Error ? error.message : t('organizations.tasks.load_failed'));
		} finally {
			loading = false;
		}
	}

	async function loadRuns(list: ScheduledTask[]) {
		const results = await Promise.allSettled(
			list.map((task) => listScheduledTaskExecutions(organizationId, task.id)),
		);
		const next = { ...executions };
		results.forEach((result, index) => {
			if (result.status === 'fulfilled') next[list[index].id] = result.value;
		});
		executions = next;
	}

	// app names for the Target column; only needed when a task runs inside an app
	async function loadAppNames() {
		if (!tasks.some((task) => task.runner === 'application_exec')) return;
		try {
			const projects = await listProjects(organizationId);
			const details = await Promise.all(projects.map((p) => getProject(organizationId, p.id)));
			appNames = Object.fromEntries(
				details.flatMap(({ environments }) =>
					environments.flatMap((environment) =>
						environment.applications.map((app) => [app.id, app.name]),
					),
				),
			);
		} catch {
			// the column falls back to the runner name
		}
	}

	async function refreshTasks() {
		try {
			const previous = new Map(tasks.map((task) => [task.id, task.active_execution_id]));
			tasks = await listScheduledTasks(organizationId);
			// a run started or finished: re-read that task's history
			const changed = tasks.filter(
				(task) => !previous.has(task.id) || previous.get(task.id) !== task.active_execution_id,
			);
			if (changed.length) void loadRuns(changed);
		} catch {
			// a missed background refresh just keeps the last list; the next tick retries
		}
	}

	function openCreate() {
		editing = null;
		cloneFrom = null;
		dialogOpen = true;
	}

	async function openEdit(task: ScheduledTask, clone = false) {
		try {
			const detail = await getScheduledTask(organizationId, task.id);
			editing = clone ? null : detail;
			cloneFrom = clone ? detail : null;
			sheetOpen = false;
			dialogOpen = true;
		} catch (error) {
			toast.error(
				error instanceof Error ? error.message : t('organizations.tasks.task_load_failed'),
			);
		}
	}

	function saved(task: ScheduledTask, created: boolean) {
		tasks = created
			? [task, ...tasks]
			: tasks.map((candidate) => (candidate.id === task.id ? task : candidate));
		if (created) void loadAppNames();
	}

	function openSheet(task: ScheduledTask, run = false) {
		sheetTaskId = task.id;
		runOnOpen = run;
		sheetOpen = true;
	}

	function navigate(step: 1 | -1) {
		if (!visible.length) return;
		const next = visible[(Math.max(0, sheetIndex) + step + visible.length) % visible.length];
		openSheet(next);
	}

	// PATCH takes the whole task, so read it first and send it back with enabled flipped
	function setEnabled(id: string, enabled: boolean) {
		tasks = tasks.map((candidate) => (candidate.id === id ? { ...candidate, enabled } : candidate));
	}

	async function toggle(task: ScheduledTask) {
		const enabled = !task.enabled;
		setEnabled(task.id, enabled);
		try {
			const detail = await getScheduledTask(organizationId, task.id);
			const updated = await updateScheduledTask(organizationId, task.id, {
				...detailToInput(detail),
				enabled,
			});
			tasks = tasks.map((candidate) => (candidate.id === updated.id ? updated : candidate));
			toast.success(
				updated.enabled
					? t('organizations.task_ui.task_resumed')
					: t('organizations.task_ui.task_paused'),
			);
		} catch (error) {
			setEnabled(task.id, !enabled);
			toast.error(
				error instanceof Error ? error.message : t('organizations.task_ui.toggle_failed'),
			);
		}
	}

	async function remove() {
		const task = deleteTask;
		if (!task) return;
		try {
			await deleteScheduledTask(organizationId, task.id);
			tasks = tasks.filter((candidate) => candidate.id !== task.id);
			if (sheetTaskId === task.id) sheetOpen = false;
			toast.success(t('organizations.tasks.deleted'));
			deleteTask = null;
		} catch (error) {
			toast.error(error instanceof Error ? error.message : t('organizations.tasks.delete_failed'));
		}
	}
</script>

{#snippet kpiHead(Icon: typeof LayersIcon, title: string, sub: string)}
	<div class="flex items-center gap-2.5">
		<span
			class="grid size-[30px] shrink-0 place-items-center rounded-lg bg-muted text-muted-foreground"
			><Icon class="size-4" /></span>
		<div class="min-w-0">
			<b class="block text-[13px] font-medium">{title}</b>
			<small class="block truncate text-xs text-muted-foreground">{sub}</small>
		</div>
	</div>
{/snippet}

<div class="flex min-w-0 flex-col gap-5 px-4 py-6 sm:px-6">
	<div class="flex flex-wrap items-center justify-between gap-4">
		<div class="min-w-0">
			<h1 class="flex flex-wrap items-center gap-2.5 text-xl font-semibold tracking-tight">
				{t('organizations.tasks.title')}
				{#if counts.failing}<Badge variant="destructive"
						>{t('organizations.task_ui.failing_count', { count: counts.failing })}</Badge
					>{/if}
			</h1>
			<p class="text-sm text-muted-foreground">
				{t('organizations.tasks.description')}
				<span class="whitespace-nowrap"
					>{t('organizations.tasks.server_time')}
					<span class="font-mono text-foreground"
						>{serverNow ?? t('organizations.common.loading')}</span
					></span>
			</p>
		</div>
		<Button onclick={openCreate}><PlusIcon />{t('organizations.task_ui.new_task')}</Button>
	</div>

	{#if loading}
		<SkeletonRows rows={4} height="h-16" label={t('organizations.tasks.loading')} />
	{:else}
		<div
			class="-mx-4 grid snap-x snap-mandatory auto-cols-[80%] grid-flow-col gap-3 overflow-x-auto px-4 sm:mx-0 sm:auto-cols-auto sm:grid-flow-row sm:grid-cols-2 sm:overflow-visible sm:px-0 xl:grid-cols-4">
			<div
				class="lp-raised flex min-w-0 snap-start flex-col gap-3.5 rounded-2xl border bg-card p-4">
				{@render kpiHead(
					LayersIcon,
					t('organizations.task_ui.kpi_active'),
					t('organizations.task_ui.kpi_paused', { count: counts.paused }),
				)}
				<div class="text-[26px] font-semibold tracking-tight tabular-nums">
					{tasks.length - counts.paused}<small
						class="ms-1 text-[13px] font-normal text-muted-foreground"
						>{t('organizations.task_ui.kpi_of', { count: tasks.length })}</small>
				</div>
				<div class="flex h-[22px] gap-1 overflow-hidden" aria-hidden="true">
					{#each tasks as task (task.id)}
						<i
							class="w-1 rounded-sm {!task.enabled
								? 'bg-primary'
								: health[task.id] === 'failing'
									? 'bg-destructive'
									: 'bg-success'}"></i>
					{/each}
				</div>
				<div class="mt-auto flex gap-3.5 text-xs text-muted-foreground">
					<span class="inline-flex items-center gap-1.5"
						><i class="h-2.5 w-[3px] rounded-sm bg-success"></i>{t(
							'organizations.task_ui.healthy_count',
							{ count: counts.healthy },
						)}</span>
					<span class="inline-flex items-center gap-1.5"
						><i class="h-2.5 w-[3px] rounded-sm bg-destructive"></i>{t(
							'organizations.task_ui.failing_count',
							{ count: counts.failing },
						)}</span>
				</div>
			</div>

			<div
				class="lp-raised flex min-w-0 snap-start flex-col gap-3.5 rounded-2xl border bg-card p-4">
				{@render kpiHead(
					ActivityIcon,
					t('organizations.task_ui.kpi_success'),
					t('organizations.task_ui.kpi_runs', { ok: okRuns, total: finishedRuns.length }),
				)}
				<div class="text-[26px] font-semibold tracking-tight tabular-nums">{successRate}%</div>
				<CellChart
					values={hourly}
					cols={40}
					rows={5}
					tone="success"
					ariaLabel={t('organizations.task_ui.kpi_success')} />
				<div class="mt-auto text-xs text-muted-foreground">
					{t('organizations.task_ui.last_hours')}
				</div>
			</div>

			<div
				class="lp-raised flex min-w-0 snap-start flex-col gap-3.5 rounded-2xl border bg-card p-4">
				{@render kpiHead(
					CircleXIcon,
					t('organizations.task_ui.kpi_failed'),
					t('organizations.task_ui.kpi_across', { count: counts.failing }),
				)}
				<div class="text-[26px] font-semibold tracking-tight tabular-nums">{failedRuns}</div>
				{#if worst}
					<button
						type="button"
						class="flex min-w-0 items-center gap-2 text-left text-[12.5px] text-muted-foreground"
						onclick={() => worst && openSheet(worst.task)}>
						{t('organizations.task_ui.worst')}
						<b class="truncate font-mono font-medium text-foreground">{worst.task.name}</b>
						<span class="ms-auto shrink-0"
							>{t('organizations.task_ui.worst_of', {
								count: worst.failed,
								total: worst.total,
							})}</span>
					</button>
				{/if}
				<div class="mt-auto flex gap-3.5 text-xs text-muted-foreground">
					<span class="inline-flex items-center gap-1.5"
						><i class="h-2.5 w-[3px] rounded-sm bg-destructive"></i>{t(
							'organizations.task_ui.failed_count',
							{ count: failedRuns },
						)}</span>
				</div>
			</div>

			<div
				class="lp-raised flex min-w-0 snap-start flex-col gap-3.5 rounded-2xl border bg-card p-4">
				{@render kpiHead(
					CalendarClockIcon,
					t('organizations.task_ui.kpi_next'),
					t('organizations.task_ui.kpi_next_hour', { count: nextHourCount }),
				)}
				<!-- same three rows with or without a next run, so toggling a task never resizes the cards -->
				{#if nextTask?.next_run_at}
					<div class="truncate text-[26px] font-semibold tracking-tight tabular-nums">
						{relativeTime(nextTask.next_run_at, clientNow.getTime(), i18n.locale)}<small
							class="ms-1.5 text-[13px] font-normal text-muted-foreground"
							>{new Date(nextTask.next_run_at).toISOString().slice(11, 16)} UTC</small>
					</div>
					<button
						type="button"
						class="flex min-w-0 items-center gap-2 text-left text-[12.5px]"
						onclick={() => nextTask && openSheet(nextTask)}>
						<b class="truncate font-mono font-medium">{nextTask.name}</b>
						{#if health[nextTask.id] === 'failing'}<Badge variant="destructive" class="ms-auto"
								>{t('organizations.task_ui.failing')}</Badge
							>{/if}
					</button>
				{:else}
					<div class="text-[26px] font-semibold tracking-tight text-muted-foreground">-</div>
					<p class="truncate text-[12.5px] text-muted-foreground">
						{t('organizations.task_ui.none_scheduled')}
					</p>
				{/if}
				<div class="lp-well mt-auto h-1 overflow-hidden rounded-full bg-muted">
					<i
						class="block h-full bg-muted-foreground"
						style="width: {nextTask?.next_run_at ? nextProgress : 0}%"></i>
				</div>
			</div>
		</div>

		<section class="lp-raised min-w-0 overflow-hidden rounded-2xl border bg-card">
			<div class="lp-groove-b flex flex-wrap items-center gap-3 px-4 py-3">
				<SegmentedSwitch
					class="w-full sm:w-auto sm:min-w-96"
					label={t('organizations.task_ui.filter')}
					bind:value={filter}
					items={(['all', 'healthy', 'failing', 'paused'] as const).map((value) => ({
						value,
						label: `${t(`organizations.task_ui.filter_${value}`)} ${counts[value]}`,
					}))} />
				<label
					class="lp-well relative flex min-w-0 flex-1 items-center rounded-full bg-muted/60 sm:ms-auto sm:max-w-xs">
					<SearchIcon class="pointer-events-none absolute left-3 size-4 text-muted-foreground" />
					<Input
						bind:value={query}
						placeholder={t('organizations.task_ui.search')}
						aria-label={t('organizations.task_ui.search')}
						class="h-9 rounded-full border-0 bg-transparent pl-9 shadow-none" />
				</label>
			</div>
			<TaskTable
				tasks={visible}
				{executions}
				{target}
				now={clientNow.getTime()}
				emptyText={tasks.length
					? t('organizations.task_ui.empty_filter')
					: t('organizations.tasks.empty')}
				onOpen={(task) => openSheet(task)}
				onRun={(task) => openSheet(task, true)}
				onEdit={(task) => void openEdit(task)}
				onDelete={(task) => (deleteTask = task)}
				onToggle={(task) => void toggle(task)} />
		</section>
	{/if}
</div>

<TaskDialog bind:open={dialogOpen} {organizationId} {editing} {cloneFrom} onSaved={saved} />

<TaskSheet
	bind:open={sheetOpen}
	{organizationId}
	task={sheetTask}
	index={Math.max(0, sheetIndex)}
	total={visible.length}
	target={sheetTask ? target(sheetTask) : ''}
	{runOnOpen}
	onNavigate={navigate}
	onEdit={(task) => void openEdit(task)}
	onClone={(task) => void openEdit(task, true)}
	onToggle={toggle}
	onDelete={(task) => (deleteTask = task)}
	onChanged={() => {
		void refreshTasks();
		if (sheetTask) void loadRuns([sheetTask]);
	}} />

<AlertDialog.Root
	open={deleteTask !== null}
	onOpenChange={(open) => {
		if (!open) deleteTask = null;
	}}>
	<AlertDialog.Portal>
		<AlertDialog.Content>
			<AlertDialog.Header>
				<AlertDialog.Title>{t('organizations.tasks.delete_confirm')}</AlertDialog.Title>
				<AlertDialog.Description>
					{t('organizations.tasks.delete_description', { name: deleteTask?.name ?? '' })}
				</AlertDialog.Description>
			</AlertDialog.Header>
			<AlertDialog.Footer>
				<AlertDialog.Cancel>{t('organizations.common.cancel')}</AlertDialog.Cancel>
				<DeleteConfirmControl label={t('organizations.tasks.delete_task')} onconfirm={remove} />
			</AlertDialog.Footer>
		</AlertDialog.Content>
	</AlertDialog.Portal>
</AlertDialog.Root>
