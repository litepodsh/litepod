//! The actions MCP runs on the spot: `capabilities::MCP_DIRECT`.
//!
//! The route already proved org-wide Editor (`perm::AssistantUse`), capped by
//! the token's scopes. `AppDeploy` is checked again at the application's own
//! environment anyway, so a future narrower route gate cannot widen this.

use serde_json::{Value, json};

use crate::{
    assistant::{capabilities, tools},
    deployment,
    security::{
        authz::{Permission, Target},
        middleware::org_scope::OrgScope,
    },
    state::state::SharedState,
};

pub struct Outcome {
    pub text: String,
    pub is_error: bool,
}

fn failure(message: impl Into<String>) -> Outcome {
    Outcome {
        text: json!({ "error": message.into() }).to_string(),
        is_error: true,
    }
}

pub async fn run(state: &SharedState, org: &OrgScope, action: &str, arguments: &Value) -> Outcome {
    if !capabilities::mcp_direct(action) || !capabilities::action_allowed(action) {
        return failure(format!("`{action}` does not run directly"));
    }
    let target = match tools::resolve_target(state, &org.organization_id, arguments).await {
        Ok(target) => target,
        Err(text) => {
            return Outcome {
                text,
                is_error: true,
            };
        }
    };
    let allowed = org.permissions.allows(
        Permission::AppDeploy,
        Target::Environment {
            project_id: &target.project_id,
            environment_id: &target.environment_id,
        },
    );
    if !allowed {
        return failure(format!(
            "This token cannot {action} {}",
            target.application_name
        ));
    }

    let user_id = Some(org.user.user_id.as_str());
    let (result, audit_action) = match action {
        "start" => (
            deployment::start(state, &target.application_id, user_id).await,
            "deployment.started",
        ),
        _ => (
            deployment::stop(state, &target.application_id, user_id).await,
            "deployment.stopped",
        ),
    };
    let deployment = match result {
        Ok(deployment) => deployment,
        Err(error) => return failure(error.to_string()),
    };

    let metadata = json!({ "source": "mcp" }).to_string();
    crate::git::repositories::audit_log::record(
        state,
        &org.organization_id,
        &org.user,
        audit_action,
        Some("application"),
        Some(&target.application_id),
        Some(&target.application_name),
        Some(&metadata),
        None,
        None,
        None,
    )
    .await
    .ok();

    Outcome {
        text: json!({
            "done": true,
            "action": action,
            "application": target.application_name,
            "deployment": deployment,
        })
        .to_string(),
        is_error: false,
    }
}
