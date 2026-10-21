//! Before an MCP agent deploys a compose file it wrote, the catalog gets a say.
//!
//! A template on templates.litepod.sh is maintained, has its secrets and
//! domain markers worked out, and updates with the catalog; a compose an agent
//! wrote from memory has none of that. So the first `deploy_compose` over MCP
//! that names something the catalog has stops here and hands the matches back,
//! for the agent to ask its human. Only a second call with `use_own_compose`
//! goes on to propose. The in-app chat is not stopped: its panel already
//! shows the user every card before anything happens.

use serde_json::{Value, json};

use crate::templates::catalog_templates::{CatalogClient, CatalogQuery};

/// Enough to recognise the app; more would be the catalog, not a question.
const MAX_MATCHES: usize = 5;

/// The catalog templates the user should hear about before this compose is
/// deployed, as the tool result to return instead. `None` lets the call
/// through: the user already chose, nothing matched, or the catalog cannot be
/// reached (it is advice, never a reason to refuse a deploy).
pub async fn templates_first(arguments: &Value) -> Option<Value> {
    if arguments.get("use_own_compose").and_then(Value::as_bool) == Some(true) {
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
    if let Some(name) = arguments.get("name").and_then(Value::as_str) {
        terms.push(name.trim().to_owned());
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
        assert!(templates_first(&arguments).await.is_none());
    }
}
