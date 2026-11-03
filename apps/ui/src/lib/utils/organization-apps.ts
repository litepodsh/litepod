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

export type AppBoardGroupKey = 'attention' | 'progress' | 'running' | 'other';
export type AppBoardGroup = { key: AppBoardGroupKey; apps: OrganizationApp[] };

/**
 * The home board, most urgent first. Failed and deploying apps are never cut:
 * `limit` only trims what is left after them. Empty groups are dropped.
 */
export function appBoardGroups(apps: OrganizationApp[], limit: number): AppBoardGroup[] {
	const sorted = filterApps(apps, '');
	const attention = sorted.filter((app) => app.status === 'failed');
	const progress = sorted.filter((app) => app.status === 'deploying');
	const running = sorted.filter((app) => app.status === 'running');
	const other = sorted.filter(
		(app) => !['failed', 'deploying', 'running'].includes(app.status ?? ''),
	);
	const room = Math.max(limit - attention.length - progress.length, 0);
	const shownRunning = running.slice(0, room);
	const groups: AppBoardGroup[] = [
		{ key: 'attention', apps: attention },
		{ key: 'progress', apps: progress },
		{ key: 'running', apps: shownRunning },
		{ key: 'other', apps: other.slice(0, room - shownRunning.length) },
	];
	return groups.filter((group) => group.apps.length > 0);
}
