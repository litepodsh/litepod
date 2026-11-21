import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, expect, it } from 'vitest';
import TaskTable from './task-table.svelte';
import type { ScheduledTask, ScheduledTaskExecution } from '#lib/clients/scheduled-tasks-client.js';

const task = (id: string, folder: string, failed = false) =>
	({
		id,
		name: id,
		description: '',
		folder,
		cron_expression: '0 3 * * *',
		runner: 'ephemeral',
		image: 'alpine',
		shell: 'sh',
		enabled: true,
		server_id: 'local',
		next_run_at: null,
		active_execution_id: null,
		failed,
	}) as unknown as ScheduledTask & { failed: boolean };

const noop = () => {};
const props = (tasks: ReturnType<typeof task>[], collapsed: string[] = []) => ({
	tasks,
	executions: Object.fromEntries(
		tasks.map((t) => [
			t.id,
			[
				{
					id: `${t.id}-run`,
					status: t.failed ? 'failed' : 'succeeded',
					queued_at: 0,
				} as ScheduledTaskExecution,
			],
		]),
	),
	target: () => 'alpine',
	emptyText: 'empty',
	collapsed,
	onOpen: noop,
	onRun: noop,
	onEdit: noop,
	onDelete: noop,
	onToggle: noop,
});

afterEach(cleanup);

it('renders a flat list while no task has a folder', () => {
	render(TaskTable, { props: props([task('a', ''), task('b', '')]) });
	expect(screen.queryByRole('button', { expanded: true })).toBeNull();
	expect(screen.getByText('a')).toBeTruthy();
});

it('groups by folder and a closed folder still shows its status', async () => {
	render(TaskTable, {
		props: props([task('backup', 'ops', true), task('prune', 'ops'), task('lone', '')], ['ops']),
	});
	const ops = screen.getByRole('button', { name: /ops/ });
	expect(ops.getAttribute('aria-expanded')).toBe('false');
	expect(screen.queryByText('backup')).toBeNull();
	expect(screen.getByText('1 failing')).toBeTruthy();
	// ungrouped tasks get their own open folder
	expect(screen.getByText('lone')).toBeTruthy();
	await fireEvent.click(ops);
	expect(screen.getByText('backup')).toBeTruthy();
});
