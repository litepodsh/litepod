//! The MCP server: Litepod as a tool for the user's own agent.
//!
//! Not a second tool system. `protocol` turns JSON-RPC into calls on
//! `assistant::tools`, so `get` and `exec` mean the same thing here as in the
//! in-app chat. The difference is that nothing waits for a person: `start` and
//! `stop` run on the spot (`direct`), and every other action is stored as a
//! proposal under the caller's MCP conversation (`session`) and confirmed in
//! the same call (`capabilities::mcp_auto_confirm`).
pub mod catalog_check;
pub mod direct;
pub mod protocol;
pub mod session;

/// The instance-wide switch, for whoever operates this install. Anything but
/// `off` leaves MCP to each organization's own setting.
pub fn instance_enabled() -> bool {
    enabled_by(std::env::var("LITEPOD_MCP").ok().as_deref())
}

fn enabled_by(value: Option<&str>) -> bool {
    !value.is_some_and(|value| value.trim().eq_ignore_ascii_case("off"))
}

#[cfg(test)]
mod tests {
    use super::enabled_by;

    #[test]
    fn only_off_turns_the_instance_off() {
        assert!(enabled_by(None));
        assert!(enabled_by(Some("on")));
        assert!(enabled_by(Some("")));
        assert!(!enabled_by(Some("off")));
        assert!(!enabled_by(Some(" OFF ")));
    }
}
