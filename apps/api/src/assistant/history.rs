//! Loading conversation history from `assistant_messages` into rig's
//! provider-agnostic [`Message`] form.
//!
//! Two orders matter: the loader returns messages in `created_at ASC` order,
//! so the model sees the same conversation it has been talking through; and
//! the cap is applied via `LIMIT` *before* the reverse, so the most recent
//! `MAX_HISTORY_MESSAGES` rows are the ones that survive, not the oldest.
//! `tool_calls.arguments` is parsed JSON — an unparseable string degrades to
//! `{}` rather than sinking the load with a decode error.
//!
//! Unknown conversation ids resolve to an empty `Vec`, not an error: the caller
//! decides whether the lack of history is a bug or a fresh thread.

use crate::{dtos::response::AppError, state::state::SharedState};

/// Cap on the messages returned by [`load`]. Matches the tool-name/call-id
/// shape rig expects, but the value is local so a future rig change cannot
/// silently shrink what we replay to the model.
pub const MAX_HISTORY_MESSAGES: i64 = 100;

#[cfg(test)]
mod tests {
    use rig::message::{AssistantContent, Message, ToolResult, ToolResultContent, UserContent};

    use super::{MAX_HISTORY_MESSAGES, load};
    use crate::testing::{cleanup_database, test_state};

    /// Insert a conversation row + N message rows under it. The first 100
    /// rows survive the cap; the rest are kept for "ordered ascending" checks
    /// when more than the cap is seeded.
    async fn seed_messages(state: &crate::state::state::SharedState, count: usize) -> String {
        let conn = crate::git::repositories::db::get_conn(state).await;
        let now = chrono::Utc::now().timestamp_millis();
        sqlx::query(
            "INSERT INTO assistant_conversations \
             (id, organization_id, user_id, title, created_at, updated_at) \
             VALUES (?1, 'organization-a', 'editor', ?1, 0, 0)",
        )
        .bind("conv-1")
        .execute(&conn)
        .await
        .unwrap();
        for index in 0..count {
            sqlx::query(
                "INSERT INTO assistant_messages \
                 (id, conversation_id, role, content, created_at) \
                 VALUES (?1, 'conv-1', 'user', ?2, ?3)",
            )
            .bind(format!("m-{index}"))
            .bind(format!("message {index}"))
            .bind(now + index as i64)
            .execute(&conn)
            .await
            .unwrap();
        }
        "conv-1".to_owned()
    }

    #[tokio::test]
    async fn load_maps_every_row_variant() {
        let (state, path) = test_state().await;
        let conn = crate::git::repositories::db::get_conn(&state).await;
        sqlx::query(
            "INSERT INTO assistant_conversations \
             (id, organization_id, user_id, title, created_at, updated_at) \
             VALUES ('conv-1', 'organization-a', 'editor', 'conv-1', 0, 0)",
        )
        .execute(&conn)
        .await
        .unwrap();

        // Four rows covering every variant the loader must handle:
        // user text, assistant text only, assistant tool_calls, tool result.
        let now = chrono::Utc::now().timestamp_millis();
        sqlx::query(
            "INSERT INTO assistant_messages \
             (id, conversation_id, role, content, created_at) \
             VALUES ('m-u', 'conv-1', 'user', 'hello', ?1)",
        )
        .bind(now)
        .execute(&conn)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO assistant_messages \
             (id, conversation_id, role, content, created_at) \
             VALUES ('m-a', 'conv-1', 'assistant', 'hi', ?1)",
        )
        .bind(now + 1)
        .execute(&conn)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO assistant_messages \
             (id, conversation_id, role, content, tool_call_id, tool_name, tool_arguments, created_at) \
             VALUES ('m-atc', 'conv-1', 'assistant_tool_call', '', 'call-1', 'get', '{\"path\":\"/apps\"}', ?1)",
        )
        .bind(now + 2)
        .execute(&conn)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO assistant_messages \
             (id, conversation_id, role, content, tool_call_id, tool_name, created_at) \
             VALUES ('m-t', 'conv-1', 'tool', 'gateway, static-site', 'call-1', 'get', ?1)",
        )
        .bind(now + 3)
        .execute(&conn)
        .await
        .unwrap();

        let messages = load(&state, "conv-1").await.unwrap();
        assert_eq!(messages.len(), 4, "all four variants must load");

        // User text.
        match &messages[0] {
            Message::User { content } => {
                assert_eq!(content.len(), 1);
                match &content[0] {
                    UserContent::Text(text) => assert_eq!(text.text, "hello"),
                    other => panic!("expected user text, got {other:?}"),
                }
            }
            other => panic!("expected user message, got {other:?}"),
        }

        // Assistant text only.
        match &messages[1] {
            Message::Assistant { content, .. } => {
                assert_eq!(content.len(), 1);
                match &content[0] {
                    AssistantContent::Text(text) => assert_eq!(text.text, "hi"),
                    other => panic!("expected assistant text, got {other:?}"),
                }
            }
            other => panic!("expected assistant message, got {other:?}"),
        }

        // Assistant tool_calls: parsed from `tool_arguments` JSON.
        match &messages[2] {
            Message::Assistant { content, .. } => {
                assert_eq!(content.len(), 1);
                match &content[0] {
                    AssistantContent::ToolCall(call) => {
                        assert_eq!(call.id.as_str(), "call-1");
                        assert_eq!(call.function.name, "get");
                        assert_eq!(
                            call.function.arguments,
                            serde_json::json!({ "path": "/apps" })
                        );
                    }
                    other => panic!("expected assistant tool call, got {other:?}"),
                }
            }
            other => panic!("expected assistant message, got {other:?}"),
        }

        // Tool result lands as a user message carrying one ToolResult.
        match &messages[3] {
            Message::User { content } => {
                assert_eq!(content.len(), 1);
                match &content[0] {
                    UserContent::ToolResult(ToolResult {
                        call,
                        name,
                        content,
                        ..
                    }) => {
                        assert_eq!(call.as_str(), "call-1");
                        assert_eq!(name, "get");
                        assert_eq!(content.len(), 1);
                        match &content[0] {
                            ToolResultContent::Text(text) => {
                                assert_eq!(text.text, "gateway, static-site")
                            }
                            other => panic!("expected tool result text, got {other:?}"),
                        }
                    }
                    other => panic!("expected user tool result, got {other:?}"),
                }
            }
            other => panic!("expected user message carrying tool result, got {other:?}"),
        }

        cleanup_database(&path);
    }

    #[tokio::test]
    async fn load_returns_empty_for_unknown_conversation() {
        let (state, path) = test_state().await;
        let messages = load(&state, "nope").await.unwrap();
        assert!(messages.is_empty());
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn load_caps_at_one_hundred_messages_and_keeps_ascending_order() {
        let (state, path) = test_state().await;
        let conv = seed_messages(&state, 150).await;

        let messages = load(&state, &conv).await.unwrap();
        assert_eq!(
            messages.len() as i64,
            MAX_HISTORY_MESSAGES,
            "the cap is enforced",
        );

        // The most-recent 100 messages survive. Seeded as 0..=149 in
        // ascending `created_at`, so after the cap we keep messages 50..=149.
        let first = match &messages[0] {
            Message::User { content } => match &content[0] {
                UserContent::Text(text) => &text.text,
                other => panic!("expected user text, got {other:?}"),
            },
            other => panic!("expected user message, got {other:?}"),
        };
        assert!(
            first.contains("message 50"),
            "oldest survivor is message 50, got: {first}"
        );
        let last = match &messages[messages.len() - 1] {
            Message::User { content } => match &content[0] {
                UserContent::Text(text) => &text.text,
                other => panic!("expected user text, got {other:?}"),
            },
            other => panic!("expected user message, got {other:?}"),
        };
        assert!(
            last.contains("message 149"),
            "newest survivor is message 149, got: {last}"
        );

        // Sanity check: the loader respects MAX_HISTORY_MESSAGES without
        // any other cap on the conversation's reasoning-depth budget.

        cleanup_database(&path);
    }
}

/// Loads the conversation transcript in the shape rig expects.
///
/// Order is `created_at ASC`, so the model can re-walk the thread as it
/// happened. The cap of [`MAX_HISTORY_MESSAGES`] is enforced via `LIMIT`
/// against the *most recent* N rows: an SQL `ORDER BY DESC LIMIT N` picks the
/// newest N, and reversing the result yields them in ascending order without
/// the model ever seeing the oldest ones that fell off.
#[allow(dead_code)] // surfaced in later tasks; reserved here for review.
pub async fn load(
    state: &SharedState,
    conversation_id: &str,
) -> Result<Vec<rig::message::Message>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(
        "SELECT role, content, tool_call_id, tool_name, tool_arguments, created_at \
         FROM assistant_messages \
         WHERE conversation_id = ?1 \
         ORDER BY created_at DESC, id DESC \
         LIMIT ?2",
    )
    .bind(conversation_id)
    .bind(MAX_HISTORY_MESSAGES)
    .fetch_all(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;

    let mut out: Vec<rig::message::Message> = Vec::with_capacity(rows.len());
    // Reverse so the model sees oldest -> newest; the SQL `DESC` was only there
    // to make `LIMIT` pick the recent end of the transcript.
    for row in rows.iter().rev() {
        use sqlx::Row;
        let role: String = row
            .try_get(0)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?;
        let content: String = row
            .try_get(1)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?;
        let tool_call_id: Option<String> = row
            .try_get(2)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?;
        let tool_name: Option<String> = row
            .try_get(3)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?;
        let tool_arguments: Option<String> = row
            .try_get(4)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?;

        out.push(row_to_message(
            &role,
            &content,
            tool_call_id.as_deref(),
            tool_name.as_deref(),
            tool_arguments.as_deref(),
        ));
    }
    Ok(out)
}

/// One `assistant_messages` row → one rig [`Message`]. The mapping table from
/// the design spec, with one extra guarantee: a tool-call row whose
/// `tool_arguments` is unparseable lands as `{}` instead of erroring the load.
fn row_to_message(
    role: &str,
    content: &str,
    tool_call_id: Option<&str>,
    tool_name: Option<&str>,
    tool_arguments: Option<&str>,
) -> rig::message::Message {
    match role {
        // A tool_call row lives in the same `assistant_messages` table but
        // represents an assistant turn whose only payload is the model-emitted
        // call. Map it onto an assistant message carrying one
        // `AssistantContent::ToolCall`.
        "assistant_tool_call" => {
            let arguments = tool_arguments
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
                .unwrap_or_else(|| serde_json::json!({}));
            let id = tool_call_id.unwrap_or("call");
            let function_name = tool_name.unwrap_or("").to_owned();
            rig::message::Message::Assistant {
                id: None,
                content: vec![rig::message::AssistantContent::ToolCall(
                    rig::message::ToolCall::new(
                        rig::message::ToolCallId::new_or_mint(id),
                        rig::message::ToolFunction {
                            name: function_name,
                            arguments,
                        },
                    ),
                )],
            }
        }
        // A tool result goes back into the transcript as a user message
        // carrying one `UserContent::ToolResult`. Both the call id and the
        // tool name are required by rig; fall back to the empty-string
        // canonical sentinels when they are missing rather than failing.
        "tool" => {
            let call = rig::message::ToolCallId::new_or_mint(tool_call_id.unwrap_or(""));
            let name = tool_name.unwrap_or("").to_owned();
            rig::message::Message::User {
                content: vec![rig::message::UserContent::ToolResult(
                    rig::message::ToolResult {
                        call,
                        provider: None,
                        name,
                        content: vec![rig::message::ToolResultContent::Text(rig::message::Text {
                            text: content.to_owned(),
                            additional_params: None,
                        })],
                    },
                )],
            }
        }
        // Notes are operator annotations, not from the user. Wrap them in a
        // user message so the model can read them as context without confusing
        // them for actual user input.
        "note" => rig::message::Message::User {
            content: vec![rig::message::UserContent::text(format!(
                "System note, not from the user: {content}"
            ))],
        },
        // Anything else — "user", "assistant" — lands as a plain text message.
        // "assistant" is unusual here (the assistant tool-call rows already
        // represent the assistant side), but kept for completeness so an
        // older transcript still replays.
        other => match other {
            "assistant" => rig::message::Message::Assistant {
                id: None,
                content: vec![rig::message::AssistantContent::text(content)],
            },
            _ => rig::message::Message::user(content),
        },
    }
}
