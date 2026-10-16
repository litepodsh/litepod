use sqlx::Row;

use crate::{dtos::response::AppError, state::state::SharedState};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BillingEventOutcome {
    Processed,
    PendingMatch,
    Failed,
    Received,
}

impl BillingEventOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Processed => "processed",
            Self::PendingMatch => "pending_match",
            Self::Failed => "failed",
            Self::Received => "received",
        }
    }
    pub fn from_db(value: &str) -> Self {
        match value {
            "processed" => Self::Processed,
            "pending_match" | "unmatched" => Self::PendingMatch,
            "failed" => Self::Failed,
            _ => Self::Received,
        }
    }
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct BillingExternalRecord {
    pub id: String,
    pub polar_subscription_id: Option<String>,
    pub polar_customer_id: String,
    pub customer_email: Option<String>,
    pub polar_product_id: Option<String>,
    pub polar_price_id: Option<String>,
    pub subscription_status: String,
    pub state: String,
    pub candidate_user_id: Option<String>,
    pub assigned_user_id: Option<String>,
    pub discovered_at: i64,
    pub items_json: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[allow(dead_code)] // produced by the next reconciliation pass; today the struct is declared but not constructed.
pub struct ReconciliationRunSummary {
    pub id: String,
    pub processed_count: i64,
    pub pending_count: i64,
    pub failed_count: i64,
    pub candidate_count: i64,
    pub unassigned_count: i64,
}

#[derive(Clone, Debug)]
#[allow(dead_code)] // some fields are populated by future reconciliation writes that aren't wired yet.
pub struct BillingAccountRow {
    pub user_id: String,
    pub polar_customer_id: Option<String>,
    pub polar_subscription_id: Option<String>,
    pub polar_product_id: Option<String>,
    pub polar_price_id: Option<String>,
    pub status: String,
    pub period_starts_at: Option<i64>,
    pub period_ends_at: Option<i64>,
    pub grace_until: Option<i64>,
    pub entitlement_json: String,
    pub base_price_id: Option<String>,
    pub per_server_price_id: Option<String>,
    pub purchased_server_quantity: i64,
    pub quantity_synced_at: Option<i64>,
    pub quantity_sync_error: Option<String>,
    pub cancel_at_period_end: bool,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct BillingInvoiceRow {
    pub order_id: String,
    pub invoice_number: Option<String>,
    pub status: String,
    pub amount: String,
    pub currency_code: String,
    pub issued_at: Option<i64>,
    pub document_url: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct WebhookDeliveryRow {
    pub id: String,
    pub polar_event_id: Option<String>,
    pub event_type: Option<String>,
    pub request_headers_json: String,
    pub request_body: String,
    pub response_status: Option<i64>,
    pub response_body: Option<String>,
    pub error_message: Option<String>,
    pub received_at: i64,
    pub completed_at: Option<i64>,
}

pub async fn record_webhook_delivery(
    state: &SharedState,
    id: &str,
    polar_event_id: Option<&str>,
    event_type: Option<&str>,
    request_headers: &axum::http::HeaderMap,
    request_body: &str,
) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "INSERT INTO billing_webhook_deliveries (id, polar_event_id, event_type, request_headers_json, request_body) VALUES (?1, ?2, ?3, ?4, ?5)",
    )
    .bind(id)
    .bind(polar_event_id)
    .bind(event_type)
    .bind(super::audit::headers(request_headers))
    .bind(super::audit::body(request_body))
    .execute(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    Ok(())
}

pub async fn complete_webhook_delivery(
    state: &SharedState,
    id: &str,
    response_status: i64,
    response_body: &str,
    error_message: Option<&str>,
) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE billing_webhook_deliveries SET response_status = ?1, response_body = ?2, error_message = ?3, completed_at = unixepoch() * 1000 WHERE id = ?4",
    )
    .bind(response_status)
    .bind(response_body)
    .bind(error_message)
    .bind(id)
    .execute(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    Ok(())
}

pub async fn recent_webhook_deliveries(
    state: &SharedState,
    limit: i64,
) -> Result<Vec<WebhookDeliveryRow>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(
        "SELECT id, polar_event_id, event_type, request_headers_json, request_body, response_status, response_body, error_message, received_at, completed_at FROM billing_webhook_deliveries ORDER BY received_at DESC, id DESC LIMIT ?1",
    )
    .bind(limit)
    .fetch_all(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    let mut deliveries = Vec::new();
    for row in &rows {
        deliveries.push(WebhookDeliveryRow {
            id: row
                .try_get(0)
                .map_err(|error| AppError::DatabaseError(error.to_string()))?,
            polar_event_id: row
                .try_get(1)
                .map_err(|error| AppError::DatabaseError(error.to_string()))?,
            event_type: row
                .try_get(2)
                .map_err(|error| AppError::DatabaseError(error.to_string()))?,
            request_headers_json: row
                .try_get(3)
                .map_err(|error| AppError::DatabaseError(error.to_string()))?,
            request_body: row
                .try_get(4)
                .map_err(|error| AppError::DatabaseError(error.to_string()))?,
            response_status: row
                .try_get(5)
                .map_err(|error| AppError::DatabaseError(error.to_string()))?,
            response_body: row
                .try_get(6)
                .map_err(|error| AppError::DatabaseError(error.to_string()))?,
            error_message: row
                .try_get(7)
                .map_err(|error| AppError::DatabaseError(error.to_string()))?,
            received_at: row
                .try_get(8)
                .map_err(|error| AppError::DatabaseError(error.to_string()))?,
            completed_at: row
                .try_get(9)
                .map_err(|error| AppError::DatabaseError(error.to_string()))?,
        });
    }
    Ok(deliveries)
}

pub async fn get_account(
    state: &SharedState,
    user_id: &str,
) -> Result<Option<BillingAccountRow>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let Some(row) = sqlx::query(
        "SELECT user_id, polar_customer_id, polar_subscription_id, polar_product_id, polar_price_id, status, period_starts_at, period_ends_at, grace_until, entitlement_json, base_price_id, per_server_price_id, purchased_server_quantity, quantity_synced_at, quantity_sync_error, cancel_at_period_end FROM billing_accounts WHERE user_id = ?1",
    )
    .bind(user_id)
    .fetch_optional(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?
    else {
        return Ok(None);
    };
    Ok(Some(BillingAccountRow {
        user_id: row
            .try_get(0)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        polar_customer_id: row
            .try_get(1)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        polar_subscription_id: row
            .try_get(2)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        polar_product_id: row
            .try_get(3)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        polar_price_id: row
            .try_get(4)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        status: row
            .try_get(5)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        period_starts_at: row
            .try_get(6)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        period_ends_at: row
            .try_get(7)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        grace_until: row
            .try_get(8)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        entitlement_json: row
            .try_get(9)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        base_price_id: row
            .try_get(10)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        per_server_price_id: row
            .try_get(11)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        purchased_server_quantity: row
            .try_get(12)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        quantity_synced_at: row
            .try_get(13)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        quantity_sync_error: row
            .try_get(14)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        cancel_at_period_end: row
            .try_get(15)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
    }))
}

pub async fn normalize_legacy_transaction_account_status(
    state: &SharedState,
    user_id: &str,
) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE billing_accounts SET status = 'active', updated_at = unixepoch() * 1000 WHERE user_id = ?1 AND status IN ('completed', 'paid')",
    )
    .bind(user_id)
    .execute(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    Ok(())
}

pub async fn create_checkout_binding(
    state: &SharedState,
    id: &str,
    user_id: &str,
    price_id: &str,
    checkout_id: &str,
) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("INSERT INTO billing_checkout_bindings (id, user_id, polar_price_id, polar_checkout_id, expires_at) VALUES (?1, ?2, ?3, ?4, unixepoch() * 1000 + 3600000)")
        .bind(id)
        .bind(user_id)
        .bind(price_id)
        .bind(checkout_id)
        .execute(&conn)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    Ok(())
}

pub async fn checkout_user(
    state: &SharedState,
    checkout_id: &str,
) -> Result<Option<String>, AppError> {
    one_string(
        state,
        "SELECT user_id FROM billing_checkout_bindings WHERE polar_checkout_id = ?1",
        checkout_id,
    )
    .await
}

pub async fn account_user(
    state: &SharedState,
    subscription_id: Option<&str>,
    customer_id: Option<&str>,
) -> Result<Option<String>, AppError> {
    if let Some(subscription_id) = subscription_id {
        if let Some(user_id) = one_string(
            state,
            "SELECT user_id FROM billing_accounts WHERE polar_subscription_id = ?1",
            subscription_id,
        )
        .await?
        {
            return Ok(Some(user_id));
        }
    }
    if let Some(customer_id) = customer_id {
        return one_string(
            state,
            "SELECT user_id FROM billing_accounts WHERE polar_customer_id = ?1",
            customer_id,
        )
        .await;
    }
    Ok(None)
}

/// `(user_id, subscription_id)` for every account already linked to a Polar
/// subscription — the pull-sync candidates for reconciliation. Accounts still
/// waiting on a first match are handled separately via `pending_events` /
/// `list_external_records`.
pub async fn list_linked_subscriptions(
    state: &SharedState,
) -> Result<Vec<(String, String)>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(
        "SELECT user_id, polar_subscription_id FROM billing_accounts WHERE polar_subscription_id IS NOT NULL",
    )
    .fetch_all(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let mut out = Vec::new();
    for row in &rows {
        out.push((
            row.try_get(0)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            row.try_get(1)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        ));
    }
    Ok(out)
}

pub async fn user_exists(state: &SharedState, user_id: &str) -> Result<bool, AppError> {
    Ok(
        one_string(state, "SELECT id FROM users WHERE id = ?1", user_id)
            .await?
            .is_some(),
    )
}

async fn one_string(
    state: &SharedState,
    sql: &str,
    value: &str,
) -> Result<Option<String>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(sql)
        .bind(value)
        .fetch_optional(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?
        .map(|row| {
            row.try_get(0)
                .map_err(|e| AppError::DatabaseError(e.to_string()))
        })
        .transpose()
}

pub async fn record_event(
    state: &SharedState,
    event_id: &str,
    event_type: &str,
    occurred_at: i64,
    payload: &str,
) -> Result<bool, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let inserted = sqlx::query("INSERT OR IGNORE INTO billing_events (polar_event_id, event_type, occurred_at, outcome, payload_json) VALUES (?1, ?2, ?3, 'received', ?4)")
        .bind(event_id)
        .bind(event_type)
        .bind(occurred_at)
        .bind(payload)
        .execute(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?
        .rows_affected();
    Ok(inserted > 0)
}

pub async fn event_outcome(
    state: &SharedState,
    event_id: &str,
) -> Result<Option<BillingEventOutcome>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("SELECT outcome FROM billing_events WHERE polar_event_id = ?1")
        .bind(event_id)
        .fetch_optional(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?
        .map(|row| {
            row.try_get::<String, _>(0)
                .map(|value| BillingEventOutcome::from_db(&value))
                .map_err(|e| AppError::DatabaseError(e.to_string()))
        })
        .transpose()
}

pub async fn mark_event_attempt(
    state: &SharedState,
    event_id: &str,
    outcome: BillingEventOutcome,
    error: Option<&str>,
) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("UPDATE billing_events SET outcome = ?1, last_error = ?2, attempt_count = attempt_count + 1, last_attempt_at = unixepoch() * 1000 WHERE polar_event_id = ?3")
        .bind(outcome.as_str())
        .bind(error)
        .bind(event_id)
        .execute(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok(())
}

pub async fn pending_events(state: &SharedState) -> Result<Vec<(String, String)>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query("SELECT polar_event_id, payload_json FROM billing_events WHERE outcome IN ('pending_match', 'unmatched') ORDER BY received_at ASC")
        .fetch_all(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let mut events = Vec::new();
    for row in &rows {
        events.push((
            row.try_get(0)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            row.try_get(1)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        ));
    }
    Ok(events)
}

#[allow(clippy::too_many_arguments)]
pub async fn upsert_external_record(
    state: &SharedState,
    subscription_id: Option<&str>,
    customer_id: &str,
    email: Option<&str>,
    product_id: Option<&str>,
    price_id: Option<&str>,
    status: &str,
    items_json: Option<&str>,
) -> Result<String, AppError> {
    let normalized = email.map(|value| value.trim().to_ascii_lowercase());
    let candidate = match normalized.as_deref() {
        Some(email) => {
            one_string(state, "SELECT id FROM users WHERE lower(email) = ?1", email).await?
        }
        None => None,
    };
    let record_id = uuid::Uuid::now_v7().to_string();
    let state_name = if candidate.is_some() {
        "candidate_match"
    } else {
        "unassigned_external"
    };
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("INSERT INTO billing_external_records (id, polar_subscription_id, polar_customer_id, customer_email, normalized_email, polar_product_id, polar_price_id, subscription_status, state, candidate_user_id, items_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) ON CONFLICT DO UPDATE SET polar_subscription_id=COALESCE(excluded.polar_subscription_id, billing_external_records.polar_subscription_id), customer_email=COALESCE(excluded.customer_email, billing_external_records.customer_email), normalized_email=COALESCE(excluded.normalized_email, billing_external_records.normalized_email), polar_product_id=COALESCE(excluded.polar_product_id, billing_external_records.polar_product_id), polar_price_id=COALESCE(excluded.polar_price_id, billing_external_records.polar_price_id), subscription_status=excluded.subscription_status, candidate_user_id=COALESCE(excluded.candidate_user_id, billing_external_records.candidate_user_id), state=CASE WHEN billing_external_records.state='assigned' THEN 'assigned' ELSE excluded.state END, items_json=COALESCE(excluded.items_json, billing_external_records.items_json), updated_at=unixepoch() * 1000")
        .bind(record_id)
        .bind(subscription_id)
        .bind(customer_id)
        .bind(email)
        .bind(normalized)
        .bind(product_id)
        .bind(price_id)
        .bind(status)
        .bind(state_name)
        .bind(candidate)
        .bind(items_json)
        .execute(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    sqlx::query("SELECT state FROM billing_external_records WHERE polar_customer_id = ?1")
        .bind(customer_id)
        .fetch_optional(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?
        .ok_or_else(|| AppError::InternalError("External payment record was not saved".into()))?
        .try_get(0)
        .map_err(|e| AppError::DatabaseError(e.to_string()))
}

pub async fn list_external_records(
    state: &SharedState,
) -> Result<Vec<BillingExternalRecord>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query("SELECT id, polar_subscription_id, polar_customer_id, customer_email, polar_product_id, polar_price_id, subscription_status, state, candidate_user_id, assigned_user_id, discovered_at, items_json FROM billing_external_records WHERE state != 'ignored' ORDER BY discovered_at DESC")
        .fetch_all(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let mut records = Vec::new();
    for row in &rows {
        records.push(BillingExternalRecord {
            id: row
                .try_get(0)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            polar_subscription_id: row
                .try_get(1)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            polar_customer_id: row
                .try_get(2)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            customer_email: row
                .try_get(3)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            polar_product_id: row
                .try_get(4)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            polar_price_id: row
                .try_get(5)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            subscription_status: row
                .try_get(6)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            state: row
                .try_get(7)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            candidate_user_id: row
                .try_get(8)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            assigned_user_id: row
                .try_get(9)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            discovered_at: row
                .try_get(10)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            items_json: row
                .try_get(11)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        });
    }
    Ok(records)
}

pub async fn assign_external_record(
    state: &SharedState,
    id: &str,
    user_id: &str,
    admin_id: &str,
) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let changed = sqlx::query("UPDATE billing_external_records SET state = 'assigned', assigned_user_id = ?1, assigned_by_user_id = ?2, assigned_at = unixepoch() * 1000, updated_at = unixepoch() * 1000 WHERE id = ?3 AND state IN ('candidate_match', 'unassigned_external')")
        .bind(user_id)
        .bind(admin_id)
        .bind(id)
        .execute(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?
        .rows_affected();
    if changed == 0 {
        return Err(AppError::ValidationError(
            "External payment is already assigned or unavailable".into(),
        ));
    }
    Ok(())
}

/// `base_price_id` is mirrored into the legacy `polar_price_id` column for
/// compatibility; both are dropped in a later cleanup migration.
/// `purchased_server_quantity` is authoritative from the caller (webhook payload
/// or Polar's own record during admin-assign) — this function does not
/// re-derive it.
pub async fn upsert_account(
    state: &SharedState,
    user_id: &str,
    customer_id: Option<&str>,
    subscription_id: Option<&str>,
    product_id: &str,
    base_price_id: &str,
    per_server_price_id: &str,
    status: &str,
    period_starts_at: Option<i64>,
    period_ends_at: Option<i64>,
    grace_until: Option<i64>,
    purchased_server_quantity: i64,
) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("INSERT INTO billing_accounts (user_id, polar_customer_id, polar_subscription_id, polar_product_id, polar_price_id, status, period_starts_at, period_ends_at, grace_until, entitlement_json, base_price_id, per_server_price_id, purchased_server_quantity, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, '{}', ?10, ?11, ?12, unixepoch() * 1000) ON CONFLICT(user_id) DO UPDATE SET polar_customer_id=COALESCE(excluded.polar_customer_id, billing_accounts.polar_customer_id), polar_subscription_id=COALESCE(excluded.polar_subscription_id, billing_accounts.polar_subscription_id), polar_product_id=excluded.polar_product_id, polar_price_id=excluded.polar_price_id, status=excluded.status, period_starts_at=COALESCE(excluded.period_starts_at, billing_accounts.period_starts_at), period_ends_at=COALESCE(excluded.period_ends_at, billing_accounts.period_ends_at), grace_until=CASE WHEN excluded.status IN ('active', 'trialing') THEN NULL ELSE COALESCE(excluded.grace_until, billing_accounts.grace_until) END, base_price_id=excluded.base_price_id, per_server_price_id=excluded.per_server_price_id, purchased_server_quantity=excluded.purchased_server_quantity, updated_at=excluded.updated_at")
        .bind(user_id)
        .bind(customer_id)
        .bind(subscription_id)
        .bind(product_id)
        .bind(base_price_id)
        .bind(status)
        .bind(period_starts_at)
        .bind(period_ends_at)
        .bind(grace_until)
        .bind(base_price_id)
        .bind(per_server_price_id)
        .bind(purchased_server_quantity)
        .execute(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok(())
}

/// Narrow update kept out of `upsert_account`, whose callers (webhook and
/// admin-assign) don't all know the cancel state.
pub async fn set_cancel_at_period_end(
    state: &SharedState,
    user_id: &str,
    cancel_at_period_end: bool,
) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("UPDATE billing_accounts SET cancel_at_period_end = ?1 WHERE user_id = ?2")
        .bind(cancel_at_period_end)
        .bind(user_id)
        .execute(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok(())
}

/// Narrow update after a successful purchase-quantity change — avoids
/// touching status/price IDs the way a full `upsert_account` call would.
pub async fn set_purchased_server_quantity(
    state: &SharedState,
    user_id: &str,
    quantity: i64,
    synced_at: i64,
) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE billing_accounts SET purchased_server_quantity = ?1, quantity_synced_at = ?2, quantity_sync_error = NULL, updated_at = unixepoch() * 1000 WHERE user_id = ?3",
    )
    .bind(quantity)
    .bind(synced_at)
    .bind(user_id)
    .execute(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn record_quantity_sync_attempt(
    state: &SharedState,
    id: &str,
    user_id: &str,
    subscription_id: &str,
    previous_quantity: i64,
    target_quantity: i64,
    proration_mode: &str,
    status: &str,
    error: Option<&str>,
) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "INSERT INTO billing_quantity_sync_attempts (id, user_id, polar_subscription_id, previous_quantity, target_quantity, proration_mode, status, error_message) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )
    .bind(id)
    .bind(user_id)
    .bind(subscription_id)
    .bind(previous_quantity)
    .bind(target_quantity)
    .bind(proration_mode)
    .bind(status)
    .bind(error)
    .execute(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    if status == "failed" {
        sqlx::query(
            "UPDATE billing_accounts SET quantity_sync_error = ?1, updated_at = unixepoch() * 1000 WHERE user_id = ?2",
        )
        .bind(error)
        .bind(user_id)
        .execute(&conn)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    }
    Ok(())
}

pub async fn recent_quantity_sync_attempts(
    state: &SharedState,
    limit: i64,
) -> Result<
    Vec<(
        String,
        String,
        i64,
        i64,
        String,
        String,
        Option<String>,
        i64,
    )>,
    AppError,
> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(
        "SELECT user_id, polar_subscription_id, previous_quantity, target_quantity, proration_mode, status, error_message, attempted_at FROM billing_quantity_sync_attempts ORDER BY attempted_at DESC LIMIT ?1",
    )
    .bind(limit)
    .fetch_all(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    let mut attempts = Vec::new();
    for row in &rows {
        attempts.push((
            row.try_get(0)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            row.try_get(1)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            row.try_get(2)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            row.try_get(3)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            row.try_get(4)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            row.try_get(5)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            row.try_get(6)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            row.try_get(7)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        ));
    }
    Ok(attempts)
}

pub async fn upsert_invoice(
    state: &SharedState,
    order_id: &str,
    user_id: &str,
    subscription_id: Option<&str>,
    invoice_number: Option<&str>,
    status: &str,
    amount: &str,
    currency_code: &str,
    issued_at: Option<i64>,
    document_url: Option<&str>,
) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("INSERT INTO billing_invoices (polar_order_id, user_id, polar_subscription_id, invoice_number, status, amount, currency_code, issued_at, document_url) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) ON CONFLICT(polar_order_id) DO UPDATE SET status=excluded.status, invoice_number=excluded.invoice_number, amount=excluded.amount, currency_code=excluded.currency_code, issued_at=excluded.issued_at, document_url=excluded.document_url, updated_at=unixepoch() * 1000")
        .bind(order_id)
        .bind(user_id)
        .bind(subscription_id)
        .bind(invoice_number)
        .bind(status)
        .bind(amount)
        .bind(currency_code)
        .bind(issued_at)
        .bind(document_url)
        .execute(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok(())
}

pub async fn payer_for_organization(
    state: &SharedState,
    organization_id: &str,
) -> Result<String, AppError> {
    one_string(
        state,
        "SELECT created_by FROM organizations WHERE id = ?1",
        organization_id,
    )
    .await?
    .ok_or(AppError::OrganizationNotFound)
}

pub async fn list_invoices(
    state: &SharedState,
    user_id: &str,
) -> Result<Vec<BillingInvoiceRow>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query("SELECT polar_order_id, invoice_number, status, amount, currency_code, issued_at, document_url FROM billing_invoices WHERE user_id = ?1 ORDER BY issued_at DESC")
        .bind(user_id)
        .fetch_all(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let mut invoices = Vec::new();
    for row in &rows {
        invoices.push(BillingInvoiceRow {
            order_id: row
                .try_get(0)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            invoice_number: row
                .try_get(1)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            status: row
                .try_get(2)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            amount: row
                .try_get(3)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            currency_code: row
                .try_get(4)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            issued_at: row
                .try_get(5)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            document_url: row
                .try_get(6)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        });
    }
    Ok(invoices)
}

pub async fn invoice_belongs_to_user(
    state: &SharedState,
    order_id: &str,
    user_id: &str,
) -> Result<bool, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    Ok(
        sqlx::query("SELECT 1 FROM billing_invoices WHERE polar_order_id = ?1 AND user_id = ?2")
            .bind(order_id)
            .bind(user_id)
            .fetch_optional(&conn)
            .await
            .map_err(|error| AppError::DatabaseError(error.to_string()))?
            .is_some(),
    )
}

pub async fn usage_for_payer(
    state: &SharedState,
    payer_id: &str,
) -> Result<(i64, i64, i64), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let organizations: i64 =
        sqlx::query("SELECT COUNT(*) FROM organizations WHERE created_by = ?1")
            .bind(payer_id)
            .fetch_optional(&conn)
            .await
            .map_err(|e| AppError::DatabaseError(e.to_string()))?
            .ok_or_else(|| AppError::InternalError("organization count missing".into()))?
            .try_get(0)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let remote_servers: i64 = sqlx::query("SELECT COUNT(*) FROM servers s JOIN organizations o ON o.id=s.organization_id WHERE o.created_by=?1 AND s.kind='ssh'")
        .bind(payer_id)
        .fetch_optional(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?
        .ok_or_else(|| AppError::InternalError("server count missing".into()))?
        .try_get(0)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let users: i64 = sqlx::query("SELECT COUNT(DISTINCT om.user_id) FROM organization_members om JOIN organizations o ON o.id=om.organization_id WHERE o.created_by=?1")
        .bind(payer_id)
        .fetch_optional(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?
        .ok_or_else(|| AppError::InternalError("user count missing".into()))?
        .try_get(0)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok((remote_servers, organizations, users))
}
