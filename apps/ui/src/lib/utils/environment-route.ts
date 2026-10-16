import type { EnvironmentDto, ProjectDetailDto } from '#lib/clients/project-client.js';

export function resolveEnvironment(
	project: ProjectDetailDto,
	environmentReference: string,
): EnvironmentDto | null {
	return (
		project.environments.find(
			(environment) =>
				environment.id === environmentReference || environment.name === environmentReference,
		) ?? null
	);
}

export function environmentApplicationsHref(projectId: string, environmentName: string) {
	return `/projects/${encodeURIComponent(projectId)}/environments/${encodeURIComponent(environmentName)}/applications`;
}
