//! Searching the Litepod template catalog the way an agent needs it.
//!
//! The catalog's own search is full text and fuzzy: `chat` comes back with
//! sixteen templates, most of them not about chat. So the catalog is fetched
//! and ranked here, over the fields that identify a template - its id, name,
//! tags and categories first, its description last.
//!
//! The point of the ranking is not the list but the answer to the question an
//! agent has to put to a person: one template matches, so use it; several
//! match, so offer them and ask; none does, so say so and ask what to do.

use serde_json::Value;

use crate::{
    dtos::response::AppError,
    templates::catalog_templates::{CatalogClient, CatalogQuery, CatalogTemplate, trusted_ui_url},
};

/// How many matches a search is worth reading. More than this and the agent
/// is back to handing the user the list it was trying to avoid.
pub const MAX_MATCHES: usize = 20;

/// How many templates to offer as choices when the answer is a question.
pub const MAX_CANDIDATES: usize = 6;

/// How many templates a search without a `query` returns: the whole catalog.
const MAX_LISTING: usize = 100;

/// The catalog's page limit, which is also its page size.
const CATALOG_LIMIT: u8 = 100;

/// A template that matched, with how it matched.
#[derive(Debug, Clone)]
pub struct CatalogMatch {
    pub id: String,
    pub name: String,
    pub description: String,
    pub categories: Vec<String>,
    pub tags: Vec<String>,
    /// Where the template lives on templates.litepod.sh, for a user who wants
    /// to look at it before answering.
    pub url: Option<String>,
    /// `id`, `name`, `tag`, `category`, `text` or `description`: what the
    /// query hit. `catalog` for a listing without a query.
    pub kind: &'static str,
    pub score: u16,
}

impl CatalogMatch {
    /// The template as the model reads it.
    pub fn to_json(&self) -> Value {
        serde_json::json!({
            "id": self.id,
            "name": self.name,
            "description": self.description,
            "categories": self.categories,
            "tags": self.tags,
            "url": self.url,
            "match": self.kind,
        })
    }
}

/// The answer to one search.
#[derive(Debug, Clone)]
pub struct CatalogSearch {
    /// The templates that matched, best first.
    pub matches: Vec<CatalogMatch>,
    /// Every category the catalog has, for a user who wants to pick one.
    pub categories: Vec<String>,
    /// What the agent should do with this answer: use a template, or ask.
    pub note: Option<String>,
    /// The templates worth putting to the user as a question.
    pub candidates: Vec<String>,
}

/// Searches the catalog.
///
/// With a `query` the templates are ranked and only the matches come back;
/// without one the whole catalog is listed. A `category` narrows both.
pub async fn search(
    client: &CatalogClient,
    query: Option<&str>,
    category: Option<&str>,
) -> Result<CatalogSearch, AppError> {
    let query = query.map(str::trim).filter(|value| !value.is_empty());
    let category = category.map(str::trim).filter(|value| !value.is_empty());

    let mut templates = client
        .list(&CatalogQuery {
            q: None,
            category: None,
            limit: Some(CATALOG_LIMIT),
        })
        .await?;
    if let Some(query) = query {
        // The catalog's own search also sees templates past the first page,
        // so its answer is merged in even though it is not ranked here.
        if let Ok(found) = client
            .list(&CatalogQuery {
                q: Some(query.to_owned()),
                category: None,
                limit: Some(CATALOG_LIMIT),
            })
            .await
        {
            for template in found {
                if !templates.iter().any(|known| known.id == template.id) {
                    templates.push(template);
                }
            }
        }
    }

    let categories: Vec<String> = {
        let mut all: Vec<String> = templates
            .iter()
            .flat_map(|template| template.categories.iter().cloned())
            .collect();
        all.sort_unstable();
        all.dedup();
        all
    };

    let scoped: Vec<CatalogTemplate> = match category {
        Some(category) => templates
            .into_iter()
            .filter(|template| {
                template
                    .categories
                    .iter()
                    .any(|known| known.eq_ignore_ascii_case(category))
            })
            .collect(),
        None => templates,
    };

    let mut scored: Vec<(u16, &'static str, CatalogTemplate)> = scoped
        .into_iter()
        .map(|template| {
            let (score, kind) = score_of(&template, query);
            (score, kind, template)
        })
        .collect();
    scored.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.2.name.cmp(&right.2.name))
    });

    let mut matches: Vec<CatalogMatch> = Vec::new();
    for (score, kind, template) in scored {
        if query.is_some() {
            if score == 0 {
                continue;
            }
            if matches.len() >= MAX_MATCHES {
                break;
            }
        } else if matches.len() >= MAX_LISTING {
            break;
        }
        let id = template.id.clone();
        let url = trusted_ui_url(&id, template.ui.as_deref()).ok().flatten();
        matches.push(CatalogMatch {
            id,
            name: template.name,
            description: template.description,
            categories: template.categories,
            tags: template.tags,
            url,
            kind,
            score,
        });
    }

    let (note, candidates) = advise(&matches, query);
    Ok(CatalogSearch {
        matches,
        categories,
        note,
        candidates,
    })
}

/// What the agent should do with the matches: use one, ask, or say there is
/// none. The `note` is the sentence the model reads before it answers.
fn advise(matches: &[CatalogMatch], query: Option<&str>) -> (Option<String>, Vec<String>) {
    let Some(query) = query else {
        return (
            Some(
                "The whole Litepod catalog. Search it with `query` (a name, a tag or \
                 what the app does, e.g. `uptime`, `email`, `database`) and narrow it with \
                 `category`. For one template's variables read `get template`."
                    .into(),
            ),
            Vec::new(),
        );
    };

    if matches.is_empty() {
        return (
            Some(format!(
                "No template in the Litepod catalog matches `{query}`. The catalog is at \
                 https://templates.litepod.sh - ask the user whether they want a different \
                 app, or their own compose file."
            )),
            Vec::new(),
        );
    }

    // A hit on the name, the id or a tag is the template itself. Anything
    // weaker - the description, the words in it - is a suggestion.
    let strong: Vec<&CatalogMatch> = matches
        .iter()
        .filter(|found| found.score >= 70)
        .collect();

    if strong.len() == 1 {
        let found = strong[0];
        let (id, name) = (found.id.clone(), found.name.clone());
        return (
            Some(format!(
                "`{query}` is `{id}` ({name}). Read `get template` with \
                 `template: {id}` for its variables, ask the user for the ones it \
                 needs, and deploy it with `create_from_template`."
            )),
            Vec::new(),
        );
    }

    if strong.len() > 1 {
        let ids: Vec<String> = strong
            .iter()
            .take(MAX_CANDIDATES)
            .map(|found| found.id.clone())
            .collect();
        let listed = ids.join(", ");
        return (
            Some(format!(
                "Several templates match `{query}`: {listed}. Ask the user which one they \
                 want before deploying anything."
            )),
            ids,
        );
    }

    let ids: Vec<String> = matches
        .iter()
        .take(MAX_CANDIDATES)
        .map(|found| found.id.clone())
        .collect();
    let listed = ids.join(", ");
    (
        Some(format!(
            "Only a loose match for `{query}`: {listed}. Ask the user whether one of them \
             is what they meant, or whether they want their own compose file."
        )),
        ids,
    )
}

/// How well a template matches a query, and what it matched on.
///
/// The id and the name are the template itself, a tag is what the catalog
/// calls it, a category is what kind of app it is, and the description is
/// only a mention.
fn score_of(template: &CatalogTemplate, query: Option<&str>) -> (u16, &'static str) {
    let Some(query) = query else {
        return (0, "catalog");
    };
    let wanted = squash(query);
    if wanted.is_empty() {
        return (0, "catalog");
    }

    let id = squash(&template.id);
    let name = squash(&template.name);
    if id == wanted || name == wanted {
        return (100, "exact");
    }
    if id.contains(&wanted) || wanted.contains(&id) {
        return (90, "id");
    }
    if name.contains(&wanted) || wanted.contains(&name) {
        return (80, "name");
    }
    for tag in &template.tags {
        if squash(tag).contains(&wanted) {
            return (70, "tag");
        }
    }
    for category in &template.categories {
        if squash(category).contains(&wanted) {
            return (60, "category");
        }
    }

    let haystack = squash(&format!(
        "{} {} {}",
        template.description,
        template.tags.join(" "),
        template.categories.join(" ")
    ));
    let words: Vec<String> = query
        .split_whitespace()
        .map(squash)
        .filter(|word| word.len() >= 3)
        .collect();
    if words.len() >= 2 && words.iter().all(|word| haystack.contains(word)) {
        return (50, "text");
    }
    if haystack.contains(&wanted) {
        return (40, "description");
    }
    (0, "none")
}

/// Lower case with the separators dropped, so `Uptime Kuma` and `uptimekuma`
/// are the same word.
fn squash(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn template(id: &str, name: &str, description: &str, tags: &[&str], categories: &[&str]) -> CatalogTemplate {
        CatalogTemplate {
            id: id.into(),
            name: name.into(),
            description: description.into(),
            version: "1".into(),
            categories: categories.iter().map(|value| (*value).into()).collect(),
            tags: tags.iter().map(|value| (*value).into()).collect(),
            ports: vec![],
            icon: None,
            ui: None,
            website: None,
            docs: None,
            source: None,
            license: None,
            manifest: None,
            compose: None,
            env: None,
            setup: None,
            requires_https: false,
            url_persisted: false,
        }
    }

    #[test]
    fn the_id_and_the_name_are_the_template_itself() {
        let kuma = template("uptime-kuma", "Uptime Kuma", "monitoring", &[], &["monitoring"]);
        assert_eq!(score_of(&kuma, Some("uptime-kuma")), (100, "exact"));
        assert_eq!(score_of(&kuma, Some("Uptime Kuma")), (100, "exact"));
        assert_eq!(score_of(&kuma, Some("uptime")), (90, "id"));
        assert_eq!(score_of(&kuma, Some("kuma")), (90, "id"));
    }

    #[test]
    fn a_tag_is_a_stronger_hit_than_a_description() {
        let kuma = template(
            "uptime-kuma",
            "Uptime Kuma",
            "Self-hosted uptime monitoring",
            &["uptime", "alerts"],
            &["monitoring"],
        );
        assert_eq!(score_of(&kuma, Some("alerts")), (70, "tag"));
        assert_eq!(score_of(&kuma, Some("monitoring")), (60, "category"));
        assert_eq!(score_of(&kuma, Some("self")), (40, "description"));
    }

    #[test]
    fn every_word_somewhere_is_a_loose_match() {
        let mail = template(
            "mailpit",
            "Mailpit",
            "An email testing tool",
            &["mail"],
            &["email", "developer-tools"],
        );
        assert_eq!(score_of(&mail, Some("email testing")), (50, "text"));
        assert_eq!(score_of(&mail, Some("nothing at all")), (0, "none"));
    }

    #[test]
    fn one_match_is_used_and_several_are_asked_about() {
        let kuma = CatalogMatch {
            id: "uptime-kuma".into(),
            name: "Uptime Kuma".into(),
            description: String::new(),
            categories: vec![],
            tags: vec![],
            url: None,
            kind: "exact",
            score: 100,
        };
        let other = CatalogMatch {
            id: "uptime-kuma-2".into(),
            name: "Uptime Kuma 2".into(),
            description: String::new(),
            categories: vec![],
            tags: vec![],
            url: None,
            kind: "name",
            score: 80,
        };

        let (note, candidates) = advise(&[kuma.clone()], Some("uptime"));
        assert!(note.unwrap().contains("get template"));
        assert!(candidates.is_empty());

        let (note, candidates) = advise(&[kuma, other], Some("uptime"));
        assert!(note.unwrap().contains("Ask the user"));
        assert_eq!(candidates.len(), 2);
    }

    #[test]
    fn no_match_says_so_and_asks() {
        let (note, candidates) = advise(&[], Some("nothing"));
        assert!(note.unwrap().contains("No template"));
        assert!(candidates.is_empty());
    }

    #[test]
    fn a_listing_explains_how_to_search() {
        let (note, candidates) = advise(&[], None);
        assert!(note.unwrap().contains("query"));
        assert!(candidates.is_empty());
    }

    /// The real catalog, when the network is there: the ranking has to hold
    /// against the templates that are actually published.
    #[tokio::test]
    #[ignore = "needs the network"]
    async fn the_real_catalog_ranks_the_template_it_is_asked_for_first() {
        let client = CatalogClient::new().unwrap();
        let found = search(&client, Some("uptime"), None).await.unwrap();
        assert_eq!(found.matches.first().map(|found| found.id.as_str()), Some("uptime-kuma"));
        // `checkmate` is an uptime monitor too, so this is a question, not a
        // template to pick by itself.
        assert!(found.note.unwrap().contains("Ask the user"));
        assert!(found.candidates.len() >= 2);

        let found = search(&client, Some("email"), None).await.unwrap();
        assert!(!found.matches.is_empty());
        assert!(found.matches.iter().all(|found| found.score > 0));

        let found = search(&client, None, Some("monitoring")).await.unwrap();
        assert!(!found.matches.is_empty());
        assert!(found
            .matches
            .iter()
            .all(|found| found.categories.iter().any(|category| category == "monitoring")));
    }
}
