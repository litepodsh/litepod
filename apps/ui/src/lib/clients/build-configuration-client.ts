import { api } from './base-client';

export type BuildConfigurationDto = { max_concurrent_builds: number; active_builds: number };

export async function getBuildConfiguration(): Promise<BuildConfigurationDto> {
	const response = await api<BuildConfigurationDto>('/api/v1/settings/build-configuration');
	return response.data!;
}

export async function updateBuildConfiguration(
	maxConcurrentBuilds: number,
): Promise<BuildConfigurationDto> {
	const response = await api<BuildConfigurationDto>('/api/v1/settings/build-configuration', {
		method: 'PATCH',
		body: JSON.stringify({ max_concurrent_builds: maxConcurrentBuilds }),
	});
	return response.data!;
}
