import { cleanup, render } from '@testing-library/svelte';
import { afterEach, expect, it } from 'vitest';
import RunBars from './run-bars.svelte';
import type { ScheduledTaskExecution } from '#lib/clients/scheduled-tasks-client.js';

afterEach(cleanup);

const run = (index: number, status: string, seconds: number): ScheduledTaskExecution => ({
	id: `run-${index}`,
	task_id: 'task',
	trigger: 'schedule',
	status,
	runner: 'ephemeral',
	image: null,
	network_access: false,
	server_id: 'local',
	queued_at: index * 1000,
	started_at: index * 1000,
	finished_at: index * 1000 + seconds * 1000,
	exit_code: status === 'failed' ? 1 : 0,
	error: null,
	logs_truncated: false,
});

it('draws one column per run, failed ones in red, height by duration', () => {
	const executions = Array.from({ length: 20 }, (_, index) =>
		run(index, index % 5 === 0 ? 'failed' : 'succeeded', index === 1 ? 0.5 : 400),
	);
	const { container } = render(RunBars, { executions });
	const columns = container.querySelectorAll('g.run');
	expect(columns).toHaveLength(20);
	expect(container.querySelectorAll('g.run.err')).toHaveLength(4);
	// executions arrive newest first and draw oldest on the left: run-0 is the last column
	const short = columns[18].querySelectorAll('rect').length;
	const long = columns[17].querySelectorAll('rect').length;
	const failed = columns[19].querySelectorAll('rect').length;
	expect(short).toBe(1);
	expect(long).toBe(5);
	expect(failed).toBe(7);
});

it('keeps only the last 20 runs', () => {
	const executions = Array.from({ length: 30 }, (_, index) => run(index, 'succeeded', 2));
	const { container } = render(RunBars, { executions });
	expect(container.querySelectorAll('g.run')).toHaveLength(20);
});
