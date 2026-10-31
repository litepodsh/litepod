use sqlx::Row;

use crate::{
    dtos::{
        dashboard::{
            DashboardApplicationDto, DashboardEnvironmentDto, OrganizationDashboardDto,
            ProjectDashboardDto,
        },
        project::{ApplicationDto, ProjectDto},
        response::AppError,
        server::{CapacityDto, ServerSummaryDto},
    },
    git::repositories::db::get_conn,
    state::state::SharedState,
};

pub async fn organization_dashboard(
    state: &SharedState,
    organization_id: &str,
) -> Result<OrganizationDashboardDto, AppError> {
    let conn = get_conn(state).await;
    let total_row = sqlx::query("SELECT COUNT(*) FROM projects WHERE organization_id = ?1")
        .bind(organization_id)
        .fetch_one(&conn)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    let total_projects: i64 = total_row
        .try_get(0)
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;

    let rows = sqlx::query(
        "WITH selected_projects AS (
                SELECT id, organization_id, name, description, created_at, updated_at, icon_url
                FROM projects WHERE organization_id = ?1 ORDER BY updated_at DESC LIMIT 6
             ), ranked_environments AS (
                SELECT e.id, e.project_id, e.name, e.is_production,
                       ROW_NUMBER() OVER (PARTITION BY e.project_id ORDER BY e.is_production DESC, e.created_at ASC) AS position
                FROM environments e INNER JOIN selected_projects p ON p.id = e.project_id
             )
             SELECT p.id, p.organization_id, p.name, p.description, p.created_at, p.updated_at,
                    e.id, e.name, e.is_production,
                    a.id, a.name, a.description, a.template_kind, a.catalog_template_id, a.catalog_icon_url,
                    s.id, s.kind, s.name, s.state, s.ingress_mode, s.facts_json, a.created_at, a.updated_at,
                    d.status, a.icon_url, p.icon_url
             FROM selected_projects p
             LEFT JOIN ranked_environments e ON e.project_id = p.id AND e.position = 1
             LEFT JOIN applications a ON a.environment_id = e.id AND a.maintenance_for IS NULL
             LEFT JOIN servers s ON s.id = a.server_id
             LEFT JOIN application_deployments d ON d.application_id = a.id
             ORDER BY p.updated_at DESC, a.created_at ASC",
    )
    .bind(organization_id)
    .fetch_all(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;

    let mut projects = Vec::<ProjectDashboardDto>::new();
    for row in &rows {
        let project_id: String = row
            .try_get(0)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?;
        let project_index = match projects
            .iter()
            .position(|item| item.project.id == project_id)
        {
            Some(index) => index,
            None => {
                projects.push(ProjectDashboardDto {
                    project: ProjectDto {
                        id: project_id,
                        organization_id: row
                            .try_get(1)
                            .map_err(|error| AppError::DatabaseError(error.to_string()))?,
                        name: row
                            .try_get(2)
                            .map_err(|error| AppError::DatabaseError(error.to_string()))?,
                        description: row
                            .try_get(3)
                            .map_err(|error| AppError::DatabaseError(error.to_string()))?,
                        created_at: row
                            .try_get(4)
                            .map_err(|error| AppError::DatabaseError(error.to_string()))?,
                        updated_at: row
                            .try_get(5)
                            .map_err(|error| AppError::DatabaseError(error.to_string()))?,
                        icon_url: row
                            .try_get(25)
                            .map_err(|error| AppError::DatabaseError(error.to_string()))?,
                    },
                    environment: row
                        .try_get::<Option<String>, _>(6)
                        .map_err(|error| AppError::DatabaseError(error.to_string()))?
                        .map(|id| DashboardEnvironmentDto {
                            id,
                            name: row.try_get(7).unwrap_or_default(),
                            is_production: row.try_get::<Option<i64>, _>(8).unwrap_or(Some(0))
                                != Some(0),
                            applications: Vec::new(),
                        }),
                });
                projects.len() - 1
            }
        };
        let Some(application_id) = row
            .try_get::<Option<String>, _>(9)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?
        else {
            continue;
        };
        let Some(environment) = projects[project_index].environment.as_mut() else {
            continue;
        };
        environment.applications.push(DashboardApplicationDto {
            application: ApplicationDto {
                id: application_id,
                environment_id: environment.id.clone(),
                name: row
                    .try_get(10)
                    .map_err(|error| AppError::DatabaseError(error.to_string()))?,
                description: row
                    .try_get(11)
                    .map_err(|error| AppError::DatabaseError(error.to_string()))?,
                template_kind: row
                    .try_get(12)
                    .map_err(|error| AppError::DatabaseError(error.to_string()))?,
                catalog_template_id: row
                    .try_get(13)
                    .map_err(|error| AppError::DatabaseError(error.to_string()))?,
                catalog_icon_url: row
                    .try_get(14)
                    .map_err(|error| AppError::DatabaseError(error.to_string()))?,
                icon_url: row
                    .try_get(24)
                    .map_err(|error| AppError::DatabaseError(error.to_string()))?,
                server: match row
                    .try_get::<Option<String>, _>(15)
                    .map_err(|error| AppError::DatabaseError(error.to_string()))?
                {
                    None => None,
                    Some(server_id) => {
                        let kind: String = row
                            .try_get(16)
                            .map_err(|error| AppError::DatabaseError(error.to_string()))?;
                        let state_value: String = row
                            .try_get(18)
                            .map_err(|error| AppError::DatabaseError(error.to_string()))?;
                        let ingress_value: String = row
                            .try_get(19)
                            .map_err(|error| AppError::DatabaseError(error.to_string()))?;
                        let facts_json: String = row
                            .try_get(20)
                            .map_err(|error| AppError::DatabaseError(error.to_string()))?;
                        let facts =
                            serde_json::from_str::<crate::servers::setup::ServerFacts>(&facts_json)
                                .unwrap_or_default();
                        let (os, distribution, os_version) =
                            crate::dtos::server::os_marks(&kind, &facts);
                        Some(ServerSummaryDto {
                            id: server_id,
                            os,
                            distribution,
                            os_version,
                            kind,
                            name: row
                                .try_get(17)
                                .map_err(|error| AppError::DatabaseError(error.to_string()))?,
                            state: state_value.parse()?,
                            ingress_mode: ingress_value.parse()?,
                            capacity: CapacityDto {
                                cpu_cores: facts.cpu_cores,
                                memory_bytes: facts.memory_bytes,
                                disk_free_bytes: facts.disk_free_bytes,
                            },
                        })
                    }
                },
                server_group: None,
                created_at: row
                    .try_get(21)
                    .map_err(|error| AppError::DatabaseError(error.to_string()))?,
                updated_at: row
                    .try_get(22)
                    .map_err(|error| AppError::DatabaseError(error.to_string()))?,
            },
            deployment_status: row
                .try_get::<Option<String>, _>(23)
                .map_err(|error| AppError::DatabaseError(error.to_string()))?
                .unwrap_or_else(|| "not_deployed".into()),
        });
    }
    Ok(OrganizationDashboardDto {
        total_projects: total_projects.max(0) as usize,
        projects,
    })
}
