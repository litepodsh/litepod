import {
	getDeployment,
	getProject,
	listProjects,
	type ApplicationDto,
	type DeploymentDto,
	type DomainDto,
	type EnvironmentDto,
	type ProjectDto,
} from '#lib/clients/project-client.js';
import type { StatusTone } from '#lib/utils/status-tone.js';

/** One application with everything the org-wide lists show about it. */
export type OrganizationApp = {
	application: ApplicationDto;
	project: ProjectDto;
	environment: Pick<EnvironmentDto, 'id' | 'name' | 'is_production'>;
	/** `null` when the deployment could not be read. */
	status: DeploymentDto['status'] | null;
	domains: DomainDto[];
	/** Last successful deploy, epoch ms; `null` when never deployed or unreadable. */
	deployedAt?: number | null;
	href: string;
};

export function applicationHref(projectId: string, environmentId: string, applicationId: string) {
	return `/projects/${encodeURIComponent(projectId)}/environments/${encodeURIComponent(environmentId)}/applications/${encodeURIComponent(applicationId)}`;
}

/**
 * Every application in every environment of the organization.
 * ponytail: one request per project plus one per app; replace with an
 * org-level endpoint when organizations grow past a few dozen apps.
 */
export async function loadOrganizationApps(organizationId: string): Promise<OrganizationApp[]> {
	const projects = await listProjects(organizationId);
	const details = await Promise.all(projects.map((p) => getProject(organizationId, p.id)));
	const rows = details.flatMap(({ project, environments }) =>
		environments.flatMap((environment) =>
			environment.applications.map((application) => ({ application, project, environment })),
		),
	);
	const deployments = await Promise.allSettled(
		rows.map(({ project, environment, application }) =>
			getDeployment(organizationId, project.id, environment.id, application.id),
		),
	);
	return rows.map(({ application, project, environment }, index) => {
		const deployment = deployments[index];
		const ok = deployment.status === 'fulfilled';
		return {
			application,
			project,
			environment: {
				id: environment.id,
				name: environment.name,
				is_production: environment.is_production,
			},
			status: ok ? deployment.value.status : null,
			domains: ok ? deployment.value.domains : [],
			deployedAt: ok ? deployment.value.deployed_at : null,
			href: applicationHref(project.id, environment.id, application.id),
		};
	});
}

export function statusTone(status: OrganizationApp['status']): StatusTone {
	switch (status) {
		case 'running':
			return 'ok';
		case 'deploying':
		case 'ready':
			return 'warn';
		case 'failed':
			return 'danger';
		default:
			return 'neutral';
	}
}

export function primaryDomain(domains: DomainDto[]): DomainDto | undefined {
	return domains.find((domain) => domain.is_primary) ?? domains[0];
}

/** Search by app, project or domain; optionally only one project. Newest first. */
export function filterApps(
	apps: OrganizationApp[],
	query: string,
	projectId: string | null = null,
): OrganizationApp[] {
	const needle = query.trim().toLowerCase();
	return apps
		.filter((app) => !projectId || app.project.id === projectId)
		.filter(
			(app) =>
				!needle ||
				app.application.name.toLowerCase().includes(needle) ||
				app.project.name.toLowerCase().includes(needle) ||
				app.domains.some((domain) => domain.host.toLowerCase().includes(needle)),
		)
		.toSorted((a, b) => b.application.updated_at - a.application.updated_at);
}
