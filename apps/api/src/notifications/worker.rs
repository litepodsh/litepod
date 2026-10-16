//! Background drain for `notification_deliveries`.
//!
//! Same shape as `backup::scheduler`: a tokio task started once at boot with a
//! ticking loop that never propagates errors upward. Nothing in the request
//! path waits on it.

use std::time::Duration;

use chrono::Utc;

use crate::state::state::SharedState;

use super::dispatch::{self, SendOutcome};
use super::event::EventPayload;
use super::repository;

const TICK: Duration = Duration::from_secs(5);
const BATCH: usize = 20;
/// Retention runs once a minute rather than every tick — it is a table scan.
const PRUNE_EVERY_TICKS: u64 = 12;

pub fn spawn(state: SharedState) {
    tokio::spawn(async move {
        // Anything left `sending` belongs to a process that no longer exists.
        match repository::requeue_interrupted(&state).await {
            Ok(count) if count > 0 => {
                tracing::info!(count, "requeued interrupted notification deliveries")
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(error = %error, "could not requeue notifications"),
        }
        run_loop(state).await;
    });
}

async fn run_loop(state: SharedState) {
    let mut ticker = tokio::time::interval(TICK);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut ticks: u64 = 0;
    loop {
        ticker.tick().await;
        ticks += 1;
        if let Err(error) = tick(&state).await {
            tracing::warn!(error = %error, "notification worker tick failed");
        }
        if ticks % PRUNE_EVERY_TICKS == 0 {
            if let Err(error) = repository::prune_deliveries(&state).await {
                tracing::warn!(error = %error, "notification retention failed");
            }
        }
    }
}

async fn tick(state: &SharedState) -> Result<(), String> {
    let due = repository::claim_due(state, BATCH)
        .await
        .map_err(|error| error.to_string())?;
    if due.is_empty() {
        return Ok(());
    }

    for delivery in due {
        // Deliveries are independent; one slow provider must not hold up the
        // rest of the batch beyond its own timeout.
        deliver(state, delivery).await;
    }
    Ok(())
}

async fn deliver(state: &SharedState, delivery: repository::DueDelivery) {
    let attempt = delivery.attempts + 1;

    // The provider is loaded per attempt rather than cached, so editing a
    // broken webhook fixes queued deliveries instead of only future ones.
    let provider = match load_provider(state, &delivery.provider_id).await {
        Ok(Some(provider)) => provider,
        Ok(None) => {
            // Cascade delete normally removes these; a race can leave one.
            let _ =
                repository::mark_dead(state, &delivery.id, attempt, "provider was deleted").await;
            return;
        }
        Err(error) => {
            requeue_or_kill(state, &delivery.id, attempt, &error, "<unknown>").await;
            return;
        }
    };

    let payload: EventPayload = match serde_json::from_str(&delivery.payload) {
        Ok(payload) => payload,
        Err(error) => {
            let _ = repository::mark_dead(
                state,
                &delivery.id,
                attempt,
                &format!("stored payload is unreadable: {error}"),
            )
            .await;
            return;
        }
    };

    let config = match provider.config() {
        Ok(config) => config,
        Err(error) => {
            let _ = repository::mark_dead(state, &delivery.id, attempt, &error.to_string()).await;
            return;
        }
    };

    match dispatch::send_once(provider.kind, &config, provider.decoration, &payload).await {
        SendOutcome::Delivered => {
            if let Err(error) = repository::mark_delivered(state, &delivery.id, attempt).await {
                tracing::warn!(error = %error, "could not record notification delivery");
            }
        }
        SendOutcome::RateLimited { retry_after } => {
            // Does not consume an attempt: the provider is healthy, just busy.
            tracing::info!(
                delivery_id = %delivery.id,
                provider_name = %provider.name,
                retry_in_secs = retry_after.as_secs(),
                "notification rate limited, retrying later"
            );
            let next = Utc::now().timestamp_millis() + retry_after.as_millis() as i64;
            let _ = repository::mark_retry(
                state,
                &delivery.id,
                delivery.attempts,
                next,
                "rate limited by provider",
            )
            .await;
        }
        SendOutcome::Transient(error) => {
            requeue_or_kill(state, &delivery.id, attempt, &error, &provider.name).await;
        }
        SendOutcome::Permanent(error) => {
            tracing::warn!(
                delivery_id = %delivery.id,
                provider = provider.kind.key(),
                provider_name = %provider.name,
                attempt,
                error = %error,
                "notification will not be retried — fix the provider configuration"
            );
            let _ = repository::mark_dead(state, &delivery.id, attempt, &error).await;
        }
    }
}

async fn requeue_or_kill(
    state: &SharedState,
    id: &str,
    attempt: i64,
    error: &str,
    provider_name: &str,
) {
    match dispatch::backoff(attempt) {
        Some(delay) => {
            tracing::warn!(
                delivery_id = %id,
                provider_name = %provider_name,
                attempt,
                retry_in_secs = delay.as_secs(),
                error = %error,
                "notification attempt failed, will retry"
            );
            let next = Utc::now().timestamp_millis() + delay.as_millis() as i64;
            let _ = repository::mark_retry(state, id, attempt, next, error).await;
        }
        None => {
            tracing::error!(
                delivery_id = %id,
                provider_name = %provider_name,
                attempt,
                error = %error,
                "notification gave up after the final attempt"
            );
            let _ = repository::mark_dead(state, id, attempt, error).await;
        }
    }
}

/// The provider lookup here is by id alone — the worker has already lost the
/// organization context and does not need it.
async fn load_provider(
    state: &SharedState,
    provider_id: &str,
) -> Result<Option<repository::ProviderRow>, String> {
    repository::get_provider_any_organization(state, provider_id)
        .await
        .map_err(|error| error.to_string())
}
