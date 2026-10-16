import type {
	ScheduledTask,
	ScheduledTaskDetail,
	ScheduledTaskExecution,
	ScheduledTaskInput,
} from '#lib/clients/scheduled-tasks-client.js';

export type TaskHealth = 'running' | 'failing' | 'healthy' | 'paused' | 'never';
export type RunOutcome = 'ok' | 'err' | 'live';

export const isActiveRun = (execution: ScheduledTaskExecution) =>
	execution.status === 'running' || execution.status === 'queued';

export function runOutcome(execution: ScheduledTaskExecution): RunOutcome {
	if (isActiveRun(execution)) return 'live';
	return execution.status === 'succeeded' ? 'ok' : 'err';
}

/** Executions arrive newest first; the latest finished one decides the task's health. */
export function taskHealth(task: ScheduledTask, executions: ScheduledTaskExecution[]): TaskHealth {
	if (task.active_execution_id) return 'running';
	const last = executions.find((execution) => !isActiveRun(execution));
	if (last && runOutcome(last) === 'err') return 'failing';
	if (!task.enabled) return 'paused';
	return last ? 'healthy' : 'never';
}

export function runDurationMs(execution: ScheduledTaskExecution, now = Date.now()) {
	if (!execution.started_at) return null;
	return Math.max(0, (execution.finished_at ?? now) - execution.started_at);
}

export function formatDuration(ms: number | null) {
	if (ms === null) return '-';
	const seconds = Math.round(ms / 1000);
	if (seconds < 60) return `${seconds}s`;
	return `${Math.floor(seconds / 60)}m ${String(seconds % 60).padStart(2, '0')}s`;
}

/** The saved task back into the request body, so a toggle can resend it unchanged. */
export function detailToInput(detail: ScheduledTaskDetail): ScheduledTaskInput {
	return {
		name: detail.name,
		description: detail.description,
		cron_expression: detail.cron_expression,
		runner: detail.runner,
		image: detail.image ?? undefined,
		application_id: detail.application_id ?? undefined,
		compose_service: detail.compose_service ?? undefined,
		shell: detail.shell,
		script: detail.script,
		timeout_seconds: detail.timeout_seconds,
		network_access: detail.network_access,
		server_id: detail.server_id,
	};
}
