use std::ffi::OsString;

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

    // A repository or a build method only means something to a git deploy.
    let deploy_type = if arguments.git_url.is_some() || arguments.build_method.is_some() {
        "git".to_owned()
    } else {
        current.deploy_type
    };
    let request = crate::dtos::deployment::UpdateSourceRequest {
        deploy_type,
        build_method: arguments.build_method.unwrap_or(current.build_method),
        git_provider_id: arguments.git_provider_id.or(current.git_provider_id),
        git_url: arguments.git_url.or(current.git_url),
        git_branch: arguments.git_branch.or(current.git_branch),
        auto_deploy: Some(arguments.auto_deploy.unwrap_or(current.auto_deploy)),
        watch_paths: arguments.watch_paths.or(current.watch_paths),
        dockerfile_path: arguments.dockerfile_path.or(current.dockerfile_path),
        compose_file_path: arguments.compose_file_path.or(current.compose_file_path),
        compose_public_service: arguments
            .compose_public_service
            .or(current.compose_public_service),
        container_port: arguments.container_port.or(current.container_port),
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
/// Makes `application_id` a Compose application running `compose`, routed to
/// whichever service publishes `port`. The port is also stored the way a
/// catalog template's primary port is, so `{{url}}` in its env resolves.
async fn write_compose(
    state: &SharedState,
    application_id: &str,
    compose: &str,
    port: Option<i64>,
) -> Result<(), AppError> {
    let service = port
        .and_then(|port| u16::try_from(port).ok())
        .and_then(|port| crate::deployment::compose_standalone::service_for_port(compose, port));
    ComposeFileStore::new(&state.data_dir, application_id)
        .write_content(compose)
        .await
        .map_err(AppError::InternalError)?;
    deployment::source::save(
        state,
        application_id,
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
            compose_public_service: service.clone(),
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
    .await?;
    let ports = super::propose::primary_port(port, service.as_deref());
    if !ports.is_empty() {
        let conn = crate::git::repositories::db::get_conn(state).await;
        crate::git::repositories::project::set_catalog_ports(
            &conn,
            application_id,
            &ports,
            false,
            false,
        )
        .await
        .map_err(AppError::DatabaseError)?;
    }
    Ok(())
}

/// The compose service a domain on `port` must be routed to, or `None` when
/// the application is not a compose one. A compose application only serves a
/// host through a route naming a service and its container port; a domain
/// without one has no upstream and the browser gets a certificate error.
async fn compose_route_target(
    state: &SharedState,
    application_id: &str,
    port: i64,
) -> Result<Option<(String, u16)>, AppError> {
    let source = deployment::source::get(state, application_id).await?;
    if source.build_method != "compose" {
        return Ok(None);
    }
    let Ok(port) = u16::try_from(port) else {
        return Ok(None);
    };
    let compose = ComposeFileStore::new(&state.data_dir, application_id)
        .read_content()
        .await
        .map_err(AppError::InternalError)?;
    let service = crate::deployment::compose_standalone::service_for_port(&compose, port)
        .or(source.compose_public_service);
    Ok(service.map(|service| (service, port)))
}

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
        if let Err(error) = write_compose(state, &application.id, compose, arguments.port).await {
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
    if let Some((service, container_port)) =
        compose_route_target(state, &stored.application_id, port).await?
    {
        crate::deployment::compose::proxy_routes::save(
            state,
            &stored.application_id,
            &domain.id,
            &service,
            container_port,
        )
        .await?;
    }

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

pub(super) async fn apply_rollback(
    state: &SharedState,
    actor: &AuthUser,
    stored: &StoredAction,
) -> Result<Applied, AppError> {
    let arguments: Rollback = serde_json::from_str(&stored.arguments)
        .map_err(|error| AppError::InternalError(error.to_string()))?;
    let job = deployment::enqueue_rollback_for_application(
        state,
        &stored.application_id,
        &arguments.run,
        Some(&actor.user_id),
    )
    .await?;

    Ok(Applied {
        audit_action: "deployment.rolled_back",
        added: Some(format!(
            "rollback to {} queued as job {}",
            arguments.run, job.id
        )),
        removed: None,
    })
}

/// Restarts the containers on one server. Returns how many were restarted, or
/// `None` when the application has no container running there and the caller
/// should redeploy instead.
async fn restart_here(
    state: &SharedState,
    application_id: &str,
    container: Option<&str>,
) -> Result<Option<usize>, AppError> {
    let names = crate::handler::deployments::container_names(state, application_id).await?;
    let runtime =
        crate::servers::runtime::ServerRuntime::for_application(state, application_id).await?;
    let targets: Vec<String> = match container {
        Some(name) => names
            .into_iter()
            .filter(|candidate| candidate == name)
            .collect(),
        None => names,
    };
    if container.is_none() {
        let mut running = false;
        for name in &targets {
            running |= crate::handler::deployments::inspect_container_state(&runtime, name).await
                == "running";
        }
        if !running {
            return Ok(None);
        }
    }
    for name in &targets {
        runtime
            .podman(&[OsString::from("restart"), OsString::from(name)])
            .await?;
    }
    Ok(Some(targets.len()))
}

pub(super) async fn apply_restart(
    state: &SharedState,
    actor: &AuthUser,
    stored: &StoredAction,
) -> Result<Applied, AppError> {
    let arguments: Restart = serde_json::from_str(&stored.arguments)
        .map_err(|error| AppError::InternalError(error.to_string()))?;
    let application_id = &stored.application_id;
    let container = arguments.container.as_deref();

    // A group of servers only resolves its runtime inside a replica scope.
    let is_group = crate::servers::replica::target_server_id(state, application_id)
        .await?
        .is_none();
    let mut restarted = 0;
    let mut idle = true;
    if is_group {
        let servers = crate::servers::groups::resolve_target_servers(state, application_id).await?;
        for server in &servers {
            let outcome = crate::servers::replica::for_replica(
                &server.id,
                restart_here(state, application_id, container),
            )
            .await?;
            if let Some(count) = outcome {
                restarted += count;
                idle = false;
            }
        }
    } else if let Some(count) = restart_here(state, application_id, container).await? {
        restarted = count;
        idle = false;
    }

    // Nothing was running, so there is nothing to restart: bring it up the
    // way a redeploy does.
    if idle {
        let job = deployment::enqueue_for_application(
            state,
            application_id,
            ASSISTANT_TRIGGER,
            Some(&actor.user_id),
        )
        .await?;
        return Ok(Applied {
            audit_action: "application.deployed",
            added: Some(format!("not running, redeploy queued as job {}", job.id)),
            removed: None,
        });
    }

    Ok(Applied {
        audit_action: "container.restarted",
        added: Some(format!("{restarted} container(s) restarted")),
        removed: None,
    })
}

/// Same service call the application's Maintenance card uses, so the route is
/// rebuilt, a failed route restores the old state, and an application on a
/// public-edge server is refused with the same message.
pub(super) async fn apply_maintenance(
    state: &SharedState,
    stored: &StoredAction,
) -> Result<Applied, AppError> {
    let arguments: SetMaintenance = serde_json::from_str(&stored.arguments)
        .map_err(|error| AppError::InternalError(error.to_string()))?;
    let maintenance = deployment::maintenance::update(
        state,
        &stored.application_id,
        crate::dtos::deployment::UpdateMaintenanceRequest {
            enabled: Some(arguments.enabled),
            page_html: arguments.page_html,
        },
    )
    .await?;

    Ok(Applied {
        audit_action: if maintenance.enabled {
            "maintenance.enabled"
        } else {
            "maintenance.disabled"
        },
        added: Some(
            if maintenance.enabled {
                "maintenance mode on"
            } else {
                "maintenance mode off"
            }
            .into(),
        ),
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

/// Creates and deploys a proposed static site. Refused before anything is
/// created when the agent has not uploaded yet; once something exists, a later
/// failure removes what this confirm created, so a half-built site never stays.
/// A card that names an existing application deploys into it instead.
pub(super) async fn apply_static_site(
    state: &SharedState,
    actor: &AuthUser,
    stored: &StoredAction,
) -> Result<Applied, AppError> {
    let site: StaticSite = serde_json::from_str(&stored.arguments)
        .map_err(|error| AppError::InternalError(error.to_string()))?;
    let Some((filename, bytes)) = super::upload::staged(state, &stored.id).await else {
        return Err(AppError::ValidationError(
            "The site has not been uploaded yet. The agent has to send it to the upload URL first."
                .into(),
        ));
    };

    if site.application_id.is_some() {
        return apply_static_site_existing(state, actor, stored, &site, filename, bytes).await;
    }

    let organization_id = &stored.organization_id;
    let created_project = stored.project_id.is_empty();
    let (project_id, environment_id) = ensure_project(state, stored, &site.project_name).await?;

    let built = async {
        let application = crate::cqrs::commands::project::create_application(
            state,
            organization_id,
            &project_id,
            &environment_id,
            &crate::dtos::project::CreateApplicationRequest {
                name: site.name.clone(),
                description: None,
                server_id: Some(site.server_id.clone()),
                server_group_id: None,
            },
        )
        .await?;
        match publish_static_site(state, actor, &application.id, &site, filename, bytes).await {
            Ok(url) => Ok((application, url)),
            Err(error) => {
                let _ = crate::cqrs::commands::project::delete_application(
                    state,
                    organization_id,
                    &project_id,
                    &environment_id,
                    &application.id,
                )
                .await;
                Err(error)
            }
        }
    }
    .await;

    let (application, url) = match built {
        Ok(built) => built,
        Err(error) => {
            if created_project {
                let _ = crate::cqrs::commands::project::delete_project(
                    state,
                    organization_id,
                    &project_id,
                )
                .await;
            }
            return Err(error);
        }
    };
    super::upload::discard(state, &stored.id).await;

    Ok(Applied {
        audit_action: "application.created",
        added: Some(match url {
            Some(url) => format!("{} deploying, {url}", application.name),
            None => format!("{} deploying ({})", application.name, application.id),
        }),
        removed: None,
    })
}

/// Deploys the site into the application the card named. Checked again here,
/// since it may have been deployed or reconfigured since the proposal. A
/// failure puts back the source it had, so an empty application stays empty.
async fn apply_static_site_existing(
    state: &SharedState,
    actor: &AuthUser,
    stored: &StoredAction,
    site: &StaticSite,
    filename: String,
    bytes: Vec<u8>,
) -> Result<Applied, AppError> {
    let target = ActionTarget {
        organization_id: stored.organization_id.clone(),
        project_id: stored.project_id.clone(),
        project_name: site.project_name.clone(),
        environment_id: stored.environment_id.clone(),
        application_id: String::new(),
        application_name: String::new(),
    };
    let found = super::propose::existing_static_target(state, &target, &site.name).await?;
    let Some((application_id, _)) =
        found.filter(|(id, _)| Some(id) == site.application_id.as_ref())
    else {
        return Err(AppError::ValidationError(format!(
            "`{}` changed since this was proposed. Ask the agent to propose it again.",
            site.name
        )));
    };

    let previous = deployment::source::get(state, &application_id).await?;
    let url = match publish_static_site(state, actor, &application_id, site, filename, bytes).await
    {
        Ok(url) => url,
        Err(error) => {
            if previous.deploy_type != "static" {
                let _ = deployment::source::save(state, &application_id, restore_source(previous))
                    .await;
            }
            return Err(error);
        }
    };
    super::upload::discard(state, &stored.id).await;

    Ok(Applied {
        audit_action: "application.deployed",
        added: Some(match url {
            Some(url) => format!("{} deploying, {url}", site.name),
            None => format!("{} deploying ({application_id})", site.name),
        }),
        removed: None,
    })
}

/// Makes `application_id` a static site with the uploaded files, serves it on
/// the card's host, and queues the deploy. Returns the host's URL.
async fn publish_static_site(
    state: &SharedState,
    actor: &AuthUser,
    application_id: &str,
    site: &StaticSite,
    filename: String,
    bytes: Vec<u8>,
) -> Result<Option<String>, AppError> {
    let current = deployment::source::get(state, application_id).await?;
    deployment::source::save(
        state,
        application_id,
        crate::dtos::deployment::UpdateSourceRequest {
            deploy_type: "static".into(),
            build_method: current.build_method,
            git_provider_id: None,
            git_url: None,
            git_branch: None,
            auto_deploy: Some(false),
            watch_paths: None,
            dockerfile_path: None,
            compose_file_path: None,
            compose_public_service: None,
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
    .await?;
    deployment::save_artifact(state, application_id, filename, bytes).await?;

    let url = ensure_primary_domain(
        state,
        application_id,
        site.host.as_deref(),
        super::propose::STATIC_PORT,
        site.https,
    )
    .await?;
    deployment::enqueue_for_application(
        state,
        application_id,
        ASSISTANT_TRIGGER,
        Some(&actor.user_id),
    )
    .await?;
    Ok(url)
}

/// Serves the application on `host` as its main URL, fixing up a domain that
/// already has that host instead of failing on the duplicate. Returns the URL.
async fn ensure_primary_domain(
    state: &SharedState,
    application_id: &str,
    host: Option<&str>,
    port: i64,
    https: Option<bool>,
) -> Result<Option<String>, AppError> {
    let Some(host) = host else {
        return Ok(None);
    };
    let existing = deployment::domains::list(state, application_id)
        .await?
        .into_iter()
        .find(|domain| domain.host.eq_ignore_ascii_case(host));
    let domain = match existing {
        Some(domain) if domain.port == port && Some(domain.https) == https && domain.is_primary => {
            domain
        }
        Some(domain) => {
            deployment::domains::update(
                state,
                application_id,
                &domain.id,
                crate::dtos::deployment::UpdateDomainRequest {
                    certificate: Default::default(),
                    host: None,
                    port: Some(port),
                    https,
                    is_primary: Some(true),
                    server_id: None,
                },
            )
            .await?
        }
        None => {
            deployment::domains::create(
                state,
                application_id,
                crate::dtos::deployment::CreateDomainRequest {
                    certificate: Default::default(),
                    host: host.to_owned(),
                    port,
                    https,
                    is_primary: Some(true),
                    server_id: None,
                },
            )
            .await?
        }
    };
    Ok(Some(domain.url))
}

/// The update that puts a source back the way `previous` read it. Secrets are
/// never read back, which is fine here: only an application with no
/// repository or image is ever converted, so it has none.
fn restore_source(
    previous: crate::dtos::deployment::SourceConfigDto,
) -> crate::dtos::deployment::UpdateSourceRequest {
    crate::dtos::deployment::UpdateSourceRequest {
        deploy_type: previous.deploy_type,
        build_method: previous.build_method,
        git_provider_id: previous.git_provider_id,
        git_url: previous.git_url,
        git_branch: previous.git_branch,
        auto_deploy: Some(previous.auto_deploy),
        watch_paths: previous.watch_paths,
        dockerfile_path: previous.dockerfile_path,
        compose_file_path: previous.compose_file_path,
        compose_public_service: previous.compose_public_service,
        container_port: previous.container_port,
        railpack_project_path: Some(previous.railpack_project_path),
        railpack_start_command: previous.railpack_start_command,
        build_cpu_cores: previous.build_cpu_cores,
        build_memory_mib: previous.build_memory_mib,
        build_swap_mib: previous.build_swap_mib,
        image_url: previous.image_url,
        registry_username: None,
        registry_password: None,
        restart_policy: Some(previous.restart_policy),
    }
}

/// The project and environment a create lands in: the stored ones, or a new
/// project (with its production environment) when the card said it creates one.
async fn ensure_project(
    state: &SharedState,
    stored: &StoredAction,
    project_name: &str,
) -> Result<(String, String), AppError> {
    if !stored.project_id.is_empty() {
        return Ok((stored.project_id.clone(), stored.environment_id.clone()));
    }
    let project = crate::cqrs::commands::project::create_project(
        state,
        &stored.organization_id,
        &crate::dtos::project::CreateProjectRequest {
            name: project_name.to_owned(),
            description: None,
        },
    )
    .await?;
    let detail =
        crate::cqrs::queries::project::get_project(state, &stored.organization_id, &project.id)
            .await?;
    let environment = detail
        .environments
        .iter()
        .find(|environment| environment.is_production)
        .or_else(|| detail.environments.first())
        .ok_or(AppError::EnvironmentNotFound)?;
    Ok((project.id, environment.id.clone()))
}

/// Creates an application from an internal or catalog template, then sets the
/// user's values, adds the domain and deploys. Whatever this confirm created is
/// removed again if a later step fails.
pub(super) async fn apply_from_template(
    state: &SharedState,
    actor: &AuthUser,
    stored: &StoredAction,
) -> Result<Applied, AppError> {
    let arguments: FromTemplate = serde_json::from_str(&stored.arguments)
        .map_err(|error| AppError::InternalError(error.to_string()))?;
    let organization_id = &stored.organization_id;
    let created_project = stored.project_id.is_empty();
    let (project_id, environment_id) =
        ensure_project(state, stored, &arguments.project_name).await?;

    let built = async {
        let application_id = create_from_template(
            state,
            organization_id,
            &project_id,
            &environment_id,
            &arguments,
        )
        .await?;
        let finished = async {
            if let Some(name) = arguments
                .name
                .as_ref()
                .filter(|_| !INTERNAL_TEMPLATES.contains(&arguments.template.as_str()))
            {
                crate::cqrs::commands::project::update_application(
                    state,
                    organization_id,
                    &project_id,
                    &environment_id,
                    &application_id,
                    &crate::dtos::project::UpdateApplicationRequest {
                        name: Some(name.clone()),
                        description: None,
                        icon_url: None,
                        server_id: None,
                        server_group_id: None,
                    },
                )
                .await?;
            }
            for (name, value) in &arguments.values {
                deployment::set_environment_variable(
                    state,
                    &application_id,
                    "runtime",
                    name,
                    value,
                )
                .await?;
            }
            let url = match (&arguments.host, arguments.port) {
                (Some(host), Some(port)) => Some(
                    deployment::domains::create(
                        state,
                        &application_id,
                        crate::dtos::deployment::CreateDomainRequest {
                            certificate: Default::default(),
                            host: host.clone(),
                            port,
                            https: arguments.https,
                            is_primary: Some(true),
                            server_id: None,
                        },
                    )
                    .await?
                    .url,
                ),
                _ => None,
            };
            deployment::enqueue_for_application(
                state,
                &application_id,
                ASSISTANT_TRIGGER,
                Some(&actor.user_id),
            )
            .await?;
            Ok::<_, AppError>(url)
        }
        .await;
        match finished {
            Ok(url) => Ok((application_id, url)),
            Err(error) => {
                let _ = crate::cqrs::commands::project::delete_application(
                    state,
                    organization_id,
                    &project_id,
                    &environment_id,
                    &application_id,
                )
                .await;
                Err(error)
            }
        }
    }
    .await;

    let (application_id, url) = match built {
        Ok(built) => built,
        Err(error) => {
            if created_project {
                let _ = crate::cqrs::commands::project::delete_project(
                    state,
                    organization_id,
                    &project_id,
                )
                .await;
            }
            return Err(error);
        }
    };

    Ok(Applied {
        audit_action: "application.created",
        added: Some(match url {
            Some(url) => format!("{} deploying, {url}", arguments.template),
            None => format!("{} deploying ({application_id})", arguments.template),
        }),
        removed: None,
    })
}

/// The template's own create, as the UI calls it. Returns the application id.
async fn create_from_template(
    state: &SharedState,
    organization_id: &str,
    project_id: &str,
    environment_id: &str,
    arguments: &FromTemplate,
) -> Result<String, AppError> {
    use crate::templates::internal_templates::{
        dragonflydb, keydb, mariadb, mongodb, postgres, sqlserver,
    };

    if !INTERNAL_TEMPLATES.contains(&arguments.template.as_str()) {
        return crate::templates::catalog_templates::create_application(
            state,
            organization_id,
            project_id,
            environment_id,
            &arguments.template,
            &arguments.server_id,
        )
        .await
        .map(|application| application.id);
    }

    let detail =
        crate::cqrs::queries::project::get_project(state, organization_id, project_id).await?;
    let environment_name = detail
        .environments
        .iter()
        .find(|environment| environment.id == environment_id)
        .map(|environment| environment.name.clone())
        .ok_or(AppError::EnvironmentNotFound)?;
    let project_name = detail.project.name;
    let public_host = crate::handler::templates::public_host(state);
    // Every database input shares these fields; the rest default, which is
    // what the UI's dialog does when nobody changes them. `create` previews
    // with this id, generating the passwords.
    let input = serde_json::json!({
        "application_name": arguments.name.clone().unwrap_or_else(|| arguments.template.clone()),
        "server_id": arguments.server_id,
        "template_instance_id": uuid::Uuid::now_v7().to_string(),
    });
    let parse = |error: serde_json::Error| AppError::InternalError(error.to_string());
    macro_rules! create {
        ($module:ident) => {
            $module::create(
                state,
                organization_id,
                project_id,
                environment_id,
                &project_name,
                &environment_name,
                &public_host,
                serde_json::from_value(input).map_err(parse)?,
            )
            .await
            .map(|created| created.application.id)
        };
    }
    match arguments.template.as_str() {
        "postgres" => create!(postgres),
        "mariadb" => create!(mariadb),
        "mongodb" => create!(mongodb),
        "dragonflydb" => create!(dragonflydb),
        "keydb" => create!(keydb),
        _ => create!(sqlserver),
    }
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
    super::upload::discard(state, action_id).await;
    super::lifecycle::audit(
        state,
        actor,
        &stored.audited(),
        "assistant.action_rejected",
        None,
    )
    .await;
    note(
        state,
        &stored,
        "rejected by the user, so do not propose it again",
    )
    .await;
    Ok(dto(&stored, "rejected", None))
}

/// Creates a compose application the way a catalog template does - compose
/// file, `.env` with its secrets generated now, domain - and deploys it.
/// Whatever this confirm created is removed again if a later step fails.
pub(super) async fn apply_deploy_compose(
    state: &SharedState,
    actor: &AuthUser,
    stored: &StoredAction,
) -> Result<Applied, AppError> {
    let arguments: ComposeApp = serde_json::from_str(&stored.arguments)
        .map_err(|error| AppError::InternalError(error.to_string()))?;
    let organization_id = &stored.organization_id;
    let created_project = stored.project_id.is_empty();
    let (project_id, environment_id) =
        ensure_project(state, stored, &arguments.project_name).await?;

    let built = async {
        let application = crate::cqrs::commands::project::create_application(
            state,
            organization_id,
            &project_id,
            &environment_id,
            &crate::dtos::project::CreateApplicationRequest {
                name: arguments.name.clone(),
                description: None,
                server_id: Some(arguments.server_id.clone()),
                server_group_id: None,
            },
        )
        .await?;
        let finished = async {
            write_compose(state, &application.id, &arguments.compose, arguments.port).await?;
            if !arguments.env.trim().is_empty() {
                let env = crate::templates::secrets::substitute_placeholder_secrets(&arguments.env);
                deployment::save_environment(state, &application.id, "runtime", &env).await?;
            }
            let url = match (&arguments.host, arguments.port) {
                (Some(host), Some(port)) => {
                    let domain = deployment::domains::create(
                        state,
                        &application.id,
                        crate::dtos::deployment::CreateDomainRequest {
                            certificate: Default::default(),
                            host: host.clone(),
                            port,
                            https: arguments.https,
                            is_primary: Some(true),
                            server_id: None,
                        },
                    )
                    .await?;
                    // The deploy that follows pushes every route to Caddy.
                    if let Some((service, container_port)) =
                        compose_route_target(state, &application.id, port).await?
                    {
                        crate::deployment::compose::proxy_routes::upsert(
                            state,
                            &application.id,
                            &domain.id,
                            &service,
                            container_port,
                        )
                        .await?;
                    }
                    Some(domain.url)
                }
                _ => None,
            };
            deployment::enqueue_for_application(
                state,
                &application.id,
                ASSISTANT_TRIGGER,
                Some(&actor.user_id),
            )
            .await?;
            Ok::<_, AppError>(url)
        }
        .await;
        match finished {
            Ok(url) => Ok((application.id, url)),
            Err(error) => {
                let _ = crate::cqrs::commands::project::delete_application(
                    state,
                    organization_id,
                    &project_id,
                    &environment_id,
                    &application.id,
                )
                .await;
                let _ = ComposeFileStore::new(&state.data_dir, &application.id)
                    .remove_all()
                    .await;
                Err(error)
            }
        }
    }
    .await;

    let (application_id, url) = match built {
        Ok(built) => built,
        Err(error) => {
            if created_project {
                let _ = crate::cqrs::commands::project::delete_project(
                    state,
                    organization_id,
                    &project_id,
                )
                .await;
            }
            return Err(error);
        }
    };

    Ok(Applied {
        audit_action: "application.created",
        added: Some(match url {
            Some(url) => format!("{} deploying, {url}", arguments.name),
            None => format!("{} deploying ({application_id})", arguments.name),
        }),
        removed: None,
    })
}

/// Creates and deploys a proposed application built from source: an uploaded
/// ZIP (`deploy_zip`) or a repository (`deploy_repository`). Like the static
/// site, nothing is created before the ZIP has arrived, and a later failure
/// removes what this confirm created or puts back the source it had.
pub(super) async fn apply_source_app(
    state: &SharedState,
    actor: &AuthUser,
    stored: &StoredAction,
) -> Result<Applied, AppError> {
    let app: SourceApp = serde_json::from_str(&stored.arguments)
        .map_err(|error| AppError::InternalError(error.to_string()))?;
    let zip = if stored.tool == DEPLOY_ZIP {
        let Some((filename, bytes)) = super::upload::staged(state, &stored.id).await else {
            return Err(AppError::ValidationError(
                "The ZIP has not been uploaded yet. The agent has to send it to the upload URL first."
                    .into(),
            ));
        };
        if filename != "site.zip" {
            return Err(AppError::ValidationError(
                "The upload is not a ZIP archive".into(),
            ));
        }
        Some(bytes)
    } else {
        None
    };

    let organization_id = &stored.organization_id;
    if let Some(application_id) = &app.application_id {
        let target = ActionTarget {
            organization_id: organization_id.clone(),
            project_id: stored.project_id.clone(),
            project_name: app.project_name.clone(),
            environment_id: stored.environment_id.clone(),
            application_id: String::new(),
            application_name: String::new(),
        };
        let found = super::propose::existing_source_target(state, &target, &app.name).await?;
        if found.as_ref() != Some(application_id) {
            return Err(AppError::ValidationError(format!(
                "`{}` changed since this was proposed. Ask the agent to propose it again.",
                app.name
            )));
        }
        let previous = deployment::source::get(state, application_id).await?;
        let url = match publish_source_app(state, actor, application_id, &app, zip).await {
            Ok(url) => url,
            Err(error) => {
                if !previous.source_upload {
                    let _ =
                        deployment::source::save(state, application_id, restore_source(previous))
                            .await;
                }
                return Err(error);
            }
        };
        super::upload::discard(state, &stored.id).await;
        return Ok(Applied {
            audit_action: "application.deployed",
            added: Some(match url {
                Some(url) => format!("{} deploying, {url}", app.name),
                None => format!("{} deploying ({application_id})", app.name),
            }),
            removed: None,
        });
    }

    let created_project = stored.project_id.is_empty();
    let (project_id, environment_id) = ensure_project(state, stored, &app.project_name).await?;
    let built = async {
        let application = crate::cqrs::commands::project::create_application(
            state,
            organization_id,
            &project_id,
            &environment_id,
            &crate::dtos::project::CreateApplicationRequest {
                name: app.name.clone(),
                description: None,
                server_id: Some(app.server_id.clone()),
                server_group_id: None,
            },
        )
        .await?;
        match publish_source_app(state, actor, &application.id, &app, zip).await {
            Ok(url) => Ok((application, url)),
            Err(error) => {
                let _ = crate::cqrs::commands::project::delete_application(
                    state,
                    organization_id,
                    &project_id,
                    &environment_id,
                    &application.id,
                )
                .await;
                Err(error)
            }
        }
    }
    .await;
    let (application, url) = match built {
        Ok(built) => built,
        Err(error) => {
            if created_project {
                let _ = crate::cqrs::commands::project::delete_project(
                    state,
                    organization_id,
                    &project_id,
                )
                .await;
            }
            return Err(error);
        }
    };
    super::upload::discard(state, &stored.id).await;

    Ok(Applied {
        audit_action: "application.created",
        added: Some(match url {
            Some(url) => format!("{} deploying, {url}", application.name),
            None => format!("{} deploying ({})", application.name, application.id),
        }),
        removed: None,
    })
}

/// Points `application_id` at the card's source (the ZIP, or the repository),
/// serves it on the card's host, and queues the deploy. Returns the host's URL.
async fn publish_source_app(
    state: &SharedState,
    actor: &AuthUser,
    application_id: &str,
    app: &SourceApp,
    zip: Option<Vec<u8>>,
) -> Result<Option<String>, AppError> {
    if let Some(bytes) = &zip {
        deployment::source_archive::store(state, application_id, bytes).await?;
    }
    let current = deployment::source::get(state, application_id).await?;
    deployment::source::save(
        state,
        application_id,
        crate::dtos::deployment::UpdateSourceRequest {
            deploy_type: "git".into(),
            build_method: app.build_method.clone(),
            git_provider_id: app.git_provider_id.clone(),
            git_url: app.git_url.clone(),
            git_branch: app.git_branch.clone(),
            auto_deploy: Some(false),
            watch_paths: None,
            dockerfile_path: app.dockerfile_path.clone(),
            compose_file_path: None,
            compose_public_service: None,
            container_port: Some(app.container_port),
            railpack_project_path: app.project_path.clone(),
            railpack_start_command: None,
            build_cpu_cores: current.build_cpu_cores,
            build_memory_mib: current.build_memory_mib,
            build_swap_mib: current.build_swap_mib,
            image_url: None,
            registry_username: None,
            registry_password: None,
            restart_policy: None,
        },
    )
    .await?;
    let url = ensure_primary_domain(
        state,
        application_id,
        app.host.as_deref(),
        app.container_port,
        app.https,
    )
    .await?;
    deployment::enqueue_for_application(
        state,
        application_id,
        ASSISTANT_TRIGGER,
        Some(&actor.user_id),
    )
    .await?;
    Ok(url)
}

pub(super) async fn apply_link_database(
    state: &SharedState,
    stored: &StoredAction,
) -> Result<Applied, AppError> {
    let arguments: LinkDatabase = serde_json::from_str(&stored.arguments)
        .map_err(|error| AppError::InternalError(error.to_string()))?;

    crate::deployment::resource_networks::attach(
        state,
        &stored.application_id,
        &arguments.database_id,
    )
    .await?;
    let connection = crate::templates::internal_templates::connection::private_connection_string(
        state,
        &stored.organization_id,
        &stored.project_id,
        &stored.environment_id,
        &arguments.database_id,
        &arguments.database_kind,
    )
    .await?;
    deployment::set_environment_variable(
        state,
        &stored.application_id,
        &arguments.scope,
        &arguments.variable,
        &connection,
    )
    .await?;

    Ok(Applied {
        audit_action: "network.attached",
        added: Some(format!("{}={}", arguments.variable, mask(&connection))),
        removed: None,
    })
}
