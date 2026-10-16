import { API_URL, api } from './base-client';

export type ScheduledTaskRunner = 'ephemeral' | 'remote_shell' | 'litepod_api' | 'application_exec';

export type ScheduledTaskShell = 'bash' | 'sh' | 'zsh' | 'nu' | 'fish';

export const SCHEDULED_TASK_SHELLS: readonly ScheduledTaskShell[] = [
	'bash',
	'sh',
	'zsh',
	'nu',
	'fish',
];

export type ScheduledTask = {
	id: string;
	organization_id: string;
	name: string;
	description: string;
	cron_expression: string;
	runner: ScheduledTaskRunner;
	image: string | null;
	application_id: string | null;
	compose_service: string | null;
	shell: ScheduledTaskShell;
	timeout_seconds: number;
	network_access: boolean;
	enabled: boolean;
	server_id: string;
	next_run_at: number | null;
	active_execution_id: string | null;
	created_at: number;
	updated_at: number;
};

export type ScheduledTaskInput = {
	name: string;
	description: string;
	cron_expression: string;
	runner: ScheduledTaskRunner;
	image?: string;
	application_id?: string;
	compose_service?: string;
	shell: ScheduledTaskShell;
	script: string;
	timeout_seconds: number;
	network_access?: boolean;
	server_id?: string;
};

export type ScheduledTaskDetail = ScheduledTask & { script: string };

export type ScheduledTaskExecution = {
	id: string;
	task_id: string;
	trigger: string;
	status: string;
	runner: string;
	image: string | null;
	network_access: boolean;
	server_id: string;
	queued_at: number;
	started_at: number | null;
	finished_at: number | null;
	exit_code: number | null;
	error: string | null;
	logs_truncated: boolean;
};

export type ScheduledTaskExecutionLog = {
	sequence: number;
	stream: 'stdout' | 'stderr' | 'system';
	message: string;
	created_at: number;
};

export type ScheduledTaskExecutionDetail = ScheduledTaskExecution & {
	/** Display name of the server the execution ran on, if it still exists. */
	server_name: string | null;
	/** SSH user used to reach the remote server. */
	server_user: string | null;
	logs: ScheduledTaskExecutionLog[];
};

const path = (organizationId: string) =>
	`/api/v1/organizations/${encodeURIComponent(organizationId)}/scheduled-tasks`;

export async function listScheduledTasks(organizationId: string): Promise<ScheduledTask[]> {
	return (await api<ScheduledTask[]>(path(organizationId))).data ?? [];
}

export async function createScheduledTask(
	organizationId: string,
	input: ScheduledTaskInput,
): Promise<ScheduledTask> {
	return (
		await api<ScheduledTask>(path(organizationId), { method: 'POST', body: JSON.stringify(input) })
	).data!;
}

export type ScheduledTaskTestEvent =
	| { type: 'started'; target: string | null }
	| { type: 'output'; stream: 'stdout' | 'stderr'; data: string }
	| {
			type: 'done';
			success: boolean;
			exit_code: number | null;
			error: string | null;
			duration_ms: number;
	  };

/**
 * Runs the form's script once without saving the task, streaming its output.
 * Server-sent events over `POST`, so this is `fetch` and a reader rather than
 * `EventSource`. Aborting `signal` stops the command on the server.
 */
export async function streamScheduledTaskTest(
	organizationId: string,
	input: ScheduledTaskInput,
	onEvent: (event: ScheduledTaskTestEvent) => void,
	signal?: AbortSignal,
): Promise<void> {
	const response = await fetch(`${API_URL}${path(organizationId)}/test`, {
		method: 'POST',
		credentials: 'include',
		headers: { 'Content-Type': 'application/json' },
		body: JSON.stringify(input),
		signal,
	});
	if (!response.ok || !response.body) {
		// validation failures arrive before the stream opens, as a JSON error body
		const raw = await response.text();
		let detail = `${response.status} ${response.statusText}`;
		try {
			detail = JSON.parse(raw).error ?? detail;
		} catch {
			// a non-JSON body (a proxy error page) leaves the status line as the message
		}
		throw new Error(detail);
	}
	const reader = response.body.getReader();
	const decoder = new TextDecoder();
	let buffer = '';
	for (;;) {
		const { done, value } = await reader.read();
		if (done) break;
		buffer += decoder.decode(value, { stream: true });
		let split = buffer.indexOf('\n\n');
		while (split !== -1) {
			const data = buffer
				.slice(0, split)
				.split('\n')
				.filter((line) => line.startsWith('data:'))
				.map((line) => line.slice(5).trim())
				.join('');
			buffer = buffer.slice(split + 2);
			if (data) {
				try {
					onEvent(JSON.parse(data) as ScheduledTaskTestEvent);
				} catch {
					// an unparseable frame is an upstream bug, not a reason to drop the run
				}
			}
			split = buffer.indexOf('\n\n');
		}
	}
}

export async function getScheduledTask(
	organizationId: string,
	taskId: string,
): Promise<ScheduledTaskDetail> {
	return (await api<ScheduledTaskDetail>(`${path(organizationId)}/${encodeURIComponent(taskId)}`))
		.data!;
}

export async function updateScheduledTask(
	organizationId: string,
	taskId: string,
	input: ScheduledTaskInput & { enabled: boolean },
): Promise<ScheduledTask> {
	return (
		await api<ScheduledTask>(`${path(organizationId)}/${encodeURIComponent(taskId)}`, {
			method: 'PATCH',
			body: JSON.stringify(input),
		})
	).data!;
}

export async function deleteScheduledTask(organizationId: string, taskId: string): Promise<void> {
	await api(`${path(organizationId)}/${encodeURIComponent(taskId)}`, { method: 'DELETE' });
}

export async function clearScheduledTaskHistory(
	organizationId: string,
	taskId: string,
): Promise<void> {
	await api(`${path(organizationId)}/${encodeURIComponent(taskId)}/history`, { method: 'DELETE' });
}

export async function runScheduledTask(
	organizationId: string,
	taskId: string,
): Promise<ScheduledTaskExecution> {
	return (
		await api<ScheduledTaskExecution>(`${path(organizationId)}/${encodeURIComponent(taskId)}/run`, {
			method: 'POST',
		})
	).data!;
}

export async function listScheduledTaskExecutions(
	organizationId: string,
	taskId: string,
): Promise<ScheduledTaskExecution[]> {
	return (
		(
			await api<ScheduledTaskExecution[]>(
				`${path(organizationId)}/${encodeURIComponent(taskId)}/executions`,
			)
		).data ?? []
	);
}

export async function cancelScheduledTaskExecution(
	organizationId: string,
	taskId: string,
	executionId: string,
): Promise<ScheduledTaskExecution> {
	return (
		await api<ScheduledTaskExecution>(
			`${path(organizationId)}/${encodeURIComponent(taskId)}/executions/${encodeURIComponent(executionId)}/cancel`,
			{ method: 'POST' },
		)
	).data!;
}

export type ScheduledTaskLiveEvent =
	| { type: 'line'; sequence: number; stream: ScheduledTaskExecutionLog['stream']; message: string }
	| { type: 'status'; status: string };

/** Live view of one execution: saved rows, then each change as it happens, then `status`. */
export function streamScheduledTaskExecution(
	organizationId: string,
	taskId: string,
	executionId: string,
): EventSource {
	return new EventSource(
		`${API_URL}${path(organizationId)}/${encodeURIComponent(taskId)}/executions/${encodeURIComponent(executionId)}/stream`,
		{ withCredentials: true },
	);
}

export async function getScheduledTaskExecution(
	organizationId: string,
	taskId: string,
	executionId: string,
): Promise<ScheduledTaskExecutionDetail> {
	return (
		await api<ScheduledTaskExecutionDetail>(
			`${path(organizationId)}/${encodeURIComponent(taskId)}/executions/${encodeURIComponent(executionId)}`,
		)
	).data!;
}
