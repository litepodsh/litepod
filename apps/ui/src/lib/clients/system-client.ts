import { api } from './base-client';

export type UpdateChannel = 'stable' | 'alpha';

export type UpdateState = {
	channel: UpdateChannel;
	current_version: string;
	latest_version: string;
	update_available: boolean;
	last_check_at: string | null;
	last_check_succeeded: boolean;
	last_check_note: string;
	updater_running: boolean;
	core_services: CoreUpdateState;
};

export type CoreServiceUpdateState = {
	image: string;
	current_tag: string;
	target_tag: string;
	update_available: boolean;
	ready: boolean;
	note: string;
	/** Runs as a systemd unit, so the System page cannot update it. */
	systemd_managed: boolean;
};

export type CoreUpdateState = {
	supported: boolean;
	caddy: CoreServiceUpdateState;
	dragonfly: CoreServiceUpdateState;
	last_apply_note: string;
};

export type SystemVersion = {
	version: string;
	git_sha: string;
	image: string;
	image_repo: string;
	image_tag: string;
	update_interval_secs: number;
	container_name: string;
	is_cloud: boolean;
	state: UpdateState;
	/** Seconds each core container has been running, read live; null when stopped or unknown. */
	uptime_secs: { api: number | null; caddy: number | null; dragonfly: number | null };
};

export type UpdateCheckResponse = {
	triggered: boolean;
	note: string;
};

export type UpdateApplyResponse = {
	triggered: boolean;
	note: string;
	/** An instance backup runs first; the install follows only if it succeeds. */
	backup_started?: boolean;
};

export async function getSystemVersion(): Promise<SystemVersion> {
	return (await api<SystemVersion>('/api/v1/system/version')).data!;
}

export async function triggerUpdateCheck(): Promise<UpdateCheckResponse> {
	return (await api<UpdateCheckResponse>('/api/v1/system/update/check', { method: 'POST' })).data!;
}

export async function applyUpdate(
	targetVersion: string,
	channel: UpdateChannel,
	coreServices: { caddy: string; dragonfly: string },
): Promise<UpdateApplyResponse> {
	return (
		await api<UpdateApplyResponse>('/api/v1/system/update/apply', {
			method: 'POST',
			body: JSON.stringify({ target_version: targetVersion, channel, core_services: coreServices }),
		})
	).data!;
}
