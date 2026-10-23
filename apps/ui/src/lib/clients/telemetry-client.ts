import { api } from './base-client';

export type TelemetrySettings = {
	enabled: boolean;
	env_locked: boolean;
	notice_acknowledged: boolean;
	instance_id: string;
};

export async function getTelemetrySettings(): Promise<TelemetrySettings> {
	const response = await api<TelemetrySettings>('/api/v1/settings/telemetry');
	return response.data!;
}

export async function updateTelemetrySettings(enabled: boolean): Promise<TelemetrySettings> {
	const response = await api<TelemetrySettings>('/api/v1/settings/telemetry', {
		method: 'PATCH',
		body: JSON.stringify({ enabled }),
	});
	return response.data!;
}

export async function acknowledgeTelemetryNotice(): Promise<TelemetrySettings> {
	const response = await api<TelemetrySettings>('/api/v1/settings/telemetry/acknowledge', {
		method: 'POST',
	});
	return response.data!;
}
