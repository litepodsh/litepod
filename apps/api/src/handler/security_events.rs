use axum::{
    Extension,
    extract::{Query, State},
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use sqlx::Row;

use crate::dtos::response::{AppError, ok};
use crate::security::middleware::auth::AuthUser;
use crate::state::state::SharedState;

const MAX_LIMIT: i64 = 200;

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SecurityEventsQuery {
    /// Events about or performed by this user.
    pub user_id: Option<String>,
    pub ip: Option<String>,
    pub action: Option<String>,
    /// Only rows the anomaly detector flagged.
    #[serde(default)]
    pub flagged: bool,
    /// Keyset cursor: the last `id` of the previous page.
    pub before: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SecurityEventDto {
    pub id: String,
    pub action: String,
    pub user_id: Option<String>,
    pub user_email: Option<String>,
    pub user_name: Option<String>,
    pub actor_user_id: Option<String>,
    pub actor_email: Option<String>,
    pub actor_token_name: Option<String>,
    pub metadata: Option<String>,
    pub source_ip: Option<String>,
    pub user_agent: Option<String>,
    pub country: Option<String>,
    pub city: Option<String>,
    pub asn: Option<String>,
    pub flag: Option<String>,
    pub created_at: String,
}

#[utoipa::path(
    get,
    path = "/api/v1/admin/security-events",
    params(
        ("user_id" = Option<String>, Query, description = "Events about or by this user"),
        ("ip" = Option<String>, Query, description = "Exact source IP"),
        ("action" = Option<String>, Query, description = "Exact action, e.g. auth.login"),
        ("flagged" = Option<bool>, Query, description = "Only suspicious events"),
        ("before" = Option<String>, Query, description = "Return events older than this id"),
        ("limit" = Option<i64>, Query, description = "Page size, at most 200"),
    ),
    responses(
        (status = 200, description = "Security events, newest first (cloud SysAdmin only)", body = [SecurityEventDto]),
    ),
    tag = "users"
)]
pub async fn list_security_events_handler(
    State(state): State<SharedState>,
    Extension(user): Extension<AuthUser>,
    Query(query): Query<SecurityEventsQuery>,
) -> Result<impl IntoResponse, AppError> {
    if !state.is_cloud {
        return Err(AppError::ResourceNotFound);
    }
    if !crate::git::repositories::user::is_sysadmin(&state, &user.user_id).await {
        return Err(AppError::InsufficientPermissions);
    }
    let blank = |value: Option<String>| value.filter(|value| !value.trim().is_empty());
    let conn = crate::git::repositories::db::get_conn(&state).await;
    let rows = sqlx::query(
        "SELECT e.id, e.action, e.user_id, u.email, u.name, e.actor_user_id, a.email, e.actor_token_name,
                e.metadata, e.source_ip, e.user_agent, e.country, e.city, e.asn, e.flag, e.created_at
         FROM security_events e
         LEFT JOIN users u ON u.id = e.user_id
         LEFT JOIN users a ON a.id = e.actor_user_id
         WHERE (?1 IS NULL OR e.user_id = ?1 OR e.actor_user_id = ?1)
           AND (?2 IS NULL OR e.source_ip = ?2)
           AND (?3 IS NULL OR e.action = ?3)
           AND (?4 = 0 OR e.flag IS NOT NULL)
           AND (?5 IS NULL OR e.id < ?5)
         ORDER BY e.id DESC
         LIMIT ?6",
    )
    .bind(blank(query.user_id))
    .bind(blank(query.ip))
    .bind(blank(query.action))
    .bind(query.flagged)
    .bind(blank(query.before))
    .bind(query.limit.unwrap_or(100).clamp(1, MAX_LIMIT))
    .fetch_all(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let events = rows
        .iter()
        .map(|row| {
            Ok(SecurityEventDto {
                id: row.try_get(0)?,
                action: row.try_get(1)?,
                user_id: row.try_get(2)?,
                user_email: row.try_get(3)?,
                user_name: row.try_get(4)?,
                actor_user_id: row.try_get(5)?,
                actor_email: row.try_get(6)?,
                actor_token_name: row.try_get(7)?,
                metadata: row.try_get(8)?,
                source_ip: row.try_get(9)?,
                user_agent: row.try_get(10)?,
                country: row.try_get(11)?,
                city: row.try_get(12)?,
                asn: row.try_get(13)?,
                flag: row.try_get(14)?,
                created_at: row.try_get(15)?,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok(ok(events))
}
