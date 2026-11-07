import { api } from './base-client';

/** Whether application domains get Litepod's page when the application does not answer. */
export type ErrorPageSettings = { enabled: boolean };

export async function getErrorPageSettings(): Promise<ErrorPageSettings> {
	const response = await api<ErrorPageSettings>('/api/v1/settings/error-page');
	return response.data!;
}

export async function updateErrorPageSettings(enabled: boolean): Promise<ErrorPageSettings> {
	const response = await api<ErrorPageSettings>('/api/v1/settings/error-page', {
		method: 'PATCH',
		body: JSON.stringify({ enabled }),
	});
	return response.data!;
}
