//! Every action checks its own permissions when it is applied: the permission
//! the matching HTTP route asks for, on the project or environment the action
//! touches, as the person applying it. Whatever brought the action here (the
//! in-app panel or MCP) and whatever that route already checked, the action
//! does not trust it: membership, role assignments and the token's scopes are
//! read again from the database.

use serde_json::Value;

use crate::{
    dtos::response::AppError,
    security::{
        authz::{Permission, PermissionSet, Target, extract::member_permissions},
        middleware::auth::AuthUser,
    },
    state::state::SharedState,
};

use super::{lifecycle::StoredAction, types::*};

/// What `tool` needs, and whether it is checked at the organization (a new
/// project, a template) instead of at the action's own project or environment.
fn required(tool: &str, arguments: &Value, new_project: bool) -> (Vec<Permission>, bool) {
    use Permission::*;
    let named = |key: &str| arguments.get(key).is_some_and(|value| !value.is_null());
    match tool {
        SET_ENVIRONMENT_VARIABLE | DELETE_ENVIRONMENT_VARIABLE | LINK_DATABASE => {
            (vec![AppEnvUpdate], false)
        }
        REDEPLOY_APPLICATION | START_APPLICATION | STOP_APPLICATION | RESTART_APPLICATION
        | ROLLBACK_APPLICATION => (vec![AppDeploy], false),
        SET_COMPOSE_FILE | UPDATE_APPLICATION | SET_SOURCE | SET_MAINTENANCE => {
            (vec![AppUpdate], false)
        }
        ADD_DOMAIN | UPDATE_DOMAIN | DELETE_DOMAIN => (vec![AppDomainsManage], false),
        CREATE_APPLICATION => (vec![AppCreate], false),
        CREATE_PROJECT => (vec![ProjectCreate], true),
        UPDATE_PROJECT => (vec![ProjectUpdate], false),
        DELETE_PROJECT => (vec![ProjectDelete], false),
        DEPLOY_STATIC_SITE | CREATE_FROM_TEMPLATE | DEPLOY_COMPOSE | DEPLOY_ZIP
        | DEPLOY_REPOSITORY => {
            // Into an existing application, or a new one; then a deploy.
            let mut needed = if named("application_id") {
                vec![AppUpdate, AppDeploy]
            } else {
                vec![AppCreate, AppDeploy]
            };
            if named("host") {
                needed.push(AppDomainsManage);
            }
            if new_project {
                needed.push(ProjectCreate);
            }
            // The template routes ask for `AppCreate` organization-wide.
            (needed, new_project || tool == CREATE_FROM_TEMPLATE)
        }
        // A tool nobody mapped is refused, never let through.
        _ => (Vec::new(), true),
    }
}

/// The caller's permissions in `organization_id`, from the database, capped by
/// the scopes of the token they came with. A revoked or expired token has none.
async fn permissions_of(
    state: &SharedState,
    organization_id: &str,
    actor: &AuthUser,
) -> Result<PermissionSet, AppError> {
    let scopes = match &actor.token_id {
        None => None,
        Some(token_id) => {
            let token =
                crate::git::repositories::personal_access_token::find_active_by_id(state, token_id)
                    .await?
                    .ok_or(AppError::InsufficientPermissions)?;
            // An MCP token works for its own organization only.
            if token
                .mcp_organization_id
                .as_deref()
                .is_some_and(|pinned| pinned != organization_id)
            {
                return Err(AppError::InsufficientPermissions);
            }
            Some(serde_json::from_str::<Vec<String>>(&token.scopes).unwrap_or_default())
        }
    };
    member_permissions(state, actor, organization_id, scopes.as_deref())
        .await
        .map(|(_, set)| set)
}

/// `Ok(Some(permission))` names the first permission the caller lacks.
pub(super) async fn missing_permission(
    state: &SharedState,
    actor: &AuthUser,
    stored: &StoredAction,
) -> Result<Option<Permission>, AppError> {
    let arguments = serde_json::from_str::<Value>(&stored.arguments).unwrap_or(Value::Null);
    let new_project = stored.project_id.is_empty();
    let (needed, at_org) = required(&stored.tool, &arguments, new_project);
    if needed.is_empty() {
        return Err(AppError::InsufficientPermissions);
    }
    let set = permissions_of(state, &stored.organization_id, actor).await?;
    // `Target` is not `Copy`, so it is built again for every check.
    let target = || {
        if at_org || new_project {
            Target::Org
        } else if stored.environment_id.is_empty()
            || matches!(stored.tool.as_str(), UPDATE_PROJECT | DELETE_PROJECT)
        {
            Target::Project(&stored.project_id)
        } else {
            Target::Environment {
                project_id: &stored.project_id,
                environment_id: &stored.environment_id,
            }
        }
    };
    Ok(needed
        .into_iter()
        .find(|permission| !set.allows(*permission, target())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_tool_the_assistant_can_apply_names_what_it_needs() {
        for tool in [
            SET_ENVIRONMENT_VARIABLE,
            REDEPLOY_APPLICATION,
            START_APPLICATION,
            STOP_APPLICATION,
            RESTART_APPLICATION,
            ROLLBACK_APPLICATION,
            SET_COMPOSE_FILE,
            DELETE_ENVIRONMENT_VARIABLE,
            UPDATE_APPLICATION,
            SET_SOURCE,
            CREATE_APPLICATION,
            CREATE_PROJECT,
            UPDATE_PROJECT,
            DELETE_PROJECT,
            ADD_DOMAIN,
            UPDATE_DOMAIN,
            DELETE_DOMAIN,
            SET_MAINTENANCE,
            DEPLOY_STATIC_SITE,
            CREATE_FROM_TEMPLATE,
            DEPLOY_COMPOSE,
            DEPLOY_ZIP,
            DEPLOY_REPOSITORY,
            LINK_DATABASE,
        ] {
            assert!(!required(tool, &json!({}), false).0.is_empty(), "{tool}");
        }
        assert!(required("drop_everything", &json!({}), false).0.is_empty());
    }

    #[test]
    fn a_deploy_into_a_new_project_with_a_domain_needs_all_of_it() {
        let (needed, at_org) = required(DEPLOY_COMPOSE, &json!({ "host": "a.example.com" }), true);
        assert!(at_org);
        for permission in [
            Permission::AppCreate,
            Permission::AppDeploy,
            Permission::AppDomainsManage,
            Permission::ProjectCreate,
        ] {
            assert!(needed.contains(&permission), "{permission:?}");
        }
    }
}
