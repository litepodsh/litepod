//! Append-only lifecycle audit for container exec (terminal) sessions.
//!
//! Mirrors `servers::terminal::start_session` / `finish_session` for the
//! container `exec` surface: an interactive shell is full control over
//! everything the container can reach, so it gets a mandatory session trail
//! that (unlike `audit_logs`) is not governed by the organization's
//! `audit_logs_enabled` toggle — the same reasoning that drove
//! `remote_terminal_sessions`.
//!
//! Only lifecycle metadata is ever persisted: actor identity (session cookie
//! or token), the target (application or server) and container, plus
//! started/finished timestamps and a coarse result. The exec command, argv,
//! stdin, output and any credential never touch the database. `started_at`
//! defaults to `datetime('now')` like the remote-shell table.
//!
//! The session row is created when the shell actually starts (at WebSocket
//! upgrade), not when the exec is created — a created-then-never-connected
//! exec still records as `failed` so nothing hangs forever.

use crate::{
    dtos::response::AppError, security::middleware::auth::AuthUser, state::state::SharedState,
};
use uuid::Uuid;

/// Opens a session row. `application_id` is `Some` for application-level exec,
/// `Some` for server-console (`servers/{id}/podman/container/{id}/exec`) — the
/// `CHECK ((application_id IS NULL) != (server_id IS NULL))` constraint and the
/// extractors make exactly one of them populated.
pub async fn start_exec_session(
    state: &SharedState,
    organization_id: &str,
    application_id: Option<&str>,
    server_id: Option<&str>,
    container_id: &str,
    actor: &AuthUser,
) -> Result<String, AppError> {
    let id = Uuid::now_v7().to_string();
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "INSERT INTO container_exec_sessions (id, organization_id, application_id, server_id, container_id, actor_user_id, actor_token_id, actor_token_name) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )
    .bind(&id)
    .bind(organization_id)
    .bind(application_id)
    .bind(server_id)
    .bind(container_id)
    .bind(&actor.user_id)
    .bind(&actor.token_id)
    .bind(&actor.token_name)
    .execute(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    Ok(id)
}

/// Closes a session row with a coarse result (`closed` by the client,
/// `disconnected` by the transport, `failed` by an exec-start error). The
/// `finished_at IS NULL` guard makes completion idempotent: the first finish
/// wins.
pub async fn finish_exec_session(
    state: &SharedState,
    id: &str,
    result: &str,
    reason: Option<&str>,
) {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let reason = reason.map(|value| value.chars().take(500).collect::<String>());
    let _ = sqlx::query("UPDATE container_exec_sessions SET finished_at = datetime('now'), result = ?1, failure_reason = ?2 WHERE id = ?3 AND finished_at IS NULL")
        .bind(result)
        .bind(reason)
        .bind(id)
        .execute(&conn)
        .await;
}

#[cfg(test)]
mod tests {
    #[test]
    fn session_audit_schema_contains_lifecycle_only() {
        let migration = include_str!("../../../migrations/0043_container_exec_sessions.sql");
        for column in [
            "organization_id",
            "actor_user_id",
            "started_at",
            "finished_at",
            "result",
        ] {
            assert!(migration.contains(column), "missing audit column: {column}");
        }
        for forbidden in ["command", "argv", "stdin", "output", "private_key", "host"] {
            assert!(
                !migration.contains(forbidden),
                "must not persist terminal {forbidden}"
            );
        }
    }
}
