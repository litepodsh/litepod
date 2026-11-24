<script lang="ts">
	import ChevronRightIcon from '@lucide/svelte/icons/chevron-right';
	import FolderIcon from '@lucide/svelte/icons/folder';
	import FolderOpenIcon from '@lucide/svelte/icons/folder-open';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import PencilIcon from '@lucide/svelte/icons/pencil';
	import PlayIcon from '@lucide/svelte/icons/play';
	import Trash2Icon from '@lucide/svelte/icons/trash-2';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import SoftSwitch from '#lib/components/custom/soft-switch.svelte';
	import type {
		ScheduledTask,
		ScheduledTaskExecution,
	} from '#lib/clients/scheduled-tasks-client.js';
	import { describeCron } from '#lib/utils/cron.js';
	import { relativeTime } from '#lib/utils/relative-time.js';
	import { i18n, t } from '#lib/i18n/index.svelte.js';
	import RunBars from './run-bars.svelte';
	import { RUNNER_ICON } from './runner-cards.svelte';
	import {
		formatDuration,
		isActiveRun,
		runDurationMs,
		runOutcome,
		taskHealth,
		type TaskHealth,
	} from './task-utils.js';

	// The jobs table: one row per task, a card per task on phones.
	let {
		tasks,
		executions,
		target,
		emptyText,
		now = Date.now(),
		onOpen,
		onRun,
		onEdit,
		onDelete,
		onToggle,
		collapsed = $bindable([]),
		expandAll = false,
		onCreateIn,
	}: {
		tasks: ScheduledTask[];
		/** Runs per task id, newest first. */
		executions: Record<string, ScheduledTaskExecution[]>;
		target: (task: ScheduledTask) => string;
		emptyText: string;
		now?: number;
		onOpen: (task: ScheduledTask) => void;
		onRun: (task: ScheduledTask) => void;
		onEdit: (task: ScheduledTask) => void;
		onDelete: (task: ScheduledTask) => void;
		onToggle: (task: ScheduledTask) => void;
		/** Folder names shown closed. */
		collapsed?: string[];
		/** Opens every folder, e.g. while searching, without touching `collapsed`. */
		expandAll?: boolean;
		/** Adds a "new task" button to each folder header. */
		onCreateIn?: (folder: string) => void;
	} = $props();

	// folders in the order their tasks arrive; ungrouped tasks only get a header once folders exist
	const groups = $derived([...Map.groupBy(tasks, (task) => task.folder)]);
	const grouped = $derived(groups.some(([folder]) => folder));

	const HEALTH_ORDER: TaskHealth[] = ['running', 'failing', 'healthy', 'never', 'paused'];
	const HEALTH_TONE: Record<TaskHealth, string> = {
		running: 'bg-warning',
		failing: 'bg-destructive',
		healthy: 'bg-success',
		never: 'bg-muted-foreground/30',
		paused: 'bg-muted-foreground/60',
	};

	function summary(list: ScheduledTask[]) {
		const healths = list
			.map((task) => taskHealth(task, executions[task.id] ?? []))
			.sort((a, b) => HEALTH_ORDER.indexOf(a) - HEALTH_ORDER.indexOf(b));
		const count = (kind: TaskHealth) => healths.filter((health) => health === kind).length;
		const next = list
			.filter((task) => task.enabled && task.next_run_at)
			.reduce<number | null>((min, task) => Math.min(min ?? Infinity, task.next_run_at!), null);
		return {
			healths,
			running: count('running'),
			failing: count('failing'),
			healthy: count('healthy'),
			paused: count('paused'),
			next,
		};
	}

	function toggleFolder(folder: string) {
		collapsed = collapsed.includes(folder)
			? collapsed.filter((name) => name !== folder)
			: [...collapsed, folder];
	}

	const utcTime = (timestamp: number) =>
		`${new Date(timestamp).toISOString().slice(0, 16).replace('T', ' ')} UTC`;
</script>

{#snippet row(task: ScheduledTask)}
			{@const runs = executions[task.id] ?? []}
			{@const last = runs.find((run) => !isActiveRun(run))}
			{@const Icon = RUNNER_ICON[task.runner]}
			<tr
				class="cursor-pointer transition-colors duration-150 not-last:lp-groove-b hover:bg-muted/60"
				onclick={(event) => {
					if (!(event.target as HTMLElement).closest('button,a,[role=switch]')) onOpen(task);
				}}>
				<td class="max-w-64 px-4 py-3">
					<button
						type="button"
						class="block max-w-full truncate text-left font-semibold hover:underline"
						onclick={() => onOpen(task)}>{task.name}</button>
					<small class="block truncate font-mono text-xs text-muted-foreground"
						>{task.description || task.shell}</small>
				</td>
				<td class="px-3 py-3 whitespace-nowrap">
					{describeCron(task.cron_expression)?.replace(' UTC', '') ?? task.cron_expression}
					<small class="block font-mono text-xs text-muted-foreground"
						>{task.cron_expression}</small>
				</td>
				<td class="px-3 py-3">
					<!-- controlled by the owner: it flips `enabled` optimistically and flips it
					     back on failure, so the switch animates both ways and never remounts -->
					<SoftSwitch
						size="sm"
						checked={task.enabled}
						aria-label={t('organizations.task_ui.enable_task', { name: task.name })}
						onCheckedChange={() => onToggle(task)} />
				</td>
				<td class="px-3 py-3 whitespace-nowrap">
					{#if task.active_execution_id}
						<Badge variant="secondary"
							><span
								class="size-1.5 animate-pulse rounded-full bg-warning motion-reduce:animate-none"
							></span
							>{t('organizations.tasks.running')}</Badge>
					{:else if !last}
						<Badge variant="outline">{t('organizations.task_ui.never_run')}</Badge>
					{:else if runOutcome(last) === 'ok'}
						<Badge variant="success">{t('organizations.task_ui.succeeded')}</Badge>
					{:else}
						<Badge variant="destructive">{t('organizations.task_ui.failed')}</Badge>
					{/if}
					{#if last}<small class="mt-0.5 block text-xs text-muted-foreground"
							>{relativeTime(last.queued_at, now, i18n.locale)}, {formatDuration(
								runDurationMs(last),
							)}</small
						>{/if}
				</td>
				<td class="px-3 py-3">
					{#if runs.length}<RunBars executions={runs} />{:else}<span
							class="text-xs text-muted-foreground">{t('organizations.task_ui.no_runs')}</span
						>{/if}
				</td>
				<!-- fixed width, one line: toggling the switch must not reflow the table -->
				<td
					class="w-40 min-w-40 px-3 py-3 whitespace-nowrap"
					title={task.enabled && task.next_run_at ? utcTime(task.next_run_at) : undefined}>
					{#if task.enabled && task.next_run_at}
						{relativeTime(task.next_run_at, now, i18n.locale)}
					{:else}
						<span class="text-muted-foreground"
							>{task.enabled ? '-' : t('organizations.task_ui.paused')}</span>
					{/if}
				</td>
				<td class="max-w-48 px-3 py-3">
					<span class="flex min-w-0 items-center gap-1.5"
						><Icon class="size-3.5 shrink-0 text-muted-foreground" /><span class="truncate"
							>{target(task)}</span
						></span>
					<small class="block font-mono text-xs text-muted-foreground">{task.runner}</small>
				</td>
				<td class="px-3 py-3 text-right">
					<span class="inline-flex items-center gap-0.5">
						<Button
							variant="ghost"
							size="icon-sm"
							aria-label={t('organizations.task_ui.run_task', { name: task.name })}
							title={t('organizations.tasks.run_now')}
							onclick={() => onRun(task)}><PlayIcon /></Button>
						<Button
							variant="ghost"
							size="icon-sm"
							aria-label={t('organizations.task_ui.kind_edit')}
							title={t('organizations.task_ui.kind_edit')}
							onclick={() => onEdit(task)}><PencilIcon /></Button>
						<Button
							variant="ghost"
							size="icon-sm"
							class="text-muted-foreground hover:text-destructive"
							aria-label={t('organizations.tasks.delete_task')}
							title={t('organizations.tasks.delete_task')}
							onclick={() => onDelete(task)}><Trash2Icon /></Button>
					</span>
				</td>
			</tr>
{/snippet}

{#snippet folderHead(folder: string, list: ScheduledTask[], open: boolean)}
	{@const info = summary(list)}
	<tr class="folder-row lp-groove-b bg-muted/35">
		<td colspan="8" class="p-0">
			<div class="flex min-w-0 items-center gap-3 px-4 py-2">
				<button
					type="button"
					class="group/folder flex min-w-0 flex-1 items-center gap-2.5 rounded-md text-left outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
					aria-expanded={open}
					onclick={() => toggleFolder(folder)}>
					<ChevronRightIcon
						class="size-4 shrink-0 text-muted-foreground transition-transform duration-200 motion-reduce:transition-none {open
							? 'rotate-90'
							: ''}" />
					<span
						class="grid size-7 shrink-0 place-items-center rounded-lg bg-background text-muted-foreground shadow-xs ring-1 ring-border/60">
						{#if open}<FolderOpenIcon class="size-3.5" />{:else}<FolderIcon
								class="size-3.5" />{/if}
					</span>
					<span
						class="truncate text-[13px] font-semibold {folder
							? ''
							: 'font-medium text-muted-foreground'}"
						>{folder || t('organizations.task_ui.folder_none')}</span>
					<span
						class="shrink-0 rounded-full bg-background px-2 py-px font-mono text-[11px] text-muted-foreground tabular-nums ring-1 ring-border/60"
						>{list.length}</span>
				</button>
				<!-- the status line: what a closed folder holds without opening it -->
				<div class="flex shrink-0 items-center gap-3 text-xs text-muted-foreground">
					<span class="hidden h-3.5 items-center gap-[3px] sm:flex" aria-hidden="true">
						{#each info.healths.slice(0, 24) as health, index (index)}
							<i
								class="h-full w-[3px] rounded-full {HEALTH_TONE[health]} {health === 'running'
									? 'animate-pulse motion-reduce:animate-none'
									: ''}"></i>
						{/each}
					</span>
					{#if info.running}<span class="inline-flex items-center gap-1.5"
							><i class="size-1.5 rounded-full bg-warning"></i>{t(
								'organizations.task_ui.folder_running',
								{ count: info.running },
							)}</span
						>{/if}
					{#if info.failing}<span class="inline-flex items-center gap-1.5 text-destructive"
							><i class="size-1.5 rounded-full bg-destructive"></i>{t(
								'organizations.task_ui.failing_count',
								{ count: info.failing },
							)}</span
						>{/if}
					{#if info.healthy}<span class="hidden items-center gap-1.5 md:inline-flex"
							><i class="size-1.5 rounded-full bg-success"></i>{t(
								'organizations.task_ui.healthy_count',
								{ count: info.healthy },
							)}</span
						>{/if}
					{#if info.paused}<span class="hidden items-center gap-1.5 md:inline-flex"
							><i class="size-1.5 rounded-full bg-muted-foreground/60"></i>{t(
								'organizations.task_ui.folder_paused',
								{ count: info.paused },
							)}</span
						>{/if}
					{#if !open && info.next}<span
							class="hidden whitespace-nowrap lg:inline"
							title={utcTime(info.next)}
							>{t('organizations.task_ui.folder_next', {
								time: relativeTime(info.next, now, i18n.locale),
							})}</span
						>{/if}
				</div>
				{#if onCreateIn}
					<Button
						variant="ghost"
						size="icon-sm"
						class="-me-1.5 shrink-0 text-muted-foreground"
						aria-label={t('organizations.task_ui.folder_new_task', {
							name: folder || t('organizations.task_ui.folder_none'),
						})}
						title={t('organizations.task_ui.folder_new_task', {
							name: folder || t('organizations.task_ui.folder_none'),
						})}
						onclick={() => onCreateIn(folder)}><PlusIcon /></Button>
				{/if}
			</div>
		</td>
	</tr>
{/snippet}

<div class="overflow-x-auto">
	<table class="jobs w-full text-sm">
		<thead>
			<tr
				class="lp-groove-b text-left font-mono text-[11px] tracking-[.04em] text-muted-foreground uppercase">
				<th class="px-4 py-2.5 font-medium">{t('organizations.task_ui.col_task')}</th>
				<th class="px-3 py-2.5 font-medium">{t('organizations.task_ui.col_schedule')}</th>
				<th class="px-3 py-2.5 font-medium">{t('organizations.task_ui.col_enabled')}</th>
				<th class="px-3 py-2.5 font-medium">{t('organizations.task_ui.col_last')}</th>
				<th class="px-3 py-2.5 font-medium">{t('organizations.task_ui.col_runs')}</th>
				<th class="px-3 py-2.5 font-medium">{t('organizations.task_ui.col_next')}</th>
				<th class="px-3 py-2.5 font-medium">{t('organizations.task_ui.col_target')}</th>
				<th class="px-3 py-2.5"><span class="sr-only">{t('organizations.tasks.run_now')}</span></th>
			</tr>
		</thead>
		<tbody>
			{#if grouped}
				{#each groups as [folder, list] (folder)}
					{@const open = expandAll || !collapsed.includes(folder)}
					{@render folderHead(folder, list, open)}
					{#if open}
						{#each list as task (task.id)}{@render row(task)}{/each}
					{/if}
				{/each}
			{:else}
				{#each tasks as task (task.id)}{@render row(task)}{/each}
			{/if}
			{#if !tasks.length}
				<tr><td colspan="8" class="px-4 py-7 text-muted-foreground">{emptyText}</td></tr>
			{/if}
		</tbody>
	</table>
</div>

<style>
	/* phones: each row becomes a small card */
	@media (max-width: 640px) {
		.jobs thead {
			display: none;
		}
		.jobs tr {
			display: grid;
			grid-template-columns: minmax(0, 1fr) auto auto;
			grid-template-areas:
				'task sw play'
				'sched sched sched'
				'last last next'
				'runs runs runs';
			gap: 8px 12px;
			align-items: center;
			padding: 14px 16px;
		}
		.jobs td {
			padding: 0;
			max-width: none;
		}
		.jobs td:nth-child(1) {
			grid-area: task;
			min-width: 0;
			overflow-wrap: anywhere;
		}
		.jobs td:nth-child(1) button {
			white-space: normal;
		}
		.jobs td:nth-child(2) {
			grid-area: sched;
			font-size: 12.5px;
			white-space: normal;
		}
		.jobs td:nth-child(2) small {
			display: inline;
			margin-left: 6px;
		}
		.jobs td:nth-child(3) {
			grid-area: sw;
		}
		.jobs td:nth-child(4) {
			grid-area: last;
		}
		.jobs td:nth-child(5) {
			grid-area: runs;
		}
		.jobs td:nth-child(5) :global(.run-bars) {
			width: auto;
			max-width: 100%;
			height: 30px;
		}
		.jobs td:nth-child(6) {
			grid-area: next;
			text-align: right;
			font-size: 12.5px;
		}
		.jobs td:nth-child(7) {
			display: none;
		}
		.jobs td:nth-child(8) {
			grid-area: play;
		}
		.jobs td[colspan] {
			grid-column: 1 / -1;
		}
		.jobs tr.folder-row {
			display: block;
			padding: 0;
		}
	}
</style>
