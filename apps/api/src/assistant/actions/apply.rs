use crate::{
    deployment::{self, compose::store::ComposeFileStore},
    dtos::response::AppError,
    security::middleware::auth::AuthUser,
    state::state::SharedState,
};

use super::lifecycle::*;
use super::types::*;

pub(super) async fn apply_environment_variable(
    state: &SharedState,
    stored: &StoredAction,
) -> Result<Applied, AppError> {
    let arguments: SetEnvironmentVariable = serde_json::from_str(&stored.arguments)
        .map_err(|error| AppError::InternalError(error.to_string()))?;

    deployment::set_environment_variable(
        state,
        &stored.application_id,
        &arguments.scope,
        &arguments.name,
        &arguments.value,
    )
    .await?;

    Ok(Applied {
        audit_action: "environment_variables.updated",
        added: Some(format!("{}={}", arguments.name, mask(&arguments.value))),
        removed: None,
    })
}

pub(super) async fn apply_update_application(
    state: &SharedState,
    stored: &StoredAction,
) -> Result<Applied, AppError> {
    let arguments: UpdateApplication = serde_json::from_str(&stored.arguments)
        .map_err(|error| AppError::InternalError(error.to_string()))?;

    crate::cqrs::commands::project::update_application(
        state,
        &stored.organization_id,
        &stored.project_id,
        &stored.environment_id,
        &stored.application_id,
        &crate::dtos::project::UpdateApplicationRequest {
            name: arguments.name.clone(),
            description: arguments.description.clone(),
            // Absent, not null: the icon is nobody's business here, and a `None`
            // inner value would clear it.
            icon_url: None,
            server_id: None,
            server_group_id: None,
        },
    )
    .await?;

    Ok(Applied {
        audit_action: "application.updated",
        added: arguments.name.map(|name| format!("name = {name}")),
        removed: None,
    })
}

/// Merges the proposed fields onto the current configuration.
///
/// `source::save` takes the whole document, so anything not proposed is copied
/// forward: a proposal about the branch must not reset the build method.
pub(super) async fn apply_source(
    state: &SharedState,
    stored: &StoredAction,
) -> Result<Applied, AppError> {
    let arguments: SetSource = serde_json::from_str(&stored.arguments)
        .map_err(|error| AppError::InternalError(error.to_string()))?;
    let current = deployment::source::get(state, &stored.application_id).await?;

    let request = crate::dtos::deployment::UpdateSourceRequest {
        deploy_type: current.deploy_type,
        build_method: current.build_method,
        git_provider_id: arguments.git_provider_id.or(current.git_provider_id),
        git_url: arguments.git_url.or(current.git_url),
        git_branch: arguments.git_branch.or(current.git_branch),
        auto_deploy: Some(arguments.auto_deploy.unwrap_or(current.auto_deploy)),
        watch_paths: arguments.watch_paths.or(current.watch_paths),
        dockerfile_path: current.dockerfile_path,
        compose_file_path: current.compose_file_path,
        compose_public_service: current.compose_public_service,
        container_port: current.container_port,
        railpack_project_path: Some(
            arguments
                .railpack_project_path
                .unwrap_or(current.railpack_project_path),
        ),
        railpack_start_command: current.railpack_start_command,
        build_cpu_cores: current.build_cpu_cores,
        build_memory_mib: current.build_memory_mib,
        build_swap_mib: current.build_swap_mib,
        image_url: current.image_url,
        registry_username: None,
        registry_password: None,
        restart_policy: None,
    };

    deployment::source::save(state, &stored.application_id, request).await?;

    Ok(Applied {
        audit_action: "application.source_updated",
        added: None,
        removed: None,
    })
}

pub(super) async fn apply_delete_environment_variable(
    state: &SharedState,
    stored: &StoredAction,
) -> Result<Applied, AppError> {
    let arguments: SetEnvironmentVariable = serde_json::from_str(&stored.arguments)
        .map_err(|error| AppError::InternalError(error.to_string()))?;

    let removed = deployment::delete_environment_variable(
        state,
        &stored.application_id,
        &arguments.scope,
        &arguments.name,
    )
    .await?;

    Ok(Applied {
        audit_action: "environment_variables.updated",
        added: None,
        removed: removed.then(|| arguments.name.clone()),
    })
}

pub(super) async fn apply_compose_file(
    state: &SharedState,
    stored: &StoredAction,
) -> Result<Applied, AppError> {
    let arguments: SetComposeFile = serde_json::from_str(&stored.arguments)
        .map_err(|error| AppError::InternalError(error.to_string()))?;

    let store = ComposeFileStore::new(&state.data_dir, &stored.application_id);
    store
        .write_content(&arguments.compose)
        .await
        .map_err(AppError::InternalError)?;

    Ok(Applied {
        audit_action: "compose.updated",
        added: Some(format!(
            "compose file replaced, {} lines",
            arguments.compose.lines().count()
        )),
        removed: None,
    })
}

/// Creates the application, then makes it a compose application if a file was
/// proposed.
///
/// The second half is rolled back by deleting the application, the same way
/// `catalog_templates` does it: an application that exists but whose source was
/// never written is a half-created thing nobody asked for.
pub(super) async fn apply_create_application(
    state: &SharedState,
    stored: &StoredAction,
) -> Result<Applied, AppError> {
    let arguments: CreateApplication = serde_json::from_str(&stored.arguments)
        .map_err(|error| AppError::InternalError(error.to_string()))?;

    let application = crate::cqrs::commands::project::create_application(
        state,
        &stored.organization_id,
        &stored.project_id,
        &stored.environment_id,
        &crate::dtos::project::CreateApplicationRequest {
            name: arguments.name.clone(),
            description: arguments.description.clone(),
            server_id: Some(arguments.server_id.clone()),
            server_group_id: None,
        },
    )
    .await?;

    if let Some(compose) = &arguments.compose {
        let service = arguments
            .port
            .and_then(|port| u16::try_from(port).ok())
            .and_then(|port| {
                crate::deployment::compose_standalone::service_for_port(compose, port)
            });
        let written = async {
            ComposeFileStore::new(&state.data_dir, &application.id)
                .write_content(compose)
                .await
                .map_err(AppError::InternalError)?;
            deployment::source::save(
                state,
                &application.id,
                crate::dtos::deployment::UpdateSourceRequest {
                    deploy_type: "compose".into(),
                    build_method: "compose".into(),
                    git_provider_id: None,
                    git_url: None,
                    git_branch: None,
                    auto_deploy: Some(false),
                    watch_paths: None,
                    dockerfile_path: None,
                    compose_file_path: None,
                    compose_public_service: service,
                    container_port: None,
                    railpack_project_path: Some(".".into()),
                    railpack_start_command: None,
                    build_cpu_cores: None,
                    build_memory_mib: None,
                    build_swap_mib: None,
                    image_url: None,
                    registry_username: None,
                    registry_password: None,
                    restart_policy: None,
                },
            )
            .await
        }
        .await;
        if let Err(error) = written {
            let _ = crate::cqrs::commands::project::delete_application(
                state,
                &stored.organization_id,
                &stored.project_id,
                &stored.environment_id,
                &application.id,
            )
            .await;
            return Err(error);
        }
    }

    Ok(Applied {
        audit_action: "application.created",
        added: Some(format!("{} ({})", application.name, application.id)),
        removed: None,
    })
}

pub(super) async fn apply_create_project(
    state: &SharedState,
    stored: &StoredAction,
) -> Result<Applied, AppError> {
    let arguments: ProjectFields = serde_json::from_str(&stored.arguments)
        .map_err(|error| AppError::InternalError(error.to_string()))?;

    let project = crate::cqrs::commands::project::create_project(
        state,
        &stored.organization_id,
        &crate::dtos::project::CreateProjectRequest {
            name: arguments.name.unwrap_or_default(),
            description: arguments.description,
        },
    )
    .await?;

    Ok(Applied {
        audit_action: "project.created",
        added: Some(format!("project {} ({})", project.name, project.id)),
        removed: None,
    })
}

pub(super) async fn apply_update_project(
    state: &SharedState,
    stored: &StoredAction,
) -> Result<Applied, AppError> {
    let arguments: ProjectFields = serde_json::from_str(&stored.arguments)
        .map_err(|error| AppError::InternalError(error.to_string()))?;

    let project = crate::cqrs::commands::project::update_project(
        state,
        &stored.organization_id,
        &stored.project_id,
        &crate::dtos::project::UpdateProjectRequest {
            name: arguments.name.clone(),
            description: arguments.description.clone(),
            // Absent, not null: a `None` inner value would clear the icon.
            icon_url: None,
        },
    )
    .await?;

    Ok(Applied {
        audit_action: "project.updated",
        added: arguments.name.map(|_| format!("name = {}", project.name)),
        removed: None,
    })
}

pub(super) async fn apply_delete_project(
    state: &SharedState,
    stored: &StoredAction,
) -> Result<Applied, AppError> {
    crate::cqrs::commands::project::delete_project(
        state,
        &stored.organization_id,
        &stored.project_id,
    )
    .await?;

    Ok(Applied {
        audit_action: "project.deleted",
        added: None,
        removed: Some(stored.project_id.clone()),
    })
}

pub(super) async fn apply_add_domain(
    state: &SharedState,
    stored: &StoredAction,
) -> Result<Applied, AppError> {
    let arguments: DomainFields = serde_json::from_str(&stored.arguments)
        .map_err(|error| AppError::InternalError(error.to_string()))?;
    let Some(host) = arguments.host else {
        return Err(AppError::InternalError("the action lost its host".into()));
    };
    let Some(port) = arguments.port else {
        return Err(AppError::InternalError("the action lost its port".into()));
    };

    let domain = deployment::domains::create(
        state,
        &stored.application_id,
        crate::dtos::deployment::CreateDomainRequest {
            certificate: Default::default(),
            host,
            port,
            https: arguments.https,
            is_primary: arguments.is_primary,
            server_id: None,
        },
    )
    .await?;

    Ok(Applied {
        audit_action: "domain.created",
        added: Some(domain.url),
        removed: None,
    })
}

pub(super) async fn apply_update_domain(
    state: &SharedState,
    stored: &StoredAction,
) -> Result<Applied, AppError> {
    let arguments: DomainFields = serde_json::from_str(&stored.arguments)
        .map_err(|error| AppError::InternalError(error.to_string()))?;
    let Some(domain_id) = arguments.domain_id else {
        return Err(AppError::InternalError("the action lost its domain".into()));
    };

    let domain = deployment::domains::update(
        state,
        &stored.application_id,
        &domain_id,
        crate::dtos::deployment::UpdateDomainRequest {
            certificate: Default::default(),
            host: arguments.host,
            port: arguments.port,
            https: arguments.https,
            is_primary: arguments.is_primary,
            server_id: None,
        },
    )
    .await?;

    Ok(Applied {
        audit_action: "domain.updated",
        added: Some(domain.url),
        removed: None,
    })
}

pub(super) async fn apply_delete_domain(
    state: &SharedState,
    stored: &StoredAction,
) -> Result<Applied, AppError> {
    let arguments: DomainFields = serde_json::from_str(&stored.arguments)
        .map_err(|error| AppError::InternalError(error.to_string()))?;
    let Some(domain_id) = arguments.domain_id else {
        return Err(AppError::InternalError("the action lost its domain".into()));
    };

    // Read it before it goes, so the settled card can still say what was removed.
    let removed = deployment::domains::get(state, &stored.application_id, &domain_id).await?;
    deployment::domains::delete(state, &stored.application_id, &domain_id).await?;

    Ok(Applied {
        audit_action: "domain.deleted",
        added: None,
        removed: Some(removed.url),
    })
}

pub(super) async fn apply_redeploy(
    state: &SharedState,
    actor: &AuthUser,
    stored: &StoredAction,
) -> Result<Applied, AppError> {
    // `assistant_deploy` rather than `manual_deploy`: the run history should say
    // where a deployment came from.
    let job = deployment::enqueue_for_application(
        state,
        &stored.application_id,
        ASSISTANT_TRIGGER,
        Some(&actor.user_id),
    )
    .await?;

    Ok(Applied {
        audit_action: "application.deployed",
        added: Some(format!("queued as job {}", job.id)),
        removed: None,
    })
}

pub(super) async fn apply_lifecycle(
    state: &SharedState,
    actor: &AuthUser,
    stored: &StoredAction,
) -> Result<Applied, AppError> {
    let (result, audit_action) = match stored.tool.as_str() {
        START_APPLICATION => (
            deployment::start(state, &stored.application_id, Some(&actor.user_id)).await,
            "deployment.started",
        ),
        _ => (
            deployment::stop(state, &stored.application_id, Some(&actor.user_id)).await,
            "deployment.stopped",
        ),
    };
    let deployment = result?;
    Ok(Applied {
        audit_action,
        added: Some(format!("status {}", deployment.status)),
        removed: None,
    })
}

/// Records that the user said no, so the model is told and does not re-propose.
pub async fn reject(
    state: &SharedState,
    organization_id: &str,
    actor: &AuthUser,
    action_id: &str,
) -> Result<ActionDto, AppError> {
    let stored = load(state, organization_id, actor, action_id).await?;
    if stored.status != "pending" {
        return Err(AppError::ValidationError(format!(
            "This change was already {}",
            stored.status
        )));
    }
    settle(state, actor, action_id, "rejected", None).await?;
    note(
        state,
        &stored,
        "rejected by the user, so do not propose it again",
    )
    .await;
    Ok(dto(&stored, "rejected", None))
}
