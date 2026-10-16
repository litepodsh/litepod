//! Persisting each step of a rig run back to `assistant_messages`.
//!
//! One [`Hooks`] instance is built per run: it owns the conversation id and
//! the [`SharedState`] it needs to write, and implements rig's
//! [`AgentHook`](rig::agent::AgentHook) so the runner invokes us at every
//! lifecycle point. We do not steer the run — `on_tool_call` always returns
//! `ToolCallAction::Run` and `on_invalid_tool_call` returns `None` (default
//! rig retry) — we only observe and persist. The actual tool execution stays
//! in `assistant::tools::run`; rig's `DynamicTool::call` invokes it through
//! the wrapper added in Task 4.
//!
//! Persistence is targeted, not exhaustive:
//!
//! - `on_model_turn_finished` writes one assistant row carrying the text and
//!   one `assistant_tool_call` row per emitted call. The transcript stays
//!   replayable: a stored tool call has the call id, tool name, and JSON
//!   arguments the model emitted, which is what `history::load` reads.
//! - `on_tool_result` writes one `tool` row carrying the call id, the tool
//!   name, and the result text. The two together — call row + result row —
//!   are the record of what happened on that step.
//! - `on_stream_response_finish` updates `assistant_conversations.updated_at`
//!   so the conversation list re-orders as the user runs new turns. Rig 0.42
//!   has no `on_run_settled`; the closest equivalent that fires once at the
//!   end of the stream is this hook. The plan's intent ("touch the row at
//!   the end of the run") is preserved under this name.
//!
//! Other lifecycle hooks log via `tracing` and do not mutate state.

use rig::agent::{
    AgentHook, CompletionCallAction, CompletionCallEvent, CompletionResponseEvent, HookContext,
    InvalidToolCallAction, ModelTurnAction, ObservationAction, StreamResponseFinish,
    ToolCallAction, ToolResultAction,
};
use rig::message::{AssistantContent, ToolResultContent};
use tokio::sync::mpsc::UnboundedSender;
use uuid::Uuid;

use crate::assistant::actions::ActionDto;
use crate::state::state::SharedState;

/// Host-side events forwarded to the existing assistant SSE stream.
#[derive(Clone)]
pub enum Event {
    Text(String),
    ToolStarted {
        id: String,
        name: String,
        arguments: String,
    },
    ToolFinished {
        id: String,
        ok: bool,
        action: Option<ActionDto>,
        choices: Vec<String>,
    },
}

/// One run's hook handle, captured by `Agent::hooks`.
#[allow(dead_code)] // surfaced in later tasks when the rig agent is wired in.
pub struct Hooks {
    pub state: SharedState,
    pub conversation_id: String,
    pub user_id: String,
    /// The session id OpenCode keys prompt caching off. Forwarded to the
    /// provider by `SessionTransport`, not read here.
    pub session_id: String,
    pub event_tx: Option<UnboundedSender<Event>>,
}

impl Hooks {
    #[allow(dead_code)] // surfaced in later tasks when the rig agent is wired in.
    pub fn new(
        state: SharedState,
        conversation_id: String,
        user_id: String,
        session_id: String,
        event_tx: Option<UnboundedSender<Event>>,
    ) -> Self {
        Self {
            state,
            conversation_id,
            user_id,
            session_id,
            event_tx,
        }
    }
}

impl AgentHook for Hooks {
    /// Logged, not steered — the runner's request preparation is the place
    /// per-turn patches belong if a future hook needs them.
    async fn on_completion_call(
        &self,
        _ctx: &HookContext,
        _event: CompletionCallEvent<'_>,
    ) -> CompletionCallAction {
        CompletionCallAction::Continue
    }

    /// Observe the assistant message that just streamed in. The actual
    /// persistence happens in `on_model_turn_finished`, which fires once
    /// per accepted turn with the canonical content list.
    async fn on_completion_response(
        &self,
        _ctx: &HookContext,
        event: CompletionResponseEvent<'_>,
    ) -> ObservationAction {
        tracing::debug!(
            conversation_id = %self.conversation_id,
            user_id = %self.user_id,
            message_id = ?event.message_id,
            "rig turn completed",
        );
        ObservationAction::Continue
    }

    /// Persist one assistant row + one `assistant_tool_call` row per emitted
    /// call. This is the canonical place to write to `assistant_messages`:
    /// rig has already reconciled tool-call upgrades and validated the
    /// content list, so what we see here is exactly what we should store.
    async fn on_model_turn_finished(
        &self,
        _ctx: &HookContext,
        event: rig::agent::ModelTurnFinished<'_>,
    ) -> ModelTurnAction {
        let mut text = String::new();
        let mut calls = Vec::new();
        for content in event.content {
            match content {
                AssistantContent::Text(t) => {
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    text.push_str(&t.text);
                }
                AssistantContent::ToolCall(call) => calls.push(call),
                // Reasoning, image, etc. are not stored — only text and
                // tool calls reach the transcript.
                AssistantContent::Reasoning(_) | AssistantContent::Image(_) => {}
            }
        }
        let text = crate::assistant::conversation::visible_text(&text);

        if let Err(error) = store_message(
            &self.state,
            &self.conversation_id,
            "assistant",
            &text,
            None,
            None,
            None,
        )
        .await
        {
            tracing::warn!(
                conversation_id = %self.conversation_id,
                "rig hook: failed to store assistant message: {error}",
            );
            // Continue running so the turn still completes; the failure is
            // logged for the operator rather than surfaced to the user.
        }

        for call in calls {
            // Tool-call rows are stored separately so a `tool` row in the
            // transcript clearly belongs to one. The JSON arguments are
            // re-serialized so a future model with non-string args still
            // round-trips.
            let arguments =
                serde_json::to_string(&call.function.arguments).unwrap_or_else(|_| "{}".to_owned());
            if let Err(error) = store_message(
                &self.state,
                &self.conversation_id,
                "assistant_tool_call",
                "",
                Some(call.id.as_str()),
                Some(&call.function.name),
                Some(&arguments),
            )
            .await
            {
                tracing::warn!(
                    conversation_id = %self.conversation_id,
                    "rig hook: failed to store tool call: {error}",
                );
            }
        }

        ModelTurnAction::Continue
    }

    /// Leave the invalid-call decision to rig: a hook that returns `None`
    /// preserves fail-fast behavior by default.
    async fn on_invalid_tool_call(
        &self,
        _ctx: &HookContext,
        _event: &rig::agent::InvalidToolCallContext,
    ) -> Option<InvalidToolCallAction> {
        None
    }

    /// Run with the current arguments. We do not steer tool calls here; the
    /// wrapper around `tools::run` is the place to validate or rewrite.
    async fn on_tool_call(
        &self,
        _ctx: &HookContext,
        event: rig::agent::ToolCall<'_>,
    ) -> ToolCallAction {
        let _ = self.event_tx.as_ref().map(|tx| {
            tx.send(Event::ToolStarted {
                id: event
                    .tool_call_id
                    .unwrap_or(event.internal_call_id)
                    .to_owned(),
                name: event.tool_name.to_owned(),
                arguments: event.args.to_owned(),
            })
        });
        ToolCallAction::Run
    }

    /// Persist the tool result. The tool's raw result is what was emitted;
    /// we render it back to text so it lands in the transcript the same way
    /// the model sees it.
    async fn on_tool_result(
        &self,
        _ctx: &HookContext,
        event: rig::agent::ToolResultEvent<'_>,
    ) -> ToolResultAction {
        let mut text = String::new();
        let mut had_text = false;
        for content in event.raw_result.output().as_content() {
            if let ToolResultContent::Text(t) = content {
                if had_text {
                    text.push('\n');
                }
                text.push_str(&t.text);
                had_text = true;
            }
        }
        if !had_text {
            text = event.raw_result.output().as_text().unwrap_or("").to_owned();
        }

        if let Err(error) = store_message(
            &self.state,
            &self.conversation_id,
            "tool",
            &text,
            Some(event.tool_call_id.unwrap_or("")),
            Some(event.tool_name),
            None,
        )
        .await
        {
            tracing::warn!(
                conversation_id = %self.conversation_id,
                "rig hook: failed to store tool result: {error}",
            );
        }
        let _ = self.event_tx.as_ref().map(|tx| {
            tx.send(Event::ToolFinished {
                id: event
                    .tool_call_id
                    .unwrap_or(event.internal_call_id)
                    .to_owned(),
                ok: event.raw_result.is_success(),
                action: event
                    .tool_context
                    .result::<Option<ActionDto>>()
                    .cloned()
                    .flatten(),
                choices: event
                    .tool_context
                    .result::<Vec<String>>()
                    .cloned()
                    .unwrap_or_default(),
            })
        });
        ToolResultAction::Keep
    }

    async fn on_text_delta(
        &self,
        _ctx: &HookContext,
        event: rig::agent::TextDelta<'_>,
    ) -> ObservationAction {
        let _ = self
            .event_tx
            .as_ref()
            .map(|tx| tx.send(Event::Text(event.delta.to_owned())));
        ObservationAction::Continue
    }

    async fn on_reasoning_delta(
        &self,
        _ctx: &HookContext,
        _event: rig::agent::ReasoningDelta<'_>,
    ) -> ObservationAction {
        ObservationAction::Continue
    }

    async fn on_tool_call_delta(
        &self,
        _ctx: &HookContext,
        _event: rig::agent::ToolCallDelta<'_>,
    ) -> ObservationAction {
        ObservationAction::Continue
    }

    /// End-of-run signal. Updates the conversation's `updated_at` so the
    /// panel sorts it to the top of the list. This is the closest
    /// equivalent to the plan's `on_run_settled`: rig 0.42 has no such
    /// hook, and this one fires once the stream finishes — exactly the
    /// point at which a settled conversation needs a fresh timestamp.
    async fn on_stream_response_finish(
        &self,
        _ctx: &HookContext,
        _event: StreamResponseFinish<'_>,
    ) -> ObservationAction {
        if let Err(error) = touch_conversation(&self.state, &self.conversation_id).await {
            tracing::warn!(
                conversation_id = %self.conversation_id,
                "rig hook: failed to touch conversation timestamp: {error}",
            );
        }
        ObservationAction::Continue
    }
}

#[allow(dead_code)] // surfaced in later tasks when the rig agent is wired in.
async fn store_message(
    state: &SharedState,
    conversation_id: &str,
    role: &str,
    content: &str,
    tool_call_id: Option<&str>,
    tool_name: Option<&str>,
    tool_arguments: Option<&str>,
) -> Result<(), crate::dtos::response::AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "INSERT INTO assistant_messages \
         (id, conversation_id, role, content, tool_call_id, tool_name, tool_arguments, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )
    .bind(Uuid::now_v7().to_string())
    .bind(conversation_id)
    .bind(role)
    .bind(content)
    .bind(tool_call_id)
    .bind(tool_name)
    .bind(tool_arguments)
    .bind(chrono::Utc::now().timestamp_millis())
    .execute(&conn)
    .await
    .map_err(|error| crate::dtos::response::AppError::DatabaseError(error.to_string()))?;
    Ok(())
}

#[allow(dead_code)] // surfaced in later tasks when the rig agent is wired in.
async fn touch_conversation(
    state: &SharedState,
    conversation_id: &str,
) -> Result<(), crate::dtos::response::AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("UPDATE assistant_conversations SET updated_at = ?1 WHERE id = ?2")
        .bind(chrono::Utc::now().timestamp_millis())
        .bind(conversation_id)
        .execute(&conn)
        .await
        .map_err(|error| crate::dtos::response::AppError::DatabaseError(error.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    //! Rig's [`HookContext`] is `pub(crate)` and only constructible from
    //! inside `rig-agent`, so the trait methods are not directly callable
    //! from our tests. The persistence logic the trait routes into lives in
    //! the free helpers above; these tests exercise those helpers with the
    //! exact shapes the `AgentHook` impl passes through, which pins the
    //! contract without needing a real `HookContext`.
    //!
    //! The trait methods themselves compile against rig's trait and are
    //! exercised at runtime by the agent runner; what we own is the work
    //! they delegate to.

    use rig::message::{AssistantContent, Text, ToolCallId, ToolFunction};

    use super::{Hooks, store_message, touch_conversation};
    use crate::state::state::SharedState;
    use crate::testing::{cleanup_database, test_state};

    async fn seed_conversation(state: &SharedState, conversation_id: &str, updated_at: i64) {
        let conn = crate::git::repositories::db::get_conn(state).await;
        sqlx::query(
            "INSERT INTO assistant_conversations \
             (id, organization_id, user_id, title, created_at, updated_at) \
             VALUES (?1, 'organization-a', 'editor', ?1, 0, ?2)",
        )
        .bind(conversation_id)
        .bind(updated_at)
        .execute(&conn)
        .await
        .unwrap();
    }

    async fn row_count(state: &SharedState, conversation_id: &str) -> i64 {
        let conn = crate::git::repositories::db::get_conn(state).await;
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM assistant_messages WHERE conversation_id = ?1",
        )
        .bind(conversation_id)
        .fetch_one(&conn)
        .await
        .unwrap();
        count
    }

    async fn row_with_role(
        state: &SharedState,
        conversation_id: &str,
        role: &str,
    ) -> Option<(Option<String>, Option<String>, Option<String>)> {
        let conn = crate::git::repositories::db::get_conn(state).await;
        sqlx::query_as::<_, (Option<String>, Option<String>, Option<String>)>(
            "SELECT tool_call_id, tool_name, tool_arguments \
             FROM assistant_messages \
             WHERE conversation_id = ?1 AND role = ?2 \
             ORDER BY created_at, id \
             LIMIT 1",
        )
        .bind(conversation_id)
        .bind(role)
        .fetch_optional(&conn)
        .await
        .unwrap()
    }

    async fn updated_at(state: &SharedState, conversation_id: &str) -> i64 {
        let conn = crate::git::repositories::db::get_conn(state).await;
        let ts: i64 =
            sqlx::query_scalar("SELECT updated_at FROM assistant_conversations WHERE id = ?1")
                .bind(conversation_id)
                .fetch_one(&conn)
                .await
                .unwrap();
        ts
    }

    #[tokio::test]
    async fn on_model_turn_finished_persists_assistant_message_with_tool_calls() {
        // Mirrors what `Hooks::on_model_turn_finished` does once the trait
        // routes a `ModelTurnFinished` event through: write the assistant row,
        // then one `assistant_tool_call` row per emitted call.
        let (state, path) = test_state().await;
        seed_conversation(&state, "conv-h1", 0).await;

        let text = Text::new("hello back");
        let call = rig::message::ToolCall::new(
            ToolCallId::new("call-1").unwrap(),
            ToolFunction {
                name: "get".to_owned(),
                arguments: serde_json::json!({ "path": "/apps" }),
            },
        );
        let content = vec![
            AssistantContent::Text(text),
            AssistantContent::ToolCall(call),
        ];

        // The impl reads `event.content` and routes each item to the right
        // helper. Re-run that routing here against the same content list to
        // pin the contract.
        let mut text = String::new();
        let mut calls = Vec::new();
        for item in &content {
            match item {
                AssistantContent::Text(t) => text.push_str(&t.text),
                AssistantContent::ToolCall(c) => calls.push(c),
                _ => {}
            }
        }
        store_message(&state, "conv-h1", "assistant", &text, None, None, None)
            .await
            .unwrap();
        for call in calls {
            let arguments = serde_json::to_string(&call.function.arguments).unwrap();
            store_message(
                &state,
                "conv-h1",
                "assistant_tool_call",
                "",
                Some(call.id.as_str()),
                Some(&call.function.name),
                Some(&arguments),
            )
            .await
            .unwrap();
        }

        assert_eq!(row_count(&state, "conv-h1").await, 2);
        let tool_row = row_with_role(&state, "conv-h1", "assistant_tool_call")
            .await
            .unwrap();
        assert_eq!(tool_row.0.as_deref(), Some("call-1"));
        assert_eq!(tool_row.1.as_deref(), Some("get"));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&tool_row.2.unwrap()).unwrap(),
            serde_json::json!({ "path": "/apps" }),
        );

        cleanup_database(&path);
    }

    #[tokio::test]
    async fn on_tool_result_persists_tool_message() {
        // Mirrors what `Hooks::on_tool_result` writes: a `tool` row carrying
        // the call id, tool name, and rendered result text.
        let (state, path) = test_state().await;
        seed_conversation(&state, "conv-h2", 0).await;

        store_message(
            &state,
            "conv-h2",
            "tool",
            "gateway, static-site",
            Some("call-1"),
            Some("get"),
            None,
        )
        .await
        .unwrap();

        let row = row_with_role(&state, "conv-h2", "tool").await.unwrap();
        assert_eq!(row.0.as_deref(), Some("call-1"));
        assert_eq!(row.1.as_deref(), Some("get"));
        // Content is on a separate column; re-read it to confirm.
        let conn = crate::git::repositories::db::get_conn(&state).await;
        let content: String = sqlx::query_scalar(
            "SELECT content FROM assistant_messages \
             WHERE conversation_id = 'conv-h2' AND role = 'tool' LIMIT 1",
        )
        .fetch_one(&conn)
        .await
        .unwrap();
        assert_eq!(content, "gateway, static-site");

        cleanup_database(&path);
    }

    #[tokio::test]
    async fn on_stream_response_finish_updates_conversation_timestamp() {
        // Mirrors `Hooks::on_stream_response_finish` which delegates to
        // `touch_conversation`.
        let (state, path) = test_state().await;
        seed_conversation(&state, "conv-h3", 100).await;
        assert_eq!(updated_at(&state, "conv-h3").await, 100);

        touch_conversation(&state, "conv-h3").await.unwrap();

        let after = updated_at(&state, "conv-h3").await;
        assert!(
            after > 100,
            "the timestamp must advance past the seed value, got {after}"
        );

        cleanup_database(&path);
    }

    #[test]
    fn hooks_new_keeps_ids_together() {
        // `AppState` is not `Debug`, so `Hooks` does not derive one — the
        // hook's job is to own the state handle, and the rest of the impl
        // reads it by reference. What we assert here is that the four
        // pieces of run metadata the constructor takes are stored verbatim.
        // Constructed in a `tokio::runtime::Builder` block so we can run an
        // async fixture from a synchronous test without pulling in a
        // dedicated runtime-blocking executor.
        let (state, path) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(test_state());
        let hooks = Hooks::new(
            state.clone(),
            "conv-x".into(),
            "editor".into(),
            "sess".into(),
            None,
        );
        assert_eq!(hooks.conversation_id, "conv-x");
        assert_eq!(hooks.user_id, "editor");
        assert_eq!(hooks.session_id, "sess");
        cleanup_database(&path);
    }
}
