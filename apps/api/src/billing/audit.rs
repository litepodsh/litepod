//! Minimal delivery diagnostics, separate from the billing event processing ledger.
use axum::http::HeaderMap;
use serde_json::{Map, Value};

use crate::{dtos::response::AppError, state::state::SharedState};

pub fn headers(headers: &HeaderMap) -> String {
    let mut safe = Map::new();
    // Header names are an allowlist: authorization, cookies, signatures, and
    // arbitrary custom headers must never become retained diagnostic data.
    for name in ["content-type", "content-length"] {
        if let Some(value) = headers.get(name).and_then(|value| value.to_str().ok()) {
            safe.insert(
                name.into(),
                Value::String(value.chars().take(128).collect()),
            );
        }
    }
    Value::Object(safe).to_string()
}

pub fn body(raw: &str) -> String {
    let payload: Value = serde_json::from_str(raw).unwrap_or(Value::Null);
    let mut safe = Map::new();
    for (key, pointer) in [
        ("event_type", "/type"),
        ("customer_id", "/data/customer_id"),
        ("subscription_id", "/data/subscription_id"),
    ] {
        if let Some(value) = payload.pointer(pointer).and_then(Value::as_str) {
            safe.insert(key.into(), Value::String(value.chars().take(128).collect()));
        }
    }
    Value::Object(safe).to_string()
}

pub async fn prune(state: &SharedState) -> Result<u64, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    Ok(sqlx::query(
        "DELETE FROM billing_webhook_deliveries WHERE received_at < (unixepoch() - 30 * 86400) * 1000",
    ).execute(&conn).await?.rows_affected())
}

pub fn spawn(state: SharedState) {
    tokio::spawn(async move {
        let mut timer = tokio::time::interval(std::time::Duration::from_secs(3600));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            timer.tick().await;
            if let Err(error) = prune(&state).await {
                tracing::error!(%error, "billing webhook delivery retention failed");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_allowlist_omits_signatures_pii_and_unknown_fields() {
        let mut input = HeaderMap::new();
        input.insert("webhook-signature", "v1,secret".parse().unwrap());
        input.insert("authorization", "Bearer secret".parse().unwrap());
        input.insert("x-customer-email", "private@example.com".parse().unwrap());
        input.insert("content-type", "application/json".parse().unwrap());
        assert_eq!(headers(&input), r#"{"content-type":"application/json"}"#);
        let safe: Value = serde_json::from_str(&body(r#"{"type":"order.paid","data":{"customer_id":"ctm_1","subscription_id":"sub_1","email":"private@example.com","address":{"city":"private"}},"unknown":"secret"}"#)).unwrap();
        assert_eq!(
            safe,
            serde_json::json!({"event_type":"order.paid","customer_id":"ctm_1","subscription_id":"sub_1"})
        );
        assert_eq!(body("not json; secret"), "{}");
    }

    #[tokio::test]
    async fn repository_redacts_before_persisting() {
        let (state, _path) = crate::testing::test_state().await;
        let mut input = HeaderMap::new();
        input.insert("webhook-signature", "v1,secret".parse().unwrap());
        input.insert("cookie", "private".parse().unwrap());
        super::super::repository::record_webhook_delivery(
            &state,
            "safe",
            Some("evt_1"),
            Some("subscription.updated"),
            &input,
            r#"{"type":"subscription.updated","data":{"email":"private@example.com"}}"#,
        )
        .await
        .unwrap();
        let row: (String, String) = sqlx::query_as("SELECT request_headers_json, request_body FROM billing_webhook_deliveries WHERE id = 'safe'")
            .fetch_one(&state.db).await.unwrap();
        assert_eq!(row.0, "{}");
        assert_eq!(row.1, r#"{"event_type":"subscription.updated"}"#);
    }

    #[tokio::test]
    async fn retention_deletes_only_expired_deliveries() {
        let (state, _path) = crate::testing::test_state().await;
        sqlx::query("INSERT INTO billing_webhook_deliveries (id, request_headers_json, request_body, received_at) VALUES ('old', '{}', '{}', (unixepoch() - 31 * 86400) * 1000), ('recent', '{}', '{}', unixepoch() * 1000)")
            .execute(&state.db).await.unwrap();
        assert_eq!(prune(&state).await.unwrap(), 1);
        let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM billing_webhook_deliveries")
            .fetch_all(&state.db)
            .await
            .unwrap();
        assert_eq!(ids, ["recent"]);
    }
}
