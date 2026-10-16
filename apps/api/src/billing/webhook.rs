use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use chrono::{DateTime, Utc};
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::Sha256;

use crate::{
    billing::{
        entitlement,
        repository::{self, BillingEventOutcome},
    },
    dtos::response::AppError,
    state::state::SharedState,
};

type HmacSha256 = Hmac<Sha256>;

/// Verifies a Standard Webhooks signature, as Polar sends it: HMAC-SHA256
/// over `"{webhook-id}.{webhook-timestamp}.{body}"`, header
/// `webhook-signature: v1,<base64> [v1,<base64> ...]`.
///
/// Two keys are tried, like Polar's own SDKs: the Standard Webhooks key
/// (base64 after `whsec_`, secrets created since 2026-09-08) and the legacy
/// Polar key (the raw UTF-8 bytes of the whole secret string).
pub fn verify_signature(
    secret: &str,
    webhook_id: &str,
    webhook_timestamp: &str,
    signature: &str,
    body: &[u8],
) -> Result<(), AppError> {
    let timestamp = webhook_timestamp
        .parse::<i64>()
        .map_err(|_| AppError::WebhookVerificationFailed)?;
    if (Utc::now().timestamp() - timestamp).abs() > 300 {
        return Err(AppError::WebhookVerificationFailed);
    }
    let mut signed = format!("{webhook_id}.{webhook_timestamp}.").into_bytes();
    signed.extend_from_slice(body);

    let mut keys = vec![secret.as_bytes().to_vec()];
    if let Ok(key) = BASE64.decode(secret.strip_prefix("whsec_").unwrap_or(secret)) {
        keys.insert(0, key);
    }
    let candidates: Vec<Vec<u8>> = signature
        .split_whitespace()
        .filter_map(|part| part.strip_prefix("v1,"))
        .filter_map(|value| BASE64.decode(value).ok())
        .collect();
    for key in &keys {
        for candidate in &candidates {
            let mut mac =
                HmacSha256::new_from_slice(key).map_err(|_| AppError::WebhookVerificationFailed)?;
            mac.update(&signed);
            if mac.verify_slice(candidate).is_ok() {
                return Ok(());
            }
        }
    }
    Err(AppError::WebhookVerificationFailed)
}

fn string(value: &Value, pointer: &str) -> Option<String> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn timestamp(value: &Value, pointer: &str) -> Option<i64> {
    string(value, pointer)
        .and_then(|value| DateTime::parse_from_rfc3339(&value).ok())
        .map(|value| value.timestamp_millis())
}

fn billing_period_start(data: &Value) -> Option<i64> {
    timestamp(data, "/current_period_start")
}

fn billing_period_end(data: &Value) -> Option<i64> {
    timestamp(data, "/current_period_end").or_else(|| timestamp(data, "/ends_at"))
}

/// The subscription's single unit-based line, classified by its product's
/// `litepod_price_role`. Persisted as `items_json` on external records so a
/// later manual admin-assign has the full picture. Polar checkouts take
/// product IDs, so `price_id` holds the product ID.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WebhookItem {
    pub price_id: String,
    pub role: Option<String>,
    pub quantity: i64,
    pub product_id: Option<String>,
}

/// Parses a Polar subscription or order payload (or a `GET
/// /v1/subscriptions/{id}` response, same shape). Polar inlines the product
/// with its metadata, so this needs no extra API call.
pub fn items(data: &Value) -> Vec<WebhookItem> {
    let Some(product_id) = string(data, "/product_id").or_else(|| string(data, "/product/id"))
    else {
        return Vec::new();
    };
    vec![WebhookItem {
        price_id: product_id.clone(),
        role: string(data, "/product/metadata/litepod_price_role"),
        quantity: data
            .get("units")
            .or_else(|| data.get("seats"))
            .and_then(Value::as_i64)
            .unwrap_or(1),
        product_id: Some(product_id),
    }]
}

pub fn find_role<'a>(items: &'a [WebhookItem], role: &str) -> Option<&'a WebhookItem> {
    items.iter().find(|item| item.role.as_deref() == Some(role))
}

/// Projects a subscription payload's units + status onto `user_id`'s billing
/// account. Shared by webhook push processing and reconciliation's pull sync,
/// so both use the same status and grace-period projection.
pub async fn project_account(
    state: &SharedState,
    user_id: &str,
    data: &Value,
    customer_id: Option<&str>,
    subscription_id: Option<&str>,
) -> Result<(), AppError> {
    let webhook_items = items(data);
    let Some(per_server) = find_role(&webhook_items, "per_server") else {
        return Ok(());
    };
    let product_id = per_server.product_id.clone().unwrap_or_default();
    // Polar keeps a subscription `active` until it is revoked, even after a
    // cancel-at-period-end request, so the status passes through unchanged.
    let status = string(data, "/status").unwrap_or_else(|| "inactive".into());
    let period_starts_at = billing_period_start(data);
    let period_ends_at = billing_period_end(data);
    let needs_grace = matches!(
        status.as_str(),
        "past_due" | "unpaid" | "paused" | "canceled"
    );
    let grace_until = needs_grace
        .then(|| period_ends_at.unwrap_or_else(|| Utc::now().timestamp_millis()))
        .map(entitlement::grace_deadline);
    // The remote unit count is always authoritative, whether the change came
    // from our own quantity update, the customer portal or a pull-sync fetch.
    repository::upsert_account(
        state,
        user_id,
        customer_id,
        subscription_id,
        &product_id,
        &per_server.price_id,
        &per_server.price_id,
        &status,
        period_starts_at,
        period_ends_at,
        grace_until,
        per_server.quantity,
    )
    .await?;
    repository::set_cancel_at_period_end(
        state,
        user_id,
        data.get("cancel_at_period_end")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    )
    .await
}

/// `event_id` is the `webhook-id` header: Polar's body carries no event id.
pub async fn process(
    state: &SharedState,
    event_id: &str,
    payload: Value,
    raw: &str,
) -> Result<(), AppError> {
    let event_type = string(&payload, "/type")
        .ok_or_else(|| AppError::ValidationError("Polar webhook omitted type".into()))?;
    let occurred_at =
        timestamp(&payload, "/timestamp").unwrap_or_else(|| Utc::now().timestamp_millis());
    match repository::event_outcome(state, event_id).await? {
        Some(BillingEventOutcome::Processed) => return Ok(()),
        Some(_) => {}
        None => {
            repository::record_event(state, event_id, &event_type, occurred_at, raw).await?;
        }
    }
    let result = apply(state, event_id, payload, true).await;
    if let Err(error) = &result {
        // Polar retries the delivery; until then the ledger says why it failed.
        repository::mark_event_attempt(
            state,
            event_id,
            BillingEventOutcome::Failed,
            Some(&error.to_string()),
        )
        .await?;
    }
    result
}

async fn apply(
    state: &SharedState,
    event_id: &str,
    payload: Value,
    replay_pending: bool,
) -> Result<(), AppError> {
    let event_type = string(&payload, "/type")
        .ok_or_else(|| AppError::ValidationError("Polar webhook omitted type".into()))?;
    let is_subscription = event_type.starts_with("subscription.");
    let is_order = event_type.starts_with("order.");
    if !is_subscription && !is_order {
        repository::mark_event_attempt(state, event_id, BillingEventOutcome::Processed, None)
            .await?;
        return Ok(());
    }
    let data = payload.get("data").unwrap_or(&Value::Null);
    let subscription_id = if is_subscription {
        string(data, "/id")
    } else {
        string(data, "/subscription_id")
    };
    let customer_id = string(data, "/customer_id");

    // Checkout sets `external_customer_id` to the Litepod user id, so this is
    // the normal path. The binding and linked-account lookups cover customers
    // created any other way.
    let external_user = match string(data, "/customer/external_id") {
        Some(id) if repository::user_exists(state, &id).await? => Some(id),
        _ => None,
    };
    let checkout_user = match string(data, "/checkout_id") {
        Some(id) => repository::checkout_user(state, &id).await?,
        None => None,
    };
    let payer_id = match external_user.or(checkout_user) {
        Some(id) => Some(id),
        None => {
            repository::account_user(state, subscription_id.as_deref(), customer_id.as_deref())
                .await?
        }
    };

    let webhook_items = items(data);

    let Some(user_id) = payer_id else {
        repository::mark_event_attempt(state, event_id, BillingEventOutcome::PendingMatch, None)
            .await?;
        if let Some(customer_id) = customer_id.as_deref().filter(|_| is_subscription) {
            let product_id = webhook_items
                .iter()
                .find_map(|item| item.product_id.clone());
            let server_price_id =
                find_role(&webhook_items, "per_server").map(|item| item.price_id.clone());
            let items_json = (!webhook_items.is_empty())
                .then(|| serde_json::to_string(&webhook_items).unwrap_or_default());
            repository::upsert_external_record(
                state,
                subscription_id.as_deref(),
                customer_id,
                string(data, "/customer/email").as_deref(),
                product_id.as_deref(),
                server_price_id.as_deref(),
                string(data, "/status").as_deref().unwrap_or("unknown"),
                items_json.as_deref(),
            )
            .await?;
        }
        tracing::info!(event_id = %event_id, event_type = %event_type, "Polar webhook pending payer match");
        return Ok(());
    };

    if is_subscription {
        project_account(
            state,
            &user_id,
            data,
            customer_id.as_deref(),
            subscription_id.as_deref(),
        )
        .await?;
    }

    if let Some(order_id) = string(data, "/id").filter(|_| is_order) {
        let amount = data
            .get("total_amount")
            .and_then(Value::as_i64)
            .unwrap_or(0)
            .to_string();
        let currency = string(data, "/currency")
            .map(|currency| currency.to_uppercase())
            .unwrap_or_else(|| "USD".into());
        repository::upsert_invoice(
            state,
            &order_id,
            &user_id,
            subscription_id.as_deref(),
            string(data, "/invoice_number").as_deref(),
            string(data, "/status").as_deref().unwrap_or("paid"),
            &amount,
            &currency,
            timestamp(data, "/created_at"),
            None,
        )
        .await?;
    }
    repository::mark_event_attempt(state, event_id, BillingEventOutcome::Processed, None).await?;
    tracing::info!(event_id = %event_id, user_id = %user_id, "Polar webhook processed");
    if replay_pending {
        for (pending_id, pending_raw) in repository::pending_events(state).await? {
            if pending_id == event_id {
                continue;
            }
            match serde_json::from_str::<Value>(&pending_raw) {
                Ok(pending) => {
                    if let Err(error) = Box::pin(apply(state, &pending_id, pending, false)).await {
                        repository::mark_event_attempt(
                            state,
                            &pending_id,
                            BillingEventOutcome::Failed,
                            Some(&error.to_string()),
                        )
                        .await?;
                        tracing::error!(event_id = %pending_id, %error, "Polar pending webhook replay failed");
                    }
                }
                Err(error) => {
                    repository::mark_event_attempt(
                        state,
                        &pending_id,
                        BillingEventOutcome::Failed,
                        Some(&error.to_string()),
                    )
                    .await?
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sign(key: &[u8], id: &str, ts: &str, body: &[u8]) -> String {
        let mut mac = HmacSha256::new_from_slice(key).unwrap();
        mac.update(format!("{id}.{ts}.").as_bytes());
        mac.update(body);
        format!("v1,{}", BASE64.encode(mac.finalize().into_bytes()))
    }

    #[test]
    fn validates_standard_webhooks_signature() {
        let key = b"0123456789abcdef0123456789abcdef";
        let secret = format!("whsec_{}", BASE64.encode(key));
        let ts = Utc::now().timestamp().to_string();
        let body = br#"{"type":"subscription.updated"}"#;
        let header = format!("v1,bm90LXRoaXM= {}", sign(key, "msg_1", &ts, body));
        assert!(verify_signature(&secret, "msg_1", &ts, &header, body).is_ok());
        // Same signature over a different body or id must fail.
        assert!(verify_signature(&secret, "msg_1", &ts, &header, b"{}").is_err());
        assert!(verify_signature(&secret, "msg_2", &ts, &header, body).is_err());
    }

    #[test]
    fn validates_legacy_polar_secret_signature() {
        let secret = "whsec_legacy-polar-secret";
        let ts = Utc::now().timestamp().to_string();
        let body = br#"{"type":"order.paid"}"#;
        let header = sign(secret.as_bytes(), "msg_1", &ts, body);
        assert!(verify_signature(secret, "msg_1", &ts, &header, body).is_ok());
    }

    #[test]
    fn rejects_stale_timestamp() {
        let secret = "whsec_legacy-polar-secret";
        let ts = (Utc::now().timestamp() - 600).to_string();
        let header = sign(secret.as_bytes(), "msg_1", &ts, b"{}");
        assert!(verify_signature(secret, "msg_1", &ts, &header, b"{}").is_err());
    }

    #[test]
    fn extracts_polar_billing_period_boundaries() {
        let subscription = serde_json::json!({
            "current_period_start": "2026-08-08T00:00:00Z",
            "current_period_end": "2026-08-24T00:00:00Z"
        });
        assert_eq!(billing_period_start(&subscription), Some(1_786_147_200_000));
        assert_eq!(billing_period_end(&subscription), Some(1_787_529_600_000));
    }

    #[test]
    fn items_reads_units_and_product_role() {
        let data = serde_json::json!({
            "product_id": "prod_1",
            "units": 3,
            "product": {"id": "prod_1", "metadata": {"litepod_price_role": "per_server"}}
        });
        let parsed = items(&data);
        assert_eq!(parsed.len(), 1);
        let per_server = find_role(&parsed, "per_server").unwrap();
        assert_eq!(per_server.quantity, 3);
        assert_eq!(per_server.price_id, "prod_1");
    }

    #[test]
    fn items_returns_empty_without_product() {
        assert!(items(&serde_json::json!({})).is_empty());
    }
}
