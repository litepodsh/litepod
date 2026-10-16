import { api, API_URL } from './base-client';

export type S3ConfigDto = {
	endpoint_url: string;
	bucket: string;
	region: string;
	access_key_id: string;
	has_secret_key: boolean;
	schedule_enabled: boolean;
	schedule_interval: 'daily' | 'weekly' | null;
	schedule_time: string | null;
};

export type UpdateS3ConfigInput = {
	endpoint_url: string;
	bucket: string;
	region: string;
	access_key_id: string;
	secret_access_key?: string;
	schedule_enabled: boolean;
	schedule_interval: 'daily' | 'weekly' | null;
	schedule_time: string | null;
};

export type BackupStatus = 'running' | 'success' | 'failed';

export type BackupRecordDto = {
	id: string;
	started_at: number;
	finished_at: number | null;
	status: BackupStatus;
	size_bytes: number | null;
	s3_key: string | null;
	error_message: string | null;
	triggered_by: 'manual' | 'scheduled';
	created_by: string | null;
};

export type RestoreRecordDto = {
	id: string;
	backup_history_id: string;
	started_at: number;
	finished_at: number | null;
	status: BackupStatus;
	error_message: string | null;
	created_by: string | null;
};

export async function getS3Config(): Promise<S3ConfigDto | null> {
	return (await api<S3ConfigDto | null>('/api/v1/settings/backup/s3-config')).data;
}

export type S3TargetDto = {
	id: string;
	name: string;
	endpoint_url: string;
	bucket: string;
	region: string;
	access_key_id: string;
	has_secret_key: boolean;
	created_at: number;
	updated_at: number;
};

export type CreateS3TargetInput = {
	name: string;
	endpoint_url: string;
	bucket: string;
	region: string;
	access_key_id: string;
	secret_access_key: string;
};

export type UpdateS3TargetInput = {
	name: string;
	endpoint_url: string;
	bucket: string;
	region: string;
	access_key_id: string;
	secret_access_key?: string;
};

/** One organization's S3 backup targets. Each organization brings its own;
 *  none are shared across organizations. Listing the connection details needs
 *  `backup.targets.manage`; selectors use {@link listS3TargetOptions}. */
function targetsPath(organizationId: string): string {
	return `/api/v1/organizations/${encodeURIComponent(organizationId)}/backup-targets`;
}

export type S3TargetOption = { id: string; name: string };

export async function listS3Targets(organizationId: string): Promise<S3TargetDto[]> {
	return (await api<S3TargetDto[]>(targetsPath(organizationId))).data!;
}

/** Id/name pairs for the database backup policy selector (`backup.read`). */
export async function listS3TargetOptions(organizationId: string): Promise<S3TargetOption[]> {
	return (await api<S3TargetOption[]>(`${targetsPath(organizationId)}/meta`)).data!;
}

export async function createS3Target(
	organizationId: string,
	input: CreateS3TargetInput,
): Promise<S3TargetDto> {
	return (
		await api<S3TargetDto>(targetsPath(organizationId), {
			method: 'POST',
			body: JSON.stringify(input),
		})
	).data!;
}

export async function updateS3Target(
	organizationId: string,
	id: string,
	input: UpdateS3TargetInput,
): Promise<S3TargetDto> {
	return (
		await api<S3TargetDto>(`${targetsPath(organizationId)}/${encodeURIComponent(id)}`, {
			method: 'PUT',
			body: JSON.stringify(input),
		})
	).data!;
}

export async function deleteS3Target(organizationId: string, id: string): Promise<void> {
	await api<void>(`${targetsPath(organizationId)}/${encodeURIComponent(id)}`, {
		method: 'DELETE',
	});
}

export async function updateS3Config(input: UpdateS3ConfigInput): Promise<S3ConfigDto> {
	return (
		await api<S3ConfigDto>('/api/v1/settings/backup/s3-config', {
			method: 'PUT',
			body: JSON.stringify(input),
		})
	).data!;
}

export async function listBackupHistory(): Promise<BackupRecordDto[]> {
	return (await api<BackupRecordDto[]>('/api/v1/maintenance/backup/history')).data!;
}

export async function triggerBackup(): Promise<BackupRecordDto> {
	return (await api<BackupRecordDto>('/api/v1/maintenance/backup/run', { method: 'POST' })).data!;
}

export async function triggerRestore(backupId: string): Promise<RestoreRecordDto> {
	return (
		await api<RestoreRecordDto>(`/api/v1/maintenance/backup/${backupId}/restore`, {
			method: 'POST',
		})
	).data!;
}

export async function listRestoreHistory(): Promise<RestoreRecordDto[]> {
	return (await api<RestoreRecordDto[]>('/api/v1/maintenance/backup/restores')).data!;
}

export type DownloadFormat = 'tar' | 'zip' | '7z';

export function downloadBackupUrl(backupId: string, format: DownloadFormat = 'tar'): string {
	return `${API_URL}/api/v1/maintenance/backup/${backupId}/download?format=${format}`;
}
