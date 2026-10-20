import { API_URL, api } from './base-client';

export type AuditLogDto = {
	id: string;
	action: string;
	target_type: string | null;
	target_id: string | null;
	target_name: string | null;
	metadata: string | null;
	actor_user_id: string | null;
	actor_email: string | null;
	actor_name: string | null;
	actor_token_id: string | null;
	actor_token_name: string | null;
	source_ip: string | null;
	user_agent: string | null;
	request_id: string | null;
	outcome: 'success' | 'failure' | null;
	error_message: string | null;
	created_at: string;
};

export type AuditLogFilters = {
	action?: string;
	target_type?: string;
	target_id?: string;
	actor_user_id?: string;
	outcome?: 'success' | 'failure';
	from?: string;
	to?: string;
};

/**
 * Human labels for the curated action strings the API writes. Anything
 * unmapped (or added later server-side) just falls back to the raw string.
 * Kept here so any UI surface that needs it (panel, detail sheet, future
 * notification cards) gets the same wording without duplicating the table.
 */
export const AUDIT_ACTION_LABELS: Record<string, string> = {
	'auth.login': 'Signed in',
	'auth.logout': 'Signed out',
	'organization.updated': 'Updated organization settings',
	'member.removed': 'Removed a member',
	'member.role_changed': "Changed a member's role",
	'server.created': 'Added a server',
	'server.renamed': 'Renamed a server',
	'server.deleted': 'Removed a server',
	'project.created': 'Created a project',
	'project.updated': 'Updated a project',
	'project.deleted': 'Deleted a project',
	'environment.created': 'Created an environment',
	'environment.updated': 'Updated an environment',
	'environment.deleted': 'Deleted an environment',
	'application.created': 'Created an application',
	'application.updated': 'Updated an application',
	'application.deleted': 'Deleted an application',
	'scheduled_task.created': 'Created a scheduled task',
	'scheduled_task.updated': 'Updated a scheduled task',
	'scheduled_task.deleted': 'Deleted a scheduled task',
	'deployment.triggered': 'Deployed an application',
	'deployment.started': 'Started an application',
	'deployment.stopped': 'Stopped an application',
	'deployment.canceled': 'Canceled a deployment',
	'deployment.rolled_back': 'Rolled back a deployment',
	'environment_variables.updated': 'Updated environment variables',
	'compose.updated': 'Updated a compose file',
	'volume.created': 'Mounted a volume',
	'volume.deleted': 'Removed a volume',
	'volume.file_updated': 'Edited a file in a volume',
	'container.started': 'Started a container',
	'container.stopped': 'Stopped a container',
	'container.restarted': 'Restarted a container',
	'git_provider.created': 'Added a git provider',
	'notification.provider_created': 'Created a notification provider',
	'notification.provider_updated': 'Updated a notification provider',
	'notification.provider_deleted': 'Deleted a notification provider',
	'notification.provider_tested': 'Tested a notification provider',
	'postgres.password_rotated': 'Rotated a database password',
	'postgres.created': 'Created a PostgreSQL database',
	'mariadb.created': 'Created a MariaDB database',
	'sqlserver.created': 'Created a SQL Server database',
	'dragonflydb.created': 'Created a DragonflyDB database',
	'keydb.created': 'Created a KeyDB database',
	'mongodb.created': 'Created a MongoDB database',
	'mongodb.password_rotated': 'Rotated a MongoDB root password',
	'network.attached': 'Attached a resource to a network',
	'network.detached': 'Detached a resource from a network',
	'github_app.manifest_started': 'Started GitHub App setup',
	'github_app.install_started': 'Started GitHub App installation',
	'assistant.action_proposed': 'Proposed a change',
	'assistant.action_rejected': 'Rejected a proposed change',
	'assistant.action_expired': 'A proposed change expired',
	'assistant.action_failed': 'A proposed change failed',
};

export function formatAuditAction(action: string): string {
	return AUDIT_ACTION_LABELS[action] ?? action;
}

export type ListAuditLogsOptions = {
	limit?: number;
	before?: string;
	filters?: AuditLogFilters;
};

export async function listAuditLogs(
	organizationId: string,
	options: ListAuditLogsOptions = {},
): Promise<AuditLogDto[]> {
	const params = new URLSearchParams();
	if (options.limit) params.set('limit', String(options.limit));
	if (options.before) params.set('before', options.before);
	if (options.filters) {
		for (const [key, value] of Object.entries(options.filters)) {
			if (value) params.set(key, String(value));
		}
	}
	const query = params.toString();
	const response = await api<AuditLogDto[]>(
		`/api/v1/organizations/${organizationId}/audit-logs${query ? `?${query}` : ''}`,
	);
	return response.data ?? [];
}

/** `from`/`to` are `YYYY-MM-DD HH:MM:SS` (SQLite's own `datetime()` format) — see
 *  `formatSqliteDate` in audit-logs-panel.svelte for how the UI builds them. */
export function exportAuditLogsUrl(
	organizationId: string,
	options: { from?: string; to?: string } = {},
): string {
	const params = new URLSearchParams();
	if (options.from) params.set('from', options.from);
	if (options.to) params.set('to', options.to);
	const query = params.toString();
	return `${API_URL}/api/v1/organizations/${organizationId}/audit-logs/export${query ? `?${query}` : ''}`;
}
