use serde::Serialize;
use utoipa::ToSchema;

/// One terminal session from the write-only audit tables
/// (`remote_terminal_sessions`, `container_exec_sessions`). Lifecycle metadata
/// and who opened it, nothing else: never what was run or any payload.
#[derive(Serialize, ToSchema, Debug)]
pub struct TerminalSessionDto {
    pub id: String,
    /// `host` (SSH/local console on the server) or `container` (exec).
    pub kind: String,
    pub container_id: Option<String>,
    pub started_at: String,
    pub finished_at: Option<String>,
    /// Whole seconds between start and finish; `None` while still open.
    pub duration_seconds: Option<i64>,
    /// `closed`, `disconnected`, `failed`, or `None` while still open.
    pub result: Option<String>,
    pub failure_reason: Option<String>,
    /// `None` when the user row was deleted after the fact.
    pub actor_name: Option<String>,
    pub actor_email: Option<String>,
    /// Set when the session was opened with a personal access token.
    pub actor_token_name: Option<String>,
}
