//! Update notices for applications created from a catalog template.
//!
//! LitePod keeps a snapshot of the template an application was created from
//! (`catalog_base_*`). When the catalog publishes a different version the three
//! of them - the snapshot, what the application runs now, and the new template -
//! are compared so the user can see what an update would change and decide.
//! Nothing here deploys: applying rewrites the stored compose/env and the user
//! redeploys when ready.

use std::collections::{BTreeMap, BTreeSet};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use similar::{ChangeTag, TextDiff};
use utoipa::ToSchema;

use super::catalog::{
    CatalogClient, DecodedTemplate, catalog_ports, decode, validate_compose,
    validate_domain_markers,
};
use crate::{
    deployment::{self, compose::store::ComposeFileStore},
    dtos::response::AppError,
    git::repositories::project,
    state::state::SharedState,
};

/// How the template's compose file relates to the one the application runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ComposeStatus {
    /// Nothing to change: identical, or only the user edited it.
    Unchanged,
    /// The template changed and the user never touched the compose file.
    Clean,
    /// The template changed and the running compose differs from the original
    /// template (or LitePod has no record of the original). Replacing it
    /// discards those edits.
    Customized,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DiffLine {
    /// `equal`, `insert` (in the new template) or `delete` (running today).
    pub tag: &'static str,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ImageChange {
    pub service: String,
    pub from: Option<String>,
    pub to: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ComposeChange {
    pub status: ComposeStatus,
    pub images: Vec<ImageChange>,
    pub diff: Vec<DiffLine>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, ToSchema)]
pub struct EnvAdded {
    pub name: String,
    /// The template's value; a `{{marker}}` is shown as is and becomes a
    /// generated secret when the update is applied.
    pub value: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, ToSchema)]
pub struct EnvDefaultChange {
    pub name: String,
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq, ToSchema)]
pub struct EnvChange {
    /// Variables the new template defines that the application lacks.
    pub added: Vec<EnvAdded>,
    /// Variables the application has that the new template dropped.
    pub removed: Vec<String>,
    /// Template defaults that moved. Informational: the application's own
    /// value is never overwritten.
    pub changed_defaults: Vec<EnvDefaultChange>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CatalogUpdateDto {
    pub template_id: String,
    pub current_version: Option<String>,
    pub latest_version: String,
    pub update_available: bool,
    /// The user skipped `latest_version`.
    pub ignored: bool,
    /// False for applications created before updates were tracked: LitePod
    /// cannot tell the user's edits from the template's.
    pub has_baseline: bool,
    pub compose: ComposeChange,
    pub env: EnvChange,
    pub requires_https: bool,
    pub url_persisted: bool,
    pub setup: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ComposeChoice {
    Replace,
    Keep,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ApplyCatalogUpdateRequest {
    /// The version the user reviewed; refused if the catalog moved on since.
    pub version: String,
    pub compose: ComposeChoice,
    /// Add the variables the new template requires. On by default.
    #[serde(default = "default_true")]
    pub add_env: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct IgnoreCatalogUpdateRequest {
    pub version: String,
}

fn images(compose: &str) -> BTreeMap<String, String> {
    let Ok(document) = serde_yaml::from_str::<serde_yaml::Value>(compose) else {
        return BTreeMap::new();
    };
    let Some(services) = document
        .get("services")
        .and_then(|value| value.as_mapping())
    else {
        return BTreeMap::new();
    };
    services
        .iter()
        .filter_map(|(name, service)| {
            Some((
                name.as_str()?.to_owned(),
                service.get("image")?.as_str()?.to_owned(),
            ))
        })
        .collect()
}

pub(crate) fn image_changes(current: &str, latest: &str) -> Vec<ImageChange> {
    let (current, latest) = (images(current), images(latest));
    let services: BTreeSet<&String> = current.keys().chain(latest.keys()).collect();
    services
        .into_iter()
        .filter(|service| current.get(*service) != latest.get(*service))
        .map(|service| ImageChange {
            service: service.clone(),
            from: current.get(service).cloned(),
            to: latest.get(service).cloned(),
        })
        .collect()
}

pub(crate) fn line_diff(current: &str, latest: &str) -> Vec<DiffLine> {
    TextDiff::from_lines(current, latest)
        .iter_all_changes()
        .map(|change| DiffLine {
            tag: match change.tag() {
                ChangeTag::Equal => "equal",
                ChangeTag::Insert => "insert",
                ChangeTag::Delete => "delete",
            },
            text: change.value().trim_end_matches('\n').to_owned(),
        })
        .collect()
}

pub(crate) fn compose_status(base: Option<&str>, current: &str, latest: &str) -> ComposeStatus {
    if current == latest {
        return ComposeStatus::Unchanged;
    }
    match base {
        Some(base) if base == latest => ComposeStatus::Unchanged,
        Some(base) if base == current => ComposeStatus::Clean,
        _ => ComposeStatus::Customized,
    }
}

pub(crate) fn env_change(
    base: Option<&[(String, String)]>,
    current: &[(String, String)],
    latest: &[(String, String)],
) -> EnvChange {
    let current_names: BTreeSet<&str> = current.iter().map(|(name, _)| name.as_str()).collect();
    let latest_names: BTreeSet<&str> = latest.iter().map(|(name, _)| name.as_str()).collect();
    let added = latest
        .iter()
        .filter(|(name, _)| !current_names.contains(name.as_str()))
        .map(|(name, value)| EnvAdded {
            name: name.clone(),
            value: value.clone(),
        })
        .collect();
    let Some(base) = base else {
        return EnvChange {
            added,
            ..EnvChange::default()
        };
    };
    let base_values: BTreeMap<&str, &str> = base
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    let removed = base
        .iter()
        .filter(|(name, _)| {
            !latest_names.contains(name.as_str()) && current_names.contains(name.as_str())
        })
        .map(|(name, _)| name.clone())
        .collect();
    let changed_defaults = latest
        .iter()
        .filter(|(name, _)| current_names.contains(name.as_str()))
        .filter_map(|(name, value)| {
            let from = base_values.get(name.as_str())?;
            (*from != value).then(|| EnvDefaultChange {
                name: name.clone(),
                from: (*from).to_owned(),
                to: value.clone(),
            })
        })
        .collect();
    EnvChange {
        added,
        removed,
        changed_defaults,
    }
}

async fn latest_template(template_id: &str) -> Result<DecodedTemplate, AppError> {
    decode(CatalogClient::new()?.get(template_id).await?)
}

async fn current_state(
    state: &SharedState,
    application_id: &str,
) -> Result<(String, Vec<(String, String)>), AppError> {
    let compose = ComposeFileStore::new(&state.data_dir, application_id)
        .read_content()
        .await
        .map_err(AppError::InternalError)?;
    let env = deployment::decrypted_environment(state, application_id, "runtime").await?;
    Ok((compose, env))
}

/// Compares the application with the newest template. `Ok(None)` when the
/// application did not come from the catalog.
pub async fn check(
    state: &SharedState,
    application_id: &str,
) -> Result<Option<CatalogUpdateDto>, AppError> {
    let Some(base) = project::get_catalog_base(state, application_id)
        .await
        .map_err(AppError::DatabaseError)?
    else {
        return Ok(None);
    };
    let latest = latest_template(&base.template_id).await?;
    compare(state, application_id, base, latest).await.map(Some)
}

async fn compare(
    state: &SharedState,
    application_id: &str,
    base: project::CatalogBase,
    latest: DecodedTemplate,
) -> Result<CatalogUpdateDto, AppError> {
    let (compose, env) = current_state(state, application_id).await?;
    let base_env = base
        .env
        .as_deref()
        .map(deployment::parse_environment_content)
        .transpose()?;
    let latest_env = deployment::parse_environment_content(&latest.environment)?;
    let has_baseline = base.version.is_some() && base.compose.is_some();
    let compose_change = ComposeChange {
        status: compose_status(base.compose.as_deref(), &compose, &latest.compose),
        images: image_changes(&compose, &latest.compose),
        diff: line_diff(&compose, &latest.compose),
    };
    let env_change = env_change(base_env.as_deref(), &env, &latest_env);
    let version = latest.template.version.clone();
    let differs = compose_change.status != ComposeStatus::Unchanged
        || !env_change.added.is_empty()
        || !env_change.removed.is_empty()
        || !env_change.changed_defaults.is_empty();
    // Without a recorded version the only evidence is the content itself.
    let newer = match &base.version {
        Some(current) => *current != version,
        None => differs,
    };
    let ignored = base.ignored_version.as_deref() == Some(version.as_str());
    Ok(CatalogUpdateDto {
        template_id: base.template_id,
        current_version: base.version,
        update_available: newer && !ignored,
        ignored: newer && ignored,
        latest_version: version,
        has_baseline,
        compose: compose_change,
        env: env_change,
        requires_https: latest.template.requires_https,
        url_persisted: latest.template.url_persisted,
        setup: (!latest.setup.is_empty()).then_some(latest.setup),
    })
}

/// How long a template fetched for the application cards is reused. Cards are
/// drawn on every page load; the catalog is a network call.
const CARD_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(300);

type CardCache = std::sync::Mutex<
    std::collections::HashMap<String, (std::time::Instant, std::sync::Arc<DecodedTemplate>)>,
>;

fn card_cache() -> &'static CardCache {
    static CACHE: std::sync::OnceLock<CardCache> = std::sync::OnceLock::new();
    CACHE.get_or_init(Default::default)
}

async fn cached_template(template_id: &str) -> Result<std::sync::Arc<DecodedTemplate>, AppError> {
    if let Some((fetched, template)) = card_cache().lock().unwrap().get(template_id) {
        if fetched.elapsed() < CARD_CACHE_TTL {
            return Ok(template.clone());
        }
    }
    let template = std::sync::Arc::new(latest_template(template_id).await?);
    card_cache().lock().unwrap().insert(
        template_id.to_owned(),
        (std::time::Instant::now(), template.clone()),
    );
    Ok(template)
}

/// The applications of one environment that have a template update waiting,
/// as `application id -> new template version`, for the "update" tag on their
/// cards. An unreachable catalog or a broken application never fails the
/// list: it only means no tag.
pub async fn pending_for_environment(
    state: &SharedState,
    environment_id: &str,
) -> Result<BTreeMap<String, String>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let ids: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM applications WHERE environment_id = ?1 AND catalog_template_id IS NOT NULL",
    )
    .bind(environment_id)
    .fetch_all(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    let mut pending = BTreeMap::new();
    for id in ids {
        let Ok(Some(base)) = project::get_catalog_base(state, &id).await else {
            continue;
        };
        let Ok(latest) = cached_template(&base.template_id).await else {
            continue;
        };
        if let Ok(update) = compare(state, &id, base, (*latest).clone()).await {
            if update.update_available {
                pending.insert(id, update.latest_version);
            }
        }
    }
    Ok(pending)
}

/// Moves the application to the template version the user reviewed. Rewrites
/// the stored compose (if chosen) and adds missing variables; it never touches
/// a value the application already has, and never redeploys.
pub async fn apply(
    state: &SharedState,
    application_id: &str,
    request: &ApplyCatalogUpdateRequest,
) -> Result<CatalogUpdateDto, AppError> {
    let base = project::get_catalog_base(state, application_id)
        .await
        .map_err(AppError::DatabaseError)?
        .ok_or_else(|| {
            AppError::ValidationError("Application was not created from a template".into())
        })?;
    let latest = latest_template(&base.template_id).await?;
    if latest.template.version != request.version {
        return Err(AppError::ValidationError(format!(
            "The template is now at {}, not {}. Review the update again.",
            latest.template.version, request.version
        )));
    }
    deployment::validate_environment_content(&latest.environment)?;
    validate_domain_markers(&latest)?;
    validate_compose(&latest.compose)?;
    let (compose, env) = current_state(state, application_id).await?;
    if request.compose == ComposeChoice::Replace && compose != latest.compose {
        ComposeFileStore::new(&state.data_dir, application_id)
            .write_content(&latest.compose)
            .await
            .map_err(AppError::InternalError)?;
    }
    if request.add_env {
        let latest_env = deployment::parse_environment_content(&latest.environment)?;
        let current_names: BTreeSet<&str> = env.iter().map(|(name, _)| name.as_str()).collect();
        for (name, value) in latest_env
            .iter()
            .filter(|(name, _)| !current_names.contains(name.as_str()))
        {
            deployment::set_environment_variable(state, application_id, "runtime", name, value)
                .await?;
        }
    }
    let conn = crate::git::repositories::db::get_conn(state).await;
    let primary = latest
        .template
        .ports
        .iter()
        .find(|port| port.primary)
        .or(latest.template.ports.first())
        .map(|port| port.container);
    project::set_catalog_metadata(
        &conn,
        application_id,
        &latest.template.id,
        latest.icon_url.as_deref(),
        primary,
        Utc::now().timestamp_millis(),
    )
    .await
    .map_err(AppError::DatabaseError)?;
    project::set_catalog_ports(
        &conn,
        application_id,
        &catalog_ports(&latest.template),
        latest.template.requires_https,
        latest.template.url_persisted,
    )
    .await
    .map_err(AppError::DatabaseError)?;
    project::set_catalog_base(
        &conn,
        application_id,
        &latest.template.version,
        &latest.compose,
        &latest.environment,
    )
    .await
    .map_err(AppError::DatabaseError)?;
    check(state, application_id)
        .await?
        .ok_or_else(|| AppError::InternalError("Application lost its template".into()))
}

pub async fn ignore(
    state: &SharedState,
    application_id: &str,
    request: &IgnoreCatalogUpdateRequest,
) -> Result<(), AppError> {
    project::set_catalog_ignored_version(state, application_id, &request.version)
        .await
        .map_err(AppError::DatabaseError)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(name, value)| ((*name).into(), (*value).into()))
            .collect()
    }

    #[test]
    fn untouched_compose_updates_cleanly() {
        assert_eq!(compose_status(Some("a"), "a", "b"), ComposeStatus::Clean);
    }

    #[test]
    fn edited_compose_is_customized() {
        assert_eq!(
            compose_status(Some("a"), "x", "b"),
            ComposeStatus::Customized
        );
        assert_eq!(compose_status(None, "x", "b"), ComposeStatus::Customized);
    }

    #[test]
    fn user_edits_alone_are_not_an_update() {
        assert_eq!(
            compose_status(Some("a"), "x", "a"),
            ComposeStatus::Unchanged
        );
        assert_eq!(
            compose_status(Some("a"), "b", "b"),
            ComposeStatus::Unchanged
        );
    }

    #[test]
    fn reports_image_bumps_and_new_services() {
        let old = "services:\n  web:\n    image: docker.io/x/web:1\n";
        let new =
            "services:\n  web:\n    image: docker.io/x/web:2\n  db:\n    image: docker.io/x/db:1\n";
        let changes = image_changes(old, new);
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].service, "db");
        assert_eq!(changes[0].from, None);
        assert_eq!(changes[1].from.as_deref(), Some("docker.io/x/web:1"));
        assert_eq!(changes[1].to.as_deref(), Some("docker.io/x/web:2"));
    }

    #[test]
    fn diff_marks_inserted_and_deleted_lines() {
        let diff = line_diff("a\nb\n", "a\nc\n");
        let tags: Vec<_> = diff.iter().map(|line| line.tag).collect();
        assert_eq!(tags, ["equal", "delete", "insert"]);
    }

    #[test]
    fn new_template_variables_are_required_additions() {
        let change = env_change(
            Some(&env(&[("A", "1")])),
            &env(&[("A", "1")]),
            &env(&[("A", "1"), ("KEY", "{{password_32}}")]),
        );
        assert_eq!(
            change.added,
            [EnvAdded {
                name: "KEY".into(),
                value: "{{password_32}}".into()
            }]
        );
    }

    #[test]
    fn user_set_variables_are_never_reported_as_added() {
        let change = env_change(None, &env(&[("A", "mine")]), &env(&[("A", "1")]));
        assert_eq!(change, EnvChange::default());
    }

    #[test]
    fn dropped_and_moved_defaults_need_a_baseline() {
        let base = env(&[("A", "1"), ("B", "1")]);
        let current = env(&[("A", "custom"), ("B", "1")]);
        let latest = env(&[("A", "2")]);
        let change = env_change(Some(&base), &current, &latest);
        assert_eq!(change.removed, ["B"]);
        assert_eq!(change.changed_defaults.len(), 1);
        assert_eq!(change.changed_defaults[0].to, "2");
        let blind = env_change(None, &current, &latest);
        assert!(blind.removed.is_empty() && blind.changed_defaults.is_empty());
    }
}
