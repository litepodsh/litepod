use chrono::Utc;
use uuid::Uuid;

use crate::dtos::project::{
    ApplicationDto, CreateApplicationRequest, CreateEnvironmentRequest, CreateProjectRequest,
    EnvironmentDto, ProjectDto, UpdateApplicationRequest, UpdateEnvironmentRequest,
    UpdateProjectRequest,
};
use crate::dtos::response::AppError;
use crate::dtos::server::{ServerSummaryDto, server_summary};
use crate::dtos::server_group::{ServerGroupSummaryDto, server_group_summary};
use crate::git::repositories::project;
use crate::state::state::SharedState;

/// Resolves a placement target. The lookup is scoped to the organization, so a
/// `server_id` that belongs to a different tenant is indistinguishable from one
/// that does not exist — an application can only ever land on a server its own
/// organization owns, or on the shared `local` server of a self-hosted install.
async fn ready_deployment_server(
    state: &SharedState,
    organization_id: &str,
    server_id: &str,
) -> Result<ServerSummaryDto, AppError> {
    if state.is_cloud && server_id == "local" {
        return Err(AppError::ValidationError(
            "Cloud mode requires a ready remote server. Connect and set up a remote server first."
                .into(),
        ));
    }
    let server =
        crate::servers::repository::get_for_organization(state, server_id, organization_id)
            .await?
            .ok_or_else(|| AppError::ValidationError("Choose a ready server".into()))?;
    if server.state != crate::servers::model::ServerState::Ready {
        return Err(AppError::ValidationError("Choose a ready server".into()));
    }
    if state.is_cloud && server.kind != "ssh" {
        return Err(AppError::ValidationError(
            "Cloud mode requires a ready remote server. Connect and set up a remote server first."
                .into(),
        ));
    }
    let facts = serde_json::from_str::<crate::servers::setup::ServerFacts>(&server.facts_json)
        .unwrap_or_default();
    Ok(server_summary(&server, &facts))
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

/// Resolves an application's placement to exactly one of a single server or a
/// server group. Both `None` and both `Some` are rejected.
async fn resolve_placement(
    state: &SharedState,
    organization_id: &str,
    server_id: Option<&str>,
    server_group_id: Option<&str>,
) -> Result<(Option<ServerSummaryDto>, Option<ServerGroupSummaryDto>), AppError> {
    match (server_id, server_group_id) {
        (Some(_), Some(_)) => Err(AppError::ValidationError(
            "Choose either a server or a server group, not both".into(),
        )),
        (None, None) => Err(AppError::ValidationError(
            "Choose a server or a server group".into(),
        )),
        (Some(server_id), None) => {
            let server = ready_deployment_server(state, organization_id, server_id).await?;
            Ok((Some(server), None))
        }
        (None, Some(group_id)) => {
            let group =
                crate::servers::groups::get_for_organization(state, group_id, organization_id)
                    .await?
                    .ok_or_else(|| {
                        AppError::ValidationError("Choose an existing server group".into())
                    })?;
            let count = crate::servers::groups::count_members(state, group_id).await?;
            Ok((None, Some(server_group_summary(&group, count))))
        }
    }
}

fn required_name<'a>(value: &'a str, label: &str) -> Result<&'a str, AppError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(AppError::ValidationError(format!(
            "{label} name is required"
        )));
    }
    Ok(trimmed)
}

/// Rejects application names that read as one of litepod's own services.
///
/// `deployment::deployment_identity` slugs the name straight into the container
/// name and network alias of every service the application deploys, and those
/// live on the same network as the control plane. Names matching a reserved
/// service (`deployment::compose::RESERVED_SERVICE_NAMES`) or the `litepod-`
/// infrastructure prefix are refused at creation and rename instead of producing
/// a runtime identity that is indistinguishable from infrastructure.
fn reject_reserved_name(name: &str) -> Result<(), AppError> {
    let slug = name
        .to_lowercase()
        .replace(|c: char| !c.is_alphanumeric(), "-");
    let slug = slug.trim_matches('-');
    let reserved = crate::deployment::compose::RESERVED_SERVICE_NAMES.contains(&slug)
        || slug == "litepod"
        || slug.starts_with("litepod-");
    if reserved {
        return Err(AppError::ValidationError(format!(
            "'{name}' is reserved by litepod's own services. Pick another application name."
        )));
    }
    Ok(())
}

fn database_error(error: String) -> AppError {
    let normalized = error.to_ascii_lowercase();
    if normalized.contains("unique constraint") || normalized.contains("sqlite_constraint_unique") {
        AppError::AlreadyExists
    } else {
        AppError::DatabaseError(error)
    }
}

fn next_updated_at(previous: i64) -> i64 {
    now_ms().max(previous.saturating_add(1))
}

pub async fn create_project(
    state: &SharedState,
    organization_id: &str,
    req: &CreateProjectRequest,
) -> Result<ProjectDto, AppError> {
    let name = required_name(&req.name, "Project")?;
    let description = req.description.as_deref().unwrap_or("");
    let id = Uuid::now_v7().to_string();
    let production_environment_id = Uuid::now_v7().to_string();
    let timestamp = now_ms();

    crate::git::repositories::db::retry_on_stale_snapshot(|| async {
        let mut tx = state
            .db
            .begin()
            .await
            .map_err(|error| AppError::DatabaseError(error.to_string()))?;

        project::insert_project(&mut *tx, &id, organization_id, name, description, timestamp)
            .await
            .map_err(database_error)?;
        project::insert_environment(
            &mut *tx,
            &production_environment_id,
            &id,
            "prod",
            "",
            true,
            timestamp,
        )
        .await
        .map_err(database_error)?;
        tx.commit()
            .await
            .map_err(|error| database_error(error.to_string()))?;
        Ok(())
    })
    .await?;

    Ok(ProjectDto {
        id,
        organization_id: organization_id.to_owned(),
        name: name.to_owned(),
        description: description.to_owned(),
        icon_url: None,
        created_at: timestamp,
        updated_at: timestamp,
    })
}

pub async fn update_project(
    state: &SharedState,
    organization_id: &str,
    project_id: &str,
    req: &UpdateProjectRequest,
) -> Result<ProjectDto, AppError> {
    let existing = project::get_project(state, organization_id, project_id)
        .await
        .map_err(AppError::DatabaseError)?
        .ok_or(AppError::ProjectNotFound)?;
    let name = match req.name.as_deref() {
        Some(value) => required_name(value, "Project")?.to_owned(),
        None => existing.name.clone(),
    };
    let description = req
        .description
        .as_deref()
        .unwrap_or(&existing.description)
        .to_owned();
    // Tri-state: field omitted keeps the existing override, `Some(None)`
    // clears it, `Some(Some(url))` sets it.
    let icon_url = req
        .icon_url
        .clone()
        .unwrap_or_else(|| existing.icon_url.clone());
    let updated_at = next_updated_at(existing.updated_at);
    project::update_project(
        state,
        organization_id,
        project_id,
        &name,
        &description,
        icon_url.as_deref(),
        updated_at,
    )
    .await
    .map_err(database_error)?;

    Ok(ProjectDto {
        id: existing.id,
        organization_id: existing.organization_id,
        name,
        description,
        icon_url,
        created_at: existing.created_at,
        updated_at,
    })
}

pub async fn delete_project(
    state: &SharedState,
    organization_id: &str,
    project_id: &str,
) -> Result<(), AppError> {
    project::get_project(state, organization_id, project_id)
        .await
        .map_err(AppError::DatabaseError)?
        .ok_or(AppError::ProjectNotFound)?;
    // Podman containers, Caddy routes and artifact directories outlive the SQL
    // cascade, so tear them down while the rows that name them still exist.
    let application_ids = project::list_project_application_ids(state, organization_id, project_id)
        .await
        .map_err(AppError::DatabaseError)?;
    for application_id in &application_ids {
        crate::deployment::cleanup(state, application_id).await;
    }
    project::delete_project(state, organization_id, project_id)
        .await
        .map_err(AppError::DatabaseError)
}

pub async fn create_environment(
    state: &SharedState,
    organization_id: &str,
    project_id: &str,
    req: &CreateEnvironmentRequest,
) -> Result<EnvironmentDto, AppError> {
    let name = required_name(&req.name, "Environment")?;
    let description = req.description.as_deref().unwrap_or("");
    project::get_project(state, organization_id, project_id)
        .await
        .map_err(AppError::DatabaseError)?
        .ok_or(AppError::ProjectNotFound)?;
    let id = Uuid::now_v7().to_string();
    let timestamp = now_ms();
    let conn = crate::git::repositories::db::get_conn(state).await;
    project::insert_environment(&conn, &id, project_id, name, description, false, timestamp)
        .await
        .map_err(database_error)?;
    Ok(EnvironmentDto {
        id,
        project_id: project_id.to_owned(),
        name: name.to_owned(),
        description: description.to_owned(),
        is_production: false,
        created_at: timestamp,
        updated_at: timestamp,
        applications: Vec::new(),
    })
}

pub async fn update_environment(
    state: &SharedState,
    organization_id: &str,
    project_id: &str,
    environment_id: &str,
    req: &UpdateEnvironmentRequest,
) -> Result<EnvironmentDto, AppError> {
    let name = required_name(&req.name, "Environment")?;
    let existing = project::get_environment(state, organization_id, project_id, environment_id)
        .await
        .map_err(AppError::DatabaseError)?
        .ok_or(AppError::EnvironmentNotFound)?;
    if existing.is_production {
        return Err(AppError::ValidationError(
            "Production environment cannot be updated".into(),
        ));
    }
    let description = req.description.as_deref().unwrap_or(&existing.description);
    let updated_at = next_updated_at(existing.updated_at);
    project::update_environment(
        state,
        organization_id,
        project_id,
        environment_id,
        name,
        description,
        updated_at,
    )
    .await
    .map_err(database_error)?;
    Ok(EnvironmentDto {
        id: existing.id,
        project_id: existing.project_id,
        name: name.to_owned(),
        description: description.to_owned(),
        is_production: false,
        created_at: existing.created_at,
        updated_at,
        applications: Vec::new(),
    })
}

pub async fn delete_environment(
    state: &SharedState,
    organization_id: &str,
    project_id: &str,
    environment_id: &str,
) -> Result<(), AppError> {
    let existing = project::get_environment(state, organization_id, project_id, environment_id)
        .await
        .map_err(AppError::DatabaseError)?
        .ok_or(AppError::EnvironmentNotFound)?;
    if existing.is_production {
        return Err(AppError::ValidationError(
            "Production environment cannot be deleted".into(),
        ));
    }
    let application_ids = project::list_environment_application_ids(
        state,
        organization_id,
        project_id,
        environment_id,
    )
    .await
    .map_err(AppError::DatabaseError)?;
    for application_id in &application_ids {
        crate::deployment::cleanup(state, application_id).await;
    }
    project::delete_environment(state, organization_id, project_id, environment_id)
        .await
        .map_err(AppError::DatabaseError)
}

pub async fn create_application(
    state: &SharedState,
    organization_id: &str,
    project_id: &str,
    environment_id: &str,
    req: &CreateApplicationRequest,
) -> Result<ApplicationDto, AppError> {
    create_application_with_id(
        state,
        organization_id,
        project_id,
        environment_id,
        &Uuid::now_v7().to_string(),
        None,
        req,
    )
    .await
}

pub async fn create_application_with_id(
    state: &SharedState,
    organization_id: &str,
    project_id: &str,
    environment_id: &str,
    application_id: &str,
    template_kind: Option<&str>,
    req: &CreateApplicationRequest,
) -> Result<ApplicationDto, AppError> {
    let name = required_name(&req.name, "Application")?;
    reject_reserved_name(name)?;
    Uuid::parse_str(application_id)
        .map_err(|_| AppError::ValidationError("Application ID must be a UUID".into()))?;
    project::get_environment(state, organization_id, project_id, environment_id)
        .await
        .map_err(AppError::DatabaseError)?
        .ok_or(AppError::EnvironmentNotFound)?;
    let timestamp = now_ms();
    let description = req.description.as_deref().unwrap_or("");
    let (server, server_group) = resolve_placement(
        state,
        organization_id,
        req.server_id.as_deref(),
        req.server_group_id.as_deref(),
    )
    .await?;
    crate::git::repositories::db::retry_on_stale_snapshot(|| async {
        let conn = crate::git::repositories::db::get_conn(state).await;
        project::insert_application(
            &conn,
            application_id,
            environment_id,
            name,
            description,
            template_kind,
            server.as_ref().map(|server| server.id.as_str()),
            server_group.as_ref().map(|group| group.id.as_str()),
            timestamp,
        )
        .await
        .map_err(database_error)
    })
    .await?;
    Ok(ApplicationDto {
        id: application_id.to_owned(),
        environment_id: environment_id.to_owned(),
        name: name.to_owned(),
        description: description.to_owned(),
        template_kind: template_kind.map(str::to_string),
        catalog_template_id: None,
        catalog_icon_url: None,
        icon_url: None,
        server,
        server_group,
        created_at: timestamp,
        updated_at: timestamp,
    })
}

pub async fn update_application(
    state: &SharedState,
    organization_id: &str,
    project_id: &str,
    environment_id: &str,
    application_id: &str,
    req: &UpdateApplicationRequest,
) -> Result<ApplicationDto, AppError> {
    let existing = project::get_application(
        state,
        organization_id,
        project_id,
        environment_id,
        application_id,
    )
    .await
    .map_err(AppError::DatabaseError)?
    .ok_or(AppError::ApplicationNotFound)?;
    let name = match req.name.as_deref() {
        Some(value) => {
            let value = required_name(value, "Application")?;
            reject_reserved_name(value)?;
            value.to_owned()
        }
        None => existing.name.clone(),
    };
    let description = req
        .description
        .as_deref()
        .unwrap_or(&existing.description)
        .to_owned();
    // Tri-state: field omitted keeps the existing override, `Some(None)`
    // clears it, `Some(Some(url))` sets it.
    let icon_url = req
        .icon_url
        .clone()
        .unwrap_or_else(|| existing.icon_url.clone());
    let server_id = req
        .server_id
        .clone()
        .unwrap_or_else(|| existing.server_id.clone());
    let server_group_id = req
        .server_group_id
        .clone()
        .unwrap_or_else(|| existing.server_group_id.clone());
    let (server, server_group) = resolve_placement(
        state,
        organization_id,
        server_id.as_deref(),
        server_group_id.as_deref(),
    )
    .await?;
    let updated_at = next_updated_at(existing.updated_at);
    project::update_application(
        state,
        organization_id,
        project_id,
        environment_id,
        application_id,
        &name,
        &description,
        icon_url.as_deref(),
        server_id.as_deref(),
        server_group_id.as_deref(),
        updated_at,
    )
    .await
    .map_err(database_error)?;
    Ok(ApplicationDto {
        id: existing.id,
        environment_id: existing.environment_id,
        name,
        description,
        template_kind: existing.template_kind,
        catalog_template_id: existing.catalog_template_id,
        catalog_icon_url: existing.catalog_icon_url,
        icon_url,
        server,
        server_group,
        created_at: existing.created_at,
        updated_at,
    })
}

pub async fn delete_application(
    state: &SharedState,
    organization_id: &str,
    project_id: &str,
    environment_id: &str,
    application_id: &str,
) -> Result<(), AppError> {
    project::get_application(
        state,
        organization_id,
        project_id,
        environment_id,
        application_id,
    )
    .await
    .map_err(AppError::DatabaseError)?
    .ok_or(AppError::ApplicationNotFound)?;
    crate::deployment::cleanup(state, application_id).await;
    project::delete_application(
        state,
        organization_id,
        project_id,
        environment_id,
        application_id,
    )
    .await
    .map_err(AppError::DatabaseError)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use uuid::Uuid;

    use super::*;
    use crate::{
        cqrs::queries::project as project_query,
        dtos::project::{
            CreateApplicationRequest, CreateEnvironmentRequest, CreateProjectRequest,
            UpdateEnvironmentRequest, UpdateProjectRequest,
        },
        testing::test_state_for as test_state,
    };

    fn assert_v7(id: &str) {
        assert_eq!(Uuid::parse_str(id).unwrap().get_version_num(), 7);
    }

    fn remove_database(path: &std::path::Path) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{}", path.display(), suffix));
        }
    }

    #[test]
    fn control_plane_names_are_refused_however_they_are_spelled() {
        for name in [
            "libsql",
            "LibSQL",
            " Caddy ",
            "app",
            "Api",
            "dragonfly",
            "litepod-api",
        ] {
            assert!(
                reject_reserved_name(name).is_err(),
                "expected '{name}' to be rejected"
            );
        }
        for name in ["libsql-db", "my app", "postgres", "litepodnauta"] {
            assert!(
                reject_reserved_name(name).is_ok(),
                "expected '{name}' to be accepted"
            );
        }
    }

    #[test]
    fn only_unique_database_failures_become_conflicts() {
        assert!(matches!(
            database_error(
                "UNIQUE constraint failed: projects.organization_id, projects.name".into()
            ),
            AppError::AlreadyExists
        ));
        assert!(matches!(
            database_error("FOREIGN KEY constraint failed".into()),
            AppError::DatabaseError(_)
        ));
        assert!(matches!(
            database_error("CHECK constraint failed: is_production".into()),
            AppError::DatabaseError(_)
        ));
    }

    #[tokio::test]
    async fn project_domain_enforces_hierarchy_validation_and_organization_scope() {
        let (state, path) = test_state(false).await;
        let project = create_project(
            &state,
            "organization-a",
            &CreateProjectRequest {
                name: " litepod ".into(),
                description: Some("Control plane".into()),
            },
        )
        .await
        .unwrap();
        assert_v7(&project.id);
        assert_eq!(project.name, "litepod");
        assert!(project.created_at > 1_000_000_000_000);

        let detail = project_query::get_project(&state, "organization-a", &project.id)
            .await
            .unwrap();
        assert_eq!(detail.environments.len(), 1);
        let production = &detail.environments[0];
        assert_eq!(production.name, "prod");
        assert!(production.is_production);
        assert_v7(&production.id);

        let stage = create_environment(
            &state,
            "organization-a",
            &project.id,
            &CreateEnvironmentRequest {
                name: "Staging".into(),
                description: Some("Pre-release testing".into()),
            },
        )
        .await
        .unwrap();
        assert_v7(&stage.id);
        assert_eq!(stage.description, "Pre-release testing");
        let stage = update_environment(
            &state,
            "organization-a",
            &project.id,
            &stage.id,
            &UpdateEnvironmentRequest {
                name: "Staging".into(),
                description: Some("Shared QA".into()),
            },
        )
        .await
        .unwrap();
        assert_eq!(stage.description, "Shared QA");
        let application = create_application(
            &state,
            "organization-a",
            &project.id,
            &stage.id,
            &CreateApplicationRequest {
                name: "Web".into(),
                description: None,
                server_id: Some("local".into()),
                server_group_id: None,
            },
        )
        .await
        .unwrap();
        assert_v7(&application.id);

        let preallocated_id = Uuid::now_v7().to_string();
        let preallocated = create_application_with_id(
            &state,
            "organization-a",
            &project.id,
            &stage.id,
            &preallocated_id,
            Some("postgres"),
            &CreateApplicationRequest {
                name: "Postgres".into(),
                description: None,
                server_id: Some("local".into()),
                server_group_id: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(preallocated.id, preallocated_id);
        assert_eq!(preallocated.template_kind.as_deref(), Some("postgres"));

        let updated = update_project(
            &state,
            "organization-a",
            &project.id,
            &UpdateProjectRequest {
                name: Some("litepod 2".into()),
                description: None,
                icon_url: None,
            },
        )
        .await
        .unwrap();
        assert!(updated.updated_at > project.updated_at);

        assert!(matches!(
            create_project(
                &state,
                "organization-a",
                &CreateProjectRequest {
                    name: " ".into(),
                    description: None
                },
            )
            .await,
            Err(AppError::ValidationError(_))
        ));
        assert!(matches!(
            create_project(
                &state,
                "organization-a",
                &CreateProjectRequest {
                    name: "litepod 2".into(),
                    description: None
                },
            )
            .await,
            Err(AppError::AlreadyExists)
        ));
        assert!(matches!(
            create_environment(
                &state,
                "organization-a",
                &project.id,
                &CreateEnvironmentRequest {
                    name: "Staging".into(),
                    description: None,
                },
            )
            .await,
            Err(AppError::AlreadyExists)
        ));
        assert!(matches!(
            create_application(
                &state,
                "organization-a",
                &project.id,
                &stage.id,
                &CreateApplicationRequest {
                    name: "Web".into(),
                    description: None,
                    server_id: Some("local".into()),
                    server_group_id: None,
                },
            )
            .await,
            Err(AppError::AlreadyExists)
        ));

        assert!(matches!(
            update_environment(
                &state,
                "organization-a",
                &project.id,
                &production.id,
                &UpdateEnvironmentRequest {
                    name: "Live".into(),
                    description: None,
                },
            )
            .await,
            Err(AppError::ValidationError(_))
        ));
        assert!(matches!(
            delete_environment(&state, "organization-a", &project.id, &production.id).await,
            Err(AppError::ValidationError(_))
        ));
        assert!(matches!(
            project_query::get_project(&state, "organization-b", &project.id).await,
            Err(AppError::ProjectNotFound)
        ));

        delete_environment(&state, "organization-a", &project.id, &stage.id)
            .await
            .unwrap();
        assert!(matches!(
            project_query::get_application(
                &state,
                "organization-a",
                &project.id,
                &stage.id,
                &application.id
            )
            .await,
            Err(AppError::ApplicationNotFound)
        ));

        drop(state);
        remove_database(&path);
    }

    #[tokio::test]
    async fn cloud_bootstrap_keeps_local_server_for_sysadmin_monitoring() {
        let (state, path) = test_state(true).await;
        crate::servers::bootstrap(&state).await.unwrap();
        assert!(
            crate::servers::repository::get(&state, "local")
                .await
                .unwrap()
                .is_some()
        );
        // Kept for watching only: no org may list it as a deploy target.
        assert!(
            crate::servers::repository::list_ready_for_organization(&state, "organization-a")
                .await
                .unwrap()
                .iter()
                .all(|server| server.id != "local")
        );
        drop(state);
        remove_database(&path);
    }

    #[tokio::test]
    async fn cloud_bootstrap_refuses_to_delete_local_server_with_applications() {
        let (state, path) = test_state(true).await;
        sqlx::query("INSERT INTO projects (id, organization_id, name, created_at, updated_at) VALUES ('project-cloud', 'organization-a', 'Cloud', 1, 1)")
            .execute(&state.db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO environments (id, project_id, name, is_production, created_at, updated_at) VALUES ('environment-cloud', 'project-cloud', 'prod', 1, 1, 1)")
            .execute(&state.db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO applications (id, environment_id, name, server_id, created_at, updated_at) VALUES ('application-local', 'environment-cloud', 'Local app', 'local', 1, 1)")
            .execute(&state.db)
            .await
            .unwrap();

        let error = crate::servers::bootstrap(&state).await.unwrap_err();
        assert!(error.to_string().contains("IS_CLOUD=true"));
        assert!(
            crate::servers::repository::get(&state, "local")
                .await
                .unwrap()
                .is_some()
        );
        drop(state);
        remove_database(&path);
    }

    #[tokio::test]
    async fn standard_bootstrap_keeps_local_server() {
        let (state, path) = test_state(false).await;
        crate::servers::bootstrap(&state).await.unwrap();
        assert_eq!(
            crate::servers::repository::get(&state, "local")
                .await
                .unwrap()
                .unwrap()
                .kind,
            "local"
        );
        drop(state);
        remove_database(&path);
    }

    /// A ready SSH server owned by `organization-a`, so placement tests exercise
    /// the same ownership shape the API enforces on registration.
    async fn cloud_state_with_ready_remote() -> (SharedState, std::path::PathBuf) {
        let (state, path) = test_state(true).await;
        insert_remote_server(&state, "ssh-ready", "organization-a").await;
        (state, path)
    }

    async fn insert_remote_server(state: &SharedState, id: &str, organization_id: &str) {
        sqlx::query(
            "INSERT INTO servers (id, kind, name, ssh_host, ssh_port, ssh_user, ingress_mode, podman_mode, state, organization_id, created_at, updated_at) VALUES (?1, 'ssh', ?1, 'remote.test', 22, 'litepod', 'private_relay', 'rootless', 'ready', ?2, 1, 1)",
        )
        .bind(id)
        .bind(organization_id)
        .execute(&state.db)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn cloud_rejects_local_as_an_application_destination() {
        let (state, path) = cloud_state_with_ready_remote().await;
        let error = ready_deployment_server(&state, "organization-a", "local")
            .await
            .unwrap_err();
        assert!(error.to_string().contains("ready remote server"));
        drop(state);
        remove_database(&path);
    }

    #[tokio::test]
    async fn cloud_accepts_a_ready_ssh_destination() {
        let (state, path) = cloud_state_with_ready_remote().await;
        let server = ready_deployment_server(&state, "organization-a", "ssh-ready")
            .await
            .unwrap();
        assert_eq!(server.kind, "ssh");
        drop(state);
        remove_database(&path);
    }

    #[tokio::test]
    async fn cloud_server_lists_exclude_local() {
        let (state, path) = cloud_state_with_ready_remote().await;
        let servers = crate::servers::repository::list(&state).await.unwrap();
        assert!(servers.iter().all(|server| server.kind == "ssh"));
        drop(state);
        remove_database(&path);
    }

    #[tokio::test]
    async fn an_application_cannot_be_placed_on_another_organizations_server() {
        let (state, path) = test_state(true).await;
        insert_remote_server(&state, "ssh-b", "organization-b").await;

        // Same id, same ready state — the only thing that differs is the owner,
        // and that alone must be enough to refuse the placement.
        let error = ready_deployment_server(&state, "organization-a", "ssh-b")
            .await
            .unwrap_err();
        assert!(matches!(error, AppError::ValidationError(_)));
        assert!(
            ready_deployment_server(&state, "organization-b", "ssh-b")
                .await
                .is_ok()
        );

        drop(state);
        remove_database(&path);
    }

    #[tokio::test]
    async fn another_organizations_server_reads_as_missing_not_forbidden() {
        let (state, path) = test_state(true).await;
        insert_remote_server(&state, "ssh-b", "organization-b").await;

        // `None`, not an error: the caller must not be able to tell an id that
        // belongs to someone else from one that was never registered.
        assert!(
            crate::servers::repository::get_for_organization(&state, "ssh-b", "organization-a")
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            crate::servers::repository::get_for_organization(&state, "ssh-b", "organization-b")
                .await
                .unwrap()
                .is_some()
        );

        drop(state);
        remove_database(&path);
    }

    #[tokio::test]
    async fn server_lists_are_scoped_to_the_asking_organization() {
        let (state, path) = test_state(false).await;
        insert_remote_server(&state, "ssh-a", "organization-a").await;
        insert_remote_server(&state, "ssh-b", "organization-b").await;

        let ids = |servers: Vec<crate::servers::repository::ServerRecord>| {
            servers
                .into_iter()
                .map(|server| server.id)
                .collect::<HashSet<_>>()
        };

        // `local` has no owner, so it stays visible to everyone on a self-hosted
        // install — that is what makes it the shared machine.
        let for_a =
            ids(
                crate::servers::repository::list_for_organization(&state, "organization-a", false)
                    .await
                    .unwrap(),
            );
        assert!(for_a.contains("ssh-a") && for_a.contains("local"));
        assert!(!for_a.contains("ssh-b"));

        let ready_for_b = ids(crate::servers::repository::list_ready_for_organization(
            &state,
            "organization-b",
        )
        .await
        .unwrap());
        assert!(ready_for_b.contains("ssh-b") && ready_for_b.contains("local"));
        assert!(!ready_for_b.contains("ssh-a"));

        drop(state);
        remove_database(&path);
    }

    #[tokio::test]
    async fn a_remote_server_cannot_be_registered_without_an_owner() {
        let (state, path) = test_state(true).await;
        let error = crate::servers::repository::insert_pending(
            &state,
            "ssh-orphan",
            "",
            "Orphan",
            "remote.test",
            22,
            "litepod",
            "encrypted",
            crate::servers::model::IngressMode::PrivateRelay,
            crate::servers::model::PodmanMode::Rootless,
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(error, AppError::ValidationError(_)));
        assert!(
            crate::servers::repository::get(&state, "ssh-orphan")
                .await
                .unwrap()
                .is_none()
        );
        drop(state);
        remove_database(&path);
    }
}
