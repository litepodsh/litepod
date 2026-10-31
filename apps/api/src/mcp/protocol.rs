//! The JSON-RPC half of MCP: message shapes and the results that do not need
//! the database. Kept free of axum and state so every shape is unit-tested.
//!
//! Dual-era on purpose. Modern clients (`2026-07-28`) send their version in
//! `_meta` on every request and may call `server/discover`; legacy clients
//! (`2025-11-25` and earlier) open with `initialize`. We never mint an
//! `Mcp-Session-Id`, which legacy servers may omit and modern ones no longer
//! have, so any API replica can answer any request.

use serde::Deserialize;
use serde_json::{Value, json};

use crate::assistant::{capabilities, tools};

pub const MODERN: &str = "2026-07-28";
/// Newest first: `initialize` falls back to the first one.
pub const LEGACY: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26"];

pub const META_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
const META_SERVER_INFO: &str = "io.modelcontextprotocol/serverInfo";

pub const PARSE_ERROR: i64 = -32700;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const UNSUPPORTED_VERSION: i64 = -32022;

/// How long a client may reuse `tools/list`. The list only changes with a
/// release, so this is about not being polled, not about freshness.
const TOOLS_TTL_MS: u64 = 300_000;
const DISCOVER_TTL_MS: u64 = 3_600_000;

pub const INSTRUCTIONS: &str = "Litepod deploys and runs applications for this organization. \
    Use `get` to read; start with `resource: projects` when you have not seen a name yet. \
    To deploy any named application (Uptime Kuma, n8n, Plausible, a database, ...), ALWAYS search the \
    template catalog first with `get` and `resource: templates` and `query`, before creating anything \
    and before writing a compose file or choosing an image such as `latest`: the Litepod catalog has \
    maintained templates with their secrets and domains worked out. When it has one, offer it and ask \
    the user which template they want instead of choosing for them, then use `create_from_template`. \
    If the project the user names does not exist, create it (`exec` `create_project`, or pass the new name as \
    `project` to `create_from_template`, `deploy_compose` or `deploy_static_site`) instead of stopping to ask. \
    To search the web, use your own web search tool; Litepod's `get web_search` appears only when the \
    organization set one up. `get repositories` lists the repositories the organization's Git providers \
    can see, private ones included, and `get url` reads one public page. \
    Write your own compose only when the catalog has nothing, or the user says so after hearing the options. \
    Use `exec` to change something: every action happens immediately and returns its result. \
    For `deploy_static_site`, send the site inline with `files` or `zip_base64` so it deploys in the same call.";

#[derive(Debug, Deserialize)]
pub struct Request {
    /// Absent on a notification, which wants no answer.
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

pub fn supported() -> Vec<&'static str> {
    std::iter::once(MODERN)
        .chain(LEGACY.iter().copied())
        .collect()
}

/// The version a modern request declared. `None` for a legacy request, which
/// fixed its version once in `initialize`.
pub fn declared_version(params: &Value) -> Option<&str> {
    params.get("_meta")?.get(META_VERSION)?.as_str()
}

pub fn success(id: &Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

pub fn error(id: &Value, code: i64, message: &str, data: Option<Value>) -> Value {
    let mut error = json!({ "code": code, "message": message });
    if let Some(data) = data {
        error["data"] = data;
    }
    json!({ "jsonrpc": "2.0", "id": id, "error": error })
}

pub fn unsupported_version(id: &Value, requested: &str) -> Value {
    error(
        id,
        UNSUPPORTED_VERSION,
        "Unsupported protocol version",
        Some(json!({ "supported": supported(), "requested": requested })),
    )
}

fn server_info(version: &str) -> Value {
    json!({ "name": "litepod", "version": version })
}

pub fn discover(version: &str) -> Value {
    json!({
        "resultType": "complete",
        "supportedVersions": supported(),
        "capabilities": { "tools": {} },
        "_meta": { META_SERVER_INFO: server_info(version) },
        "instructions": INSTRUCTIONS,
        "ttlMs": DISCOVER_TTL_MS,
        // Per organization and per caller: no shared cache may reuse it.
        "cacheScope": "private",
    })
}

pub fn initialize(requested: Option<&str>, version: &str) -> Value {
    let chosen = requested
        .filter(|requested| LEGACY.contains(requested))
        .unwrap_or(LEGACY[0]);
    json!({
        "protocolVersion": chosen,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": server_info(version),
        "instructions": INSTRUCTIONS,
    })
}

/// The assistant's own tools, in its own order (deterministic, as the spec
/// asks, so clients and prompt caches can reuse it).
///
/// `web_search` is offered only when the organization has one set up and on:
/// the client's own web search is the default, and this is the fallback for an
/// agent that has none.
pub fn tools_list(web_search: bool) -> Value {
    let tools: Vec<Value> = tools::definitions_for(capabilities::ACTIONS, web_search)
        .into_iter()
        .map(|definition| {
            let function = definition.function;
            let read_only = function.name == tools::GET;
            json!({
                "name": function.name,
                "description": function.description,
                "inputSchema": function.parameters,
                "annotations": {
                    "readOnlyHint": read_only,
                    "destructiveHint": !read_only,
                    "idempotentHint": read_only,
                    "openWorldHint": false,
                },
            })
        })
        .collect();
    json!({
        "resultType": "complete",
        "tools": tools,
        "ttlMs": TOOLS_TTL_MS,
        "cacheScope": "private",
    })
}

/// A tool's answer. A tool failure is a result with `isError`, not a JSON-RPC
/// error, so the model reads it and can correct itself.
pub fn tool_result(text: String, is_error: bool) -> Value {
    json!({
        "resultType": "complete",
        "content": [{ "type": "text", "text": text }],
        "isError": is_error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_list_is_the_assistants_two_tools_with_hints() {
        let list = tools_list(true);
        assert_eq!(list["resultType"], "complete");
        assert!(list["ttlMs"].is_u64());
        let tools = list["tools"].as_array().unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(names, vec!["get", "exec"]);
        assert_eq!(tools[0]["annotations"]["readOnlyHint"], true);
        assert_eq!(tools[1]["annotations"]["destructiveHint"], true);
        assert_eq!(tools[0]["inputSchema"]["type"], "object");
    }

    #[test]
    fn web_search_is_offered_only_when_asked_for() {
        let with = tools_list(true)["tools"][0]["description"]
            .as_str()
            .unwrap()
            .to_owned();
        let without = tools_list(false);
        let without_description = without["tools"][0]["description"].as_str().unwrap();
        let without_enum =
            without["tools"][0]["inputSchema"]["properties"]["resource"]["enum"].to_string();
        assert!(with.contains("web_search"));
        assert!(!without_description.contains("`web_search`"));
        assert!(!without_enum.contains("web_search"));
        assert!(without_enum.contains("repositories"), "{without_enum}");
    }

    #[test]
    fn exec_says_what_runs_now_and_what_waits() {
        let list = tools_list(true);
        let exec = list["tools"][1]["description"].as_str().unwrap();
        assert!(exec.contains("Every action happens immediately"), "{exec}");
        assert!(!exec.contains("This NEVER performs it"), "{exec}");
    }

    #[test]
    fn a_modern_request_names_its_version_in_meta() {
        let params = json!({ "_meta": { META_VERSION: "2026-07-28" } });
        assert_eq!(declared_version(&params), Some("2026-07-28"));
        assert_eq!(declared_version(&json!({})), None);
    }

    #[test]
    fn an_unknown_version_is_refused_with_the_ones_we_speak() {
        let reply = unsupported_version(&json!(1), "1900-01-01");
        assert_eq!(reply["error"]["code"], UNSUPPORTED_VERSION);
        assert_eq!(reply["error"]["data"]["requested"], "1900-01-01");
        assert_eq!(reply["error"]["data"]["supported"][0], MODERN);
    }

    #[test]
    fn a_legacy_client_gets_the_version_it_asked_for_when_we_have_it() {
        assert_eq!(
            initialize(Some("2025-06-18"), "1.0")["protocolVersion"],
            "2025-06-18"
        );
        assert_eq!(
            initialize(Some("2024-01-01"), "1.0")["protocolVersion"],
            LEGACY[0]
        );
        assert_eq!(initialize(None, "1.0")["protocolVersion"], LEGACY[0]);
    }

    #[test]
    fn discover_is_cacheable_and_names_the_server() {
        let found = discover("9.9.9");
        assert_eq!(found["resultType"], "complete");
        assert_eq!(found["supportedVersions"][0], MODERN);
        assert_eq!(
            found["_meta"]["io.modelcontextprotocol/serverInfo"]["version"],
            "9.9.9"
        );
        assert_eq!(found["cacheScope"], "private");
    }
}
