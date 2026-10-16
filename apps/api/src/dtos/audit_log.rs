use serde::Serialize;
use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
pub struct AuditLogDto {
    pub id: String,
    pub action: String,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    /// Snapshot of the target's name at the time of the action — survives
    /// the target itself being renamed or deleted later.
    pub target_name: Option<String>,
    pub metadata: Option<String>,
    /// `None` when the actor's user row was hard-deleted after the fact — the
    /// entry survives (`ON DELETE SET NULL`), the identity does not.
    pub actor_user_id: Option<String>,
    pub actor_email: Option<String>,
    pub actor_name: Option<String>,
    /// Set when the action was performed via a personal access token instead
    /// of a browser session — the token's own id/name, not the user's.
    pub actor_token_id: Option<String>,
    pub actor_token_name: Option<String>,
    /// Provenance at request time. Always populated for HTTP-driven writes
    /// (set by `request_context_middleware`); `None` for rows written by a
    /// background job or CLI that never had a request to stamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_ip: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
    /// Echoed back in `X-Request-Id` so the same id appears in the response,
    /// the audit row, and any error log/ticket referring to the same call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// `None` if the caller didn't classify the outcome, `"success"` /
    /// `"failure"` otherwise. Distinguishes "I don't know" from "I know it
    /// failed" so reports can show unknown-rate as its own column rather than
    /// lumping it with failures.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub created_at: String,
}

/// Field-level filters accepted by `GET /audit-logs`. Every field is optional;
/// absent means "no filter on this dimension". `before` is the keyset cursor
/// (`id` of the last row from the previous page) and is unrelated to the
/// `to` date bound.
#[derive(Default, Debug)]
pub struct AuditLogFilters<'a> {
    pub action: Option<&'a str>,
    pub target_type: Option<&'a str>,
    pub target_id: Option<&'a str>,
    pub actor_user_id: Option<&'a str>,
    pub outcome: Option<&'a str>,
    pub from: Option<&'a str>,
    pub to: Option<&'a str>,
}
