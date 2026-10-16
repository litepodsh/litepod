//! Two tools: `get` reads, `exec` proposes a change.
//!
//! A capability is a value of `resource` or `action`, never a new tool and
//! never a new endpoint. A model that has learned `get` has learned every read
//! in the product, and the one thing it must understand about `exec` - that it
//! proposes rather than does - is stated once instead of once per tool.
//!
//! Reads run against the same query layer the UI uses, always scoped to the
//! organization the caller is already authorised for. Values of environment
//! variables are never returned, so a secret cannot be read back out through a
//! conversation, and a value cannot smuggle a directive into the next turn.
//!
//! `exec` writes nothing. It validates and stores an action for a person to
//! confirm; see `actions`.

use serde::Serialize;
use serde_json::json;

pub mod wrappers;

use crate::{
    assistant::{
        actions::{self, ActionTarget, SetComposeFile, SetEnvironmentVariable},
        capabilities, redaction,
    },
    cqrs::queries::{organization as organization_queries, project as project_queries},
    deployment::{self, compose::store::ComposeFileStore},
    security::middleware::auth::AuthUser,
    servers::runtime::ServerRuntime,
    state::state::SharedState,
};

pub const GET: &str = "get";
pub const EXEC: &str = "exec";

/// The provider-neutral definition Rig turns into a dynamic tool.
#[derive(Debug, Clone, Serialize)]
pub struct ToolDefinition {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub function: FunctionDefinition,
}

#[derive(Debug, Clone, Serialize)]
pub struct FunctionDefinition {
    pub name: &'static str,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// Log lines handed to the model per call. Enough to see a failure, small
/// enough that one noisy build does not crowd out the conversation.
const LOG_TAIL: usize = 200;

/// How many suggestions are worth offering. More than a handful stops being a
/// choice and starts being the list the user was trying to avoid reading.
const MAX_CANDIDATES: usize = 6;

/// What each `get` resource returns, as the model reads it.
const RESOURCE_HELP: &[(&str, &str)] = &[
    (
        "organizations",
        "`organizations` (the ones this user belongs to, with the current one marked; \
        you can only read and change the current one)",
    ),
    ("projects", "`projects` (no other argument)"),
    ("applications", "`applications` (needs `project`)"),
    (
        "environment",
        "`environment` (variable NAMES only, never values; needs `application`)",
    ),
    (
        "deployments",
        "`deployments` (the last runs, newest first; needs `application`)",
    ),
    (
        "deployment_logs",
        "`deployment_logs` (the tail of one run; needs `application`, optionally `run` and `level`)",
    ),
    (
        "containers",
        "`containers` (live container state from the runtime; needs `application`)",
    ),
    (
        "container_logs",
        "`container_logs` (what the containers themselves printed; needs `application`, optionally one `container` name. \
        This is not `deployment_logs`, which is litepod's own record of the deploy. A container that ran once and \
        exited still has its output, so read this before saying there is nothing to show)",
    ),
    (
        "source",
        "`source` (how an application is fetched and built: provider, repo, branch, root path, auto deploy; needs `application`)",
    ),
    (
        "domains",
        "`domains` (the hosts an application is served on, with their ids; needs `application`)",
    ),
    (
        "servers",
        "`servers` (the ready servers a new application can be placed on)",
    ),
    (
        "git_providers",
        "`git_providers` (the providers this organization has, for the ids `set_source` needs)",
    ),
    (
        "compose",
        "`compose` (the compose file; needs `application`, and only compose applications have one)",
    ),
];

/// What each `exec` action proposes, as the model reads it.
const ACTION_HELP: &[(&str, &str)] = &[
    (
        "set_environment_variable",
        "`set_environment_variable` (needs `name` and `value`, optionally `scope`)",
    ),
    (
        "delete_environment_variable",
        "`delete_environment_variable` (needs `name`, optionally `scope`)",
    ),
    ("redeploy", "`redeploy` (nothing else)"),
    ("start", "`start` (nothing else; starts a stopped application)"),
    ("stop", "`stop` (nothing else; stops the application's containers)"),
    (
        "set_compose_file",
        "`set_compose_file` (needs the complete new `compose`; send the whole file, never a fragment)",
    ),
    (
        "update_application",
        "`update_application` (rename or re-describe: `name`, `description`, or both)",
    ),
    (
        "set_source",
        "`set_source` (how it is fetched and built: `git_provider_id`, `git_url`, `git_branch`, \
        `railpack_project_path`, `auto_deploy`, `watch_paths` - send only the fields that change, \
        read `get source` first, and a repository with no branch deploys from `main`)",
    ),
    (
        "add_domain",
        "`add_domain` (needs `host` and `port`, optionally `https` and `is_primary`)",
    ),
    (
        "update_domain",
        "`update_domain` (needs `domain` - a host or an id from `get domains` - plus the fields that change)",
    ),
    ("delete_domain", "`delete_domain` (needs `domain`)"),
    (
        "create_project",
        "`create_project` (needs `name`, optionally `description`; takes no `application`)",
    ),
    (
        "create_application",
        "`create_application` (needs `project` and `name`; pass the whole `compose` file and the \
        container `port` it publishes to create a Compose application, and `environment` and \
        `server` when the project has more than one of either)",
    ),
    (
        "update_project",
        "`update_project` (needs `project`, plus `name`, `description`, or both)",
    ),
    (
        "delete_project",
        "`delete_project` (needs `project`; this destroys every application in it, so read \
        `get applications` first and say what goes with it)",
    ),
];

fn help_for(names: &[&str], help: &[(&str, &str)]) -> String {
    names
        .iter()
        .filter_map(|name| {
            help.iter()
                .find(|(key, _)| key == name)
                .map(|(_, text)| *text)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The tool list sent on every request, built from the allowlist in
/// [`capabilities`]: a resource or action that is not allowed is not in the
/// enum and not in the description, and a tool with nothing allowed is left
/// out. Stable order and content, so it stays a cacheable prefix.
pub fn definitions() -> Vec<ToolDefinition> {
    definitions_with(&[])
}

/// The two tools, for a surface where `direct` actions run on the spot rather
/// than being proposed (the MCP endpoint passes `capabilities::MCP_DIRECT`).
/// The in-app chat passes nothing, so its `exec` still says it never acts.
pub fn definitions_with(direct: &[&str]) -> Vec<ToolDefinition> {
    let mut tools = Vec::new();
    if !capabilities::RESOURCES.is_empty() {
        tools.push(ToolDefinition {
            kind: "function",
            function: FunctionDefinition {
                name: GET,
                description: format!(
                    "Read something. `resource` picks what: {}. \
                    Start with `projects` when the user names something you have not seen. \
                    Every result also says where the application came from: an internal template, a catalog template, or the user.",
                    help_for(capabilities::RESOURCES, RESOURCE_HELP)
                ),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "resource": {
                            "type": "string",
                            "enum": capabilities::RESOURCES
                        },
                        "project": { "type": "string", "description": "Project name or id." },
                        "application": { "type": "string", "description": "Application name or id." },
                        "run": { "type": "string", "description": "Deployment run id. Omit for the latest run." },
                        "container": { "type": "string", "description": "One container name from `get containers`. Omit to read every container of the application." },
                        "level": { "type": "string", "enum": ["error", "warning", "info"], "description": "Only lines of this level. `error` is the fastest way to find why a deploy failed." },
                        "scope": { "type": "string", "enum": ["runtime", "build"] }
                    },
                    "required": ["resource"],
                    "additionalProperties": false
                }),
            },
        });
    }
    if !capabilities::ACTIONS.is_empty() {
        let opening = if direct.is_empty() {
            "Propose a change. This NEVER performs it: every action returns an id the user has to confirm in the panel, so say you have proposed it and never that it is done, live or deploying. ".to_owned()
        } else {
            let named: Vec<String> = direct.iter().map(|action| format!("`{action}`")).collect();
            format!(
                "Make a change. {} happen immediately and return their result. \
                Every other action is only proposed: it returns an id the user has to confirm in Litepod, \
                so say you have proposed it and never that it is done, live or deploying. ",
                named.join(" and ")
            )
        };
        tools.push(ToolDefinition {
            kind: "function",
            function: FunctionDefinition {
                name: EXEC,
                description: format!(
                    "{opening}`action` picks what: {}. \
                    `application` is required for every action except `create_application`, `create_project`, \
                    `update_project` and `delete_project`, which take `project` instead (`create_project` \
                    takes neither). \
                    Always pass `reason`: it is the one line the user reads before deciding.",
                    help_for(capabilities::ACTIONS, ACTION_HELP)
                ),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": capabilities::ACTIONS
                        },
                        "project": { "type": "string", "description": "Project name or id. Narrows an ambiguous application name, and names the project the `*_project` actions act on." },
                        "application": { "type": "string", "description": "Application name or id." },
                        "name": { "type": "string", "description": "Variable name, upper case with underscores." },
                        "value": { "type": "string" },
                        "scope": { "type": "string", "enum": ["runtime", "build"] },
                        "compose": { "type": "string", "description": "The complete replacement compose file." },
                        "description": { "type": "string" },
                        "git_provider_id": { "type": "string", "description": "From `get git_providers`." },
                        "git_url": { "type": "string", "description": "Repository URL." },
                        "git_branch": { "type": "string", "description": "Defaults to `main` when a repository is set without one." },
                        "railpack_project_path": { "type": "string", "description": "Directory inside the repository Railpack builds from." },
                        "auto_deploy": { "type": "boolean", "description": "Redeploy on every push to the branch." },
                        "watch_paths": { "type": "string", "description": "Comma-separated path prefixes; a push only deploys if it touched one." },
                        "environment": { "type": "string", "description": "Environment name or id. Defaults to the project's production environment." },
                        "server": { "type": "string", "description": "Server name or id from `get servers`. Defaults to the only ready one." },
                        "domain": { "type": "string", "description": "A host, or a domain id from `get domains`." },
                        "host": { "type": "string", "description": "Hostname to serve the application on." },
                        "port": { "type": "integer", "description": "The port this host fronts." },
                        "https": { "type": "boolean" },
                        "is_primary": { "type": "boolean", "description": "Make this the application's main URL." },
                        "reason": { "type": "string", "description": "One line on why, shown to the user on the confirmation card." }
                    },
                    "required": ["action"],
                    "additionalProperties": false
                }),
            },
        });
    }
    tools
}

/// What running a tool produced: the JSON handed back to the model, plus the
/// action it created, if any.
pub struct ToolOutcome {
    pub result: String,
    pub action: Option<actions::ActionDto>,
    /// Names the user can pick from when a lookup missed or was ambiguous. The
    /// panel renders them as buttons, so a typo costs one click rather than a
    /// retyped sentence.
    pub choices: Vec<String>,
}

impl ToolOutcome {
    fn data(value: serde_json::Value) -> Self {
        Self {
            result: value.to_string(),
            action: None,
            choices: Vec::new(),
        }
    }

    /// A tool failure is an answer, not an exception: the model is told what
    /// went wrong so it can correct itself or say so.
    fn failure(message: impl Into<String>) -> Self {
        Self {
            result: json!({ "error": message.into() }).to_string(),
            action: None,
            choices: Vec::new(),
        }
    }

    /// A miss the user can fix by picking. The candidates go to the model too,
    /// so it can name them if it wants, and to the panel as buttons.
    fn miss(message: impl Into<String>, choices: Vec<String>) -> Self {
        Self {
            result: json!({ "error": message.into(), "candidates": choices }).to_string(),
            action: None,
            choices,
        }
    }

    fn proposed(action: actions::ActionDto, note: &str) -> Self {
        Self {
            result: json!({
                "proposed": true,
                "action_id": action.id,
                "title": action.title,
                "note": note,
            })
            .to_string(),
            action: Some(action),
            choices: Vec::new(),
        }
    }
}

// ── Resolution ──────────────────────────────────────────────────────────────

/// One application, resolved from whatever the model called it.
struct Resolved {
    target: ActionTarget,
    origin: String,
}

/// A lookup that missed, with what the user probably meant.
struct Miss {
    message: String,
    candidates: Vec<String>,
}

#[derive(Clone)]
struct ProjectSummary {
    id: String,
    name: String,
}

#[derive(Clone)]
struct ApplicationEntry {
    environment_id: String,
    environment_name: String,
    id: String,
    name: String,
    /// How the application came to exist. The assistant needs it to know
    /// whether a compose file is the user's to change or a template's.
    origin: String,
}

/// Plain words for `template_kind` / `catalog_template_id`, which mean nothing
/// to a model on their own.
fn origin_of(template_kind: Option<&str>, catalog_template_id: Option<&str>) -> String {
    match (template_kind, catalog_template_id) {
        (Some(kind), _) => format!("internal template ({kind})"),
        (None, Some(id)) => format!("catalog template ({id})"),
        (None, None) => "created by the user".into(),
    }
}

/// Every application in the organization, as `(project, application)` pairs.
async fn catalog(
    state: &SharedState,
    organization_id: &str,
) -> Result<Vec<(ProjectSummary, ApplicationEntry)>, String> {
    let projects = project_queries::list_projects(state, organization_id)
        .await
        .map_err(|error| error.to_string())?;

    let mut out = Vec::new();
    for summary in projects {
        let detail = project_queries::get_project(state, organization_id, &summary.id)
            .await
            .map_err(|error| error.to_string())?;
        let project = ProjectSummary {
            id: summary.id.clone(),
            name: summary.name.clone(),
        };
        for environment in detail.environments {
            for app in environment.applications {
                out.push((
                    project.clone(),
                    ApplicationEntry {
                        environment_id: environment.id.clone(),
                        environment_name: environment.name.clone(),
                        origin: origin_of(
                            app.template_kind.as_deref(),
                            app.catalog_template_id.as_deref(),
                        ),
                        id: app.id,
                        name: app.name,
                    },
                ));
            }
        }
    }
    Ok(out)
}

/// Finds an application by name or id inside the organization, optionally
/// narrowed by project.
///
/// A miss and an ambiguity are both answered with candidates rather than a
/// guess: two applications called `web` in different projects is a question,
/// and `statci` is a typo the user can fix with one click.
async fn resolve_application(
    state: &SharedState,
    organization_id: &str,
    application: &str,
    project: Option<&str>,
) -> Result<Resolved, Miss> {
    let wanted = application.trim().to_ascii_lowercase();
    let everything = catalog(state, organization_id)
        .await
        .map_err(|message| Miss {
            message,
            candidates: Vec::new(),
        })?;

    let in_scope: Vec<_> = everything
        .iter()
        .filter(|(summary, _)| match project {
            None => true,
            Some(filter) => {
                let filter = filter.trim().to_ascii_lowercase();
                summary.name.to_ascii_lowercase() == filter
                    || summary.id.to_ascii_lowercase() == filter
            }
        })
        .collect();

    // A project filter that matches nothing is itself the mistake, so fall back
    // to the whole organization for suggestions.
    let searched = if in_scope.is_empty() {
        everything.iter().collect::<Vec<_>>()
    } else {
        in_scope
    };

    let mut matches: Vec<_> = searched
        .iter()
        .filter(|(_, app)| {
            app.name.to_ascii_lowercase() == wanted || app.id.to_ascii_lowercase() == wanted
        })
        .collect();

    if matches.len() == 1 {
        let (summary, app) = matches.remove(0);
        return Ok(Resolved {
            target: ActionTarget {
                organization_id: organization_id.to_owned(),
                project_id: summary.id.clone(),
                project_name: summary.name.clone(),
                environment_id: app.environment_id.clone(),
                application_name: app.name.clone(),
                application_id: app.id.clone(),
            },
            origin: app.origin.clone(),
        });
    }

    let labels: Vec<String> = if matches.is_empty() {
        rank(&wanted, searched.iter().map(|(_, app)| app.name.as_str()))
            .into_iter()
            .map(|name| label(&searched, &name))
            .collect()
    } else {
        matches
            .iter()
            .map(|(summary, app)| format!("{}/{}", summary.name, app.name))
            .take(MAX_CANDIDATES)
            .collect()
    };

    Err(Miss {
        message: if matches.is_empty() {
            format!("No application called `{application}` in this organization.")
        } else {
            format!("`{application}` matches {} applications.", matches.len())
        },
        candidates: labels,
    })
}

fn label(searched: &[&(ProjectSummary, ApplicationEntry)], application: &str) -> String {
    searched
        .iter()
        .find(|(_, app)| app.name == application)
        .map(|(summary, app)| format!("{}/{}", summary.name, app.name))
        .unwrap_or_else(|| application.to_owned())
}

/// The names closest to what was asked for, nearest first.
///
/// Substring hits come first because they are almost always the intent; the
/// rest are ranked by edit distance, so a transposition like `statci` still
/// finds `static`. When nothing is close the whole (capped) list comes back:
/// something to pick beats nothing at all.
fn rank<'a>(wanted: &str, names: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut scored: Vec<(usize, String)> = names
        .map(|name| {
            let lowered = name.to_ascii_lowercase();
            let score = if lowered.contains(wanted) || wanted.contains(&lowered) {
                0
            } else {
                distance(wanted, &lowered)
            };
            (score, name.to_owned())
        })
        .collect();
    scored.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    scored.dedup_by(|left, right| left.1 == right.1);

    let tolerance = (wanted.chars().count() / 2).max(2);
    let close: Vec<String> = scored
        .iter()
        .filter(|(score, _)| *score <= tolerance)
        .map(|(_, name)| name.clone())
        .take(MAX_CANDIDATES)
        .collect();

    if close.is_empty() {
        scored
            .into_iter()
            .map(|(_, name)| name)
            .take(MAX_CANDIDATES)
            .collect()
    } else {
        close
    }
}

/// Levenshtein distance, two rows at a time. Small enough to keep here rather
/// than take a dependency for one call site.
fn distance(left: &str, right: &str) -> usize {
    let right: Vec<char> = right.chars().collect();
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    let mut current = vec![0; right.len() + 1];

    for (i, l) in left.chars().enumerate() {
        current[0] = i + 1;
        for (j, r) in right.iter().enumerate() {
            let substitute = previous[j] + usize::from(l != *r);
            current[j + 1] = substitute.min(previous[j + 1] + 1).min(current[j] + 1);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[right.len()]
}

// ── Dispatch ────────────────────────────────────────────────────────────────

fn string_of(arguments: &serde_json::Value, key: &str) -> Option<String> {
    arguments
        .get(key)
        .and_then(|value| value.as_str())
        .map(|value| value.to_owned())
}

fn scope_of(arguments: &serde_json::Value) -> String {
    string_of(arguments, "scope").unwrap_or_else(|| "runtime".into())
}

/// The `application` / `project` pair every resource but `projects` takes.
///
/// The failure is boxed: a `ToolOutcome` carries a whole action, and a `Result`
/// that wide costs every caller a copy on the happy path.
async fn resolve(
    state: &SharedState,
    organization_id: &str,
    arguments: &serde_json::Value,
) -> Result<Resolved, Box<ToolOutcome>> {
    let Some(application) = string_of(arguments, "application") else {
        return Err(Box::new(ToolOutcome::failure("`application` is required")));
    };
    let project = string_of(arguments, "project");
    resolve_application(state, organization_id, &application, project.as_deref())
        .await
        .map_err(|miss| Box::new(ToolOutcome::miss(miss.message, miss.candidates)))
}

/// The application an `application` / `project` pair names, for callers
/// outside this module (the MCP endpoint's direct actions). A miss comes back
/// as the same JSON the model would have read, candidates included.
pub async fn resolve_target(
    state: &SharedState,
    organization_id: &str,
    arguments: &serde_json::Value,
) -> Result<ActionTarget, String> {
    resolve(state, organization_id, arguments)
        .await
        .map(|resolved| resolved.target)
        .map_err(|outcome| outcome.result)
}

/// Runs one tool call. Never returns `Err` for a tool's own failure: the model
/// has to see what happened to recover from it.
pub async fn run(
    state: &SharedState,
    organization_id: &str,
    conversation_id: &str,
    actor: &AuthUser,
    name: &str,
    arguments_json: &str,
) -> ToolOutcome {
    // Argument escaping from a model is not stable, so this is always parsed.
    let arguments: serde_json::Value = match serde_json::from_str(arguments_json) {
        Ok(value) => value,
        Err(error) => {
            return ToolOutcome::failure(format!("arguments were not valid JSON: {error}"));
        }
    };

    // The schema already leaves out what is not allowed; a model can still
    // send it, so the allowlist is checked here as well.
    let refused = match name {
        GET => string_of(&arguments, "resource")
            .filter(|resource| !capabilities::resource_allowed(resource)),
        EXEC => {
            string_of(&arguments, "action").filter(|action| !capabilities::action_allowed(action))
        }
        _ => None,
    };
    if let Some(refused) = refused {
        return ToolOutcome::failure(format!(
            "`{refused}` is not available to the assistant on this instance"
        ));
    }

    match name {
        GET => get(state, organization_id, actor, &arguments).await,
        EXEC => exec(state, organization_id, conversation_id, actor, &arguments).await,
        other => ToolOutcome::failure(format!("`{other}` is not a tool this build has")),
    }
}

async fn get(
    state: &SharedState,
    organization_id: &str,
    actor: &AuthUser,
    arguments: &serde_json::Value,
) -> ToolOutcome {
    let Some(resource) = string_of(arguments, "resource") else {
        return ToolOutcome::failure("`resource` is required");
    };

    match resource.as_str() {
        // The one read that is not scoped to the current organization: it is
        // the caller's own membership list, and it names which one is current
        // so the model never has to guess what "here" means.
        "organizations" => {
            match organization_queries::list_my_organizations(state, &actor.user_id).await {
                Ok(organizations) => ToolOutcome::data(json!({
                    "current": organization_id,
                    "organizations": organizations
                        .into_iter()
                        .map(|organization| json!({
                            "id": organization.id,
                            "name": organization.name,
                            "current": organization.id == organization_id,
                            "assistant_enabled": organization.assistant_enabled,
                        }))
                        .collect::<Vec<_>>()
                })),
                Err(error) => ToolOutcome::failure(error.to_string()),
            }
        }

        "projects" => match project_queries::list_projects(state, organization_id).await {
            Ok(projects) => ToolOutcome::data(json!({
                "projects": projects
                    .into_iter()
                    .map(|project| json!({ "id": project.id, "name": project.name }))
                    .collect::<Vec<_>>()
            })),
            Err(error) => ToolOutcome::failure(error.to_string()),
        },

        "applications" => {
            let Some(project) = string_of(arguments, "project") else {
                return ToolOutcome::failure("`project` is required for `applications`");
            };
            applications_in(state, organization_id, &project).await
        }

        "environment" => {
            let resolved = match resolve(state, organization_id, arguments).await {
                Ok(resolved) => resolved,
                Err(outcome) => return *outcome,
            };
            let scope = scope_of(arguments);
            match deployment::environment(state, &resolved.target.application_id, &scope, false)
                .await
            {
                // Names only. A value is never returned by any resource.
                Ok(variables) => ToolOutcome::data(json!({
                    "application": resolved.target.application_name,
                    "origin": resolved.origin,
                    "scope": scope,
                    "names": variables
                        .into_iter()
                        .map(|variable| variable.name)
                        .collect::<Vec<_>>()
                })),
                Err(error) => ToolOutcome::failure(error.to_string()),
            }
        }

        "deployments" => {
            let resolved = match resolve(state, organization_id, arguments).await {
                Ok(resolved) => resolved,
                Err(outcome) => return *outcome,
            };
            match deployment::history::list(state, &resolved.target.application_id).await {
                Err(error) => ToolOutcome::failure(error.to_string()),
                Ok(runs) => ToolOutcome::data(json!({
                    "application": resolved.target.application_name,
                    "origin": resolved.origin,
                    "runs": runs
                        .into_iter()
                        .map(|run| json!({
                            "id": run.id,
                            "status": run.status,
                            "source": run.source_type,
                            "started_at": run.started_at,
                            "duration_ms": run.duration_ms,
                            "error": run.last_error,
                            "commit": run.commit_sha,
                            "triggered_by": run.triggered_by,
                        }))
                        .collect::<Vec<_>>()
                })),
            }
        }

        "deployment_logs" => {
            let resolved = match resolve(state, organization_id, arguments).await {
                Ok(resolved) => resolved,
                Err(outcome) => return *outcome,
            };
            deployment_logs(
                state,
                &resolved,
                string_of(arguments, "run"),
                string_of(arguments, "level"),
            )
            .await
        }

        "git_providers" => {
            match crate::git::git_providers::provider::list(state, organization_id).await {
                Ok(providers) => ToolOutcome::data(json!({
                    "providers": providers
                        .into_iter()
                        .map(|provider| json!({
                            "id": provider.id,
                            "name": provider.name,
                            "type": provider.provider_type,
                            "host": provider.host_url,
                        }))
                        .collect::<Vec<_>>()
                })),
                Err(error) => ToolOutcome::failure(error.to_string()),
            }
        }

        "source" => {
            let resolved = match resolve(state, organization_id, arguments).await {
                Ok(resolved) => resolved,
                Err(outcome) => return *outcome,
            };
            match deployment::source::get(state, &resolved.target.application_id).await {
                Err(error) => ToolOutcome::failure(error.to_string()),
                Ok(source) => ToolOutcome::data(json!({
                    "application": resolved.target.application_name,
                    "origin": resolved.origin,
                    "deploy_type": source.deploy_type,
                    "build_method": source.build_method,
                    "git_provider_id": source.git_provider_id,
                    "git_url": source.git_url,
                    "git_branch": source.git_branch,
                    "railpack_project_path": source.railpack_project_path,
                    "auto_deploy": source.auto_deploy,
                    "watch_paths": source.watch_paths,
                })),
            }
        }

        "servers" => {
            match crate::servers::repository::list_ready_for_organization(state, organization_id)
                .await
            {
                Ok(servers) => ToolOutcome::data(json!({
                    "servers": servers
                        .into_iter()
                        .map(|server| json!({
                            "id": server.id,
                            "name": server.name,
                            "kind": server.kind,
                        }))
                        .collect::<Vec<_>>()
                })),
                Err(error) => ToolOutcome::failure(error.to_string()),
            }
        }

        "domains" => {
            let resolved = match resolve(state, organization_id, arguments).await {
                Ok(resolved) => resolved,
                Err(outcome) => return *outcome,
            };
            match deployment::domains::list(state, &resolved.target.application_id).await {
                Err(error) => ToolOutcome::failure(error.to_string()),
                Ok(domains) => ToolOutcome::data(json!({
                    "application": resolved.target.application_name,
                    "origin": resolved.origin,
                    "domains": domains
                        .into_iter()
                        .map(|domain| json!({
                            "id": domain.id,
                            "host": domain.host,
                            "port": domain.port,
                            "https": domain.https,
                            "url": domain.url,
                            "is_primary": domain.is_primary,
                            "is_local": domain.is_local,
                            "route_health": domain.route_health,
                            "required_dns_target": domain.required_dns_target,
                        }))
                        .collect::<Vec<_>>()
                })),
            }
        }

        "containers" => {
            let resolved = match resolve(state, organization_id, arguments).await {
                Ok(resolved) => resolved,
                Err(outcome) => return *outcome,
            };
            containers(state, &resolved).await
        }

        "container_logs" => {
            let resolved = match resolve(state, organization_id, arguments).await {
                Ok(resolved) => resolved,
                Err(outcome) => return *outcome,
            };
            container_logs(state, &resolved, string_of(arguments, "container")).await
        }

        "compose" => {
            let resolved = match resolve(state, organization_id, arguments).await {
                Ok(resolved) => resolved,
                Err(outcome) => return *outcome,
            };
            let store = ComposeFileStore::new(&state.data_dir, &resolved.target.application_id);
            match store.read_content().await {
                Ok(content) => ToolOutcome::data(json!({
                    "application": resolved.target.application_name,
                    "origin": resolved.origin,
                    // A compose file carries passwords in clear text, and it is
                    // about to leave this machine.
                    "compose": scrub(state, &resolved.target.application_id, &content).await,
                })),
                Err(error) => ToolOutcome::failure(format!(
                    "No compose file for `{}`: {error}",
                    resolved.target.application_name
                )),
            }
        }

        other => ToolOutcome::failure(format!(
            "`{other}` is not a resource. Use organizations, projects, applications, environment, deployments, deployment_logs, containers, container_logs, source, domains, servers, git_providers or compose."
        )),
    }
}

async fn exec(
    state: &SharedState,
    organization_id: &str,
    conversation_id: &str,
    actor: &AuthUser,
    arguments: &serde_json::Value,
) -> ToolOutcome {
    let Some(action) = string_of(arguments, "action") else {
        return ToolOutcome::failure("`action` is required");
    };
    let reason = string_of(arguments, "reason");

    // The project-scoped actions come first: they have no application to
    // resolve, and `create_project` does not even have a project yet.
    if let Some(outcome) = project_action(
        state,
        organization_id,
        conversation_id,
        actor,
        &action,
        arguments,
        reason.clone(),
    )
    .await
    {
        return outcome;
    }

    let resolved = match resolve(state, organization_id, arguments).await {
        Ok(resolved) => resolved,
        Err(outcome) => return *outcome,
    };

    match action.as_str() {
        "set_environment_variable" => {
            let (Some(name), Some(value)) =
                (string_of(arguments, "name"), string_of(arguments, "value"))
            else {
                return ToolOutcome::failure("`name` and `value` are required");
            };
            match actions::propose_environment_variable(
                state,
                conversation_id,
                actor,
                &resolved.target,
                SetEnvironmentVariable {
                    name,
                    value,
                    scope: scope_of(arguments),
                    reason: reason.clone(),
                },
            )
            .await
            {
                Ok(action) => ToolOutcome::proposed(
                    action,
                    "Not applied. The user has to confirm this in the panel.",
                ),
                Err(error) => ToolOutcome::failure(error.to_string()),
            }
        }

        "delete_environment_variable" => {
            let Some(name) = string_of(arguments, "name") else {
                return ToolOutcome::failure("`name` is required");
            };
            match actions::propose_delete_environment_variable(
                state,
                conversation_id,
                actor,
                &resolved.target,
                SetEnvironmentVariable {
                    name,
                    value: String::new(),
                    scope: scope_of(arguments),
                    reason,
                },
            )
            .await
            {
                Ok(action) => ToolOutcome::proposed(
                    action,
                    "Not removed. The user has to confirm this in the panel.",
                ),
                Err(error) => ToolOutcome::failure(error.to_string()),
            }
        }

        "update_application" => {
            match actions::propose_update_application(
                state,
                conversation_id,
                actor,
                &resolved.target,
                actions::UpdateApplication {
                    name: string_of(arguments, "name"),
                    description: string_of(arguments, "description"),
                    reason,
                },
            )
            .await
            {
                Ok(action) => ToolOutcome::proposed(
                    action,
                    "Not renamed. The user has to confirm this in the panel.",
                ),
                Err(error) => ToolOutcome::failure(error.to_string()),
            }
        }

        "set_source" => {
            match actions::propose_source(
                state,
                conversation_id,
                actor,
                &resolved.target,
                actions::SetSource {
                    git_provider_id: string_of(arguments, "git_provider_id"),
                    git_url: string_of(arguments, "git_url"),
                    git_branch: string_of(arguments, "git_branch"),
                    railpack_project_path: string_of(arguments, "railpack_project_path"),
                    auto_deploy: arguments.get("auto_deploy").and_then(|v| v.as_bool()),
                    watch_paths: string_of(arguments, "watch_paths"),
                    reason,
                },
            )
            .await
            {
                Ok(action) => ToolOutcome::proposed(
                    action,
                    "Not changed. The user has to confirm this in the panel.",
                ),
                Err(error) => ToolOutcome::failure(error.to_string()),
            }
        }

        "add_domain" => {
            match actions::propose_add_domain(
                state,
                conversation_id,
                actor,
                &resolved.target,
                domain_fields(arguments, reason),
            )
            .await
            {
                Ok(action) => ToolOutcome::proposed(
                    action,
                    "Not added. The user has to confirm this in the panel.",
                ),
                Err(error) => ToolOutcome::failure(error.to_string()),
            }
        }

        "update_domain" => {
            match actions::propose_update_domain(
                state,
                conversation_id,
                actor,
                &resolved.target,
                domain_fields(arguments, reason),
            )
            .await
            {
                Ok(action) => ToolOutcome::proposed(
                    action,
                    "Not changed. The user has to confirm this in the panel.",
                ),
                Err(error) => ToolOutcome::failure(error.to_string()),
            }
        }

        "delete_domain" => {
            match actions::propose_delete_domain(
                state,
                conversation_id,
                actor,
                &resolved.target,
                domain_fields(arguments, reason),
            )
            .await
            {
                Ok(action) => ToolOutcome::proposed(
                    action,
                    "Not removed. The user has to confirm this in the panel.",
                ),
                Err(error) => ToolOutcome::failure(error.to_string()),
            }
        }

        "redeploy" => {
            match actions::propose_redeploy(state, conversation_id, actor, &resolved.target, reason)
                .await
            {
                Ok(action) => ToolOutcome::proposed(
                    action,
                    "Nothing is deploying. The user has to confirm this in the panel.",
                ),
                Err(error) => ToolOutcome::failure(error.to_string()),
            }
        }

        "start" | "stop" => {
            let tool = if action == "start" {
                actions::START_APPLICATION
            } else {
                actions::STOP_APPLICATION
            };
            match actions::propose_lifecycle(
                state,
                conversation_id,
                actor,
                &resolved.target,
                tool,
                reason,
            )
            .await
            {
                Ok(action) => ToolOutcome::proposed(
                    action,
                    "Nothing has changed yet. The user has to confirm this in the panel.",
                ),
                Err(error) => ToolOutcome::failure(error.to_string()),
            }
        }

        "set_compose_file" => {
            let Some(compose) = string_of(arguments, "compose") else {
                return ToolOutcome::failure(
                    "`compose` is required, and must be the complete replacement file",
                );
            };
            match actions::propose_compose_file(
                state,
                conversation_id,
                actor,
                &resolved.target,
                SetComposeFile { compose, reason },
            )
            .await
            {
                Ok(action) => ToolOutcome::proposed(
                    action,
                    "Not written. The user has to confirm this in the panel.",
                ),
                Err(error) => ToolOutcome::failure(error.to_string()),
            }
        }

        other => ToolOutcome::failure(format!(
            "`{other}` is not an action. Use set_environment_variable, delete_environment_variable, update_application, set_source, redeploy, set_compose_file, add_domain, update_domain, delete_domain, create_application, create_project, update_project or delete_project."
        )),
    }
}

/// The domain arguments, in the shape `actions` validates. Absent fields stay
/// absent: an update only moves what the model actually sent.
fn domain_fields(arguments: &serde_json::Value, reason: Option<String>) -> actions::DomainFields {
    actions::DomainFields {
        domain_id: string_of(arguments, "domain"),
        host: string_of(arguments, "host"),
        port: arguments.get("port").and_then(|value| value.as_i64()),
        https: arguments.get("https").and_then(|value| value.as_bool()),
        is_primary: arguments
            .get("is_primary")
            .and_then(|value| value.as_bool()),
        reason,
    }
}

/// Handles the three project-scoped actions, or `None` when the action is one
/// of the application-scoped ones the caller goes on to resolve.
async fn project_action(
    state: &SharedState,
    organization_id: &str,
    conversation_id: &str,
    actor: &AuthUser,
    action: &str,
    arguments: &serde_json::Value,
    reason: Option<String>,
) -> Option<ToolOutcome> {
    let fields = || actions::ProjectFields {
        name: string_of(arguments, "name"),
        description: string_of(arguments, "description"),
        reason: reason.clone(),
    };

    // A create has no project to look up: the target carries the organization
    // and nothing else, and the id only exists once the user confirms.
    if action == "create_project" {
        let target = ActionTarget::project(organization_id, "", "");
        return Some(
            match actions::propose_create_project(state, conversation_id, actor, &target, fields())
                .await
            {
                Ok(action) => ToolOutcome::proposed(
                    action,
                    "Not created. The user has to confirm this in the panel.",
                ),
                Err(error) => ToolOutcome::failure(error.to_string()),
            },
        );
    }

    if !matches!(
        action,
        "update_project" | "delete_project" | "create_application"
    ) {
        return None;
    }

    let Some(wanted) = string_of(arguments, "project") else {
        return Some(ToolOutcome::failure("`project` is required"));
    };
    let mut target = match resolve_project(state, organization_id, &wanted).await {
        Ok(target) => target,
        Err(miss) => return Some(ToolOutcome::miss(miss.message, miss.candidates)),
    };

    if action == "create_application" {
        // The environment and the server are both "the only one" in the common
        // case; naming either is only needed when there is a choice to make.
        if let Err(miss) = fill_environment(
            state,
            &mut target,
            string_of(arguments, "environment").as_deref(),
        )
        .await
        {
            return Some(ToolOutcome::miss(miss.message, miss.candidates));
        }
        let server_id = match resolve_server(
            state,
            organization_id,
            string_of(arguments, "server").as_deref(),
        )
        .await
        {
            Ok(server_id) => server_id,
            Err(miss) => return Some(ToolOutcome::miss(miss.message, miss.candidates)),
        };

        return Some(
            match actions::propose_create_application(
                state,
                conversation_id,
                actor,
                &target,
                actions::CreateApplication {
                    name: string_of(arguments, "name").unwrap_or_default(),
                    description: string_of(arguments, "description"),
                    server_id,
                    compose: string_of(arguments, "compose"),
                    port: arguments.get("port").and_then(|value| value.as_i64()),
                    reason,
                },
            )
            .await
            {
                Ok(action) => ToolOutcome::proposed(
                    action,
                    "Not created. The user has to confirm this in the panel.",
                ),
                Err(error) => ToolOutcome::failure(error.to_string()),
            },
        );
    }

    Some(if action == "update_project" {
        match actions::propose_update_project(state, conversation_id, actor, &target, fields())
            .await
        {
            Ok(action) => ToolOutcome::proposed(
                action,
                "Not renamed. The user has to confirm this in the panel.",
            ),
            Err(error) => ToolOutcome::failure(error.to_string()),
        }
    } else {
        match actions::propose_delete_project(state, conversation_id, actor, &target, fields())
            .await
        {
            Ok(action) => ToolOutcome::proposed(
                action,
                "Nothing is deleted. The user has to confirm this in the panel, and it cannot be undone.",
            ),
            Err(error) => ToolOutcome::failure(error.to_string()),
        }
    })
}

/// Finds a project by name or id. Same miss-with-candidates contract as
/// `resolve_application`: a typo is a question, never a guess - and guessing
/// here would delete the wrong project.
async fn resolve_project(
    state: &SharedState,
    organization_id: &str,
    project: &str,
) -> Result<ActionTarget, Miss> {
    let wanted = project.trim().to_ascii_lowercase();
    let projects = project_queries::list_projects(state, organization_id)
        .await
        .map_err(|error| Miss {
            message: error.to_string(),
            candidates: Vec::new(),
        })?;

    let mut matches: Vec<_> = projects
        .iter()
        .filter(|summary| {
            summary.name.to_ascii_lowercase() == wanted || summary.id.to_ascii_lowercase() == wanted
        })
        .collect();

    if matches.len() == 1 {
        let summary = matches.remove(0);
        return Ok(ActionTarget::project(
            organization_id,
            &summary.id,
            &summary.name,
        ));
    }

    Err(Miss {
        message: if matches.is_empty() {
            format!("No project called `{project}` in this organization.")
        } else {
            format!("`{project}` matches {} projects.", matches.len())
        },
        candidates: rank(
            &wanted,
            projects.iter().map(|summary| summary.name.as_str()),
        ),
    })
}

/// Puts the environment the new application goes in on the target: the one the
/// model named, or the project's production environment when it named none.
async fn fill_environment(
    state: &SharedState,
    target: &mut ActionTarget,
    wanted: Option<&str>,
) -> Result<(), Miss> {
    let detail = project_queries::get_project(state, &target.organization_id, &target.project_id)
        .await
        .map_err(|error| Miss {
            message: error.to_string(),
            candidates: Vec::new(),
        })?;

    let chosen = match wanted.map(str::trim).filter(|wanted| !wanted.is_empty()) {
        Some(wanted) => detail.environments.iter().find(|environment| {
            environment.name.eq_ignore_ascii_case(wanted) || environment.id == wanted
        }),
        None => detail
            .environments
            .iter()
            .find(|environment| environment.is_production)
            .or_else(|| detail.environments.first()),
    };

    match chosen {
        Some(environment) => {
            target.environment_id = environment.id.clone();
            Ok(())
        }
        None => Err(Miss {
            message: match wanted {
                Some(wanted) => {
                    format!(
                        "`{}` has no environment called `{wanted}`.",
                        detail.project.name
                    )
                }
                None => format!("`{}` has no environments.", detail.project.name),
            },
            candidates: detail
                .environments
                .iter()
                .map(|environment| environment.name.clone())
                .take(MAX_CANDIDATES)
                .collect(),
        }),
    }
}

/// The server a new application is placed on. With one ready server there is
/// nothing to ask; with several, picking one for the user is picking where
/// their workload runs.
async fn resolve_server(
    state: &SharedState,
    organization_id: &str,
    wanted: Option<&str>,
) -> Result<String, Miss> {
    let servers = crate::servers::repository::list_ready_for_organization(state, organization_id)
        .await
        .map_err(|error| Miss {
            message: error.to_string(),
            candidates: Vec::new(),
        })?;

    let names = || {
        servers
            .iter()
            .map(|server| server.name.clone())
            .take(MAX_CANDIDATES)
            .collect::<Vec<_>>()
    };

    match wanted.map(str::trim).filter(|wanted| !wanted.is_empty()) {
        Some(wanted) => servers
            .iter()
            .find(|server| server.name.eq_ignore_ascii_case(wanted) || server.id == wanted)
            .map(|server| server.id.clone())
            .ok_or_else(|| Miss {
                message: format!("No ready server called `{wanted}`."),
                candidates: names(),
            }),
        None if servers.len() == 1 => Ok(servers[0].id.clone()),
        None if servers.is_empty() => Err(Miss {
            message: "This organization has no ready server to place an application on.".into(),
            candidates: Vec::new(),
        }),
        None => Err(Miss {
            message: "Which server should it run on?".into(),
            candidates: names(),
        }),
    }
}

async fn applications_in(state: &SharedState, organization_id: &str, project: &str) -> ToolOutcome {
    let wanted = project.trim().to_ascii_lowercase();
    let everything = match catalog(state, organization_id).await {
        Ok(everything) => everything,
        Err(error) => return ToolOutcome::failure(error),
    };

    let matched: Vec<_> = everything
        .iter()
        .filter(|(summary, _)| {
            summary.name.to_ascii_lowercase() == wanted || summary.id.to_ascii_lowercase() == wanted
        })
        .collect();

    if matched.is_empty() {
        let names: Vec<String> = everything
            .iter()
            .map(|(summary, _)| summary.name.clone())
            .collect();
        return ToolOutcome::miss(
            format!("No project called `{project}` in this organization."),
            rank(&wanted, names.iter().map(String::as_str)),
        );
    }

    ToolOutcome::data(json!({
        "project": matched[0].0.name,
        "applications": matched
            .iter()
            .map(|(_, app)| json!({
                "id": app.id,
                "name": app.name,
                "environment": app.environment_name,
                "origin": app.origin,
            }))
            .collect::<Vec<_>>()
    }))
}

/// Live container state, asked of the runtime rather than of the database.
///
/// "We could not ask" and "the runtime says it is gone" are different facts, and
/// only the first one means the container may well still be up on a server this
/// control plane cannot currently reach. Both are reported as themselves.
async fn containers(state: &SharedState, resolved: &Resolved) -> ToolOutcome {
    let application_id = &resolved.target.application_id;
    let names = match crate::handler::deployments::container_names(state, application_id).await {
        Ok(names) => names,
        Err(error) => return ToolOutcome::failure(error.to_string()),
    };
    if names.is_empty() {
        return ToolOutcome::data(json!({
            "application": resolved.target.application_name,
            "containers": [],
            "note": "Not deployed: this application has no container."
        }));
    }

    let Ok(runtime) = ServerRuntime::for_application(state, application_id).await else {
        return ToolOutcome::data(json!({
            "application": resolved.target.application_name,
            "containers": names
                .into_iter()
                .map(|name| json!({ "name": name, "status": "unavailable" }))
                .collect::<Vec<_>>(),
            "note": "The server could not be reached, so this is not a statement about the containers."
        }));
    };

    let mut out = Vec::with_capacity(names.len());
    for name in names {
        let status = crate::handler::deployments::inspect_container_state(&runtime, &name).await;
        out.push(json!({
            "name": name,
            "status": status,
            "label": crate::handler::deployments::container_status_label(&status),
        }));
    }

    ToolOutcome::data(json!({
        "application": resolved.target.application_name,
        "origin": resolved.origin,
        "containers": out,
    }))
}

/// What the containers printed, which is a different question from what the
/// deploy printed. An exited container still holds its output - the hello-world
/// images say their piece once and stop - so "it exited" is never a reason to
/// report that there is nothing to read.
async fn container_logs(
    state: &SharedState,
    resolved: &Resolved,
    container: Option<String>,
) -> ToolOutcome {
    let application_id = &resolved.target.application_id;
    let targets = match crate::handler::deployments::log_targets(state, application_id).await {
        Ok(targets) => targets,
        Err(error) => return ToolOutcome::failure(error.to_string()),
    };
    let targets: Vec<_> = match &container {
        Some(name) => targets
            .into_iter()
            .filter(|target| &target.container_name == name)
            .collect(),
        None => targets,
    };
    if targets.is_empty() {
        return ToolOutcome::failure(match container {
            Some(name) => format!(
                "`{name}` is not a container of this application. Use `get containers` for the names."
            ),
            None => {
                "Not deployed: this application has no container to read logs from.".to_string()
            }
        });
    }

    let runtime = match ServerRuntime::for_application(state, application_id).await {
        Ok(runtime) => runtime,
        Err(error) => {
            return ToolOutcome::failure(format!(
                "The server could not be reached, so this is not a statement about the logs: {error}"
            ));
        }
    };

    let mut out = Vec::with_capacity(targets.len());
    for target in targets {
        let entry = match runtime
            .podman(&crate::handler::deployments::logs_argv(
                &target.container_name,
                LOG_TAIL,
                false,
            ))
            .await
        {
            // Podman splits the container's two streams across stdout and
            // stderr; both are the container talking.
            Ok(output) => {
                let text = output
                    .stdout
                    .lines()
                    .chain(output.stderr.lines())
                    .map(deployment::log_filter::strip_ansi_sequences)
                    .collect::<Vec<_>>()
                    .join("\n");
                json!({
                    "container": target.container_name,
                    "service": target.service,
                    "status": target.status,
                    // Container output is where a leaked credential shows up.
                    "logs": scrub(state, application_id, &text).await,
                })
            }
            Err(error) => json!({
                "container": target.container_name,
                "service": target.service,
                "status": target.status,
                "error": error.to_string(),
            }),
        };
        out.push(entry);
    }

    ToolOutcome::data(json!({
        "application": resolved.target.application_name,
        "origin": resolved.origin,
        "tail": LOG_TAIL,
        "containers": out,
    }))
}

/// The tail of one run's logs, optionally only the lines of one level.
async fn deployment_logs(
    state: &SharedState,
    resolved: &Resolved,
    run: Option<String>,
    level: Option<String>,
) -> ToolOutcome {
    let application_id = &resolved.target.application_id;

    // No run named means the one the user is almost certainly asking about.
    let run_id = match run {
        Some(id) => id,
        None => match deployment::history::list(state, application_id).await {
            Err(error) => return ToolOutcome::failure(error.to_string()),
            Ok(runs) => match runs.into_iter().next() {
                Some(latest) => latest.id,
                None => {
                    return ToolOutcome::data(json!({
                        "application": resolved.target.application_name,
                        "runs": [],
                        "note": "This application has never been deployed."
                    }));
                }
            },
        },
    };

    match deployment::history::get(state, application_id, &run_id, Some(LOG_TAIL)).await {
        Err(error) => ToolOutcome::failure(error.to_string()),
        Ok(detail) => {
            let wanted = level.map(|value| value.to_ascii_lowercase());
            let lines: Vec<_> = detail
                .logs
                .iter()
                .filter(|line| {
                    wanted
                        .as_ref()
                        .is_none_or(|level| line.level.to_ascii_lowercase() == *level)
                })
                .map(|line| json!({ "level": line.level, "message": line.message }))
                .collect();
            // A log line prints whatever the application printed.
            let lines = scrub_json(state, application_id, lines).await;
            ToolOutcome::data(json!({
                "application": resolved.target.application_name,
                "origin": resolved.origin,
                "run": run_id,
                "status": detail.run.status,
                "error": detail.run.last_error,
                "lines": lines,
            }))
        }
    }
}

/// Everything the model is about to read, with this application's own secrets
/// taken out.
///
/// The decrypted values are the precise pass and never leave this function; a
/// failure to read them degrades to pattern matching rather than to sending the
/// text through untouched.
async fn scrub(state: &SharedState, application_id: &str, text: &str) -> String {
    let mut values = deployment::decrypted_environment(state, application_id, "runtime")
        .await
        .unwrap_or_default();
    values.extend(
        deployment::decrypted_environment(state, application_id, "build")
            .await
            .unwrap_or_default(),
    );
    redaction::redact(text, &values)
}

/// The same, over a list of log lines, reading the values once.
async fn scrub_json(
    state: &SharedState,
    application_id: &str,
    lines: Vec<serde_json::Value>,
) -> Vec<serde_json::Value> {
    let mut values = deployment::decrypted_environment(state, application_id, "runtime")
        .await
        .unwrap_or_default();
    values.extend(
        deployment::decrypted_environment(state, application_id, "build")
            .await
            .unwrap_or_default(),
    );

    lines
        .into_iter()
        .map(|mut line| {
            if let Some(message) = line.get("message").and_then(|value| value.as_str()) {
                let clean = redaction::redact(message, &values);
                line["message"] = json!(clean);
            }
            line
        })
        .collect()
}

/// Cap on how much of a tool result reaches the model, so one long list cannot
/// crowd out the conversation. Larger than a comfortable answer on purpose: a
/// compose file plus two hundred log lines has to fit.
pub const MAX_RESULT_BYTES: usize = 16_000;

pub fn truncate(result: String) -> String {
    if result.len() <= MAX_RESULT_BYTES {
        return result;
    }
    let mut cut = MAX_RESULT_BYTES;
    while !result.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}… (truncated)", &result[..cut])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_transposed_typo_still_finds_the_name() {
        // The case that shipped broken: `statci` for `static`.
        let names = ["static", "api-gateway", "admin-ui"];
        assert_eq!(rank("statci", names.into_iter()).first().unwrap(), "static");
    }

    #[test]
    fn a_substring_beats_a_near_miss() {
        let names = ["gateway", "getaway-jobs"];
        assert_eq!(rank("gate", names.into_iter()).first().unwrap(), "gateway");
    }

    #[test]
    fn nothing_close_still_offers_something_to_pick() {
        let names = ["alpha", "beta"];
        let ranked = rank("zzzzzzzz", names.into_iter());
        assert_eq!(
            ranked.len(),
            2,
            "an empty list gives the user nothing to do"
        );
    }

    #[test]
    fn the_list_stays_short_enough_to_be_a_choice() {
        let names: Vec<String> = (0..30).map(|index| format!("app-{index}")).collect();
        let ranked = rank("app", names.iter().map(String::as_str));
        assert_eq!(ranked.len(), MAX_CANDIDATES);
    }

    #[test]
    fn distance_counts_the_edits_it_should() {
        assert_eq!(distance("static", "static"), 0);
        assert_eq!(distance("statci", "static"), 2);
        assert_eq!(distance("", "abc"), 3);
    }

    #[test]
    fn truncation_is_marked_rather_than_silent() {
        let long = "x".repeat(MAX_RESULT_BYTES + 50);
        let cut = truncate(long);
        assert!(cut.ends_with("… (truncated)"));
        assert!(cut.len() < MAX_RESULT_BYTES + 50);
    }

    #[test]
    fn the_surface_is_two_tools_and_stays_that_way() {
        // A capability is a `resource` or an `action`, never a third tool. If
        // this fails, something reached for a new tool instead of a new value.
        let names: Vec<&str> = definitions()
            .iter()
            .map(|definition| definition.function.name)
            .collect();
        assert_eq!(names, vec![GET, EXEC]);
    }

    #[test]
    fn the_tool_schema_offers_exactly_the_allowlist() {
        let tools = definitions();
        let get = &tools[0].function;
        let exec = &tools[1].function;
        assert_eq!(
            get.parameters["properties"]["resource"]["enum"],
            json!(capabilities::RESOURCES)
        );
        assert_eq!(
            exec.parameters["properties"]["action"]["enum"],
            json!(capabilities::ACTIONS)
        );
        // Every allowed value is explained, so the description and the enum
        // cannot drift apart.
        for resource in capabilities::RESOURCES {
            assert!(
                get.description.contains(&format!("`{resource}` (")),
                "{resource}"
            );
        }
        for action in capabilities::ACTIONS {
            assert!(
                exec.description.contains(&format!("`{action}` (")),
                "{action}"
            );
        }
    }

    #[tokio::test]
    async fn a_call_outside_the_allowlist_is_refused_before_it_runs() {
        let (state, path) = crate::testing::test_state().await;
        let actor = crate::testing::auth_user("user-a");
        for (tool, arguments) in [
            (EXEC, r#"{"action":"run_shell","application":"x"}"#),
            (GET, r#"{"resource":"secrets"}"#),
        ] {
            let outcome = run(
                &state,
                "organization-a",
                "conversation",
                &actor,
                tool,
                arguments,
            )
            .await;
            assert!(
                outcome
                    .result
                    .to_string()
                    .contains("not available to the assistant"),
                "{tool}: {}",
                outcome.result
            );
        }
        crate::testing::cleanup_database(&path);
    }

    #[tokio::test]
    async fn container_logs_are_a_resource_of_their_own() {
        // The bug this pins: asked what the containers printed, the model read
        // `deployment_logs` - litepod's record of the deploy - saw the run had
        // finished, and reported that there was nothing to show, while the same
        // output sat in the UI. An undeployed application says so; it does not
        // return an empty log.
        let (state, path) = crate::testing::test_state().await;
        let (_, _, application) = crate::testing::seed_application(&state, "organization-a").await;

        let outcome = run(
            &state,
            "organization-a",
            "conversation",
            &crate::testing::auth_user("editor"),
            GET,
            &json!({ "resource": "container_logs", "application": application }).to_string(),
        )
        .await;

        assert!(
            outcome.result.contains("Not deployed"),
            "{}",
            outcome.result
        );
        assert!(
            !outcome.result.contains("is not a resource"),
            "the schema advertises a resource the dispatch does not handle: {}",
            outcome.result
        );

        crate::testing::cleanup_database(&path);
    }

    #[tokio::test]
    async fn the_membership_list_marks_the_one_this_turn_is_in() {
        // The current organization comes from the request's authorisation, not
        // from the model, so it is reported rather than inferred.
        let (state, path) = crate::testing::test_state().await;

        let outcome = run(
            &state,
            "organization-a",
            "conversation",
            &crate::testing::auth_user("editor"),
            GET,
            &json!({ "resource": "organizations" }).to_string(),
        )
        .await;

        let value: serde_json::Value = serde_json::from_str(&outcome.result).unwrap();
        assert_eq!(value["current"], "organization-a");

        let listed = value["organizations"].as_array().unwrap();
        let current: Vec<_> = listed
            .iter()
            .filter(|entry| entry["current"] == serde_json::json!(true))
            .collect();
        assert_eq!(
            current.len(),
            1,
            "exactly one organization is the current one"
        );
        assert_eq!(current[0]["id"], "organization-a");

        crate::testing::cleanup_database(&path);
    }

    #[tokio::test]
    async fn the_membership_list_is_the_callers_own() {
        // `viewer` belongs only to organization-a; `editor` to both. The list is
        // the caller's membership, never every organization on the install.
        let (state, path) = crate::testing::test_state().await;

        let outcome = run(
            &state,
            "organization-a",
            "conversation",
            &crate::testing::auth_user("viewer"),
            GET,
            &json!({ "resource": "organizations" }).to_string(),
        )
        .await;

        let value: serde_json::Value = serde_json::from_str(&outcome.result).unwrap();
        let ids: Vec<String> = value["organizations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["id"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(ids, vec!["organization-a".to_string()]);

        crate::testing::cleanup_database(&path);
    }

    #[tokio::test]
    async fn another_organization_is_not_reachable_even_by_exact_id() {
        // Every read resolves against `catalog`, which is scoped to the caller's
        // organization. A foreign id therefore matches nothing rather than
        // returning someone else's data.
        let (state, path) = crate::testing::test_state().await;
        let (_, _, mine) = crate::testing::seed_application(&state, "organization-a").await;
        let (_, _, theirs) = crate::testing::seed_application(&state, "organization-b").await;

        let outcome = run(
            &state,
            "organization-a",
            "conversation",
            &crate::testing::auth_user("editor"),
            GET,
            &json!({ "resource": "environment", "application": theirs }).to_string(),
        )
        .await;

        assert!(
            outcome.result.contains("\"error\""),
            "an id from another organization returned data: {}",
            outcome.result
        );
        // Echoing the id back is fine - the caller typed it. What must not
        // appear is anything belonging to the other organization, and the
        // candidates come only from the caller's own catalog.
        assert!(
            !outcome.result.contains("organization-b-project"),
            "a miss leaked the other organization's catalog: {}",
            outcome.result
        );

        // The caller's own application still resolves, so the guard is scoping
        // and not a blanket failure.
        let ours = run(
            &state,
            "organization-a",
            "conversation",
            &crate::testing::auth_user("editor"),
            GET,
            &json!({ "resource": "environment", "application": mine }).to_string(),
        )
        .await;
        assert!(!ours.result.contains("\"error\""), "{}", ours.result);

        crate::testing::cleanup_database(&path);
    }

    #[test]
    fn origin_says_where_an_application_came_from() {
        assert_eq!(
            origin_of(Some("postgres"), None),
            "internal template (postgres)"
        );
        assert_eq!(
            origin_of(None, Some("plausible")),
            "catalog template (plausible)"
        );
        assert_eq!(origin_of(None, None), "created by the user");
    }
}
