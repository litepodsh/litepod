use crate::{
    deployment::{self, compose::store::ComposeFileStore},
    dtos::response::AppError,
    security::middleware::auth::AuthUser,
    state::state::SharedState,
};

use super::lifecycle::*;
use super::types::*;

/// Validates the model's arguments and stores the change without applying it.
pub async fn propose_environment_variable(
    state: &SharedState,
    conversation_id: &str,
    actor: &AuthUser,
    target: &ActionTarget,
    arguments: SetEnvironmentVariable,
) -> Result<ActionDto, AppError> {
    if !deployment::is_environment_name(&arguments.name) {
        return Err(AppError::ValidationError(format!(
            "`{}` is not a valid environment variable name",
            arguments.name
        )));
    }
    if !matches!(arguments.scope.as_str(), "runtime" | "build") {
        return Err(AppError::ValidationError(
            "Environment scope must be runtime or build".into(),
        ));
    }
    if arguments.value.is_empty() {
        return Err(AppError::ValidationError(
            "An environment variable needs a value".into(),
        ));
    }
    if arguments.value.len() > MAX_VALUE_BYTES {
        return Err(AppError::ValidationError(format!(
            "Environment values are limited to {MAX_VALUE_BYTES} bytes"
        )));
    }

    // Existing names only, never values: the summary must not become a way to
    // read a secret back out through the assistant.
    let existing = deployment::environment(state, &target.application_id, &arguments.scope, false)
        .await?
        .into_iter()
        .any(|variable| variable.name == arguments.name);

    let title = if existing {
        format!("Update {} on {}", arguments.name, target.application_name)
    } else {
        format!("Add {} to {}", arguments.name, target.application_name)
    };
    let summary = if existing {
        format!(
            "Replaces the current {} value in the {} environment of {}. Nothing deploys yet.",
            arguments.name, arguments.scope, target.application_name
        )
    } else {
        format!(
            "Adds one variable to the {} environment of {}. Nothing deploys yet.",
            arguments.scope, target.application_name
        )
    };

    let encoded = serde_json::to_string(&arguments)
        .map_err(|error| AppError::InternalError(error.to_string()))?;

    let stored = store_action(
        state,
        conversation_id,
        actor,
        target,
        SET_ENVIRONMENT_VARIABLE,
        &encoded,
        title,
        summary,
    )
    .await?;

    Ok(ActionDto {
        added: Some(format!("{}={}", arguments.name, mask(&arguments.value))),
        replaces: existing.then(|| arguments.name.clone()),
        ..stored
    })
}

/// Stores a rename or a new description without applying either.
pub async fn propose_update_application(
    state: &SharedState,
    conversation_id: &str,
    actor: &AuthUser,
    target: &ActionTarget,
    arguments: UpdateApplication,
) -> Result<ActionDto, AppError> {
    let name = arguments
        .name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty());
    let description = arguments.description.as_deref().map(str::trim);
    if name.is_none() && description.is_none() {
        return Err(AppError::ValidationError(
            "Nothing to change: pass a name, a description, or both".into(),
        ));
    }
    if name.is_some_and(|name| name.chars().count() > 64) {
        return Err(AppError::ValidationError(
            "An application name is limited to 64 characters".into(),
        ));
    }

    let title = match name {
        Some(name) => format!("Rename {} to {name}", target.application_name),
        None => format!("Change the description of {}", target.application_name),
    };
    let summary = arguments
        .reason
        .clone()
        .unwrap_or_else(|| match (name, description) {
            (Some(_), Some(_)) => "Renames the application and replaces its description.".into(),
            (Some(_), None) => "Renames the application. Nothing is redeployed.".into(),
            _ => "Replaces the application's description. Nothing is redeployed.".into(),
        });

    let encoded = serde_json::to_string(&arguments)
        .map_err(|error| AppError::InternalError(error.to_string()))?;
    let stored = store_action(
        state,
        conversation_id,
        actor,
        target,
        UPDATE_APPLICATION,
        &encoded,
        title,
        summary,
    )
    .await?;

    Ok(ActionDto {
        added: name.map(|name| format!("name = {name}")),
        ..stored
    })
}

/// Stores a source change without applying it.
///
/// The current configuration is read here so the card can show the fields that
/// actually move, and so a change to an application with no source at all is
/// refused now rather than at confirm time.
pub async fn propose_source(
    state: &SharedState,
    conversation_id: &str,
    actor: &AuthUser,
    target: &ActionTarget,
    mut arguments: SetSource,
) -> Result<ActionDto, AppError> {
    let current = deployment::source::get(state, &target.application_id).await?;

    // A repository without a branch deploys from `main` unless the user picked
    // another one. Filled in here so the card states it rather than leaving it
    // to whatever the deploy step would have guessed.
    if arguments.git_url.is_some() && arguments.git_branch.is_none() && current.git_branch.is_none()
    {
        arguments.git_branch = Some(DEFAULT_BRANCH.to_owned());
    }
    if arguments.git_branch.as_deref().is_some_and(str::is_empty) {
        arguments.git_branch = Some(DEFAULT_BRANCH.to_owned());
    }

    let mut changes = Vec::new();
    let mut note = |label: &str, before: Option<&str>, after: Option<&str>| {
        if let Some(after) = after
            && before != Some(after)
        {
            changes.push(format!("{label} = {after}"));
        }
    };
    note(
        "provider",
        current.git_provider_id.as_deref(),
        arguments.git_provider_id.as_deref(),
    );
    note(
        "repo",
        current.git_url.as_deref(),
        arguments.git_url.as_deref(),
    );
    note(
        "branch",
        current.git_branch.as_deref(),
        arguments.git_branch.as_deref(),
    );
    note(
        "root path",
        Some(current.railpack_project_path.as_str()),
        arguments.railpack_project_path.as_deref(),
    );
    note(
        "watch paths",
        current.watch_paths.as_deref(),
        arguments.watch_paths.as_deref(),
    );
    if let Some(auto_deploy) = arguments.auto_deploy
        && auto_deploy != current.auto_deploy
    {
        changes.push(format!("auto deploy = {auto_deploy}"));
    }

    if changes.is_empty() {
        return Err(AppError::ValidationError(
            "That is the source configuration it already has".into(),
        ));
    }

    let title = format!("Update the source of {}", target.application_name);
    let summary = arguments.reason.clone().unwrap_or_else(|| {
        format!(
            "Changes how {} is fetched and built. It takes effect on the next deployment.",
            target.application_name
        )
    });
    let encoded = serde_json::to_string(&arguments)
        .map_err(|error| AppError::InternalError(error.to_string()))?;

    let stored = store_action(
        state,
        conversation_id,
        actor,
        target,
        SET_SOURCE,
        &encoded,
        title,
        summary,
    )
    .await?;

    Ok(ActionDto {
        added: Some(changes.join(", ")),
        ..stored
    })
}

/// Stores the removal of one variable without removing it.
///
/// The variable has to exist now: proposing to delete something that is not
/// there produces a card the user cannot make sense of.
pub async fn propose_delete_environment_variable(
    state: &SharedState,
    conversation_id: &str,
    actor: &AuthUser,
    target: &ActionTarget,
    arguments: SetEnvironmentVariable,
) -> Result<ActionDto, AppError> {
    if !matches!(arguments.scope.as_str(), "runtime" | "build") {
        return Err(AppError::ValidationError(
            "Environment scope must be runtime or build".into(),
        ));
    }
    let exists = deployment::environment(state, &target.application_id, &arguments.scope, false)
        .await?
        .into_iter()
        .any(|variable| variable.name == arguments.name);
    if !exists {
        return Err(AppError::ValidationError(format!(
            "`{}` does not set `{}` in its {} environment",
            target.application_name, arguments.name, arguments.scope
        )));
    }

    let title = format!("Remove {} from {}", arguments.name, target.application_name);
    let summary = match &arguments.reason {
        Some(reason) => reason.clone(),
        None => format!(
            "Removes {} from the {} environment of {}. It stops reaching the container on the next deployment.",
            arguments.name, arguments.scope, target.application_name
        ),
    };
    let encoded = serde_json::to_string(&arguments)
        .map_err(|error| AppError::InternalError(error.to_string()))?;

    let stored = store_action(
        state,
        conversation_id,
        actor,
        target,
        DELETE_ENVIRONMENT_VARIABLE,
        &encoded,
        title,
        summary,
    )
    .await?;

    Ok(ActionDto {
        removed: Some(arguments.name.clone()),
        ..stored
    })
}

/// Stores a redeploy without running it.
///
/// There is nothing to validate beyond the application existing, which
/// resolving it already proved, so this is only the confirm gate. Queueing is
/// the whole effect, and it happens on confirm.
pub async fn propose_redeploy(
    state: &SharedState,
    conversation_id: &str,
    actor: &AuthUser,
    target: &ActionTarget,
    reason: Option<String>,
) -> Result<ActionDto, AppError> {
    let title = format!("Redeploy {}", target.application_name);
    let summary = match reason {
        Some(reason) => format!(
            "Queues a new deployment of {} from its current source and environment. {reason}",
            target.application_name
        ),
        None => format!(
            "Queues a new deployment of {} from its current source and environment.",
            target.application_name
        ),
    };
    store_action(
        state,
        conversation_id,
        actor,
        target,
        REDEPLOY_APPLICATION,
        "{}",
        title,
        summary,
    )
    .await
    .map(|action| ActionDto {
        added: None,
        ..action
    })
}

/// Stores a start or a stop without running it. Like a redeploy there is
/// nothing to validate beyond the application existing; the effect happens on
/// confirm.
pub async fn propose_lifecycle(
    state: &SharedState,
    conversation_id: &str,
    actor: &AuthUser,
    target: &ActionTarget,
    tool: &'static str,
    reason: Option<String>,
) -> Result<ActionDto, AppError> {
    let (verb, effect) = match tool {
        START_APPLICATION => ("Start", "Starts the containers of"),
        STOP_APPLICATION => ("Stop", "Stops the containers of"),
        other => {
            return Err(AppError::ValidationError(format!(
                "`{other}` is not a start or a stop"
            )));
        }
    };
    let title = format!("{verb} {}", target.application_name);
    let summary = match reason {
        Some(reason) => format!("{effect} {}. {reason}", target.application_name),
        None => format!("{effect} {}.", target.application_name),
    };
    store_action(state, conversation_id, actor, target, tool, "{}", title, summary)
        .await
        .map(|action| ActionDto {
            added: None,
            ..action
        })
}

/// Stores a replacement compose file without writing it.
///
/// The current file is read here so the card can say how the shape changes,
/// and so an application with no compose file is refused now rather than at
/// confirm time.
pub async fn propose_compose_file(
    state: &SharedState,
    conversation_id: &str,
    actor: &AuthUser,
    target: &ActionTarget,
    arguments: SetComposeFile,
) -> Result<ActionDto, AppError> {
    if arguments.compose.trim().is_empty() {
        return Err(AppError::ValidationError(
            "An empty compose file would take the application down".into(),
        ));
    }
    if arguments.compose.len() > MAX_COMPOSE_BYTES {
        return Err(AppError::ValidationError(format!(
            "Compose files are limited to {MAX_COMPOSE_BYTES} bytes"
        )));
    }

    let store = ComposeFileStore::new(&state.data_dir, &target.application_id);
    let current = store.read_content().await.map_err(|error| {
        AppError::ValidationError(format!(
            "`{}` has no compose file to replace: {error}",
            target.application_name
        ))
    })?;
    if current.trim() == arguments.compose.trim() {
        return Err(AppError::ValidationError(
            "That is the file it already has".into(),
        ));
    }

    let title = format!("Replace the compose file of {}", target.application_name);
    let summary = match &arguments.reason {
        Some(reason) => reason.clone(),
        None => format!(
            "Rewrites the compose file of {}. It takes effect on the next deployment.",
            target.application_name
        ),
    };
    let encoded = serde_json::to_string(&arguments)
        .map_err(|error| AppError::InternalError(error.to_string()))?;

    let stored = store_action(
        state,
        conversation_id,
        actor,
        target,
        SET_COMPOSE_FILE,
        &encoded,
        title,
        summary,
    )
    .await?;

    Ok(ActionDto {
        // The whole file is far too much for a card; the shape of the change is
        // what the user needs before opening it.
        added: Some(format!(
            "compose file, {} lines -> {} lines",
            current.lines().count(),
            arguments.compose.lines().count()
        )),
        ..stored
    })
}

/// Stores a new application without creating it.
///
/// The compose file is parsed here rather than at confirm time: a `port` that
/// no service publishes means the application would come up with no public
/// URL, and that is worth refusing while it is still a proposal.
pub async fn propose_create_application(
    state: &SharedState,
    conversation_id: &str,
    actor: &AuthUser,
    target: &ActionTarget,
    arguments: CreateApplication,
) -> Result<ActionDto, AppError> {
    let name = arguments.name.trim().to_owned();
    if name.is_empty() {
        return Err(AppError::ValidationError(
            "An application needs a name".into(),
        ));
    }
    if name.chars().count() > 64 {
        return Err(AppError::ValidationError(
            "An application name is limited to 64 characters".into(),
        ));
    }

    let detail = crate::cqrs::queries::project::get_project(
        state,
        &target.organization_id,
        &target.project_id,
    )
    .await?;
    let environment = detail
        .environments
        .iter()
        .find(|environment| environment.id == target.environment_id)
        .ok_or(AppError::EnvironmentNotFound)?;
    if environment
        .applications
        .iter()
        .any(|application| application.name.eq_ignore_ascii_case(&name))
    {
        return Err(AppError::ValidationError(format!(
            "`{}` already has an application called `{name}`",
            environment.name
        )));
    }

    let service = match (&arguments.compose, arguments.port) {
        (None, _) => None,
        (Some(compose), port) => {
            if compose.trim().is_empty() {
                return Err(AppError::ValidationError(
                    "An empty compose file creates nothing".into(),
                ));
            }
            if compose.len() > MAX_COMPOSE_BYTES {
                return Err(AppError::ValidationError(format!(
                    "Compose files are limited to {MAX_COMPOSE_BYTES} bytes"
                )));
            }
            match port {
                None => None,
                Some(port) => {
                    let port = u16::try_from(port).map_err(|_| {
                        AppError::ValidationError("Port must be between 1 and 65535".into())
                    })?;
                    Some(
                        crate::deployment::compose_standalone::service_for_port(compose, port)
                            .ok_or_else(|| {
                                AppError::ValidationError(format!(
                                    "No compose service publishes port {port}, so nothing could be routed to it"
                                ))
                            })?,
                    )
                }
            }
        }
    };

    let title = format!(
        "Create {name} in {}/{}",
        detail.project.name, environment.name
    );
    let summary = arguments.reason.clone().unwrap_or_else(|| match &service {
        Some(service) => format!(
            "Creates a compose application in {}/{}, routed to the `{service}` service. Nothing is deployed yet.",
            detail.project.name, environment.name
        ),
        None if arguments.compose.is_some() => format!(
            "Creates a compose application in {}/{}. Nothing is deployed yet.",
            detail.project.name, environment.name
        ),
        None => format!(
            "Creates an empty application in {}/{}. Its source still has to be configured.",
            detail.project.name, environment.name
        ),
    });

    let encoded = serde_json::to_string(&CreateApplication {
        name: name.clone(),
        ..arguments.clone()
    })
    .map_err(|error| AppError::InternalError(error.to_string()))?;
    let stored = store_action(
        state,
        conversation_id,
        actor,
        target,
        CREATE_APPLICATION,
        &encoded,
        title,
        summary,
    )
    .await?;

    Ok(ActionDto {
        added: Some(match (&arguments.compose, &service) {
            (Some(compose), Some(service)) => format!(
                "{name}, compose ({} lines), routed to {service}",
                compose.lines().count()
            ),
            (Some(compose), None) => {
                format!("{name}, compose ({} lines)", compose.lines().count())
            }
            _ => name.clone(),
        }),
        ..stored
    })
}

/// Stores a new project without creating it.
///
/// Only the name is required: a project is created with a `prod` environment
/// and nothing else, so there is no source or deployment to get wrong here.
pub async fn propose_create_project(
    state: &SharedState,
    conversation_id: &str,
    actor: &AuthUser,
    target: &ActionTarget,
    arguments: ProjectFields,
) -> Result<ActionDto, AppError> {
    let name = project_name(arguments.name.as_deref())?;

    // A second project with the same name is legal but almost never meant, so
    // the card says it rather than letting the user find out afterwards.
    let taken = crate::cqrs::queries::project::list_projects(state, &target.organization_id)
        .await?
        .into_iter()
        .any(|project| project.name.eq_ignore_ascii_case(&name));

    let title = format!("Create the project {name}");
    let summary = arguments.reason.clone().unwrap_or_else(|| {
        if taken {
            format!("Creates a second project called {name}, with a `prod` environment. Nothing is deployed.")
        } else {
            format!("Creates the project {name} with a `prod` environment. Nothing is deployed.")
        }
    });

    let encoded = serde_json::to_string(&ProjectFields {
        name: Some(name.clone()),
        ..arguments
    })
    .map_err(|error| AppError::InternalError(error.to_string()))?;
    let stored = store_action(
        state,
        conversation_id,
        actor,
        target,
        CREATE_PROJECT,
        &encoded,
        title,
        summary,
    )
    .await?;

    Ok(ActionDto {
        added: Some(format!("project {name}")),
        ..stored
    })
}

/// Stores a rename or a new description of an existing project.
pub async fn propose_update_project(
    state: &SharedState,
    conversation_id: &str,
    actor: &AuthUser,
    target: &ActionTarget,
    arguments: ProjectFields,
) -> Result<ActionDto, AppError> {
    let name = match arguments.name.as_deref() {
        Some(name) => Some(project_name(Some(name))?),
        None => None,
    };
    let description = arguments.description.as_deref().map(str::trim);
    if name.is_none() && description.is_none() {
        return Err(AppError::ValidationError(
            "Nothing to change: pass a name, a description, or both".into(),
        ));
    }

    let title = match &name {
        Some(name) => format!("Rename {} to {name}", target.project_name),
        None => format!("Change the description of {}", target.project_name),
    };
    let summary = arguments
        .reason
        .clone()
        .unwrap_or_else(|| match (&name, description) {
            (Some(_), Some(_)) => "Renames the project and replaces its description.".into(),
            (Some(_), None) => "Renames the project. Nothing is redeployed.".into(),
            _ => "Replaces the project's description. Nothing is redeployed.".into(),
        });

    let encoded = serde_json::to_string(&ProjectFields {
        name: name.clone(),
        ..arguments
    })
    .map_err(|error| AppError::InternalError(error.to_string()))?;
    let stored = store_action(
        state,
        conversation_id,
        actor,
        target,
        UPDATE_PROJECT,
        &encoded,
        title,
        summary,
    )
    .await?;

    Ok(ActionDto {
        added: name.map(|name| format!("name = {name}")),
        ..stored
    })
}

/// Stores the removal of a project without removing it.
///
/// What the project holds is counted now, so the card says how much goes with
/// it. This is the one action whose confirm is not recoverable.
pub async fn propose_delete_project(
    state: &SharedState,
    conversation_id: &str,
    actor: &AuthUser,
    target: &ActionTarget,
    arguments: ProjectFields,
) -> Result<ActionDto, AppError> {
    let detail = crate::cqrs::queries::project::get_project(
        state,
        &target.organization_id,
        &target.project_id,
    )
    .await?;
    let applications: usize = detail
        .environments
        .iter()
        .map(|environment| environment.applications.len())
        .sum();

    let title = format!("Delete the project {}", target.project_name);
    let summary = format!(
        "Deletes {} with its {} environment(s) and {} application(s), including their containers, \
domains and deployment history. This cannot be undone.{}",
        target.project_name,
        detail.environments.len(),
        applications,
        match &arguments.reason {
            Some(reason) => format!(" {reason}"),
            None => String::new(),
        }
    );

    let encoded = serde_json::to_string(&arguments)
        .map_err(|error| AppError::InternalError(error.to_string()))?;
    let stored = store_action(
        state,
        conversation_id,
        actor,
        target,
        DELETE_PROJECT,
        &encoded,
        title,
        summary,
    )
    .await?;

    Ok(ActionDto {
        removed: Some(format!(
            "{} ({applications} application(s))",
            target.project_name
        )),
        ..stored
    })
}

/// A project name, trimmed and bounded. Same ceiling as an application name.
pub(super) fn project_name(name: Option<&str>) -> Result<String, AppError> {
    let name = name.map(str::trim).unwrap_or_default();
    if name.is_empty() {
        return Err(AppError::ValidationError("A project needs a name".into()));
    }
    if name.chars().count() > 64 {
        return Err(AppError::ValidationError(
            "A project name is limited to 64 characters".into(),
        ));
    }
    Ok(name.to_owned())
}

/// The domain the model named, by id or by host. Resolved at propose time so
/// the confirm replays an id rather than a name that may have moved.
pub(super) async fn find_domain(
    state: &SharedState,
    target: &ActionTarget,
    wanted: Option<&str>,
) -> Result<crate::dtos::deployment::DomainDto, AppError> {
    let wanted = wanted.map(str::trim).unwrap_or_default();
    if wanted.is_empty() {
        return Err(AppError::ValidationError(
            "Which domain? Pass the host or its id".into(),
        ));
    }
    let domains = deployment::domains::list(state, &target.application_id).await?;
    domains
        .into_iter()
        .find(|domain| domain.id == wanted || domain.host.eq_ignore_ascii_case(wanted))
        .ok_or_else(|| {
            AppError::ValidationError(format!(
                "`{}` has no domain called `{wanted}`",
                target.application_name
            ))
        })
}

/// Stores a new domain without routing anything to it.
pub async fn propose_add_domain(
    state: &SharedState,
    conversation_id: &str,
    actor: &AuthUser,
    target: &ActionTarget,
    arguments: DomainFields,
) -> Result<ActionDto, AppError> {
    let host = arguments
        .host
        .as_deref()
        .map(|host| host.trim().to_ascii_lowercase())
        .unwrap_or_default();
    if host.is_empty() {
        return Err(AppError::ValidationError("A domain needs a host".into()));
    }
    let Some(port) = arguments.port else {
        return Err(AppError::ValidationError(
            "A domain needs a port: the container port it fronts".into(),
        ));
    };
    let https = arguments
        .https
        .unwrap_or(!deployment::domains::is_local(&host));
    // The same validation the dialog runs, so a bad host or a reserved port is
    // refused while it is still a proposal.
    deployment::domains::validate(state, &host, port, https)?;

    let existing = deployment::domains::list(state, &target.application_id).await?;
    if existing
        .iter()
        .any(|domain| domain.host.eq_ignore_ascii_case(&host) && domain.port == port)
    {
        return Err(AppError::ValidationError(format!(
            "`{}` already serves {host} on port {port}",
            target.application_name
        )));
    }
    // The first domain is primary whether or not anyone asks, which is what
    // `domains::create` does; saying so keeps the card honest.
    let is_primary = arguments.is_primary.unwrap_or(false) || existing.is_empty();

    let title = format!("Add {host} to {}", target.application_name);
    let summary = arguments.reason.clone().unwrap_or_else(|| {
        format!(
            "Routes {host} to port {port} of {}. It takes effect as soon as it is confirmed.",
            target.application_name
        )
    });

    let encoded = serde_json::to_string(&DomainFields {
        host: Some(host.clone()),
        port: Some(port),
        https: Some(https),
        is_primary: Some(is_primary),
        ..arguments
    })
    .map_err(|error| AppError::InternalError(error.to_string()))?;
    let stored = store_action(
        state,
        conversation_id,
        actor,
        target,
        ADD_DOMAIN,
        &encoded,
        title,
        summary,
    )
    .await?;

    Ok(ActionDto {
        added: Some(format!(
            "{}://{host} -> port {port}{}",
            if https { "https" } else { "http" },
            if is_primary { ", primary" } else { "" }
        )),
        ..stored
    })
}

/// Stores a change to an existing domain. Only the fields present move; the
/// rest are read back at confirm time from the row as it is then.
pub async fn propose_update_domain(
    state: &SharedState,
    conversation_id: &str,
    actor: &AuthUser,
    target: &ActionTarget,
    mut arguments: DomainFields,
) -> Result<ActionDto, AppError> {
    let current = find_domain(state, target, arguments.domain_id.as_deref()).await?;
    arguments.domain_id = Some(current.id.clone());

    let host = arguments
        .host
        .as_deref()
        .map(|host| host.trim().to_ascii_lowercase());
    let port = arguments.port.unwrap_or(current.port);
    let https = arguments.https.unwrap_or(current.https);
    deployment::domains::validate(state, host.as_deref().unwrap_or(&current.host), port, https)?;

    let mut changes = Vec::new();
    if let Some(host) = &host
        && *host != current.host
    {
        changes.push(format!("host = {host}"));
    }
    if port != current.port {
        changes.push(format!("port = {port}"));
    }
    if https != current.https {
        changes.push(format!("https = {https}"));
    }
    if arguments.is_primary == Some(true) && !current.is_primary {
        changes.push("primary".into());
    }
    if changes.is_empty() {
        return Err(AppError::ValidationError(
            "That is the domain it already has".into(),
        ));
    }

    let title = format!("Update {} on {}", current.host, target.application_name);
    let summary = arguments.reason.clone().unwrap_or_else(|| {
        format!(
            "Changes how {} is served for {}. It takes effect as soon as it is confirmed.",
            current.host, target.application_name
        )
    });

    let encoded = serde_json::to_string(&DomainFields {
        host,
        port: Some(port),
        https: Some(https),
        ..arguments
    })
    .map_err(|error| AppError::InternalError(error.to_string()))?;
    let stored = store_action(
        state,
        conversation_id,
        actor,
        target,
        UPDATE_DOMAIN,
        &encoded,
        title,
        summary,
    )
    .await?;

    Ok(ActionDto {
        added: Some(changes.join(", ")),
        ..stored
    })
}

/// Stores the removal of a domain without removing its route.
pub async fn propose_delete_domain(
    state: &SharedState,
    conversation_id: &str,
    actor: &AuthUser,
    target: &ActionTarget,
    mut arguments: DomainFields,
) -> Result<ActionDto, AppError> {
    let current = find_domain(state, target, arguments.domain_id.as_deref()).await?;
    arguments.domain_id = Some(current.id.clone());

    let title = format!("Remove {} from {}", current.host, target.application_name);
    let summary = arguments.reason.clone().unwrap_or_else(|| {
        format!(
            "Stops serving {} for {}. Anything pointing at it stops resolving to this application.{}",
            current.host,
            target.application_name,
            if current.is_primary {
                " It is the primary domain, so another one takes over."
            } else {
                ""
            }
        )
    });

    let encoded = serde_json::to_string(&arguments)
        .map_err(|error| AppError::InternalError(error.to_string()))?;
    let stored = store_action(
        state,
        conversation_id,
        actor,
        target,
        DELETE_DOMAIN,
        &encoded,
        title,
        summary,
    )
    .await?;

    Ok(ActionDto {
        removed: Some(current.url.clone()),
        ..stored
    })
}
