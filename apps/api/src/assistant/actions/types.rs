use serde::{Deserialize, Serialize};

use crate::state::state::SharedState;

use super::lifecycle::StoredAction;

/// The write tools. Each one stores an action here instead of acting.
pub const SET_ENVIRONMENT_VARIABLE: &str = "set_environment_variable";

pub const REDEPLOY_APPLICATION: &str = "redeploy_application";

pub const START_APPLICATION: &str = "start_application";

pub const STOP_APPLICATION: &str = "stop_application";

pub const RESTART_APPLICATION: &str = "restart_application";

pub const ROLLBACK_APPLICATION: &str = "rollback_application";

pub const SET_COMPOSE_FILE: &str = "set_compose_file";

pub const DELETE_ENVIRONMENT_VARIABLE: &str = "delete_environment_variable";

pub const UPDATE_APPLICATION: &str = "update_application";

pub const SET_SOURCE: &str = "set_source";

pub const CREATE_APPLICATION: &str = "create_application";

pub const CREATE_PROJECT: &str = "create_project";

pub const UPDATE_PROJECT: &str = "update_project";

pub const DELETE_PROJECT: &str = "delete_project";

pub const ADD_DOMAIN: &str = "add_domain";

pub const UPDATE_DOMAIN: &str = "update_domain";

pub const DELETE_DOMAIN: &str = "delete_domain";

pub const DEPLOY_STATIC_SITE: &str = "deploy_static_site";

pub const CREATE_FROM_TEMPLATE: &str = "create_from_template";

pub const DEPLOY_COMPOSE: &str = "deploy_compose";

pub const DEPLOY_ZIP: &str = "deploy_zip";

pub const DEPLOY_REPOSITORY: &str = "deploy_repository";

/// Litepod's own database templates. Anything else is a catalog template id.
pub const INTERNAL_TEMPLATES: &[&str] = &[
    "postgres",
    "mariadb",
    "mongodb",
    "dragonflydb",
    "keydb",
    "sqlserver",
];

/// The branch a repository is assumed to deploy from when nobody says
/// otherwise. Named here rather than left to the model, so a forgotten field
/// cannot become a deploy from whatever branch it invented.
pub(super) const DEFAULT_BRANCH: &str = "main";

/// How a deployment the assistant queued shows up in the run history, next to
/// `manual_deploy` and the webhook triggers.
///
/// `triggered_by` is a closed CHECK list on `application_deployment_runs`;
/// this value was added by `migrations/0025_assistant_deploy_trigger.sql`, and
/// `the_deploy_trigger_is_one_the_schema_accepts` fails if the two drift.
pub(super) const ASSISTANT_TRIGGER: &str = "assistant_deploy";

/// An action is only good for as long as the reading it was decided from is
/// plausibly still current.
pub(super) const TTL_MS: i64 = 15 * 60 * 1_000;

pub(super) const MAX_VALUE_BYTES: usize = 4_096;

/// A compose file the assistant proposes. Big enough for anything real, small
/// enough that a runaway generation is refused rather than written.
pub(super) const MAX_COMPOSE_BYTES: usize = 64_000;

/// Validated arguments for [`SET_ENVIRONMENT_VARIABLE`]. The shape the model's
/// tool schema produces, and the shape replayed at confirm time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetEnvironmentVariable {
    pub name: String,
    /// Empty for a delete, which needs the name and nothing else.
    #[serde(default)]
    pub value: String,
    /// `runtime` or `build`. Defaults to `runtime`, which is what a question
    /// about "the environment" almost always means.
    #[serde(default = "runtime_scope")]
    pub scope: String,
    /// The one line the user reads before deciding.
    #[serde(default)]
    pub reason: Option<String>,
}

pub(super) fn runtime_scope() -> String {
    "runtime".into()
}

/// Validated arguments for [`ROLLBACK_APPLICATION`]: the run to go back to,
/// already resolved at proposal time so a later deploy cannot change what the
/// card promised.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rollback {
    pub run: String,
}

/// Validated arguments for [`RESTART_APPLICATION`]. No container means the
/// whole application.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Restart {
    #[serde(default)]
    pub container: Option<String>,
}

/// Validated arguments for [`SET_COMPOSE_FILE`]. Always the whole document: a
/// fragment would have to be merged, and merging someone else's YAML by guess
/// is how a service disappears.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetComposeFile {
    pub compose: String,
    /// The one line the user reads before deciding.
    pub reason: Option<String>,
}

/// Validated arguments for [`UPDATE_APPLICATION`]. Both optional, at least one
/// present - a card that changes nothing is a card nobody should be shown.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateApplication {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
}

/// Validated arguments for [`SET_SOURCE`].
///
/// Every field is optional and only the ones present change: the confirm reads
/// the current configuration and merges, so a proposal about the branch cannot
/// quietly reset the build method.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SetSource {
    #[serde(default)]
    pub git_provider_id: Option<String>,
    #[serde(default)]
    pub git_url: Option<String>,
    #[serde(default)]
    pub git_branch: Option<String>,
    #[serde(default)]
    pub railpack_project_path: Option<String>,
    /// Path of the Dockerfile inside the repository, for `docker`.
    #[serde(default)]
    pub dockerfile_path: Option<String>,
    /// Path of the compose file inside the repository, for `compose`.
    #[serde(default)]
    pub compose_file_path: Option<String>,
    /// The compose service the proxy routes to, for `compose`.
    #[serde(default)]
    pub compose_public_service: Option<String>,
    /// `railpack`, `docker` or `compose`. Setting it, or a repository, makes
    /// the application a git deploy; a new repository without one is `railpack`.
    #[serde(default)]
    pub build_method: Option<String>,
    /// The port the built container listens on; git deploys require one.
    #[serde(default)]
    pub container_port: Option<i64>,
    #[serde(default)]
    pub auto_deploy: Option<bool>,
    #[serde(default)]
    pub watch_paths: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
}

/// Validated arguments for [`CREATE_APPLICATION`].
///
/// `compose` is optional: without it the application is created empty and its
/// source is configured afterwards with `set_source`. With it, the application
/// is a Compose application from the moment it exists, which is the only way
/// to get one without a template.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CreateApplication {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// A ready server of this organization. Resolved before the card is stored.
    #[serde(default)]
    pub server_id: String,
    #[serde(default)]
    pub compose: Option<String>,
    /// The container port the proxy should route to. Names the Compose service
    /// that publishes it, so the application gets a public URL.
    #[serde(default)]
    pub port: Option<i64>,
    #[serde(default)]
    pub reason: Option<String>,
}

/// Validated arguments for [`CREATE_PROJECT`], [`UPDATE_PROJECT`] and
/// [`DELETE_PROJECT`]. One shape for all three: a delete carries only the
/// reason, and the project it targets is on the action row, not in here.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectFields {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
}

/// Validated arguments for [`ADD_DOMAIN`], [`UPDATE_DOMAIN`] and
/// [`DELETE_DOMAIN`].
///
/// `domain_id` is resolved when the action is proposed, never taken from the
/// model: it named a host, and the row it meant is looked up once so the
/// confirm cannot land on a different domain than the card described.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DomainFields {
    #[serde(default)]
    pub domain_id: Option<String>,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub port: Option<i64>,
    #[serde(default)]
    pub https: Option<bool>,
    #[serde(default)]
    pub is_primary: Option<bool>,
    #[serde(default)]
    pub reason: Option<String>,
}

/// Validated arguments for [`DEPLOY_STATIC_SITE`]: a project (existing or
/// new), a static application in it, the site uploaded to the action's own URL,
/// a deploy, and optionally a domain. One card, so one confirmation.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StaticSite {
    /// Created on confirm when the project does not exist yet.
    pub project_name: String,
    pub name: String,
    #[serde(default)]
    pub server_id: String,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub https: Option<bool>,
    /// SHA-256 of the upload token: the token itself is only ever handed to the
    /// agent that proposed this, never stored.
    #[serde(default)]
    pub upload_token_hash: String,
    #[serde(default)]
    pub reason: Option<String>,
    /// An application of that name the site goes into instead of a new one:
    /// one never deployed and with no source, or an existing static site.
    #[serde(default)]
    pub application_id: Option<String>,
}

/// Validated arguments for [`DEPLOY_ZIP`] and [`DEPLOY_REPOSITORY`]: a project
/// (existing or new), an application built from source - an uploaded ZIP or a
/// Git repository - an optional domain, and a deploy. One card.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SourceApp {
    pub project_name: String,
    pub name: String,
    #[serde(default)]
    pub server_id: String,
    /// `railpack` (default) or `docker`.
    #[serde(default)]
    pub build_method: String,
    /// The folder of the project inside the ZIP or repository, for Railpack.
    #[serde(default)]
    pub project_path: Option<String>,
    #[serde(default)]
    pub dockerfile_path: Option<String>,
    #[serde(default)]
    pub container_port: i64,
    /// Repository deploys only.
    #[serde(default)]
    pub git_url: Option<String>,
    #[serde(default)]
    pub git_branch: Option<String>,
    #[serde(default)]
    pub git_provider_id: Option<String>,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub https: Option<bool>,
    /// ZIP deploys only; see [`StaticSite::upload_token_hash`].
    #[serde(default)]
    pub upload_token_hash: String,
    #[serde(default)]
    pub reason: Option<String>,
    /// An application of that name the source goes into instead of a new one.
    #[serde(default)]
    pub application_id: Option<String>,
}

/// Validated arguments for [`CREATE_FROM_TEMPLATE`]: a project (existing or
/// new), an application from an internal or catalog template, the variable
/// values the user gave, an optional domain, and a deploy. One card.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FromTemplate {
    pub project_name: String,
    pub template: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub server_id: String,
    /// Runtime variables the user wrote, by name. Only for catalog templates:
    /// a database template generates its own credentials.
    #[serde(default)]
    pub values: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub https: Option<bool>,
    /// The template's primary container port, resolved when proposed.
    #[serde(default)]
    pub port: Option<i64>,
    #[serde(default)]
    pub reason: Option<String>,
}

/// Validated arguments for [`DEPLOY_COMPOSE`]: a Compose application written
/// the way a catalog template is - the compose file plus its `.env` - in a
/// project (existing or new), with an optional domain, then a deploy. One card.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ComposeApp {
    pub project_name: String,
    pub name: String,
    #[serde(default)]
    pub server_id: String,
    pub compose: String,
    /// `.env` content: `KEY=VALUE` lines. Secret markers (`{{password_32}}`)
    /// are expanded on confirm, never stored in the action; domain markers
    /// (`{{url}}`) stay and resolve on every deploy.
    #[serde(default)]
    pub env: String,
    /// The container port the proxy routes to; it is also what `{{url}}` names.
    #[serde(default)]
    pub port: Option<i64>,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub https: Option<bool>,
    #[serde(default)]
    pub reason: Option<String>,
}

/// Where an action lives, resolved once so the confirm never re-derives it.
#[derive(Debug, Clone)]
pub struct ActionTarget {
    pub organization_id: String,
    pub project_id: String,
    pub project_name: String,
    pub environment_id: String,
    /// Empty for an action about a project rather than one application.
    pub application_id: String,
    pub application_name: String,
}

impl ActionTarget {
    /// A project with no application under it, for the project-scoped tools.
    pub fn project(organization_id: &str, project_id: &str, project_name: &str) -> Self {
        Self {
            organization_id: organization_id.to_owned(),
            project_id: project_id.to_owned(),
            project_name: project_name.to_owned(),
            environment_id: String::new(),
            application_id: String::new(),
            application_name: String::new(),
        }
    }

    /// The page in the UI where this change happens, so the panel can take the
    /// user there rather than describing a screen they have to find.
    ///
    /// This is a *UI* route, not an API path: no organization segment, because
    /// the browser reads that from the selected organization. It mirrors
    /// `ui/src/routes/(app)/projects/[project_id]/environments/[environment_id]/applications/[application_id]`.
    ///
    /// The tool decides the tab, so a domain change does not drop the user on
    /// the environment editor and a project change does not point at an
    /// application page that may not exist yet.
    pub fn route(&self, tool: &str) -> String {
        match tool {
            // The list, for all three: the panel navigates before it applies,
            // so a create has no page yet and a delete's page is about to stop
            // existing - and a rename is read on the list anyway.
            CREATE_PROJECT | UPDATE_PROJECT | DELETE_PROJECT | DEPLOY_STATIC_SITE
            | CREATE_FROM_TEMPLATE | DEPLOY_COMPOSE | DEPLOY_ZIP | DEPLOY_REPOSITORY => {
                "/projects".into()
            }
            // The application does not exist yet, so the environment it lands
            // in is the closest page there is.
            CREATE_APPLICATION => format!(
                "/projects/{}/environments/{}",
                self.project_id, self.environment_id
            ),
            _ => {
                let tab = match tool {
                    ADD_DOMAIN | UPDATE_DOMAIN | DELETE_DOMAIN => "domains",
                    _ => "environment",
                };
                format!(
                    "/projects/{}/environments/{}/applications/{}?tab={tab}",
                    self.project_id, self.environment_id, self.application_id
                )
            }
        }
    }
}

/// What the panel renders. The value is never echoed back: an action card has
/// to be readable over someone's shoulder.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct ActionDto {
    pub id: String,
    pub tool: String,
    pub title: String,
    pub summary: String,
    pub status: String,
    /// Where in the UI the change happens.
    pub route: String,
    /// The application the change targets, so the panel can broadcast an
    /// `application-updated` event after apply.
    pub application_id: String,
    /// The line the change adds, with the value masked.
    pub added: Option<String>,
    /// Present when the key already exists, so the card can say it replaces.
    pub replaces: Option<String>,
    /// The name the change removes.
    pub removed: Option<String>,
    /// A truncated unified diff of the change. Empty for tools that have no
    /// meaningful line-level delta (the env-var cards keep their masked `added`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff: Option<String>,
    pub expires_at: i64,
    pub failure: Option<String>,
}

/// Cap so a chat card never ships a multi-thousand-line compose file. The
/// truncation happens tail-first: keeping the start preserves the file
/// header (and the changed keys, since most edits land near the top), which
/// is also what a developer would scroll to.
pub(super) const DIFF_PREVIEW_BYTES: usize = 4 * 1024;

/// Renders a unified diff of `before` → `after`, capped to a few KB so the
/// action card stays small. Used by `set_compose_file`; line-based tools get
/// a real `added`/`removed` summary instead.
pub(super) fn compose_diff(before: &str, after: &str) -> String {
    use similar::{ChangeTag, TextDiff};

    let diff = TextDiff::from_lines(before, after);
    let mut out = String::new();
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Equal => out.push_str(&format!(" {}\n", change)),
            ChangeTag::Insert => out.push_str(&format!("+{}\n", change)),
            ChangeTag::Delete => out.push_str(&format!("-{}\n", change)),
        }
    }
    if out.len() > DIFF_PREVIEW_BYTES {
        // Truncate by line count so we don't break mid-UTF-8, then append an
        // honest marker. The full file is always recoverable from the editor
        // the action's "route" links to.
        let mut truncated = String::with_capacity(DIFF_PREVIEW_BYTES);
        let mut bytes = 0;
        for line in out.lines() {
            let next = bytes + line.len() + 1;
            if next > DIFF_PREVIEW_BYTES {
                break;
            }
            truncated.push_str(line);
            truncated.push('\n');
            bytes = next;
        }
        truncated.push_str(&format!(
            "\n… diff truncated, full file is in the editor.\n"
        ));
        return truncated;
    }
    out
}

/// An action card is read over someone's shoulder and the value may be a
/// secret, so the card shows the shape of the value, never the value.
pub(super) fn mask(value: &str) -> String {
    let visible: String = value.chars().take(4).collect();
    format!("{visible}\u{2026} ({} characters)", value.chars().count())
}

/// What a write tool did, in the two shapes the caller still needs.
pub(super) struct Applied {
    pub(super) audit_action: &'static str,
    /// The line the panel shows on the settled card, value masked.
    pub(super) added: Option<String>,
    /// The name the change removed, when it removed one.
    pub(super) removed: Option<String>,
}

/// Writes the outcome into the conversation, so the next turn knows what the
/// user decided without them having to retype it. Best effort: an action that
/// applied must not fail because a note could not be stored.
pub(super) async fn note(state: &SharedState, stored: &StoredAction, outcome: &str) {
    let text = format!("The change \"{}\" was {}.", stored.title, outcome);
    let _ = crate::assistant::conversation::store_message(
        state,
        &stored.conversation_id,
        "note",
        &text,
        None,
        None,
        None,
    )
    .await;
}
