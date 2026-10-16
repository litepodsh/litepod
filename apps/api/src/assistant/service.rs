//! The rig-side turn runner.
//!
//! The authenticated assistant turn runner. Persistence into
//! `assistant_messages` is the hook's job, not this module's.

use std::{
    collections::HashSet,
    sync::{LazyLock, Mutex},
};

use futures_util::StreamExt;
use rig::streaming::StreamingPrompt;

use crate::{
    assistant::{
        config::AssistantConfig,
        history, hooks, providers,
        tools::{self, wrappers::ToolRunContext},
    },
    dtos::response::AppError,
    security::middleware::auth::AuthUser,
    state::state::SharedState,
};

#[allow(dead_code)] // used by the contextual rig turn entry point next.
const LOCK_TTL_SECONDS: u64 = 60;

/// Covers the same-process case if Dragonfly is temporarily unavailable.
///
/// ponytail: process-local fallback only; require Redis for cross-process
/// exclusion if Litepod is ever run with multiple API replicas.
#[allow(dead_code)] // used by the contextual rig turn entry point next.
static LOCAL_LOCKS: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

/// A lease held for one assistant conversation turn.
///
/// Redis is the cross-process source of truth. The local entry prevents two
/// concurrent turns in this process while Redis is down, and is always removed
/// synchronously on drop. Redis cleanup is best-effort because `Drop` cannot
/// await; the TTL is the safety net if the runtime is already shutting down.
#[allow(dead_code)] // returned by the contextual rig turn entry point next.
pub struct ConversationLock {
    state: crate::state::state::SharedState,
    key: String,
    remote: bool,
}

impl Drop for ConversationLock {
    fn drop(&mut self) {
        if let Ok(mut locks) = LOCAL_LOCKS.lock() {
            locks.remove(&self.key);
        }
        if self.remote {
            let state = self.state.clone();
            let key = self.key.clone();
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(async move {
                    let _ = state.ephemeral.delete(&key).await;
                });
            }
        }
    }
}

/// Acquire a per-conversation lease. A busy conversation returns the existing
/// 409-shaped `ResourceInUse` error instead of adding a duplicate error type.
#[allow(dead_code)] // called by the contextual rig turn entry point next.
pub async fn acquire(
    state: &crate::state::state::SharedState,
    conversation_id: &str,
) -> Result<ConversationLock, AppError> {
    let key = format!("conversation:{conversation_id}:lock");
    {
        let mut locks = LOCAL_LOCKS
            .lock()
            .map_err(|_| AppError::InternalError("conversation lock is poisoned".into()))?;
        if !locks.insert(key.clone()) {
            return Err(AppError::ResourceInUse("Conversation is busy".into()));
        }
    }

    let remote = match state
        .ephemeral
        .set_json_if_absent(&key, &"locked", LOCK_TTL_SECONDS)
        .await
    {
        Ok(true) => true,
        Ok(false) => {
            LOCAL_LOCKS
                .lock()
                .expect("conversation lock is not poisoned")
                .remove(&key);
            return Err(AppError::ResourceInUse("Conversation is busy".into()));
        }
        Err(error) => {
            tracing::warn!(
                conversation_id,
                "assistant lock using local fallback: {error}"
            );
            false
        }
    };

    Ok(ConversationLock {
        state: state.clone(),
        key,
        remote,
    })
}

/// Context only the authenticated HTTP flow can create. It stays outside the
/// prompt and is injected into tools through Rig's host-only `ToolContext`.
pub struct RigTurnContext {
    pub state: SharedState,
    pub organization_id: String,
    pub conversation_id: String,
    pub actor: AuthUser,
    pub preamble: String,
    pub event_tx: Option<tokio::sync::mpsc::UnboundedSender<hooks::Event>>,
}

/// Run the rig agent for an already-persisted user message.
pub async fn run_contextual_turn(
    context: RigTurnContext,
    config: &AssistantConfig,
    message: &str,
) -> Result<String, AppError> {
    let _lock = acquire(&context.state, &context.conversation_id).await?;
    let model = providers::build(config, &context.conversation_id, &config.model).await?;
    let tools = tools::definitions()
        .into_iter()
        .map(tools::wrappers::wrap)
        .collect();
    let agent = rig::agent::AgentBuilder::new(model)
        .preamble(&context.preamble)
        .max_tokens(crate::assistant::config::ANTHROPIC_MAX_TOKENS.into())
        .dynamic_tools(tools)
        .build();
    let mut tool_context = rig::tool::ToolContext::new();
    tool_context.insert(ToolRunContext::new(
        context.state.clone(),
        context.organization_id,
        context.conversation_id.clone(),
        context.actor.clone(),
    ));

    // The HTTP flow persisted this user message before reaching us. Drop its
    // final replayed row so Rig sees it exactly once as the runner prompt.
    let mut history = history::load(&context.state, &context.conversation_id).await?;
    let prompt_is_last_history_item = matches!(
        history.last(),
        Some(rig::message::Message::User { content })
            if content.iter().any(|item| matches!(
                item,
                rig::message::UserContent::Text(text) if text.text == message
            ))
    );
    if prompt_is_last_history_item {
        history.pop();
    }
    let session_id = context.conversation_id.clone();
    let mut stream = agent
        .stream_prompt(rig::message::Message::user(message))
        .history(history)
        .tool_context(tool_context)
        .max_turns(crate::assistant::config::MAX_STEPS as usize)
        .max_invalid_tool_call_retries(2)
        .add_hook(hooks::Hooks::new(
            context.state,
            context.conversation_id,
            context.actor.user_id,
            session_id,
            context.event_tx,
        ))
        .await;
    let mut output = String::new();
    while let Some(item) = stream.next().await {
        match item {
            Ok(rig::agent::MultiTurnStreamItem::FinalResponse(response)) => {
                output = response.output
            }
            Ok(_) => {}
            Err(error) => return Err(AppError::InternalError(error.to_string())),
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use axum::{
        Json, Router,
        extract::State,
        http::{HeaderMap, header::CONTENT_TYPE},
        response::{IntoResponse, Response},
        routing::post,
    };

    use super::{RigTurnContext, acquire, run_contextual_turn};
    use crate::{
        assistant::config::{AssistantConfig, Protocol},
        dtos::response::AppError,
        security::middleware::auth::AuthUser,
        testing::{cleanup_database, test_state},
    };

    #[tokio::test]
    async fn acquire_returns_conflict_when_already_held() {
        let (state, path) = test_state().await;
        let _first = acquire(&state, "conv-lock").await.unwrap();
        assert!(matches!(
            acquire(&state, "conv-lock").await,
            Err(AppError::ResourceInUse(_))
        ));
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn rig_turn_persists_the_final_assistant_message() {
        let calls = Arc::new(AtomicUsize::new(0));
        let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
        let app = Router::new()
            .route("/v1/chat/completions", post(mock_openai_completion))
            .with_state(calls.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let (state, path) = test_state().await;
        sqlx::query(
            "INSERT INTO assistant_conversations \
             (id, organization_id, user_id, title, created_at, updated_at) \
             VALUES ('conv-rig', 'organization-a', 'editor', 'Test', 0, 0)",
        )
        .execute(&state.db)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO assistant_messages (id, conversation_id, role, content, created_at) \
             VALUES ('user-rig', 'conv-rig', 'user', 'hello', 1)",
        )
        .execute(&state.db)
        .await
        .unwrap();

        let output = run_contextual_turn(
            RigTurnContext {
                state: state.clone(),
                organization_id: "organization-a".into(),
                conversation_id: "conv-rig".into(),
                actor: test_actor(),
                preamble: "Be brief.".into(),
                event_tx: Some(event_tx),
            },
            &AssistantConfig {
                api_key: Some("test-key".into()),
                base_url: format!("http://{address}/v1"),
                model: "test-model".into(),
                protocol: Protocol::OpenAi,
                opencode: true,
                allow_local: true,
            },
            "hello",
        )
        .await
        .unwrap();

        assert_eq!(output, "hello from rig");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(
            matches!(event_rx.recv().await, Some(crate::assistant::hooks::Event::Text(text)) if text == "hello ")
        );
        assert!(
            matches!(event_rx.recv().await, Some(crate::assistant::hooks::Event::Text(text)) if text == "from rig")
        );
        let stored: String = sqlx::query_scalar(
            "SELECT content FROM assistant_messages \
             WHERE conversation_id = 'conv-rig' AND role = 'assistant'",
        )
        .fetch_one(&state.db)
        .await
        .unwrap();
        assert_eq!(stored, "hello from rig");
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn rig_turn_supports_anthropic_messages() {
        let calls = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route("/v1/messages", post(mock_anthropic_completion))
            .with_state(calls.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let (state, path) = test_state().await;
        sqlx::query(
            "INSERT INTO assistant_conversations \
             (id, organization_id, user_id, title, created_at, updated_at) \
             VALUES ('conv-anthropic', 'organization-a', 'editor', 'Test', 0, 0)",
        )
        .execute(&state.db)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO assistant_messages (id, conversation_id, role, content, created_at) \
             VALUES ('user-anthropic', 'conv-anthropic', 'user', 'hello', 1)",
        )
        .execute(&state.db)
        .await
        .unwrap();

        let output = run_contextual_turn(
            RigTurnContext {
                state: state.clone(),
                organization_id: "organization-a".into(),
                conversation_id: "conv-anthropic".into(),
                actor: test_actor(),
                preamble: "Be brief.".into(),
                event_tx: None,
            },
            &AssistantConfig {
                api_key: Some("test-key".into()),
                base_url: format!("http://{address}/v1"),
                model: "opencode-go/minimax-m3".into(),
                protocol: Protocol::Anthropic,
                opencode: true,
                allow_local: true,
            },
            "hello",
        )
        .await
        .unwrap();

        assert_eq!(output, "hello from anthropic");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        cleanup_database(&path);
    }

    async fn mock_openai_completion(
        State(calls): State<Arc<AtomicUsize>>,
        headers: HeaderMap,
        Json(request): Json<serde_json::Value>,
    ) -> Response {
        calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(request["model"], "test-model");
        assert_eq!(request["tools"].as_array().map(Vec::len), Some(2));
        assert_eq!(headers["x-opencode-session"], "conv-rig");
        assert_eq!(request["stream"], true);
        (
            [(CONTENT_TYPE, "text/event-stream")],
            concat!(
                "data: {\"id\":\"chatcmpl-test\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"test-model\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hello \"},\"finish_reason\":null}]}\n\n",
                "data: {\"id\":\"chatcmpl-test\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"test-model\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"from rig\"},\"finish_reason\":null}]}\n\n",
                "data: {\"id\":\"chatcmpl-test\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"test-model\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
                "data: [DONE]\n\n",
            ),
        )
            .into_response()
    }

    async fn mock_anthropic_completion(
        State(calls): State<Arc<AtomicUsize>>,
        headers: HeaderMap,
        Json(request): Json<serde_json::Value>,
    ) -> Response {
        calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(request["model"], "opencode-go/minimax-m3");
        assert_eq!(request["tools"].as_array().map(Vec::len), Some(2));
        assert_eq!(headers["x-opencode-session"], "conv-anthropic");
        assert_eq!(request["stream"], true);
        (
            [(CONTENT_TYPE, "text/event-stream")],
            concat!(
                "event: message_start\n",
                "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg-test\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"test-model\",\"content\":[],\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\n",
                "event: content_block_start\n",
                "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
                "event: content_block_delta\n",
                "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hello from anthropic\"}}\n\n",
                "event: content_block_stop\n",
                "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
                "event: message_delta\n",
                "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":1}}\n\n",
                "event: message_stop\n",
                "data: {\"type\":\"message_stop\"}\n\n",
            ),
        )
            .into_response()
    }

    fn test_actor() -> AuthUser {
        AuthUser {
            user_id: "editor".into(),
            email: "editor@example.test".into(),
            name: "Editor".into(),
            is_sysadmin: false,
            token_id: None,
            token_name: None,
        }
    }
}
