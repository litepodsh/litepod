use serde::Serialize;
use utoipa::ToSchema;

use crate::dtos::project::{ApplicationDto, ProjectDto};

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct OrganizationDashboardDto {
    pub total_projects: usize,
    pub projects: Vec<ProjectDashboardDto>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ProjectDashboardDto {
    pub project: ProjectDto,
    pub environment: Option<DashboardEnvironmentDto>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DashboardEnvironmentDto {
    pub id: String,
    pub name: String,
    pub is_production: bool,
    pub applications: Vec<DashboardApplicationDto>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DashboardApplicationDto {
    pub application: ApplicationDto,
    pub deployment_status: String,
}
