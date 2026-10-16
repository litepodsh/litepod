<script lang="ts">
	import { onMount } from 'svelte';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import SkeletonRows from '#lib/components/custom/skeleton-rows.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
	import DeleteConfirmControl from '#lib/components/custom/delete-confirm-control.svelte';
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
	import { detailToInput } from '#lib/components/scheduled-tasks/task-utils.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';

	// The org's scheduled tasks that exec inside this app, with a "new task" that
	// opens the task dialog already set to application_exec on this app.
	let {
		organizationId,
		projectId,
		environmentId,
		applicationId,
		applicationName,
	}: {
		organizationId: string;
		projectId: string;
		environmentId: string;
		applicationId: string;
		applicationName: string;
	} = $props();

	let all = $state<ScheduledTask[]>([]);
	let executions = $state<Record<string, ScheduledTaskExecution[]>>({});
	let loading = $state(true);
	let dialogOpen = $state(false);
	let editing = $state<ScheduledTaskDetail | null>(null);
	let cloneFrom = $state<ScheduledTaskDetail | null>(null);
	let sheetOpen = $state(false);
	let deleteTask = $state<ScheduledTask | null>(null);
	let sheetTaskId = $state<string | null>(null);
	let runOnOpen = $state(false);
	let now = $state(Date.now());

	const tasks = $derived(all.filter((task) => task.application_id === applicationId));
	const sheetTask = $derived(tasks.find((task) => task.id === sheetTaskId) ?? null);
	const sheetIndex = $derived(tasks.findIndex((task) => task.id === sheetTaskId));
	const locked = $derived({
		id: applicationId,
		label: applicationName,
		projectId,
		environmentId,
	});
	const target = (task: ScheduledTask) =>
		[applicationName, task.compose_service].filter(Boolean).join(' / ');

	onMount(() => {
		void load();
		const timer = setInterval(() => {
			now = Date.now();
			if (document.visibilityState === 'visible') void load();
		}, 5000);
		return () => clearInterval(timer);
	});

	async function load() {
		try {
			all = await listScheduledTasks(organizationId);
			const results = await Promise.allSettled(
				tasks.map((task) => listScheduledTaskExecutions(organizationId, task.id)),
			);
			const next: Record<string, ScheduledTaskExecution[]> = {};
			results.forEach((result, index) => {
				if (result.status === 'fulfilled') next[tasks[index].id] = result.value;
			});
			executions = next;
		} catch (error) {
			if (loading)
				toast.error(error instanceof Error ? error.message : t('organizations.tasks.load_failed'));
		} finally {
			loading = false;
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

	function openSheet(task: ScheduledTask, run = false) {
		sheetTaskId = task.id;
		runOnOpen = run;
		sheetOpen = true;
	}

	function setEnabled(id: string, enabled: boolean) {
		all = all.map((candidate) => (candidate.id === id ? { ...candidate, enabled } : candidate));
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
			all = all.map((candidate) => (candidate.id === updated.id ? updated : candidate));
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
			all = all.filter((candidate) => candidate.id !== task.id);
			if (sheetTaskId === task.id) sheetOpen = false;
			toast.success(t('organizations.tasks.deleted'));
			deleteTask = null;
		} catch (error) {
			toast.error(error instanceof Error ? error.message : t('organizations.tasks.delete_failed'));
		}
	}
</script>

<section class="lp-raised min-w-0 overflow-hidden rounded-2xl border bg-card">
	<div class="lp-groove-b flex flex-wrap items-center justify-between gap-3 px-4 py-3">
		<div class="min-w-0">
			<h2 class="text-[15px] font-semibold tracking-tight">{t('organizations.tasks.title')}</h2>
			<p class="text-[12.5px] text-muted-foreground">
				{t('organizations.task_ui.app_tasks_sub', { count: tasks.length })}
			</p>
		</div>
		<Button size="sm" variant="outline" onclick={openCreate}
			><PlusIcon />{t('organizations.task_ui.new_in_app')}</Button>
	</div>
	{#if loading}
		<div class="p-4">
			<SkeletonRows rows={3} height="h-12" label={t('organizations.tasks.loading')} />
		</div>
	{:else}
		<TaskTable
			{tasks}
			{executions}
			{target}
			{now}
			emptyText={t('organizations.task_ui.app_tasks_empty')}
			onOpen={(task) => openSheet(task)}
			onRun={(task) => openSheet(task, true)}
			onEdit={(task) => void openEdit(task)}
			onDelete={(task) => (deleteTask = task)}
			onToggle={(task) => void toggle(task)} />
	{/if}
</section>

<TaskDialog
	bind:open={dialogOpen}
	{organizationId}
	{editing}
	{cloneFrom}
	lockedApplication={locked}
	onSaved={(task, created) => {
		all = created
			? [task, ...all]
			: all.map((candidate) => (candidate.id === task.id ? task : candidate));
	}} />

<TaskSheet
	bind:open={sheetOpen}
	{organizationId}
	task={sheetTask}
	index={Math.max(0, sheetIndex)}
	total={tasks.length}
	target={sheetTask ? target(sheetTask) : ''}
	{runOnOpen}
	onNavigate={(step) => {
		if (tasks.length)
			openSheet(tasks[(Math.max(0, sheetIndex) + step + tasks.length) % tasks.length]);
	}}
	onEdit={(task) => void openEdit(task)}
	onClone={(task) => void openEdit(task, true)}
	onToggle={toggle}
	onChanged={() => void load()} />

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
