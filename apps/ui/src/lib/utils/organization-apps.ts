import {
	listOrganizationAppsSummary,
	type AppSummaryDomainDto,
	type AppSummaryDto,
} from '#lib/clients/organization-client.js';
import type { StatusTone } from '#lib/utils/status-tone.js';

/** One application with everything the org-wide lists show about it. */
export type OrganizationApp = {
	application: AppSummaryDto['application'];
	project: AppSummaryDto['project'];
	environment: AppSummaryDto['environment'];
	status: string | null;
	domains: AppSummaryDomainDto[];
	/** Last successful deploy, epoch ms; `null` when never deployed or unreadable. */
	deployedAt: number | null;
	/** Running/total containers; `null` when the application's server was unreachable. */
	containers: { running: number; total: number } | null;
	href: string;
};

export function applicationHref(projectId: string, environmentId: string, applicationId: string) {
	return `/projects/${encodeURIComponent(projectId)}/environments/${encodeURIComponent(environmentId)}/applications/${encodeURIComponent(applicationId)}`;
}

/**
 * Every application in every environment of the organization, in one request.
 * This used to fan out one `getDeployment` call per application — a 35-request
 * burst that the global rate limit answered with 429s, so most rows lost their
 * status, domains and deploy date.
 */
export async function loadOrganizationApps(organizationId: string): Promise<OrganizationApp[]> {
	const summary = await listOrganizationAppsSummary(organizationId);
	return summary.map((app) => ({
		application: app.application,
		project: app.project,
		environment: app.environment,
		status: app.status,
		domains: app.domains,
		deployedAt: app.deployed_at,
		containers: app.containers,
		href: applicationHref(app.project.id, app.environment.id, app.application.id),
	}));
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

export function primaryDomain(domains: AppSummaryDomainDto[]): AppSummaryDomainDto | undefined {
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
