use sqlx::Row;
use uuid::Uuid;

use crate::dtos::audit_log::{AuditLogDto, AuditLogFilters};
use crate::security::middleware::auth::AuthUser;
use crate::security::middleware::request_context::RequestContext;
use crate::state::state::SharedState;

/// Coarse-grained outcome — `None` means "unknown" and shows up distinctly in
/// reports rather than getting folded into `success`. Handlers decide based on
/// whether their work succeeded; CLI/batch callers (which don't have an
/// outcome story) pass `None`.
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)] // variants are part of the planned audit reporting interface.
pub enum AuditOutcome {
    Success,
    Failure,
}

impl AuditOutcome {
    fn as_str(self) -> &'static str {
        match self {
            AuditOutcome::Success => "success",
            AuditOutcome::Failure => "failure",
        }
    }
}

/// Writes one audit log row, unless the organization has audit logging
/// turned off. This is the only gate — call sites never check the toggle
/// themselves, so a mutation handler can't forget it and can't get it wrong.
///
/// Takes the whole `AuthUser` rather than a bare id so a personal access
/// token's identity (`token_id`/`token_name`, set by `auth_middleware` when
/// the request is authenticated by `Authorization: Bearer` instead of the
/// session cookie) is stamped on the row automatically — every existing
/// caller already had an `AuthUser` in scope, so this cost callers a
/// `.user_id` deletion each rather than a new argument.
///
/// `RequestContext` carries the IP, user agent and request id captured by
/// `request_context_middleware` so audit rows can answer "where did this
/// delete come from". Pass `&RequestContext::synthetic()` from a background
/// job or CLI seed — recording nothing is better than inventing an IP.
///
/// Never fails the caller's real operation: logging errors are the caller's
/// to swallow (`.await.ok()`) or warn on, not to propagate.
#[allow(clippy::too_many_arguments)]
pub async fn record(
    state: &SharedState,
    organization_id: &str,
    actor: &AuthUser,
    action: &str,
    target_type: Option<&str>,
    target_id: Option<&str>,
    target_name: Option<&str>,
    metadata: Option<&str>,
    context: Option<&RequestContext>,
    outcome: Option<AuditOutcome>,
    error_message: Option<&str>,
) -> Result<(), String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let enabled: Option<i64> =
        sqlx::query_scalar("SELECT audit_logs_enabled FROM organizations WHERE id = ?1")
            .bind(organization_id)
            .fetch_optional(&conn)
            .await
            .map_err(|e| e.to_string())?;
    if enabled.unwrap_or(0) == 0 {
        return Ok(());
    }
    // `None` for the request context = background work (reconcile, prune,
    // webhook delivery) where there is no live HTTP request. Synthesize a row
    // id so the column is non-null and the log entry can still be referenced;
    // leave IP and user-agent `NULL` since we genuinely don't have them.
    let context = context.cloned().unwrap_or_else(RequestContext::synthetic);
    let ip_str = context.source_ip.map(|ip| ip.to_string());
    sqlx::query(
        "INSERT INTO audit_logs (id, organization_id, actor_user_id, action, target_type, target_id, target_name, metadata, actor_token_id, actor_token_name, source_ip, user_agent, request_id, outcome, error_message) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
    )
    .bind(Uuid::now_v7().to_string())
    .bind(organization_id)
    // An empty id is a non-user actor (SCIM), named by `token_name` instead.
    .bind(Some(actor.user_id.as_str()).filter(|id| !id.is_empty()))
    .bind(action)
    .bind(target_type)
    .bind(target_id)
    .bind(target_name)
    .bind(metadata)
    .bind(&actor.token_id)
    .bind(&actor.token_name)
    .bind(ip_str)
    .bind(context.user_agent.as_deref())
    .bind(&context.request_id)
    .bind(outcome.map(|o| o.as_str()))
    .bind(error_message)
    .execute(&conn)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn row_to_dto(row: &sqlx::sqlite::SqliteRow) -> AuditLogDto {
    AuditLogDto {
        id: row.try_get(0).unwrap(),
        action: row.try_get(1).unwrap(),
        target_type: row.try_get(2).unwrap_or_default(),
        target_id: row.try_get(3).unwrap_or_default(),
        target_name: row.try_get(4).unwrap_or_default(),
        metadata: row.try_get(5).unwrap_or_default(),
        actor_user_id: row.try_get(6).unwrap_or_default(),
        actor_email: row.try_get(7).unwrap_or_default(),
        actor_name: row.try_get(8).unwrap_or_default(),
        actor_token_id: row.try_get(9).unwrap_or_default(),
        actor_token_name: row.try_get(10).unwrap_or_default(),
        source_ip: row.try_get(11).unwrap_or_default(),
        user_agent: row.try_get(12).unwrap_or_default(),
        request_id: row.try_get(13).unwrap_or_default(),
        outcome: row.try_get(14).unwrap_or_default(),
        error_message: row.try_get(15).unwrap_or_default(),
        created_at: row.try_get(16).unwrap(),
    }
}

/// Keyset pagination on `id` (a time-ordered UUIDv7): `before` is the last id
/// seen on the previous page, or `None` for the first page. `filters` lets
/// callers narrow by action / target / actor / outcome / date; `None` for any
/// field means "no filter on this dimension".
pub async fn list_for_organization(
    state: &SharedState,
    organization_id: &str,
    limit: i64,
    before: Option<&str>,
    filters: &AuditLogFilters<'_>,
) -> Result<Vec<AuditLogDto>, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    // Build the query with `1=1` predicates plus optional `AND`s so the
    // branch logic lives in SQL bind parameters, not in string concatenation.
    let rows = sqlx::query(
        "SELECT a.id, a.action, a.target_type, a.target_id, a.target_name, a.metadata, \
                a.actor_user_id, u.email, u.name, a.actor_token_id, a.actor_token_name, \
                a.source_ip, a.user_agent, a.request_id, a.outcome, a.error_message, a.created_at \
         FROM audit_logs a \
         LEFT JOIN users u ON u.id = a.actor_user_id \
         WHERE a.organization_id = ?1 \
           AND (?2 IS NULL OR a.id < ?2) \
           AND (?3 IS NULL OR a.action = ?3) \
           AND (?4 IS NULL OR a.target_type = ?4) \
           AND (?5 IS NULL OR a.target_id = ?5) \
           AND (?6 IS NULL OR a.actor_user_id = ?6) \
           AND (?7 IS NULL OR a.outcome = ?7) \
           AND (?8 IS NULL OR a.created_at >= ?8) \
           AND (?9 IS NULL OR a.created_at <= ?9) \
         ORDER BY a.id DESC \
         LIMIT ?10",
    )
    .bind(organization_id)
    .bind(before)
    .bind(filters.action)
    .bind(filters.target_type)
    .bind(filters.target_id)
    .bind(filters.actor_user_id)
    .bind(filters.outcome)
    .bind(filters.from)
    .bind(filters.to)
    .bind(limit)
    .fetch_all(&conn)
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows.iter().map(row_to_dto).collect())
}

/// Full window for CSV export rather than a page: `from`/`to` are inclusive
/// `YYYY-MM-DD HH:MM:SS` strings (SQLite's own `datetime()` format) or `None`
/// for an open end. Capped at 50k rows — a sane ceiling for a CSV response
/// held in memory, not a real pagination scheme; ask for a narrower range if
/// you hit it.
const EXPORT_ROW_LIMIT: i64 = 50_000;

pub async fn list_for_export(
    state: &SharedState,
    organization_id: &str,
    from: Option<&str>,
    to: Option<&str>,
) -> Result<Vec<AuditLogDto>, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(
        "SELECT a.id, a.action, a.target_type, a.target_id, a.target_name, a.metadata, a.actor_user_id, u.email, u.name, a.actor_token_id, a.actor_token_name, a.source_ip, a.user_agent, a.request_id, a.outcome, a.error_message, a.created_at
         FROM audit_logs a
         LEFT JOIN users u ON u.id = a.actor_user_id
         WHERE a.organization_id = ?1
           AND (?2 IS NULL OR a.created_at >= ?2)
           AND (?3 IS NULL OR a.created_at <= ?3)
         ORDER BY a.id DESC
         LIMIT ?4",
    )
    .bind(organization_id)
    .bind(from)
    .bind(to)
    .bind(EXPORT_ROW_LIMIT)
    .fetch_all(&conn)
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows.iter().map(row_to_dto).collect())
}

/// Deletes every entry past its organization's own retention window, in one
/// query across all orgs — no per-org loop needed since the cutoff is a join
/// condition, not an application-side constant.
pub async fn purge_expired(state: &SharedState) -> Result<u64, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let result = sqlx::query(
        "DELETE FROM audit_logs WHERE id IN (
            SELECT a.id FROM audit_logs a
            JOIN organizations o ON o.id = a.organization_id
            WHERE a.created_at < datetime('now', '-' || o.audit_logs_retention_days || ' days')
        )",
    )
    .execute(&conn)
    .await
    .map_err(|e| e.to_string())?;
    Ok(result.rows_affected())
}

/// Same shape as `scheduled_tasks::scheduler::spawn` / `backup::scheduler::spawn`:
/// purge once at startup, then daily.
pub fn spawn(state: SharedState) {
    tokio::spawn(async move {
        if let Err(error) = purge_expired(&state).await {
            tracing::warn!(%error, "audit log purge failed");
        }
        let mut daily = tokio::time::interval(std::time::Duration::from_secs(24 * 60 * 60));
        loop {
            daily.tick().await;
            if let Err(error) = purge_expired(&state).await {
                tracing::warn!(%error, "audit log purge failed");
            }
        }
    });
}
