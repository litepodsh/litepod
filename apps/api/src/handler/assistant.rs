//! Three routes for the whole assistant.
//!
//! What it can do is a tool list, not a route table: a new capability is a new
//! tool, and the endpoints below never change. Sending a message returns the
//! actions the assistant wants to take; settling one is the only way anything
//! is written.

use axum::{
    Json,
    extract::{Path, Query, State},
    response::{
        IntoResponse,
        sse::{Event, KeepAlive, Sse},
    },
};
use futures_util::{Stream, StreamExt};
use serde::Deserialize;
use std::convert::Infallible;

use crate::{
    assistant::{actions, connections, conversation, search},
    dtos::response::{AppError, created, ok},
    security::authz::extract::{RequireOrg, perm},
    state::state::SharedState,
};

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ListConversationsQuery {
    /// Substring matched against title and message content (case-insensitive).
    pub q: Option<String>,
}

/// The organization's kill switch, and the only gate every route shares.
///
/// Deliberately says nothing about models. Reading a transcript or rejecting a
/// pending action has to keep working while a key is broken, so resolving a
/// model is a separate step that only sending a message takes.
///
/// Returns the organization name from the same validated lookup rather than a
/// second query, so what a turn is told about its scope and what was authorised
/// cannot disagree.
async fn assistant_on(state: &SharedState, organization_id: &str) -> Result<String, AppError> {
    let organization =
        crate::git::repositories::organization::get_organization_by_id(state, organization_id)
            .await
            .ok_or(AppError::OrganizationNotFound)?;

    if !organization.assistant_enabled {
        return Err(AppError::ValidationError(
            "The assistant is turned off for this organization. An admin can turn it on in organization settings.".into(),
        ));
    }
    Ok(organization.name)
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SendMessageRequest {
    /// Continues an existing conversation; omit to start one.
    pub conversation_id: Option<String>,
    pub message: String,
    /// The connection to use. Omitted, a new conversation takes the
    /// organization's default and an existing one keeps what it was pinned to.
    pub connection_id: Option<String>,
    pub model: Option<String>,
}

/// Streams the turn as it happens: each tool the assistant runs is visible
/// while it runs, not once everything is over. Anything that goes wrong after
/// the first byte arrives as an `error` event, because the status line has
/// already shipped by then.
#[utoipa::path(
    post,
    path = "/api/v1/organizations/{organization_id}/assistant/messages",
    tag = "assistant",
    responses((status = 200, description = "Server-sent events for one turn"))
)]
pub async fn send_message_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::AssistantUse>,
    Json(request): Json<SendMessageRequest>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let organization_name = assistant_on(&state, &org.organization_id).await?;
    let message = request.message.trim().to_owned();
    if message.is_empty() {
        return Err(AppError::ValidationError("Ask something first".into()));
    }

    // A thread stays on the model it started with. What the request names only
    // applies when the conversation has not pinned one already.
    let pinned = match &request.conversation_id {
        Some(id) => conversation::pinned_model(&state, &org.organization_id, &org.user, id).await?,
        None => None,
    };
    let (connection_id, model) = match pinned {
        Some(pinned) => pinned,
        None => (request.connection_id.clone(), request.model.clone()),
    };
    let config = connections::resolve(
        &state,
        &org.organization_id,
        connection_id.as_deref(),
        model.as_deref(),
    )
    .await?;

    let resolved_model = config.model.clone();
    let events = conversation::stream(
        state,
        config,
        org.organization_id,
        organization_name,
        org.user,
        request.conversation_id,
        message,
        connection_id,
        Some(resolved_model),
    )
    .map(|event| {
        Ok(
            Event::default().data(serde_json::to_string(&event).unwrap_or_else(|error| {
                serde_json::json!({ "type": "error", "message": error.to_string() }).to_string()
            })),
        )
    });

    Ok(Sse::new(events).keep_alive(KeepAlive::default()))
}

/// The collection beside the single conversation below. Still not a route per
/// capability: what the assistant can do is a tool list, and this is the second
/// half of one REST resource.
#[utoipa::path(
    get,
    path = "/api/v1/organizations/{organization_id}/assistant/conversations",
    tag = "assistant",
    params(
        ("organization_id" = String, Path, description = "Organization scope"),
        ("q" = Option<String>, Query, description = "Optional substring to search by title and message content")
    ),
    responses((status = 200, description = "This user's conversations, newest first", body = [conversation::ConversationSummaryDto]))
)]
pub async fn list_conversations_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgRead>,
    Query(query): Query<ListConversationsQuery>,
) -> Result<impl IntoResponse, AppError> {
    assistant_on(&state, &org.organization_id).await?;
    Ok(ok(conversation::list(
        &state,
        &org.organization_id,
        &org.user,
        query.q.as_deref(),
    )
    .await?))
}

#[utoipa::path(
    get,
    path = "/api/v1/organizations/{organization_id}/assistant/conversations/{conversation_id}",
    tag = "assistant",
    responses((status = 200, description = "Transcript and actions", body = conversation::ConversationDto))
)]
pub async fn read_conversation_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgRead>,
    Path((_organization_id, conversation_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    assistant_on(&state, &org.organization_id).await?;
    Ok(ok(conversation::read(
        &state,
        &org.organization_id,
        &org.user,
        &conversation_id,
    )
    .await?))
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SettleActionRequest {
    /// `confirm` applies the change; `reject` records that the user said no.
    pub decision: String,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct RenameConversationRequest {
    /// New title. Trimmed; 1..=80 characters.
    pub title: String,
}

/// Renames one conversation. The list endpoint already caps rows at the same
/// length the first-message title uses, so a renamed row still fits.
#[utoipa::path(
    patch,
    path = "/api/v1/organizations/{organization_id}/assistant/conversations/{conversation_id}",
    tag = "assistant",
    params(
        ("organization_id" = String, Path, description = "Organization scope"),
        ("conversation_id" = String, Path, description = "Conversation to rename")
    ),
    request_body = RenameConversationRequest,
    responses((status = 200, description = "The renamed conversation", body = conversation::ConversationSummaryDto))
)]
pub async fn rename_conversation_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::AssistantUse>,
    Path((_organization_id, conversation_id)): Path<(String, String)>,
    Json(request): Json<RenameConversationRequest>,
) -> Result<impl IntoResponse, AppError> {
    assistant_on(&state, &org.organization_id).await?;
    Ok(ok(conversation::rename(
        &state,
        &org.organization_id,
        &org.user,
        &conversation_id,
        &request.title,
    )
    .await?))
}

#[utoipa::path(
    post,
    path = "/api/v1/organizations/{organization_id}/assistant/actions/{action_id}",
    tag = "assistant",
    responses((status = 200, description = "The settled action", body = actions::ActionDto))
)]
pub async fn settle_action_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::AssistantUse>,
    Path((_organization_id, action_id)): Path<(String, String)>,
    Json(request): Json<SettleActionRequest>,
) -> Result<impl IntoResponse, AppError> {
    assistant_on(&state, &org.organization_id).await?;
    let settled = match request.decision.as_str() {
        "confirm" => actions::confirm(&state, &org.organization_id, &org.user, &action_id).await?,
        "reject" => actions::reject(&state, &org.organization_id, &org.user, &action_id).await?,
        other => {
            return Err(AppError::ValidationError(format!(
                "`{other}` is not a decision. Send `confirm` or `reject`."
            )));
        }
    };
    Ok(ok(settled))
}

// ── Model connections ────────────────────────────────────────────────────────
//
// These sit beside the assistant's own routes rather than in their own file
// because they are the assistant's configuration, not a separate resource: the
// gate above and the connections below answer the same question, which is
// whether this organization can talk to a model at all.
//
// A key never comes back. `ConnectionDto` carries `api_key_hint`, which
// identifies a key without being one.

/// The provider kinds the "add connection" dialog offers.
///
/// Served rather than duplicated in TypeScript so the base URLs, protocols and
/// key policies have exactly one home. Not organization-scoped: it holds no
/// secrets. Cloud leaves out the local-model kinds it refuses to save.
#[utoipa::path(
    get,
    path = "/api/v1/assistant/providers",
    tag = "assistant",
    responses((status = 200, description = "Provider kinds", body = [crate::assistant::config::Provider]))
)]
pub async fn list_providers_handler(
    State(state): State<SharedState>,
) -> Result<impl IntoResponse, AppError> {
    Ok(ok(crate::assistant::config::offered(state.is_cloud)))
}

#[utoipa::path(
    get,
    path = "/api/v1/organizations/{organization_id}/assistant/connections",
    tag = "assistant",
    responses((status = 200, description = "This organization's model connections", body = [connections::ConnectionDto]))
)]
pub async fn list_connections_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgRead>,
) -> Result<impl IntoResponse, AppError> {
    Ok(ok(connections::list(&state, &org.organization_id).await?))
}

/// Creating a connection probes it first: the key is only stored once it has
/// listed the provider's models. A key that cannot do that cannot run a turn
/// either, so it fails here rather than inside somebody's conversation.
#[utoipa::path(
    post,
    path = "/api/v1/organizations/{organization_id}/assistant/connections",
    tag = "assistant",
    request_body = connections::CreateConnectionRequest,
    responses((status = 201, description = "The stored connection", body = connections::ConnectionDto))
)]
pub async fn create_connection_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::AssistantUse>,
    Json(request): Json<connections::CreateConnectionRequest>,
) -> Result<impl IntoResponse, AppError> {
    let connection = connections::create(&state, &org.organization_id, request).await?;
    record_connection_change(
        &state,
        &org,
        "assistant_connection.created",
        &connection,
        None,
    )
    .await;
    Ok(created(connection))
}

#[utoipa::path(
    patch,
    path = "/api/v1/organizations/{organization_id}/assistant/connections/{connection_id}",
    tag = "assistant",
    request_body = connections::UpdateConnectionRequest,
    responses((status = 200, description = "The updated connection", body = connections::ConnectionDto))
)]
pub async fn update_connection_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::AssistantUse>,
    Path((_organization_id, connection_id)): Path<(String, String)>,
    Json(request): Json<connections::UpdateConnectionRequest>,
) -> Result<impl IntoResponse, AppError> {
    // Read first, so the trail records what the endpoint was as well as what it
    // became. "Someone repointed our connection" is the event worth seeing, and
    // the new value alone does not show it.
    let before = connections::get(&state, &org.organization_id, &connection_id)
        .await
        .ok();
    let connection =
        connections::update(&state, &org.organization_id, &connection_id, request).await?;
    record_connection_change(
        &state,
        &org,
        "assistant_connection.updated",
        &connection,
        before.as_ref().map(|previous| previous.base_url.as_str()),
    )
    .await;
    Ok(ok(connection))
}

/// Re-reads a connection's catalog. Unlike a save, a failure is recorded on the
/// row instead of refused: the connection already exists, and the list is more
/// use showing why it broke than claiming it is fine.
#[utoipa::path(
    post,
    path = "/api/v1/organizations/{organization_id}/assistant/connections/{connection_id}/refresh",
    tag = "assistant",
    responses((status = 200, description = "The re-probed connection", body = connections::ConnectionDto))
)]
pub async fn refresh_connection_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::AssistantUse>,
    Path((_organization_id, connection_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let connection = connections::refresh(&state, &org.organization_id, &connection_id).await?;
    // A refresh sends the stored key to the stored endpoint, so it belongs in
    // the trail beside the writes even though it changes no credential.
    record_connection_change(
        &state,
        &org,
        "assistant_connection.refreshed",
        &connection,
        None,
    )
    .await;
    Ok(ok(connection))
}

#[utoipa::path(
    delete,
    path = "/api/v1/organizations/{organization_id}/assistant/connections/{connection_id}",
    tag = "assistant",
    responses((status = 200, description = "The deleted connection", body = connections::ConnectionDto))
)]
pub async fn delete_connection_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::AssistantUse>,
    Path((_organization_id, connection_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let connection = connections::delete(&state, &org.organization_id, &connection_id).await?;
    record_connection_change(
        &state,
        &org,
        "assistant_connection.deleted",
        &connection,
        None,
    )
    .await;
    Ok(ok(connection))
}

// ── Web search ───────────────────────────────────────────────────────────────
//
// The search provider the assistant and MCP agents use: a SearXNG address or a
// Brave key. The key goes in and never comes back; all a person can do with a
// saved setting is turn it off or delete it.

#[utoipa::path(
    get,
    path = "/api/v1/organizations/{organization_id}/assistant/web-search",
    tag = "assistant",
    responses((status = 200, description = "The organization's web search setting, if any", body = Option<search::WebSearchDto>))
)]
pub async fn get_web_search_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgRead>,
) -> Result<impl IntoResponse, AppError> {
    Ok(ok(search::get(&state, &org.organization_id).await?))
}

/// Saving runs a real search first, so a wrong address or key fails here.
#[utoipa::path(
    post,
    path = "/api/v1/organizations/{organization_id}/assistant/web-search",
    tag = "assistant",
    request_body = search::SaveWebSearchRequest,
    responses((status = 201, description = "The stored setting", body = search::WebSearchDto))
)]
pub async fn save_web_search_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::AssistantUse>,
    Json(request): Json<search::SaveWebSearchRequest>,
) -> Result<impl IntoResponse, AppError> {
    let saved = search::save(&state, &org.organization_id, request).await?;
    record_web_search_change(&state, &org, "assistant_web_search.created", &saved).await;
    Ok(created(saved))
}

#[utoipa::path(
    patch,
    path = "/api/v1/organizations/{organization_id}/assistant/web-search",
    tag = "assistant",
    request_body = search::UpdateWebSearchRequest,
    responses((status = 200, description = "The updated setting", body = search::WebSearchDto))
)]
pub async fn update_web_search_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::AssistantUse>,
    Json(request): Json<search::UpdateWebSearchRequest>,
) -> Result<impl IntoResponse, AppError> {
    let updated = search::set_enabled(&state, &org.organization_id, request.enabled).await?;
    let action = if updated.enabled {
        "assistant_web_search.enabled"
    } else {
        "assistant_web_search.disabled"
    };
    record_web_search_change(&state, &org, action, &updated).await;
    Ok(ok(updated))
}

#[utoipa::path(
    delete,
    path = "/api/v1/organizations/{organization_id}/assistant/web-search",
    tag = "assistant",
    responses((status = 200, description = "The deleted setting", body = search::WebSearchDto))
)]
pub async fn delete_web_search_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::AssistantUse>,
) -> Result<impl IntoResponse, AppError> {
    let deleted = search::delete(&state, &org.organization_id).await?;
    record_web_search_change(&state, &org, "assistant_web_search.deleted", &deleted).await;
    Ok(ok(deleted))
}

/// A credential changed hands, so it leaves a trail. The provider and address go
/// in; the key never does.
async fn record_web_search_change(
    state: &SharedState,
    org: &crate::security::middleware::org_scope::OrgScope,
    action: &str,
    setting: &search::WebSearchDto,
) {
    crate::git::repositories::audit_log::record(
        state,
        &org.organization_id,
        &org.user,
        action,
        Some("assistant_web_search"),
        Some(&org.organization_id),
        Some(&setting.provider),
        Some(
            &serde_json::json!({
                "provider": setting.provider,
                "base_url": setting.base_url,
                "enabled": setting.enabled,
            })
            .to_string(),
        ),
        None,
        None,
        None,
    )
    .await
    .ok();
}

/// These are credentials, so every write leaves a trail. The provider, endpoint
/// and key hint go in; the key itself never does.
async fn record_connection_change(
    state: &SharedState,
    org: &crate::security::middleware::org_scope::OrgScope,
    action: &str,
    connection: &connections::ConnectionDto,
    previous_base_url: Option<&str>,
) {
    crate::git::repositories::audit_log::record(
        state,
        &org.organization_id,
        &org.user,
        action,
        Some("assistant_connection"),
        Some(&connection.id),
        Some(&connection.name),
        Some(
            &serde_json::json!({
                "provider": connection.provider,
                "base_url": connection.base_url,
                "api_key_hint": connection.api_key_hint,
                "is_default": connection.is_default,
                "status": connection.status,
                // Only when it moved: an unchanged endpoint in every row is noise.
                "previous_base_url": previous_base_url
                    .filter(|previous| *previous != connection.base_url),
            })
            .to_string(),
        ),
        None,
        None,
        None,
    )
    .await
    .ok();
}

#[cfg(test)]
mod security_tests {
    use super::*;
    use axum::{
        Extension, Router,
        body::Body,
        http::{Request, StatusCode},
        routing::post,
    };
    use tower::ServiceExt;

    #[tokio::test]
    async fn assistant_messages_and_actions_reject_other_organizations() {
        let (state, _path) = crate::testing::test_state().await;
        sqlx::query("UPDATE organization_members SET role_id = 2 WHERE user_id = 'viewer' AND organization_id = 'organization-a'")
            .execute(&state.db).await.unwrap();
        sqlx::query("UPDATE organizations SET assistant_enabled = 1")
            .execute(&state.db)
            .await
            .unwrap();
        let router = Router::new()
            .route(
                "/organizations/{organization_id}/messages",
                post(send_message_handler),
            )
            .route(
                "/organizations/{organization_id}/actions/{action_id}",
                post(settle_action_handler),
            )
            .layer(Extension(crate::testing::auth_user("viewer")))
            .with_state(state);
        for path in [
            "/organizations/organization-b/messages",
            "/organizations/organization-b/actions/action-b",
        ] {
            let response = router
                .clone()
                .oneshot(
                    Request::post(path)
                        .header("content-type", "application/json")
                        .body(Body::from(r#"{"message":"hello","decision":"confirm"}"#))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
        }
    }
}
