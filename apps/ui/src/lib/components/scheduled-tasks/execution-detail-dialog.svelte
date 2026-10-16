<script lang="ts">
	import { untrack } from 'svelte';
	import ChevronDownIcon from '@lucide/svelte/icons/chevron-down';
	import ChevronRightIcon from '@lucide/svelte/icons/chevron-right';
	import ChevronUpIcon from '@lucide/svelte/icons/chevron-up';
	import CircleCheckIcon from '@lucide/svelte/icons/circle-check';
	import CircleXIcon from '@lucide/svelte/icons/circle-x';
	import CopyIcon from '@lucide/svelte/icons/copy';
	import PauseIcon from '@lucide/svelte/icons/pause';
	import PencilIcon from '@lucide/svelte/icons/pencil';
	import PlayIcon from '@lucide/svelte/icons/play';
	import RotateCcwIcon from '@lucide/svelte/icons/rotate-ccw';
	import Trash2Icon from '@lucide/svelte/icons/trash-2';
	import XIcon from '@lucide/svelte/icons/x';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
	import * as Sheet from '#lib/components/ui/sheet/index.js';
	import DeleteConfirmControl from '#lib/components/custom/delete-confirm-control.svelte';
	import {
		clearScheduledTaskHistory,
		listScheduledTaskExecutions,
		runScheduledTask,
		type ScheduledTask,
		type ScheduledTaskExecution,
	} from '#lib/clients/scheduled-tasks-client.js';
	import { describeCron } from '#lib/utils/cron.js';
	import { relativeTime } from '#lib/utils/relative-time.js';
	import { toast } from '#lib/utils/toast.js';
	import { i18n, t } from '#lib/i18n/index.svelte.js';
	import { cn } from '#lib/utils.js';
	import { RUNNER_ICON } from './runner-cards.svelte';
	import RunLog from './run-log.svelte';
	import {
		formatDuration,
		isActiveRun,
		runDurationMs,
		runOutcome,
		taskHealth,
	} from './task-utils.js';

	// Right-hand sheet for one task: header with actions, then its runs, each
	// expandable to its log. Replaces the old history and execution dialogs.
	let {
		open = $bindable(false),
		organizationId,
		task,
		index = 0,
		total = 1,
		target = '',
		runOnOpen = false,
		onNavigate,
		onEdit,
		onClone,
		onToggle,
		onDelete,
		onChanged,
	}: {
		open?: boolean;
		organizationId: string;
		task: ScheduledTask | null;
		/** Position in the visible list, for the "3 of 8" label and prev/next. */
		index?: number;
		total?: number;
		/** Where it runs, already resolved by the owner (server or app name). */
		target?: string;
		/** Queue a manual run as soon as the sheet opens (the row's play button). */
		runOnOpen?: boolean;
		onNavigate?: (step: 1 | -1) => void;
		onEdit?: (task: ScheduledTask) => void;
		onClone?: (task: ScheduledTask) => void;
		onToggle?: (task: ScheduledTask) => Promise<unknown> | unknown;
		onDelete?: (task: ScheduledTask) => void;
		/** Runs or history changed; the owner refreshes its list. */
		onChanged?: () => void;
	} = $props();

	let executions = $state<ScheduledTaskExecution[]>([]);
	let expanded = $state<string | null>(null);
	let queueing = $state(false);
	let toggling = $state(false);
	let clearOpen = $state(false);

	const taskId = $derived(task?.id ?? null);
	const health = $derived(task ? taskHealth(task, executions) : 'never');
	const succeeded = $derived(executions.filter((run) => runOutcome(run) === 'ok').length);
	const Icon = $derived(task ? RUNNER_ICON[task.runner] : null);

	async function refresh() {
		const id = taskId;
		if (!id) return;
		try {
			const list = await listScheduledTaskExecutions(organizationId, id);
			if (id === taskId) executions = list;
		} catch (error) {
			toast.error(
				error instanceof Error ? error.message : t('organizations.tasks.history_load_failed'),
			);
		}
	}

	// a new task: load its runs, open the live one (or the latest), maybe run it
	$effect(() => {
		if (!open || !taskId) return;
		untrack(() => {
			executions = [];
			expanded = task?.active_execution_id ?? null;
			const run = runOnOpen;
			void refresh().then(() => {
				expanded ??= executions[0]?.id ?? null;
				if (run) void runNow();
			});
		});
	});

	// keeps statuses fresh while the sheet is open, like the old history dialog
	$effect(() => {
		if (!open || !taskId) return;
		const timer = setInterval(() => void refresh(), 2000);
		return () => clearInterval(timer);
	});

	async function runNow() {
		if (!task || queueing) return;
		queueing = true;
		try {
			const execution = await runScheduledTask(organizationId, task.id);
			toast.success(t('organizations.tasks.queued'));
			expanded = execution.id;
			await refresh();
			onChanged?.();
		} catch (error) {
			toast.error(error instanceof Error ? error.message : t('organizations.tasks.queue_failed'));
		} finally {
			queueing = false;
		}
	}

	async function toggle() {
		if (!task || !onToggle) return;
		toggling = true;
		try {
			await onToggle(task);
		} finally {
			toggling = false;
		}
	}

	async function clearHistory() {
		if (!task) return;
		try {
			await clearScheduledTaskHistory(organizationId, task.id);
			executions = [];
			expanded = null;
			toast.success(t('organizations.tasks.history_cleared'));
			clearOpen = false;
			onChanged?.();
		} catch (error) {
			toast.error(
				error instanceof Error ? error.message : t('organizations.tasks.history_clear_failed'),
			);
		}
	}

	const healthBadge = $derived(
		{
			running: { variant: 'secondary', label: t('organizations.tasks.running') },
			failing: { variant: 'destructive', label: t('organizations.task_ui.failing') },
			healthy: { variant: 'success', label: t('organizations.task_ui.healthy') },
			paused: { variant: 'outline', label: t('organizations.task_ui.paused') },
			never: { variant: 'outline', label: t('organizations.task_ui.never_run') },
		}[health] as { variant: 'secondary' | 'destructive' | 'success' | 'outline'; label: string },
	);
</script>

<Sheet.Root bind:open>
	<Sheet.Content
		side="right"
		showCloseButton={false}
		class="w-full gap-0 data-[side=right]:w-full data-[side=right]:sm:max-w-[680px]">
		{#if task}
			<header class="lp-groove-b grid gap-2 px-5 pt-3 pb-4">
				<div class="flex items-center gap-1">
					<span class="font-mono text-[10.5px] tracking-[.06em] text-muted-foreground uppercase"
						>{task.id.slice(0, 8)}</span>
					<span class="ms-auto me-1 font-mono text-[10.5px] text-muted-foreground"
						>{t('organizations.task_ui.position', { index: index + 1, total })}</span>
					<Button
						variant="ghost"
						size="icon-sm"
						disabled={total < 2}
						aria-label={t('organizations.task_ui.previous')}
						onclick={() => onNavigate?.(-1)}><ChevronUpIcon /></Button>
					<Button
						variant="ghost"
						size="icon-sm"
						disabled={total < 2}
						aria-label={t('organizations.task_ui.next')}
						onclick={() => onNavigate?.(1)}><ChevronDownIcon /></Button>
					<Sheet.Close>
						{#snippet child({ props })}
							<Button
								{...props}
								variant="ghost"
								size="icon-sm"
								class="bg-secondary"
								aria-label={t('organizations.common.close')}><XIcon /></Button>
						{/snippet}
					</Sheet.Close>
				</div>
				<Sheet.Title
					class="flex flex-wrap items-center gap-2.5 text-lg font-semibold tracking-tight">
					<span class="min-w-0 truncate">{task.name}</span>
					<Badge variant={healthBadge.variant}>
						{#if health === 'running'}<span
								class="size-1.5 animate-pulse rounded-full bg-warning motion-reduce:animate-none"
							></span
							>{/if}
						{healthBadge.label}
					</Badge>
				</Sheet.Title>
				<Sheet.Description
					class="flex flex-wrap items-center gap-x-2.5 gap-y-1 text-[13px] text-muted-foreground">
					<span class="font-mono text-foreground">{task.cron_expression}</span>
					<span>{describeCron(task.cron_expression) ?? ''}</span>
					{#if Icon}<span class="inline-flex items-center gap-1"
							><Icon class="size-3.5" />{target}</span
						>{/if}
					<span class="font-mono text-muted-foreground/80">{task.runner}</span>
				</Sheet.Description>
				{#if task.description}<p class="text-sm text-muted-foreground">{task.description}</p>{/if}
				<div class="flex flex-wrap gap-2 pt-1">
					<Button disabled={queueing || !!task.active_execution_id} onclick={() => void runNow()}>
						<PlayIcon />{task.active_execution_id
							? t('organizations.tasks.running')
							: t('organizations.tasks.run_now')}
					</Button>
					<Button variant="outline" onclick={() => onEdit?.(task)}
						><PencilIcon />{t('organizations.common.edit')}</Button>
					{#if onClone}
						<Button variant="outline" onclick={() => onClone?.(task)}
							><CopyIcon />{t('organizations.task_ui.clone')}</Button>
					{/if}
					{#if onToggle}
						<Button variant="outline" disabled={toggling} onclick={() => void toggle()}>
							{#if task.enabled}<PauseIcon />{t(
									'organizations.task_ui.pause',
								)}{:else}<PlayIcon />{t('organizations.task_ui.resume')}{/if}
						</Button>
					{/if}
					{#if onDelete}
						<Button
							variant="ghost"
							class="ms-auto text-destructive hover:bg-destructive/10 hover:text-destructive"
							onclick={() => onDelete?.(task)}
							><Trash2Icon />{t('organizations.tasks.delete_task')}</Button>
					{/if}
				</div>
			</header>

			<div class="min-h-0 flex-1 overflow-y-auto px-5 py-3">
				<div
					class="flex flex-wrap items-center gap-x-3.5 gap-y-1 font-mono text-[12.5px] text-muted-foreground">
					<span>{t('organizations.task_ui.runs_count', { count: executions.length })}</span>
					<span>{t('organizations.task_ui.succeeded_count', { count: succeeded })}</span>
					<button
						type="button"
						class="ms-auto font-sans text-xs text-destructive hover:underline disabled:opacity-40 disabled:no-underline"
						disabled={executions.length === 0}
						onclick={() => (clearOpen = true)}>{t('organizations.tasks.clear_history')}</button>
				</div>

				<div class="flex flex-col">
					{#each executions as execution (execution.id)}
						{@const outcome = runOutcome(execution)}
						{@const isOpen = expanded === execution.id}
						<div class="py-3.5 not-first:lp-groove-t">
							<button
								type="button"
								class="grid w-full grid-cols-[20px_minmax(0,1fr)_auto_auto] items-center gap-2.5 text-left"
								aria-expanded={isOpen}
								onclick={() => (expanded = isOpen ? null : execution.id)}>
								<span class="grid size-[18px] place-items-center">
									{#if outcome === 'live'}
										<span
											class="size-4 animate-spin rounded-full border-[1.5px] border-warning border-t-transparent motion-reduce:animate-none"
										></span>
									{:else if outcome === 'ok'}
										<CircleCheckIcon class="size-[18px] text-success" />
									{:else}
										<CircleXIcon class="size-[18px] text-destructive" />
									{/if}
								</span>
								<span class="min-w-0 truncate">
									<b class="font-mono text-[13px] font-medium">{execution.id.slice(0, 8)}</b>
									<span
										class="lp-raised ms-1.5 rounded-md bg-muted px-1.5 py-px text-[11px] text-muted-foreground"
										>{execution.trigger === 'manual'
											? t('organizations.task_ui.manual')
											: t('organizations.task_ui.scheduled')}</span>
								</span>
								<span class="font-mono text-xs text-muted-foreground"
									>{isActiveRun(execution)
										? t('organizations.tasks.running')
										: formatDuration(runDurationMs(execution))}</span>
								<ChevronRightIcon
									class={cn(
										'size-4 text-muted-foreground transition-transform duration-300 ease-[var(--lp-ease)] motion-reduce:transition-none',
										isOpen && 'rotate-90',
									)} />
							</button>
							<p class="ms-[30px] mt-1 text-[12.5px] text-muted-foreground">
								<span title={new Date(execution.queued_at).toLocaleString()}
									>{relativeTime(execution.queued_at, Date.now(), i18n.locale)}</span
								>{#if outcome === 'err'}, {execution.status}{#if execution.exit_code !== null},
										{t('organizations.tasks.exit_code').toLowerCase()}
										{execution.exit_code}{/if}{/if}
							</p>
							{#if isOpen}
								<div class="ms-[30px] mt-2.5 grid gap-2.5">
									<RunLog
										{organizationId}
										taskId={task.id}
										executionId={execution.id}
										onFinished={() => {
											void refresh();
											onChanged?.();
										}} />
									{#if outcome === 'err'}
										<div>
											<Button
												size="sm"
												variant="outline"
												disabled={queueing || !!task.active_execution_id}
												onclick={() => void runNow()}
												><RotateCcwIcon />{t('organizations.task_ui.retry')}</Button>
										</div>
									{/if}
								</div>
							{/if}
						</div>
					{:else}
						<p class="py-8 text-center text-sm text-muted-foreground">
							{t('organizations.tasks.no_executions')}
						</p>
					{/each}
				</div>
			</div>
		{/if}
	</Sheet.Content>
</Sheet.Root>

<AlertDialog.Root bind:open={clearOpen}>
	<AlertDialog.Portal>
		<AlertDialog.Content>
			<AlertDialog.Header>
				<AlertDialog.Title>{t('organizations.tasks.clear_history_confirm')}</AlertDialog.Title>
				<AlertDialog.Description>
					{t('organizations.tasks.clear_history_description', { name: task?.name ?? '' })}
				</AlertDialog.Description>
			</AlertDialog.Header>
			<AlertDialog.Footer>
				<AlertDialog.Cancel>{t('organizations.common.cancel')}</AlertDialog.Cancel>
				<DeleteConfirmControl
					label={t('organizations.tasks.clear_history')}
					onconfirm={clearHistory} />
			</AlertDialog.Footer>
		</AlertDialog.Content>
	</AlertDialog.Portal>
</AlertDialog.Root>
