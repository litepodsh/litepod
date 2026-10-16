//! Rig adapters for the assistant's two existing tools.
//!
//! Authorization belongs to the HTTP-side conversation flow, so it is passed
//! through Rig's host-only `ToolContext`, never derived from model arguments.

use rig::tool::{DynamicTool, ToolContext, ToolExecutionError, ToolOutput};

use crate::{
    assistant::tools::{self, ToolDefinition, ToolOutcome},
    security::middleware::auth::AuthUser,
    state::state::SharedState,
};

/// Values a Rig tool needs that must never become model-visible prompt data.
#[derive(Clone)]
#[allow(dead_code)] // wired by the rig service in the next migration step.
pub struct ToolRunContext {
    pub state: SharedState,
    pub organization_id: String,
    pub conversation_id: String,
    pub actor: AuthUser,
}

impl ToolRunContext {
    #[allow(dead_code)] // used by the rig service once providers are wired.
    pub fn new(
        state: SharedState,
        organization_id: String,
        conversation_id: String,
        actor: AuthUser,
    ) -> Self {
        Self {
            state,
            organization_id,
            conversation_id,
            actor,
        }
    }
}

/// Wrap one existing assistant tool without duplicating its authorization or
/// execution logic. The outcome is attached as host-only result metadata so a
/// stream adapter can still render proposals and choices for the UI.
#[allow(dead_code)] // wired by the rig service in the next migration step.
pub fn wrap(def: ToolDefinition) -> DynamicTool {
    let name = def.function.name.to_owned();
    DynamicTool::new(
        name.clone(),
        def.function.description,
        def.function.parameters,
        move |context: &mut ToolContext, arguments: serde_json::Value| {
            let run_context = context
                .require::<ToolRunContext>()
                .map(Clone::clone)
                .map_err(|error| ToolExecutionError::other(error.to_string()));
            let name = name.clone();

            Box::pin(async move {
                let run_context = run_context?;
                let arguments = serde_json::to_string(&arguments)
                    .map_err(|error| ToolExecutionError::other(error.to_string()))?;
                let outcome = tools::run(
                    &run_context.state,
                    &run_context.organization_id,
                    &run_context.conversation_id,
                    &run_context.actor,
                    &name,
                    &arguments,
                )
                .await;
                Ok(ToolOutput::text(to_output(context, outcome)))
            })
        },
    )
}

#[allow(dead_code)] // reached through `wrap` once the rig service is active.
fn to_output(context: &mut ToolContext, outcome: ToolOutcome) -> String {
    context.insert_result(outcome.action);
    context.insert_result(outcome.choices);
    tools::truncate(outcome.result)
}

#[cfg(test)]
mod tests {
    use rig::tool::{ToolContext, ToolSet};

    use super::{ToolRunContext, wrap};
    use crate::assistant::tools::{FunctionDefinition, ToolDefinition};
    use crate::{
        security::middleware::auth::AuthUser,
        testing::{cleanup_database, test_state},
    };

    #[test]
    fn wrap_preserves_name_description_and_parameters() {
        let tool = wrap(ToolDefinition {
            kind: "function",
            function: FunctionDefinition {
                name: "get".into(),
                description: "Read".into(),
                parameters: serde_json::json!({"type": "object"}),
            },
        });

        assert_eq!(tool.name(), "get");
        assert_eq!(tool.definition().description, "Read");
        assert_eq!(
            tool.definition().parameters,
            serde_json::json!({"type": "object"})
        );
    }

    #[tokio::test]
    async fn wrapped_tool_receives_host_only_turn_context() {
        let (state, path) = test_state().await;
        let mut context = ToolContext::new();
        context.insert(ToolRunContext::new(
            state.clone(),
            "organization-a".into(),
            "conversation-a".into(),
            AuthUser {
                user_id: "editor".into(),
                email: "editor@example.test".into(),
                name: "Editor".into(),
                is_sysadmin: false,
                token_id: None,
                token_name: None,
            },
        ));
        let tools = ToolSet::from_dynamic_tools(
            crate::assistant::tools::definitions()
                .into_iter()
                .map(wrap)
                .collect(),
        );

        let result = tools
            .execute("get", r#"{"resource":"projects"}"#, &mut context)
            .await;
        assert!(result.is_success());
        assert!(result.output().as_text().unwrap().contains("projects"));
        cleanup_database(&path);
    }
}
