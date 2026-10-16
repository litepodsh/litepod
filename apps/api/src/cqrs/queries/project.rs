use crate::dtos::project::{
    ApplicationDto, EnvironmentDto, ProjectDetailDto, ProjectDto, ResourceBaselineDto,
};
use crate::dtos::response::AppError;
use crate::dtos::server::server_summary;
use crate::dtos::server_group::server_group_summary;
use crate::git::repositories::project;
use crate::state::state::SharedState;
use podman_api::Podman;

fn project_dto(row: project::ProjectRow) -> ProjectDto {
    ProjectDto {
        id: row.id,
        organization_id: row.organization_id,
        name: row.name,
        description: row.description,
        icon_url: row.icon_url,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

async fn application_dto(
    state: &SharedState,
    row: project::ApplicationRow,
) -> Result<ApplicationDto, AppError> {
    let server = match row.server_id.as_deref() {
        Some(server_id) => {
            let server = crate::servers::repository::get(state, server_id)
                .await?
                .ok_or_else(|| {
                    AppError::DatabaseError("Application references a missing server".into())
                })?;
            let facts =
                serde_json::from_str::<crate::servers::setup::ServerFacts>(&server.facts_json)
                    .unwrap_or_default();
            Some(server_summary(&server, &facts))
        }
        None => None,
    };
    let server_group = match row.server_group_id.as_deref() {
        Some(group_id) => {
            let group = crate::servers::groups::get(state, group_id)
                .await?
                .ok_or_else(|| {
                    AppError::DatabaseError("Application references a missing server group".into())
                })?;
            let count = crate::servers::groups::count_members(state, group_id).await?;
            Some(server_group_summary(&group, count))
        }
        None => None,
    };
    Ok(ApplicationDto {
        id: row.id,
        environment_id: row.environment_id,
        name: row.name,
        description: row.description,
        template_kind: row.template_kind,
        catalog_template_id: row.catalog_template_id,
        catalog_icon_url: row.catalog_icon_url,
        icon_url: row.icon_url,
        server,
        server_group,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

pub async fn list_projects(
    state: &SharedState,
    organization_id: &str,
) -> Result<Vec<ProjectDto>, AppError> {
    project::list_projects(state, organization_id)
        .await
        .map_err(AppError::DatabaseError)
        .map(|rows| rows.into_iter().map(project_dto).collect())
}

pub async fn get_project(
    state: &SharedState,
    organization_id: &str,
    project_id: &str,
) -> Result<ProjectDetailDto, AppError> {
    let project = project::get_project(state, organization_id, project_id)
        .await
        .map_err(AppError::DatabaseError)?
        .ok_or(AppError::ProjectNotFound)?;
    let environment_rows = project::list_environments(state, organization_id, project_id)
        .await
        .map_err(AppError::DatabaseError)?;
    let mut environments = Vec::with_capacity(environment_rows.len());
    for environment in environment_rows {
        let application_rows =
            project::list_applications(state, organization_id, project_id, &environment.id)
                .await
                .map_err(AppError::DatabaseError)?;
        let mut applications = Vec::with_capacity(application_rows.len());
        for application in application_rows {
            applications.push(application_dto(state, application).await?);
        }
        environments.push(EnvironmentDto {
            id: environment.id,
            project_id: environment.project_id,
            name: environment.name,
            description: environment.description,
            is_production: environment.is_production,
            created_at: environment.created_at,
            updated_at: environment.updated_at,
            applications,
        });
    }
    Ok(ProjectDetailDto {
        project: project_dto(project),
        environments,
    })
}

pub async fn get_application(
    state: &SharedState,
    organization_id: &str,
    project_id: &str,
    environment_id: &str,
    application_id: &str,
) -> Result<ApplicationDto, AppError> {
    let application = project::get_application(
        state,
        organization_id,
        project_id,
        environment_id,
        application_id,
    )
    .await
    .map_err(AppError::DatabaseError)?
    .ok_or(AppError::ApplicationNotFound)?;
    let mut dto = application_dto(state, application).await?;
    if let Some(server) = &mut dto.server
        && server.kind == "local"
        && let Ok(podman) = Podman::new(&state.podman_socket)
        && let Ok(Ok(info)) =
            tokio::time::timeout(std::time::Duration::from_secs(2), podman.info()).await
        && let Some(host) = info.host
    {
        server.capacity.cpu_cores = host
            .cpus
            .filter(|count| *count > 0)
            .map(|count| count as f64);
        server.capacity.memory_bytes = host
            .mem_total
            .and_then(|bytes| u64::try_from(bytes).ok())
            .filter(|bytes| *bytes > 0);
    }
    Ok(dto)
}

pub async fn sample_resource_baseline(
    state: &SharedState,
    organization_id: &str,
    project_id: &str,
    environment_id: &str,
    application_id: &str,
) -> Result<Option<ResourceBaselineDto>, AppError> {
    let dto = get_application(
        state,
        organization_id,
        project_id,
        environment_id,
        application_id,
    )
    .await?;
    let Some(server_summary) = dto.server else {
        return Ok(None);
    };
    if server_summary.kind != "local"
        && let Ok(Some(server)) = crate::servers::repository::get(state, &server_summary.id).await
    {
        let mut cpu_sum = 0.0;
        let mut memory_sum = 0_u128;
        let mut samples = 0_u64;
        for sample in 0..3 {
            if sample > 0 {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(3),
                crate::servers::setup::command(&server, &["ps", "-C", "caddy", "-o", "%cpu=,rss="]),
            )
            .await;
            if let Ok(Ok(output)) = result {
                let processes = output
                    .lines()
                    .filter_map(|line| {
                        let mut fields = line.split_whitespace();
                        let cpu = fields.next()?.parse::<f64>().ok()?;
                        let memory_kib = fields.next()?.parse::<u64>().ok()?;
                        Some((cpu / 100.0, memory_kib))
                    })
                    .collect::<Vec<_>>();
                if !processes.is_empty() {
                    cpu_sum += processes.iter().map(|(cpu, _)| cpu).sum::<f64>();
                    memory_sum += processes
                        .iter()
                        .map(|(_, memory)| u128::from(*memory) * 1024)
                        .sum::<u128>();
                    samples += 1;
                }
            }
        }
        if samples > 0 {
            return Ok(Some(ResourceBaselineDto {
                cpu_cores: cpu_sum / samples as f64,
                memory_bytes: (memory_sum / u128::from(samples)).min(u128::from(u64::MAX)) as u64,
            }));
        }
    }
    if server_summary.kind == "local"
        && let Ok(runtime) =
            crate::servers::runtime::ServerRuntime::for_application(state, application_id).await
    {
        let filters = vec![
            state.container_name.clone(),
            state.caddy_container.clone(),
            "litepod-dragonfly".to_string(),
        ];
        let mut cpu_sum = 0.0;
        let mut memory_sum = 0_u128;
        let mut samples = 0_u64;
        for sample in 0..3 {
            if sample > 0 {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(3),
                crate::podman::podman_metrics::collect_runtime_container_metrics(
                    &runtime, &filters,
                ),
            )
            .await;
            if let Ok(Ok(snapshot)) = result {
                let observed = snapshot
                    .metrics
                    .iter()
                    .filter(|metric| {
                        metric.cpu_percent.is_some() && metric.memory_usage_bytes.is_some()
                    })
                    .collect::<Vec<_>>();
                if !observed.is_empty() {
                    cpu_sum += observed
                        .iter()
                        .map(|metric| metric.cpu_percent.unwrap_or(0.0) / 100.0)
                        .sum::<f64>();
                    memory_sum += observed
                        .iter()
                        .map(|metric| u128::from(metric.memory_usage_bytes.unwrap_or(0)))
                        .sum::<u128>();
                    samples += 1;
                }
            }
        }
        if samples > 0 {
            return Ok(Some(ResourceBaselineDto {
                cpu_cores: cpu_sum / samples as f64,
                memory_bytes: (memory_sum / u128::from(samples)).min(u128::from(u64::MAX)) as u64,
            }));
        }
    }
    Ok(None)
}
