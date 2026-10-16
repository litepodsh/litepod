use crate::{
    dtos::response::AppError, security::middleware::auth::AuthUser, state::state::SharedState,
};
use uuid::Uuid;

pub async fn start_session(
    state: &SharedState,
    organization_id: &str,
    server_id: &str,
    actor: &AuthUser,
) -> Result<String, AppError> {
    let id = Uuid::now_v7().to_string();
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("INSERT INTO remote_terminal_sessions (id, organization_id, server_id, actor_user_id, actor_token_id, actor_token_name) VALUES (?1, ?2, ?3, ?4, ?5, ?6)")
        .bind(&id).bind(organization_id).bind(server_id).bind(&actor.user_id).bind(&actor.token_id).bind(&actor.token_name)
        .execute(&conn).await.map_err(|error| AppError::DatabaseError(error.to_string()))?;
    Ok(id)
}

pub async fn finish_session(state: &SharedState, id: &str, result: &str, reason: Option<&str>) {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let reason = reason.map(|value| value.chars().take(500).collect::<String>());
    let _ = sqlx::query("UPDATE remote_terminal_sessions SET finished_at = datetime('now'), result = ?1, failure_reason = ?2 WHERE id = ?3 AND finished_at IS NULL")
        .bind(result).bind(reason).bind(id).execute(&conn).await;
}

#[cfg(test)]
mod tests {
    #[test]
    fn session_audit_schema_contains_lifecycle_only() {
        let migration = include_str!("../../migrations/0019_remote_terminal_sessions.sql");
        for column in [
            "organization_id",
            "server_id",
            "actor_user_id",
            "started_at",
            "finished_at",
            "result",
        ] {
            assert!(migration.contains(column), "missing audit column: {column}");
        }
        for forbidden in ["command", "output", "private_key", "ssh_host"] {
            assert!(
                !migration.contains(forbidden),
                "must not persist terminal {forbidden}"
            );
        }
    }
}
