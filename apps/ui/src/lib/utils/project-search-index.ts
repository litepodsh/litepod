import type {
	ApplicationDto,
	EnvironmentDto,
	ProjectDetailDto,
	ProjectDto,
} from '#lib/clients/project-client.js';

export type ProjectSearchEntry =
	| { type: 'project'; id: string; project: ProjectDto }
	| {
			type: 'application';
			id: string;
			project: ProjectDto;
			environment: EnvironmentDto;
			application: ApplicationDto;
	  };

export function buildProjectSearchIndex(details: ProjectDetailDto[]): ProjectSearchEntry[] {
	const entries: ProjectSearchEntry[] = [];
	for (const detail of details) {
		entries.push({ type: 'project', id: detail.project.id, project: detail.project });
		for (const environment of detail.environments) {
			for (const application of environment.applications) {
				entries.push({
					type: 'application',
					id: application.id,
					project: detail.project,
					environment,
					application,
				});
			}
		}
	}
	return entries;
}
