//! All SQL for `notification_providers`, `notification_subscriptions` and
//! `notification_deliveries`.

use chrono::Utc;
use serde::Serialize;
use sqlx::Row;

use crate::dtos::response::AppError;
use crate::state::state::SharedState;

use super::event::Action;
use super::provider::{ProviderConfig, ProviderKind};

fn db(error: impl std::fmt::Display) -> AppError {
    AppError::DatabaseError(error.to_string())
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

/// Internal shape: carries the encrypted blob. Never serialized to a client.
#[derive(Debug, Clone)]
pub struct ProviderRow {
    pub id: String,
    pub organization_id: String,
    pub name: String,
    pub kind: ProviderKind,
    pub enabled: bool,
    pub decoration: bool,
    pub config_encrypted: String,
    pub config_hint: String,
    pub created_at: i64,
    pub updated_at: i64,
}

impl ProviderRow {
    pub fn config(&self) -> Result<ProviderConfig, AppError> {
        ProviderConfig::decrypt(&self.config_encrypted)
    }
}

/// Client-facing shape. The credential is represented only by `config_hint`,
/// the same way `S3ConfigDto` exposes `has_secret_key` instead of the key.
#[derive(Debug, Clone, Serialize)]
pub struct ProviderDto {
    pub id: String,
    pub organization_id: String,
    pub name: String,
    pub kind: String,
    pub enabled: bool,
    pub decoration: bool,
    pub config_hint: String,
    pub actions: Vec<String>,
    pub created_at: i64,
    pub updated_at: i64,
    /// Most recent delivery for this provider, so the UI can show health
    /// without a second round trip per card.
    pub last_delivery: Option<DeliveryDto>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeliveryDto {
    pub id: String,
    pub provider_id: String,
    pub action: String,
    pub status: String,
    pub attempts: i64,
    pub next_attempt_at: i64,
    pub title: String,
    pub last_error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

const PROVIDER_COLUMNS: &str = "id, organization_id, name, kind, enabled, decoration, config_encrypted, config_hint, created_at, updated_at";

fn read_provider(row: &sqlx::sqlite::SqliteRow) -> Result<ProviderRow, AppError> {
    let kind_raw: String = row.try_get(3).map_err(db)?;
    let kind = ProviderKind::parse(&kind_raw)
        .ok_or_else(|| AppError::InternalError(format!("unknown provider kind '{kind_raw}'")))?;
    let enabled: i64 = row.try_get(4).map_err(db)?;
    let decoration: i64 = row.try_get(5).map_err(db)?;
    Ok(ProviderRow {
        id: row.try_get(0).map_err(db)?,
        organization_id: row.try_get(1).map_err(db)?,
        name: row.try_get(2).map_err(db)?,
        kind,
        enabled: enabled != 0,
        decoration: decoration != 0,
        config_encrypted: row.try_get(6).map_err(db)?,
        config_hint: row.try_get(7).map_err(db)?,
        created_at: row.try_get(8).map_err(db)?,
        updated_at: row.try_get(9).map_err(db)?,
    })
}

// ── Providers ─────────────────────────────────────────────────────────────

pub async fn list_providers(
    state: &SharedState,
    organization_id: &str,
) -> Result<Vec<ProviderRow>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(&format!(
        "SELECT {PROVIDER_COLUMNS} FROM notification_providers WHERE organization_id = ?1 ORDER BY created_at ASC"
    ))
    .bind(organization_id)
    .fetch_all(&conn)
    .await
    .map_err(db)?;
    rows.iter().map(read_provider).collect()
}

pub async fn get_provider(
    state: &SharedState,
    organization_id: &str,
    id: &str,
) -> Result<Option<ProviderRow>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(&format!(
        "SELECT {PROVIDER_COLUMNS} FROM notification_providers WHERE id = ?1 AND organization_id = ?2"
    ))
    .bind(id)
    .bind(organization_id)
    .fetch_optional(&conn)
    .await
    .map_err(db)?
    .map(|row| read_provider(&row))
    .transpose()
}

/// Lookup by id alone, for the worker — by the time a delivery is drained the
/// organization context is gone and is not needed.
pub async fn get_provider_any_organization(
    state: &SharedState,
    id: &str,
) -> Result<Option<ProviderRow>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(&format!(
        "SELECT {PROVIDER_COLUMNS} FROM notification_providers WHERE id = ?1"
    ))
    .bind(id)
    .fetch_optional(&conn)
    .await
    .map_err(db)?
    .map(|row| read_provider(&row))
    .transpose()
}

pub struct NewProvider<'a> {
    pub organization_id: &'a str,
    pub name: &'a str,
    pub kind: ProviderKind,
    pub enabled: bool,
    pub decoration: bool,
    pub config_encrypted: String,
    pub config_hint: String,
    pub created_by: Option<&'a str>,
}

pub async fn insert_provider(
    state: &SharedState,
    new: NewProvider<'_>,
) -> Result<String, AppError> {
    let id = uuid::Uuid::now_v7().to_string();
    let now = now_ms();
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "INSERT INTO notification_providers (id, organization_id, name, kind, enabled, decoration, config_encrypted, config_hint, created_by, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
    )
    .bind(id.clone())
    .bind(new.organization_id)
    .bind(new.name)
    .bind(new.kind.key())
    .bind(new.enabled as i64)
    .bind(new.decoration as i64)
    .bind(new.config_encrypted)
    .bind(new.config_hint)
    .bind(new.created_by)
    .bind(now)
    .bind(now)
    .execute(&conn)
    .await
    .map_err(|error| {
        // The (organization_id, name) unique index is the only one that can trip here.
        if error.to_string().contains("UNIQUE") {
            AppError::AlreadyExists
        } else {
            db(error)
        }
    })?;
    Ok(id)
}

pub async fn update_provider(
    state: &SharedState,
    id: &str,
    name: &str,
    enabled: bool,
    decoration: bool,
    config_encrypted: &str,
    config_hint: &str,
) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE notification_providers
         SET name = ?1, enabled = ?2, decoration = ?3, config_encrypted = ?4, config_hint = ?5, updated_at = ?6
         WHERE id = ?7",
    )
    .bind(name)
    .bind(enabled as i64)
    .bind(decoration as i64)
    .bind(config_encrypted)
    .bind(config_hint)
    .bind(now_ms())
    .bind(id)
    .execute(&conn)
    .await
    .map_err(|error| {
        if error.to_string().contains("UNIQUE") {
            AppError::AlreadyExists
        } else {
            db(error)
        }
    })?;
    Ok(())
}

pub async fn delete_provider(
    state: &SharedState,
    organization_id: &str,
    id: &str,
) -> Result<u64, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("DELETE FROM notification_providers WHERE id = ?1 AND organization_id = ?2")
        .bind(id)
        .bind(organization_id)
        .execute(&conn)
        .await
        .map_err(db)
        .map(|result| result.rows_affected())
}

// ── Subscriptions ─────────────────────────────────────────────────────────

pub async fn list_actions(state: &SharedState, provider_id: &str) -> Result<Vec<String>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(
        "SELECT action FROM notification_subscriptions WHERE provider_id = ?1 ORDER BY action",
    )
    .bind(provider_id)
    .fetch_all(&conn)
    .await
    .map_err(db)?;
    let mut out = Vec::new();
    for row in &rows {
        out.push(row.try_get(0).map_err(db)?);
    }
    Ok(out)
}

/// Replaces the whole subscription set for a provider. Simpler and less
/// error-prone than diffing, and the set is at most five rows.
pub async fn replace_actions(
    state: &SharedState,
    provider_id: &str,
    actions: &[Action],
) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("DELETE FROM notification_subscriptions WHERE provider_id = ?1")
        .bind(provider_id)
        .execute(&conn)
        .await
        .map_err(db)?;
    for action in actions {
        sqlx::query("INSERT INTO notification_subscriptions (provider_id, action) VALUES (?1, ?2)")
            .bind(provider_id)
            .bind(action.key())
            .execute(&conn)
            .await
            .map_err(db)?;
    }
    Ok(())
}

/// Enabled providers subscribed to `action` within one organization.
pub async fn subscribers_for_organization(
    state: &SharedState,
    organization_id: &str,
    action: Action,
) -> Result<Vec<ProviderRow>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(&format!(
        "SELECT {} FROM notification_providers p
         JOIN notification_subscriptions s ON s.provider_id = p.id
         WHERE p.organization_id = ?1 AND s.action = ?2 AND p.enabled = 1",
        PROVIDER_COLUMNS
            .split(", ")
            .map(|column| format!("p.{column}"))
            .collect::<Vec<_>>()
            .join(", ")
    ))
    .bind(organization_id)
    .bind(action.key())
    .fetch_all(&conn)
    .await
    .map_err(db)?;
    rows.iter().map(read_provider).collect()
}

/// Enabled providers on the root organization subscribed to a platform-wide action.
pub async fn subscribers_for_global(
    state: &SharedState,
    action: Action,
) -> Result<Vec<ProviderRow>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(&format!(
        "SELECT {} FROM notification_providers p
         JOIN notification_subscriptions s ON s.provider_id = p.id
         JOIN organizations t ON t.id = p.organization_id
         WHERE s.action = ?1 AND p.enabled = 1 AND t.is_root = 1",
        PROVIDER_COLUMNS
            .split(", ")
            .map(|column| format!("p.{column}"))
            .collect::<Vec<_>>()
            .join(", ")
    ))
    .bind(action.key())
    .fetch_all(&conn)
    .await
    .map_err(db)?;
    rows.iter().map(read_provider).collect()
}

pub async fn organization_is_root(
    state: &SharedState,
    organization_id: &str,
) -> Result<bool, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let Some(row) = sqlx::query("SELECT is_root FROM organizations WHERE id = ?1")
        .bind(organization_id)
        .fetch_optional(&conn)
        .await
        .map_err(db)?
    else {
        return Err(AppError::OrganizationNotFound);
    };
    let is_root: i64 = row.try_get(0).map_err(db)?;
    Ok(is_root != 0)
}

// ── Deployment context ────────────────────────────────────────────────────

pub struct RunContext {
    pub organization_id: String,
    pub application_id: String,
    pub application_name: String,
    pub project_id: String,
    pub project_name: String,
    pub environment_id: String,
    pub environment_name: String,
    pub commit_sha: Option<String>,
    pub commit_message: Option<String>,
    pub triggered_by: String,
    pub started_at: i64,
}

/// `application_deployment_runs` has no `organization_id`; walk up to the project to
/// find it. Also grabs the names the message needs, in one query.
pub async fn run_context(
    state: &SharedState,
    run_id: &str,
) -> Result<Option<RunContext>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let Some(row) = sqlx::query(
        "SELECT p.organization_id, a.id, a.name, p.id, p.name, e.id, e.name,
                r.commit_sha, r.commit_message, r.triggered_by, r.started_at
         FROM application_deployment_runs r
         JOIN applications a ON a.id = r.application_id
         JOIN environments e ON e.id = a.environment_id
         JOIN projects p ON p.id = e.project_id
         WHERE r.id = ?1",
    )
    .bind(run_id)
    .fetch_optional(&conn)
    .await
    .map_err(db)?
    else {
        return Ok(None);
    };
    Ok(Some(RunContext {
        organization_id: row.try_get(0).map_err(db)?,
        application_id: row.try_get(1).map_err(db)?,
        application_name: row.try_get(2).map_err(db)?,
        project_id: row.try_get(3).map_err(db)?,
        project_name: row.try_get(4).map_err(db)?,
        environment_id: row.try_get(5).map_err(db)?,
        environment_name: row.try_get(6).map_err(db)?,
        commit_sha: row.try_get(7).map_err(db)?,
        commit_message: row.try_get(8).map_err(db)?,
        triggered_by: row.try_get(9).map_err(db)?,
        started_at: row.try_get(10).map_err(db)?,
    }))
}

/// The run's status *before* the terminal write, which is what distinguishes a
/// build failure from a deploy failure.
pub async fn run_phase(state: &SharedState, run_id: &str) -> Result<Option<String>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("SELECT status FROM application_deployment_runs WHERE id = ?1")
        .bind(run_id)
        .fetch_optional(&conn)
        .await
        .map_err(db)?
        .map(|row| row.try_get(0).map_err(db))
        .transpose()
}

// ── Deliveries ────────────────────────────────────────────────────────────

const DELIVERY_COLUMNS: &str = "id, provider_id, action, status, attempts, next_attempt_at, payload, last_error, created_at, updated_at";

fn read_delivery(row: &sqlx::sqlite::SqliteRow) -> Result<DeliveryDto, AppError> {
    let payload: String = row.try_get(6).map_err(db)?;
    // The title is the only part of the payload the UI needs; parsing it here
    // keeps the raw event JSON out of the API response.
    let title = serde_json::from_str::<serde_json::Value>(&payload)
        .ok()
        .and_then(|value| {
            value
                .get("title")
                .and_then(|title| title.as_str())
                .map(|title| title.to_string())
        })
        .unwrap_or_default();
    Ok(DeliveryDto {
        id: row.try_get(0).map_err(db)?,
        provider_id: row.try_get(1).map_err(db)?,
        action: row.try_get(2).map_err(db)?,
        status: row.try_get(3).map_err(db)?,
        attempts: row.try_get(4).map_err(db)?,
        next_attempt_at: row.try_get(5).map_err(db)?,
        title,
        last_error: row.try_get(7).map_err(db)?,
        created_at: row.try_get(8).map_err(db)?,
        updated_at: row.try_get(9).map_err(db)?,
    })
}

pub async fn enqueue_delivery(
    state: &SharedState,
    provider_id: &str,
    action: Action,
    payload_json: &str,
) -> Result<String, AppError> {
    let id = uuid::Uuid::now_v7().to_string();
    let now = now_ms();
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "INSERT INTO notification_deliveries (id, provider_id, action, status, attempts, next_attempt_at, payload, created_at, updated_at)
         VALUES (?1, ?2, ?3, 'pending', 0, ?4, ?5, ?6, ?7)",
    )
    .bind(id.clone())
    .bind(provider_id)
    .bind(action.key())
    .bind(now)
    .bind(payload_json)
    .bind(now)
    .bind(now)
    .execute(&conn)
    .await
    .map_err(db)?;
    Ok(id)
}

pub struct DueDelivery {
    pub id: String,
    pub provider_id: String,
    pub attempts: i64,
    pub payload: String,
}

/// Claims due rows by flipping them to `sending` first, so a second worker (or
/// a second process) cannot pick up the same delivery.
pub async fn claim_due(state: &SharedState, limit: usize) -> Result<Vec<DueDelivery>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let now = now_ms();
    let rows = sqlx::query(
        "SELECT id, provider_id, attempts, payload FROM notification_deliveries
         WHERE status = 'pending' AND next_attempt_at <= ?1
         ORDER BY next_attempt_at ASC LIMIT ?2",
    )
    .bind(now)
    .bind(limit as i64)
    .fetch_all(&conn)
    .await
    .map_err(db)?;
    let mut candidates = Vec::new();
    for row in &rows {
        candidates.push(DueDelivery {
            id: row.try_get(0).map_err(db)?,
            provider_id: row.try_get(1).map_err(db)?,
            attempts: row.try_get(2).map_err(db)?,
            payload: row.try_get(3).map_err(db)?,
        });
    }

    let mut claimed = Vec::new();
    for candidate in candidates {
        let changed = sqlx::query(
            "UPDATE notification_deliveries SET status = 'sending', updated_at = ?1
             WHERE id = ?2 AND status = 'pending'",
        )
        .bind(now)
        .bind(candidate.id.clone())
        .execute(&conn)
        .await
        .map_err(db)?
        .rows_affected();
        if changed == 1 {
            claimed.push(candidate);
        }
    }
    Ok(claimed)
}

pub async fn mark_delivered(state: &SharedState, id: &str, attempts: i64) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE notification_deliveries SET status = 'delivered', attempts = ?1, last_error = NULL, updated_at = ?2 WHERE id = ?3",
    )
    .bind(attempts)
    .bind(now_ms())
    .bind(id)
    .execute(&conn)
    .await
    .map_err(db)?;
    Ok(())
}

pub async fn mark_retry(
    state: &SharedState,
    id: &str,
    attempts: i64,
    next_attempt_at: i64,
    error: &str,
) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE notification_deliveries SET status = 'pending', attempts = ?1, next_attempt_at = ?2, last_error = ?3, updated_at = ?4 WHERE id = ?5",
    )
    .bind(attempts)
    .bind(next_attempt_at)
    .bind(error)
    .bind(now_ms())
    .bind(id)
    .execute(&conn)
    .await
    .map_err(db)?;
    Ok(())
}

pub async fn mark_dead(
    state: &SharedState,
    id: &str,
    attempts: i64,
    error: &str,
) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE notification_deliveries SET status = 'dead', attempts = ?1, last_error = ?2, updated_at = ?3 WHERE id = ?4",
    )
    .bind(attempts)
    .bind(error)
    .bind(now_ms())
    .bind(id)
    .execute(&conn)
    .await
    .map_err(db)?;
    Ok(())
}

/// Deliveries whose provider disappeared mid-flight, or that were `sending`
/// when the process died, are put back on the queue at boot — the same
/// reasoning as `backup::run::fail_interrupted`.
pub async fn requeue_interrupted(state: &SharedState) -> Result<u64, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE notification_deliveries SET status = 'pending', next_attempt_at = ?1, updated_at = ?1 WHERE status = 'sending'",
    )
    .bind(now_ms())
    .execute(&conn)
    .await
    .map_err(db)
    .map(|result| result.rows_affected())
}

pub async fn list_deliveries(
    state: &SharedState,
    provider_id: &str,
    limit: usize,
) -> Result<Vec<DeliveryDto>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(&format!(
        "SELECT {DELIVERY_COLUMNS} FROM notification_deliveries WHERE provider_id = ?1 ORDER BY created_at DESC LIMIT ?2"
    ))
    .bind(provider_id)
    .bind(limit as i64)
    .fetch_all(&conn)
    .await
    .map_err(db)?;
    rows.iter().map(read_delivery).collect()
}

pub async fn latest_delivery(
    state: &SharedState,
    provider_id: &str,
) -> Result<Option<DeliveryDto>, AppError> {
    Ok(list_deliveries(state, provider_id, 1)
        .await?
        .into_iter()
        .next())
}

/// Keeps the table bounded: age cap for everything, plus a per-provider row
/// cap using the same `LIMIT -1 OFFSET n` idiom as the deployment history.
pub async fn prune_deliveries(state: &SharedState) -> Result<(), AppError> {
    const MAX_AGE_MS: i64 = 7 * 24 * 60 * 60 * 1000;
    const MAX_PER_PROVIDER: i64 = 200;
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "DELETE FROM notification_deliveries WHERE created_at < ?1 AND status IN ('delivered', 'dead', 'failed')",
    )
    .bind(now_ms() - MAX_AGE_MS)
    .execute(&conn)
    .await
    .map_err(db)?;
    sqlx::query(
        "DELETE FROM notification_deliveries WHERE id IN (
             SELECT id FROM (
                 SELECT id, ROW_NUMBER() OVER (
                     PARTITION BY provider_id ORDER BY created_at DESC
                 ) AS position
                 FROM notification_deliveries
             ) WHERE position > ?1
         )",
    )
    .bind(MAX_PER_PROVIDER)
    .execute(&conn)
    .await
    .map_err(db)?;
    Ok(())
}
