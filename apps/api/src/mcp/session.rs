//! Where an MCP caller's proposals live.
//!
//! `assistant_actions` hangs off a conversation, and confirming happens in the
//! assistant panel, so every token gets one conversation of its own, titled
//! after the token. The user finds the agent's proposals there, next to their
//! chats. Renaming that conversation starts a fresh one on the next proposal,
//! which is harmless.

use uuid::Uuid;

use crate::{
    dtos::response::AppError, security::middleware::auth::AuthUser, state::state::SharedState,
};

pub async fn conversation_for(
    state: &SharedState,
    organization_id: &str,
    actor: &AuthUser,
) -> Result<String, AppError> {
    let title = format!("MCP · {}", actor.token_name.as_deref().unwrap_or("session"));
    let conn = crate::git::repositories::db::get_conn(state).await;

    let found: Option<String> = sqlx::query_scalar(
        "SELECT id FROM assistant_conversations WHERE organization_id = ?1 AND user_id = ?2 AND title = ?3 ORDER BY created_at LIMIT 1",
    )
    .bind(organization_id)
    .bind(&actor.user_id)
    .bind(&title)
    .fetch_optional(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    if let Some(id) = found {
        return Ok(id);
    }

    let id = Uuid::now_v7().to_string();
    let now = chrono::Utc::now().timestamp_millis();
    sqlx::query(
        "INSERT INTO assistant_conversations (id, organization_id, user_id, title, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
    )
    .bind(&id)
    .bind(organization_id)
    .bind(&actor.user_id)
    .bind(&title)
    .bind(now)
    .execute(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn one_token_one_conversation() {
        let (state, path) = crate::testing::test_state().await;
        let mut actor = crate::testing::auth_user("editor");
        actor.token_name = Some("laptop".into());

        let first = conversation_for(&state, "organization-a", &actor)
            .await
            .unwrap();
        let again = conversation_for(&state, "organization-a", &actor)
            .await
            .unwrap();
        assert_eq!(first, again);

        actor.token_name = Some("ci".into());
        let other = conversation_for(&state, "organization-a", &actor)
            .await
            .unwrap();
        assert_ne!(first, other);
        crate::testing::cleanup_database(&path);
    }
}
