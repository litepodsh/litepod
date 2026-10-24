//! Before an MCP agent deploys a compose file it wrote, the catalog gets a say.
//!
//! A template on templates.litepod.sh is maintained, has its secrets and
//! domain markers worked out, and updates with the catalog; a compose an agent
//! wrote from memory has none of that. So the first `deploy_compose` over MCP
//! that names something the catalog has stops here and hands the matches back,
//! for the agent to ask its human. Only a second call with `use_own_compose`
//! goes on to propose, and only after this check has answered once for the
//! same caller and app: an agent cannot skip the question by sending the flag
//! up front. The in-app chat is not stopped: its panel already shows the user
//! every card before anything happens.

use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
    time::{Duration, Instant},
};

use serde_json::{Value, json};

use crate::templates::catalog_templates::{CatalogClient, CatalogQuery};

/// Enough to recognise the app; more would be the catalog, not a question.
const MAX_MATCHES: usize = 5;

/// How long an answered question stays answered. Long enough for a person to
/// read the matches and reply; short enough that a later session is asked again.
const ASKED_TTL: Duration = Duration::from_secs(30 * 60);

/// Callers (organization, user, app) the catalog already answered for.
static ASKED: LazyLock<Mutex<HashMap<String, Instant>>> = LazyLock::new(Default::default);

fn asked_key(caller: &str, arguments: &Value) -> String {
    let app = ["name", "application"]
        .iter()
        .find_map(|key| arguments.get(key).and_then(Value::as_str))
        .unwrap_or_default();
    format!(
        "{caller}:{}",
        app.chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect::<String>()
    )
}

fn was_asked(key: &str) -> bool {
    let mut asked = ASKED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    asked.retain(|_, at| at.elapsed() < ASKED_TTL);
    asked.contains_key(key)
}

fn remember_asked(key: String) {
    ASKED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(key, Instant::now());
}

/// The catalog templates the user should hear about before this compose is
/// deployed, as the tool result to return instead. `None` lets the call
/// through: the user already chose, nothing matched, or the catalog cannot be
/// reached (it is advice, never a reason to refuse a deploy).
///
/// `caller` identifies who is asking (organization and user), so
/// `use_own_compose` only counts once this has answered for them.
pub async fn templates_first(caller: &str, arguments: &Value) -> Option<Value> {
    let key = asked_key(caller, arguments);
    if arguments.get("use_own_compose").and_then(Value::as_bool) == Some(true) && was_asked(&key) {
        return None;
    }
    let client = CatalogClient::new().ok()?;
    let mut matches: Vec<Value> = Vec::new();
    for term in search_terms(arguments) {
        let Ok(found) = client
            .list(&CatalogQuery {
                q: Some(term.clone()),
                category: None,
                limit: Some(MAX_MATCHES as u8),
            })
            .await
        else {
            continue;
        };
        // The catalog's search is full text: `n8n` also finds the runners that
        // mention it. Only a template that *is* the thing is worth a question.
        for template in found
            .into_iter()
            .filter(|template| same_app(&term, &template.id) || same_app(&term, &template.name))
        {
            if matches.len() < MAX_MATCHES && !matches.iter().any(|m| m["id"] == template.id) {
                matches.push(json!({
                    "id": template.id,
                    "name": template.name,
                    "description": template.description,
                }));
            }
        }
    }
    if matches.is_empty() {
        return None;
    }
    remember_asked(key);
    Some(json!({
        "done": false,
        "templates_found": matches,
        "next": "Nothing was created. The Litepod catalog already has a template for this. \
            Tell the user which ones and ask before going on: to use one, read `get template` \
            and call `create_from_template`; to deploy their own compose anyway, call \
            `deploy_compose` again with the same arguments plus `use_own_compose: true`.",
    }))
}

/// What the catalog is searched for: the application's name and the image
/// of the service the port routes to (`docker.io/louislam/uptime-kuma:1` is
/// `uptime-kuma`). Not every image: the database beside an app would match
/// the `postgres` template and ask about the wrong thing.
fn search_terms(arguments: &Value) -> Vec<String> {
    let mut terms = Vec::new();
    // `name` creates an app; `application` names the one whose compose is replaced.
    for key in ["name", "application"] {
        if let Some(name) = arguments.get(key).and_then(Value::as_str) {
            terms.push(name.trim().to_owned());
        }
    }
    let compose = arguments
        .get("compose")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let port = arguments
        .get("port")
        .and_then(Value::as_u64)
        .and_then(|port| u16::try_from(port).ok());
    if let Some(service) =
        port.and_then(|port| crate::deployment::compose_standalone::service_for_port(compose, port))
        && let Some(image) = serde_yaml::from_str::<serde_yaml::Value>(compose)
            .ok()
            .and_then(|document| {
                document
                    .get("services")?
                    .get(service.as_str())?
                    .get("image")?
                    .as_str()
                    .map(str::to_owned)
            })
    {
        terms.push(image_name(&image));
    }
    terms.retain(|term| !term.is_empty());
    terms.dedup();
    terms
}

/// `uptime-kuma` and `Uptime Kuma` are the same app; so is `uptimekuma`, and
/// `kuma` is close enough to ask. `my-app` is not `aptabase`.
fn same_app(term: &str, candidate: &str) -> bool {
    let squash = |text: &str| -> String {
        text.chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect()
    };
    let (term, candidate) = (squash(term), squash(candidate));
    term.len() >= 3 && (candidate.contains(&term) || term.contains(&candidate))
}

/// The repository part of an image reference, without registry, owner, tag
/// or digest.
fn image_name(image: &str) -> String {
    let without_digest = image.split('@').next().unwrap_or(image);
    let last = without_digest.rsplit('/').next().unwrap_or(without_digest);
    last.split(':').next().unwrap_or(last).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_image_is_searched_by_its_repository_name() {
        assert_eq!(
            image_name("docker.io/louislam/uptime-kuma:1"),
            "uptime-kuma"
        );
        assert_eq!(image_name("n8nio/n8n"), "n8n");
        assert_eq!(image_name("ghcr.io/a/b/c@sha256:abc"), "c");
    }

    #[test]
    fn only_a_template_that_is_the_app_counts() {
        assert!(same_app("uptimekuma", "uptime-kuma"));
        assert!(same_app("Uptime Kuma", "uptime-kuma"));
        assert!(same_app("kuma", "Uptime Kuma"));
        assert!(!same_app("n8n", "forgejo-runner"));
        assert!(!same_app("mywebapp", "aptabase"));
        assert!(!same_app("db", "dbgate"), "too short to mean anything");
    }

    #[test]
    fn only_the_routed_service_image_is_searched() {
        let arguments = json!({
            "name": "status",
            "port": 3001,
            "compose": "services:\n  kuma:\n    image: louislam/uptime-kuma:1\n    expose: [\"3001\"]\n  db:\n    image: postgres:16\n",
        });
        assert_eq!(search_terms(&arguments), vec!["status", "uptime-kuma"]);
    }

    #[tokio::test]
    async fn the_users_choice_is_not_asked_again() {
        let arguments = json!({ "name": "uptime-kuma", "use_own_compose": true });
        remember_asked(asked_key("org:user", &arguments));
        assert!(templates_first("org:user", &arguments).await.is_none());
    }

    #[test]
    fn the_flag_alone_does_not_count_as_a_choice() {
        let arguments = json!({ "name": "never-asked-app", "use_own_compose": true });
        assert!(!was_asked(&asked_key("org:user", &arguments)));
        // Another caller's answer is not this caller's.
        remember_asked(asked_key("org:other", &arguments));
        assert!(!was_asked(&asked_key("org:user", &arguments)));
    }
}
