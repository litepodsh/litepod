//! Everything the assistant may do, and nothing else.
//!
//! These two lists are the whole allowlist. The tool definitions the model sees
//! are built from them, a call naming anything else is refused, and a pending
//! proposal is checked against them again when someone confirms it. They live
//! in code on purpose: no setting, request or account can widen them, only a
//! release can. To take something away from the assistant, delete its line.

/// The `get` resources the assistant may read.
pub const RESOURCES: &[&str] = &[
    "organizations",
    "projects",
    "applications",
    "environment",
    "deployments",
    "deployment_logs",
    "containers",
    "container_logs",
    "source",
    "domains",
    "servers",
    "git_providers",
    "compose",
];

/// The `exec` actions the assistant may propose.
pub const ACTIONS: &[&str] = &[
    "set_environment_variable",
    "delete_environment_variable",
    "redeploy",
    "start",
    "stop",
    "set_compose_file",
    "update_application",
    "set_source",
    "add_domain",
    "update_domain",
    "delete_domain",
    "create_project",
    "create_application",
    "update_project",
    "delete_project",
];

pub fn resource_allowed(resource: &str) -> bool {
    RESOURCES.contains(&resource)
}

pub fn action_allowed(action: &str) -> bool {
    ACTIONS.contains(&action)
}

/// The `exec` actions the MCP endpoint runs on the spot instead of storing.
///
/// Only there, never in the in-app chat: an MCP client asks its human before a
/// destructive tool call, and both of these undo each other. Everything else an
/// agent asks for over MCP is still a proposal a person confirms in Litepod.
pub const MCP_DIRECT: &[&str] = &["start", "stop"];

pub fn mcp_direct(action: &str) -> bool {
    MCP_DIRECT.contains(&action)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_mcp_runs_directly_is_a_subset_of_the_allowlist() {
        for action in MCP_DIRECT {
            assert!(action_allowed(action), "{action} must also be an allowed action");
        }
        assert_eq!(MCP_DIRECT, &["start", "stop"], "widening this is a security decision");
        assert!(!mcp_direct("redeploy"));
        assert!(!mcp_direct("delete_project"));
    }

    #[test]
    fn nothing_outside_the_lists_is_allowed() {
        assert!(!action_allowed("run_shell"));
        assert!(!action_allowed("redeploy_application"));
        assert!(!resource_allowed("secrets"));
    }
}
