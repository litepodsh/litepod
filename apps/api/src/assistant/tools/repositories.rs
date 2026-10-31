//! `get repositories`: what the organization's Git providers can see.
//!
//! Reads go through the providers the organization already connected, so a
//! private repository is visible exactly as it is in the "choose a repository"
//! picker, and nothing here reaches the internet beyond what that picker does.
//! A public repository with no provider is reached by `git_url` instead, which
//! lists its branches.

use serde_json::json;

use crate::{git::git_providers::provider, state::state::SharedState};

use super::{ToolOutcome, rank};

/// Offering more than this stops being a list a person can read.
const MAX_REPOSITORIES: usize = 30;

pub(super) async fn repositories(
    state: &SharedState,
    organization_id: &str,
    provider_filter: Option<&str>,
    query: Option<&str>,
    git_url: Option<&str>,
) -> ToolOutcome {
    let providers = match provider::list(state, organization_id).await {
        Ok(providers) => providers,
        Err(error) => return ToolOutcome::failure(error.to_string()),
    };
    let wanted = provider_filter
        .map(|wanted| wanted.trim().to_ascii_lowercase())
        .filter(|wanted| !wanted.is_empty());
    let chosen: Vec<_> = providers
        .iter()
        .filter(|candidate| match &wanted {
            None => true,
            Some(wanted) => {
                candidate.id.to_ascii_lowercase() == *wanted
                    || candidate.name.to_ascii_lowercase() == *wanted
            }
        })
        .collect();
    if let Some(wanted) = &wanted
        && chosen.is_empty()
    {
        return ToolOutcome::miss(
            format!("No Git provider called `{wanted}` in this organization."),
            rank(
                wanted,
                providers.iter().map(|candidate| candidate.name.as_str()),
            ),
        );
    }

    if let Some(git_url) = git_url {
        return branches(state, organization_id, &chosen, git_url).await;
    }

    if chosen.is_empty() {
        return ToolOutcome::data(json!({
            "repositories": [],
            "note": "No Git provider is connected. A public repository can still be deployed: \
                pass its HTTPS URL as `git_url` to list its branches, or read its page with `get url`.",
        }));
    }

    let needle = query
        .map(|query| squash(query))
        .filter(|query| !query.is_empty());
    let mut found = Vec::new();
    let mut problems = Vec::new();
    for candidate in chosen {
        match provider::repositories(state, organization_id, &candidate.id).await {
            Ok(repositories) => {
                for repository in repositories {
                    if needle
                        .as_ref()
                        .is_some_and(|needle| !squash(&repository.full_name).contains(needle))
                    {
                        continue;
                    }
                    found.push(json!({
                        "provider_id": candidate.id,
                        "provider": candidate.name,
                        "full_name": repository.full_name,
                        "git_url": repository.clone_url,
                        "default_branch": repository.default_branch,
                        "private": repository.private,
                    }));
                }
            }
            Err(error) => problems.push(json!({
                "provider": candidate.name,
                "error": error.to_string(),
            })),
        }
    }

    let total = found.len();
    // Names and branches come from whoever owns the repository.
    found.truncate(MAX_REPOSITORIES);
    let mut result = json!({
        "repositories": found,
        "total": total,
        "truncated": total > MAX_REPOSITORIES,
        "untrusted": crate::assistant::web::UNTRUSTED,
    });
    if total > MAX_REPOSITORIES {
        result["note"] = json!("More matched than are shown. Narrow it with `query`.");
    }
    if !problems.is_empty() {
        result["unreachable_providers"] = json!(problems);
    }
    ToolOutcome::data(result)
}

/// Branches of one repository: through the named provider, or the only one the
/// organization has that hosts it, or as a public repository.
async fn branches(
    state: &SharedState,
    organization_id: &str,
    chosen: &[&crate::dtos::git_provider::GitProviderDto],
    git_url: &str,
) -> ToolOutcome {
    let host = url::Url::parse(git_url.trim())
        .ok()
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase));
    let hosting: Vec<_> = chosen
        .iter()
        .filter(|candidate| {
            host.as_deref().is_some_and(|host| {
                provider::effective_host_url(&candidate.provider_type, &candidate.host_url)
                    .ok()
                    .and_then(|base| url::Url::parse(&base).ok())
                    .and_then(|base| base.host_str().map(str::to_ascii_lowercase))
                    .as_deref()
                    == Some(host)
            })
        })
        .collect();

    let outcome = match hosting.as_slice() {
        [only] => provider::branches(state, organization_id, &only.id, git_url).await,
        _ => provider::public_branches(git_url).await,
    };
    match outcome {
        Ok(branches) => ToolOutcome::data(json!({
            "git_url": git_url,
            "provider_id": hosting.first().map(|only| only.id.clone()),
            "branches": branches,
            "untrusted": crate::assistant::web::UNTRUSTED,
        })),
        Err(error) => ToolOutcome::failure(format!(
            "Could not list the branches of `{git_url}`: {error}. If it is private, it needs a connected Git provider."
        )),
    }
}

fn squash(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_query_ignores_case_and_separators() {
        assert!(squash("louislam/Uptime-Kuma").contains(&squash("uptime kuma")));
        assert!(!squash("louislam/uptime-kuma").contains(&squash("n8n")));
    }
}
