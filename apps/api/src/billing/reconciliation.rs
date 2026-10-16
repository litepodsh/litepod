//! Cloud-only recovery for verified Polar events awaiting a user match.

use serde::Serialize;
use uuid::Uuid;

use serde_json::Value;

use crate::{
    billing::{
        polar,
        repository::{self, BillingEventOutcome},
        webhook,
    },
    dtos::response::AppError,
    state::state::SharedState,
};

#[derive(Clone, Debug, Serialize)]
pub struct ReconciliationSummary {
    pub processed_count: i64,
    pub pending_count: i64,
    pub failed_count: i64,
    pub candidate_count: i64,
    pub unassigned_count: i64,
    pub synced_count: i64,
}

pub async fn reconcile(state: &SharedState) -> Result<ReconciliationSummary, AppError> {
    if !state.is_cloud || state.polar.is_none() {
        return Err(AppError::ResourceNotFound);
    }
    let run_id = Uuid::now_v7().to_string();
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("INSERT INTO billing_reconciliation_runs (id) VALUES (?1)")
        .bind(run_id.clone())
        .execute(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let mut processed = 0;
    let mut pending = 0;
    let mut failed = 0;
    let mut candidate = 0;
    let mut unassigned = 0;
    let mut synced = 0;

    // Pull sync: accounts already linked to a Polar subscription have no
    // other path back to reality if a webhook never arrives (no reachable
    // local tunnel, a dropped delivery, ...) — push-only leaves them stuck
    // on whatever status they last had. Fetch each one straight from Polar
    // and re-project it the same way a webhook would.
    for (user_id, subscription_id) in repository::list_linked_subscriptions(state).await? {
        match polar::get_subscription(state, &subscription_id).await {
            Ok(data) => {
                if let Err(error) = webhook::project_account(
                    state,
                    &user_id,
                    &data,
                    data.get("customer_id").and_then(Value::as_str),
                    Some(subscription_id.as_str()),
                )
                .await
                {
                    tracing::warn!(user_id, subscription_id, %error, "Reconciliation pull sync failed to project account");
                    failed += 1;
                } else {
                    synced += 1;
                }
            }
            Err(error) => {
                tracing::warn!(user_id, subscription_id, %error, "Reconciliation pull sync failed to fetch subscription");
                failed += 1;
            }
        }
    }

    for (event_id, raw) in repository::pending_events(state).await? {
        let outcome = match serde_json::from_str(&raw) {
            Ok(payload) => webhook::process(state, &event_id, payload, &raw).await,
            Err(error) => Err(AppError::ValidationError(error.to_string())),
        };
        match outcome {
            Ok(()) => match repository::event_outcome(state, &event_id).await? {
                Some(BillingEventOutcome::Processed) => processed += 1,
                Some(BillingEventOutcome::PendingMatch)
                | Some(BillingEventOutcome::Received)
                | None => pending += 1,
                Some(BillingEventOutcome::Failed) => failed += 1,
            },
            Err(error) => {
                repository::mark_event_attempt(
                    state,
                    &event_id,
                    BillingEventOutcome::Failed,
                    Some(&error.to_string()),
                )
                .await?;
                failed += 1;
            }
        }
    }

    for subscription in polar::subscriptions_for_reconciliation(state).await? {
        let status = subscription
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !matches!(status, "active" | "trialing" | "past_due" | "paused") {
            continue;
        }
        let Some(customer_id) = subscription.get("customer_id").and_then(Value::as_str) else {
            continue;
        };
        let subscription_id = subscription.get("id").and_then(Value::as_str);
        if repository::account_user(state, subscription_id, Some(customer_id))
            .await?
            .is_some()
        {
            continue;
        }
        let items = webhook::items(&subscription);
        let price_id = webhook::find_role(&items, "per_server").map(|item| item.price_id.as_str());
        let product_id = items.iter().find_map(|item| item.product_id.as_deref());
        let items_json =
            (!items.is_empty()).then(|| serde_json::to_string(&items).unwrap_or_default());
        let email = subscription
            .pointer("/customer/email")
            .and_then(Value::as_str);
        match repository::upsert_external_record(
            state,
            subscription_id,
            customer_id,
            email,
            product_id,
            price_id,
            status,
            items_json.as_deref(),
        )
        .await?
        .as_str()
        {
            "candidate_match" => candidate += 1,
            "unassigned_external" => unassigned += 1,
            _ => {}
        }
    }
    sqlx::query("UPDATE billing_reconciliation_runs SET completed_at = unixepoch() * 1000, processed_count = ?1, pending_count = ?2, failed_count = ?3, candidate_count = ?4, unassigned_count = ?5, synced_count = ?6 WHERE id = ?7").bind(processed).bind(pending).bind(failed).bind(candidate).bind(unassigned).bind(synced).bind(run_id).execute(&conn).await.map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok(ReconciliationSummary {
        processed_count: processed,
        pending_count: pending,
        failed_count: failed,
        candidate_count: candidate,
        unassigned_count: unassigned,
        synced_count: synced,
    })
}
