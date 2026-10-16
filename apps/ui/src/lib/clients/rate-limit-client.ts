import { api } from './base-client';

export type RateLimitSettings = {
	enabled: boolean;
	global_per_second: number;
	global_burst: number;
	auth_per_second: number;
	auth_burst: number;
	auth_max_attempts: number;
	auth_ban_minutes: number;
};

export async function getRateLimitSettings(): Promise<RateLimitSettings> {
	const response = await api<RateLimitSettings>('/api/v1/settings/rate-limit');
	return response.data!;
}

export async function updateRateLimitSettings(
	settings: RateLimitSettings,
): Promise<RateLimitSettings> {
	const response = await api<RateLimitSettings>('/api/v1/settings/rate-limit', {
		method: 'PATCH',
		body: JSON.stringify(settings),
	});
	return response.data!;
}
