//! `POST /api/v1/organizations/{organization_id}/mcp` - the MCP endpoint.
//!
//! One route, stateless, JSON in and JSON out. The organization is in the
//! path, so `RequireOrg<perm::AssistantUse>` does membership, org-wide Editor
//! and token capping before any JSON-RPC is read. Transport-level refusals
//! (off, not a member, wrong role) are HTTP statuses; everything after that is
//! a JSON-RPC answer, and a tool's own failure is a result with `isError`.

use axum::{
    Json,
    body::Bytes,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};

use crate::{
    assistant::{actions, capabilities, tools},
    dtos::response::AppError,
    mcp::{self as server, direct, protocol, session},
    security::{
        authz::extract::{RequireOrg, perm},
        middleware::org_scope::OrgScope,
    },
    state::state::SharedState,
};

pub(crate) async fn mcp_on(state: &SharedState, organization_id: &str) -> Result<(), AppError> {
    // Off for the whole install: the route should look like it does not exist.
    if !server::instance_enabled() {
        return Err(AppError::ResourceNotFound);
    }
    let organization =
        crate::git::repositories::organization::get_organization_by_id(state, organization_id)
            .await
            .ok_or(AppError::OrganizationNotFound)?;
    if !organization.mcp_enabled {
        return Err(AppError::ValidationError(
            "The MCP server is turned off for this organization. An admin can turn it on in organization settings.".into(),
        ));
    }
    Ok(())
}

#[utoipa::path(
    post,
    path = "/api/v1/organizations/{organization_id}/mcp",
    tag = "assistant",
    params(("organization_id" = String, Path, description = "Organization scope")),
    request_body(content = String, description = "One JSON-RPC 2.0 request or notification", content_type = "application/json"),
    responses(
        (status = 200, description = "A JSON-RPC response"),
        (status = 202, description = "A notification was accepted")
    )
)]
pub async fn mcp_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::AssistantUse>,
    body: Bytes,
) -> Result<Response, AppError> {
    mcp_on(&state, &org.organization_id).await?;

    let request: protocol::Request = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(error) => {
            let reply = protocol::error(
                &Value::Null,
                protocol::PARSE_ERROR,
                &error.to_string(),
                None,
            );
            return Ok(Json(reply).into_response());
        }
    };
    let Some(id) = request.id.clone() else {
        return Ok(StatusCode::ACCEPTED.into_response());
    };
    if let Some(requested) = protocol::declared_version(&request.params)
        && !protocol::supported().contains(&requested)
    {
        return Ok(Json(protocol::unsupported_version(&id, requested)).into_response());
    }

    let reply = match request.method.as_str() {
        "server/discover" => protocol::success(&id, protocol::discover(&state.version)),
        "initialize" => protocol::success(
            &id,
            protocol::initialize(
                request
                    .params
                    .get("protocolVersion")
                    .and_then(Value::as_str),
                &state.version,
            ),
        ),
        "ping" => protocol::success(&id, json!({})),
        "tools/list" => protocol::success(&id, protocol::tools_list()),
        "tools/call" => match call(&state, &org, &request.params).await {
            Ok(result) => protocol::success(&id, result),
            Err(message) => protocol::error(&id, protocol::INVALID_PARAMS, &message, None),
        },
        other => protocol::error(
            &id,
            protocol::METHOD_NOT_FOUND,
            &format!("`{other}` is not a method this server has"),
            None,
        ),
    };
    Ok(Json(reply).into_response())
}

/// `tools/call`. `Err` is a malformed call (JSON-RPC invalid params); anything
/// the tool itself has to say comes back as `Ok` with `isError`.
async fn call(state: &SharedState, org: &OrgScope, params: &Value) -> Result<Value, String> {
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return Err("`name` is required".into());
    };
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));

    if name == tools::EXEC
        && let Some(action) = arguments.get("action").and_then(Value::as_str)
        && capabilities::mcp_direct(action)
    {
        let outcome = direct::run(state, org, action, &arguments).await;
        return Ok(protocol::tool_result(outcome.text, outcome.is_error));
    }

    // Only a proposal needs a conversation to hang off; a read creates none.
    let conversation_id = if name == tools::EXEC {
        session::conversation_for(state, &org.organization_id, &org.user)
            .await
            .map_err(|error| error.to_string())?
    } else {
        String::new()
    };
    let outcome = tools::run(
        state,
        &org.organization_id,
        &conversation_id,
        &org.user,
        name,
        &arguments.to_string(),
    )
    .await;

    // Confirmed here, as the token's user, instead of waiting in the panel. A
    // proposal that failed to store has no action and falls through, and so does
    // a static site still waiting for its upload.
    let uploaded = serde_json::from_str::<Value>(&outcome.result)
        .ok()
        .and_then(|result| result.get("uploaded").and_then(Value::as_bool))
        .unwrap_or(false);
    if name == tools::EXEC
        && let Some(action) = arguments.get("action").and_then(Value::as_str)
        && capabilities::mcp_auto_confirm(action)
        && (action != "deploy_static_site" || uploaded)
        && let Some(proposed) = outcome.action.as_ref()
    {
        let (result, is_error) =
            match actions::confirm(state, &org.organization_id, &org.user, &proposed.id).await {
                Ok(applied) => (
                    json!({
                        "done": true,
                        "action": action,
                        "action_id": applied.id,
                        "title": applied.title,
                        "status": applied.status,
                    }),
                    false,
                ),
                Err(error) => (
                    json!({ "error": error.to_string(), "action_id": proposed.id }),
                    true,
                ),
            };
        return Ok(protocol::tool_result(result.to_string(), is_error));
    }

    let is_error = serde_json::from_str::<Value>(&outcome.result)
        .ok()
        .is_some_and(|result| result.get("error").is_some());
    Ok(protocol::tool_result(
        tools::truncate(outcome.result),
        is_error,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Extension, Router,
        body::Body,
        http::{Request, StatusCode},
        routing::post,
    };
    use serde_json::{Value, json};
    use tower::ServiceExt;

    async fn router(user: &str, enabled: bool) -> (Router, std::path::PathBuf) {
        let (state, path) = crate::testing::test_state().await;
        crate::testing::seed_application(&state, "organization-a").await;
        sqlx::query("UPDATE organizations SET mcp_enabled = ?1 WHERE id = 'organization-a'")
            .bind(i64::from(enabled))
            .execute(&state.db)
            .await
            .unwrap();
        let router = Router::new()
            .route("/organizations/{organization_id}/mcp", post(mcp_handler))
            .layer(Extension(crate::testing::auth_user(user)))
            .with_state(state);
        (router, path)
    }

    async fn send(router: &Router, organization: &str, body: Value) -> (StatusCode, Value) {
        let response = router
            .clone()
            .oneshot(
                Request::post(format!("/organizations/{organization}/mcp"))
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, value)
    }

    fn call(id: i64, method: &str, params: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
    }

    #[tokio::test]
    async fn an_editor_lists_the_two_tools() {
        let (router, path) = router("editor", true).await;
        let (status, reply) =
            send(&router, "organization-a", call(1, "tools/list", json!({}))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(reply["id"], 1);
        assert_eq!(reply["result"]["tools"][0]["name"], "get");
        assert_eq!(reply["result"]["tools"][1]["name"], "exec");
        crate::testing::cleanup_database(&path);
    }

    #[tokio::test]
    async fn off_means_refused() {
        let (router, path) = router("editor", false).await;
        let (status, _) = send(&router, "organization-a", call(1, "tools/list", json!({}))).await;
        // Same status as the assistant's own off switch (`ValidationError`).
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        crate::testing::cleanup_database(&path);
    }

    #[tokio::test]
    async fn a_viewer_cannot_drive_the_organization() {
        let (router, path) = router("viewer", true).await;
        let (status, _) = send(&router, "organization-a", call(1, "tools/list", json!({}))).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        crate::testing::cleanup_database(&path);
    }

    #[tokio::test]
    async fn another_organization_is_not_reachable() {
        let (router, path) = router("viewer", true).await;
        let (status, _) = send(&router, "organization-b", call(1, "tools/list", json!({}))).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        crate::testing::cleanup_database(&path);
    }

    #[tokio::test]
    async fn a_notification_gets_accepted_and_no_body() {
        let (router, path) = router("editor", true).await;
        let (status, _) = send(
            &router,
            "organization-a",
            json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);
        crate::testing::cleanup_database(&path);
    }

    #[tokio::test]
    async fn an_unknown_modern_version_is_refused_in_json_rpc() {
        let (router, path) = router("editor", true).await;
        let params =
            json!({ "_meta": { "io.modelcontextprotocol/protocolVersion": "1900-01-01" } });
        let (_, reply) = send(&router, "organization-a", call(2, "tools/list", params)).await;
        assert_eq!(reply["error"]["code"], -32022);
        crate::testing::cleanup_database(&path);
    }

    #[tokio::test]
    async fn get_reads_through_the_assistant() {
        let (router, path) = router("editor", true).await;
        let params = json!({ "name": "get", "arguments": { "resource": "projects" } });
        let (_, reply) = send(&router, "organization-a", call(3, "tools/call", params)).await;
        assert_eq!(reply["result"]["isError"], false, "{reply}");
        let text = reply["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("organization-a-project"), "{text}");
        crate::testing::cleanup_database(&path);
    }

    #[tokio::test]
    async fn a_direct_stop_of_something_that_does_not_exist_says_so() {
        let (router, path) = router("editor", true).await;
        let params = json!({
            "name": "exec",
            "arguments": { "action": "stop", "application": "no-such-app" }
        });
        let (_, reply) = send(&router, "organization-a", call(4, "tools/call", params)).await;
        assert_eq!(reply["result"]["isError"], true, "{reply}");
        crate::testing::cleanup_database(&path);
    }

    #[tokio::test]
    async fn any_other_exec_is_confirmed_in_the_same_call() {
        let (router, path) = router("editor", true).await;
        let params = json!({
            "name": "exec",
            "arguments": { "action": "create_project", "name": "fresh", "reason": "test" }
        });
        let (_, reply) = send(&router, "organization-a", call(5, "tools/call", params)).await;
        assert_eq!(reply["result"]["isError"], false, "{reply}");
        let text = reply["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("\"done\":true"), "{text}");
        assert!(text.contains("\"status\":\"applied\""), "{text}");
        crate::testing::cleanup_database(&path);
    }

    #[tokio::test]
    async fn a_static_site_without_its_upload_still_waits() {
        let (router, path) = router("editor", true).await;
        let params = json!({
            "name": "exec",
            "arguments": { "action": "deploy_static_site", "project": "fresh", "name": "site", "server": "organization-a-server", "reason": "test" }
        });
        let (_, reply) = send(&router, "organization-a", call(5, "tools/call", params)).await;
        let text = reply["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("\"proposed\":true"), "{text}");
        assert!(text.contains("upload_url"), "{text}");
        crate::testing::cleanup_database(&path);
    }

    #[tokio::test]
    async fn an_unknown_method_is_a_json_rpc_error() {
        let (router, path) = router("editor", true).await;
        let (_, reply) = send(
            &router,
            "organization-a",
            call(6, "resources/list", json!({})),
        )
        .await;
        assert_eq!(reply["error"]["code"], -32601);
        crate::testing::cleanup_database(&path);
    }
}
