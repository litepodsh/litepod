import { api } from './base-client';
import type { ApplicationDto, ProjectDto } from './project-client';

export type DashboardApplicationDto = {
	application: ApplicationDto;
	deployment_status: string;
};

export type DashboardEnvironmentDto = {
	id: string;
	name: string;
	is_production: boolean;
	applications: DashboardApplicationDto[];
};

export type ProjectDashboardDto = {
	project: ProjectDto;
	environment: DashboardEnvironmentDto | null;
};

export type OrganizationDashboardDto = {
	total_projects: number;
	projects: ProjectDashboardDto[];
};

export async function getOrganizationDashboard(
	organizationId: string,
): Promise<OrganizationDashboardDto> {
	const response = await api<OrganizationDashboardDto>(
		`/api/v1/organizations/${encodeURIComponent(organizationId)}/dashboard`,
	);
	return response.data!;
}
