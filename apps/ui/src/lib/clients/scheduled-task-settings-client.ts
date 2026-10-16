import { api } from './base-client';

export type ScheduledTaskHistorySettings = { history_retention_days: number };

export async function getScheduledTaskHistorySettings(): Promise<ScheduledTaskHistorySettings> {
	const response = await api<ScheduledTaskHistorySettings>(
		'/api/v1/settings/scheduled-task-history',
	);
	return response.data!;
}

export async function updateScheduledTaskHistorySettings(
	historyRetentionDays: number,
): Promise<ScheduledTaskHistorySettings> {
	const response = await api<ScheduledTaskHistorySettings>(
		'/api/v1/settings/scheduled-task-history',
		{
			method: 'PATCH',
			body: JSON.stringify({ history_retention_days: historyRetentionDays }),
		},
	);
	return response.data!;
}
