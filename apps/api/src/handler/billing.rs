use axum::{
    Extension, Json,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::{
    billing::{entitlement, polar, repository},
    dtos::{
        billing::{
            BillingEstimateDto, BillingOverviewResponse, BillingPlanDto, BillingUsage,
            CheckoutRequest, CheckoutResponse, InvoicePdfResponse, PortalResponse,
            UpdateServerQuantityRequest,
        },
        response::{AppError, created, ok},
    },
    security::middleware::auth::AuthUser,
    state::state::SharedState,
};

fn require_cloud_customer(state: &SharedState, user: &AuthUser) -> Result<(), AppError> {
    if !state.is_cloud {
        return Err(AppError::ResourceNotFound);
    }
    if user.is_sysadmin {
        return Err(AppError::InsufficientPermissions);
    }
    if state.polar.is_none() {
        return Err(AppError::ValidationError(
            "Polar Billing is not configured".into(),
        ));
    }
    Ok(())
}

fn require_cloud_sysadmin(state: &SharedState, user: &AuthUser) -> Result<(), AppError> {
    if !state.is_cloud || state.polar.is_none() {
        return Err(AppError::ResourceNotFound);
    }
    if !user.is_sysadmin {
        return Err(AppError::InsufficientPermissions);
    }
    Ok(())
}

/// Where Polar sends the browser back to: the UI the request came from when
/// it is an allowed origin, otherwise the server's public URL.
async fn return_base(state: &SharedState, headers: &HeaderMap) -> String {
    match crate::security::middleware::csrf::trusted_request_origin(headers) {
        Some(origin) => origin,
        None => crate::handler::settings::public_base_url(state).await,
    }
}

fn has_current_subscription(status: &str) -> bool {
    matches!(status, "active" | "trialing" | "completed" | "paid")
}

/// Multiplies the single slot amount by quantity as raw minor-unit
/// strings. Amounts are minor units (e.g. "500" = $5.00); parsing
/// failures fall back to 0 rather than failing the whole overview response.
fn estimated_total(server_amount: &str, quantity: i64) -> String {
    (server_amount.parse::<i64>().unwrap_or(0) * quantity).to_string()
}

#[derive(Serialize)]
struct AdminBillingAccount {
    user_id: String,
    name: String,
    email: String,
    status: Option<String>,
    polar_customer_id: Option<String>,
    polar_subscription_id: Option<String>,
    polar_product_id: Option<String>,
    period_starts_at: Option<i64>,
    period_ends_at: Option<i64>,
    grace_until: Option<i64>,
}

#[derive(Deserialize)]
pub struct AssignExternalPaymentRequest {
    user_id: String,
}

pub async fn admin_accounts_handler(
    State(state): State<SharedState>,
    Extension(user): Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    require_cloud_sysadmin(&state, &user)?;
    use sqlx::Row;
    let conn = crate::git::repositories::db::get_conn(&state).await;
    let rows = sqlx::query("SELECT u.id, u.name, u.email, ba.status, ba.polar_customer_id, ba.polar_subscription_id, ba.polar_product_id, ba.period_starts_at, ba.period_ends_at, ba.grace_until FROM users u LEFT JOIN billing_accounts ba ON ba.user_id = u.id ORDER BY u.email")
        .fetch_all(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let mut accounts = Vec::new();
    for row in &rows {
        accounts.push(AdminBillingAccount {
            user_id: row
                .try_get(0)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            name: row
                .try_get(1)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            email: row
                .try_get(2)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            status: row
                .try_get(3)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            polar_customer_id: row
                .try_get(4)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            polar_subscription_id: row
                .try_get(5)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            polar_product_id: row
                .try_get(6)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            period_starts_at: row
                .try_get(7)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            period_ends_at: row
                .try_get(8)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            grace_until: row
                .try_get(9)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        });
    }
    Ok(ok(accounts))
}

pub async fn admin_external_records_handler(
    State(state): State<SharedState>,
    Extension(user): Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    require_cloud_sysadmin(&state, &user)?;
    Ok(ok(repository::list_external_records(&state).await?))
}

pub async fn admin_reconcile_handler(
    State(state): State<SharedState>,
    Extension(user): Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    require_cloud_sysadmin(&state, &user)?;
    let summary = crate::billing::reconciliation::reconcile(&state).await?;
    Ok(ok(summary))
}

/// Parses `items_json` captured on the external record (from a webhook or a
/// reconciliation subscription fetch) into the pieces `upsert_account` needs.
fn record_items(
    record: &repository::BillingExternalRecord,
) -> Result<(String, i64, String), AppError> {
    let items_json = record.items_json.as_deref()
        .ok_or_else(|| AppError::ValidationError("This external payment has no captured subscription items yet. Run reconciliation again to refresh it.".into()))?;
    let items: Vec<Value> = serde_json::from_str(items_json).map_err(|_| {
        AppError::ValidationError("This external payment's captured items are unreadable.".into())
    })?;
    let find = |role: &str| {
        items
            .iter()
            .find(|item| item.get("role").and_then(Value::as_str) == Some(role))
    };
    let per_server = find("per_server").ok_or_else(|| {
        AppError::ValidationError("This external payment has no per-server price item.".into())
    })?;
    let per_server_price_id = per_server
        .get("price_id")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::InternalError("per_server item omitted price_id".into()))?
        .to_string();
    let quantity = per_server
        .get("quantity")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let product_id = per_server
        .get("product_id")
        .and_then(Value::as_str)
        .or(record.polar_product_id.as_deref())
        .ok_or_else(|| {
            AppError::ValidationError("This external payment has no product ID.".into())
        })?
        .to_string();
    Ok((per_server_price_id, quantity, product_id))
}

pub async fn admin_assign_external_record_handler(
    State(state): State<SharedState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<String>,
    Json(request): Json<AssignExternalPaymentRequest>,
) -> Result<impl IntoResponse, AppError> {
    require_cloud_sysadmin(&state, &user)?;
    if !repository::user_exists(&state, &request.user_id).await? {
        return Err(AppError::ValidationError(
            "The selected Litepod user does not exist".into(),
        ));
    }
    let record = repository::list_external_records(&state)
        .await?
        .into_iter()
        .find(|record| record.id == id)
        .ok_or(AppError::ResourceNotFound)?;
    let (server_price_id, quantity, product_id) = record_items(&record)?;
    let grace_until = matches!(
        record.subscription_status.as_str(),
        "past_due" | "paused" | "canceled" | "cancelled"
    )
    .then(|| entitlement::grace_deadline(chrono::Utc::now().timestamp_millis()));
    repository::upsert_account(
        &state,
        &request.user_id,
        Some(&record.polar_customer_id),
        record.polar_subscription_id.as_deref(),
        &product_id,
        &server_price_id,
        &server_price_id,
        &record.subscription_status,
        None,
        None,
        grace_until,
        quantity,
    )
    .await?;
    repository::assign_external_record(&state, &id, &request.user_id, &user.user_id).await?;
    Ok(ok(()))
}

pub async fn overview_handler(
    State(state): State<SharedState>,
    Extension(user): Extension<AuthUser>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    require_cloud_customer(&state, &user)?;
    let config = state.polar.as_ref().expect("checked above");
    let prices = polar::active_plan_prices(&state).await?;
    let subscription = entitlement::status_for_payer(&state, &user.user_id).await?;
    let (remote_servers, organizations, users) =
        repository::usage_for_payer(&state, &user.user_id).await?;
    let interval = subscription
        .server_price_id
        .as_ref()
        .and_then(|id| prices.iter().find(|price| &price.id == id))
        .map(|price| price.interval.clone())
        .unwrap_or_else(|| "month".into());
    let purchased_server_quantity = subscription.purchased_server_quantity;
    // The estimate reflects what's actually billed — the purchased quantity,
    // not the live server count, which can be lower than what's paid for.
    let estimate = polar::server_price(&prices, &interval).map(|server_price| BillingEstimateDto {
        interval: interval.clone(),
        server_amount: server_price.amount.clone(),
        server_quantity: purchased_server_quantity,
        currency_code: server_price.currency_code.clone(),
        estimated_total: estimated_total(&server_price.amount, purchased_server_quantity),
    });
    let plan = BillingPlanDto {
        product_id: subscription.product_id.clone().unwrap_or_default(),
        name: "Litepod Cloud".into(),
        description: "One server slot per unit.".into(),
        prices,
    };
    Ok(ok(BillingOverviewResponse {
        polar_environment: config.environment.as_str().into(),
        plan,
        subscription,
        usage: BillingUsage {
            remote_servers,
            organizations,
            users,
            purchased_server_quantity,
        },
        estimate,
    }))
}

pub async fn checkout_handler(
    State(state): State<SharedState>,
    Extension(user): Extension<AuthUser>,
    headers: HeaderMap,
    Json(request): Json<CheckoutRequest>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    require_cloud_customer(&state, &user)?;
    if repository::get_account(&state, &user.user_id)
        .await?
        .is_some_and(|account| has_current_subscription(&account.status))
    {
        return Err(AppError::ValidationError(
            "You already have an active subscription. Manage it from the customer portal.".into(),
        ));
    }
    if !matches!(request.interval.as_str(), "month" | "year") {
        return Err(AppError::ValidationError(
            "interval must be month or year".into(),
        ));
    }
    let prices = polar::active_plan_prices(&state).await?;
    let server_price = polar::server_price(&prices, &request.interval).ok_or_else(|| {
        AppError::ValidationError("That billing interval is not available".into())
    })?;
    let (server_quantity, _, _) = repository::usage_for_payer(&state, &user.user_id).await?;
    // Polar's unit-based price needs at least 1 unit, so a brand-new
    // account with no servers yet must still be quoted at least 1.
    let checkout_url = polar::create_checkout(
        &state,
        &user.user_id,
        &user.email,
        &user.name,
        &server_price.id,
        server_quantity.max(1),
        &return_base(&state, &headers).await,
    )
    .await?;
    Ok(created(CheckoutResponse { checkout_url }))
}

#[derive(Debug, PartialEq, Eq)]
enum QuantityChange {
    Increase,
    Decrease,
}

/// Pure so it's cheaply unit-testable: the handler fetches `current_purchased`
/// and `live_servers` itself, this only decides whether the requested total
/// is allowed and which proration direction it implies.
fn validate_quantity_change(
    requested: i64,
    current_purchased: i64,
    live_servers: i64,
) -> Result<QuantityChange, AppError> {
    if requested < 1 {
        return Err(AppError::ValidationError(
            "You must keep at least 1 server slot".into(),
        ));
    }
    if requested == current_purchased {
        return Err(AppError::ValidationError(
            "That's already your purchased quantity".into(),
        ));
    }
    if requested > current_purchased {
        return Ok(QuantityChange::Increase);
    }
    if requested < live_servers {
        return Err(AppError::ValidationError(format!(
            "You have {live_servers} active server(s). Remove one before dropping below {live_servers}."
        )));
    }
    Ok(QuantityChange::Decrease)
}

pub async fn update_server_quantity_handler(
    State(state): State<SharedState>,
    Extension(user): Extension<AuthUser>,
    Json(request): Json<UpdateServerQuantityRequest>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    require_cloud_customer(&state, &user)?;
    entitlement::require_active_subscription(&state, &user.user_id).await?;
    let account = repository::get_account(&state, &user.user_id)
        .await?
        .ok_or_else(|| AppError::ValidationError("No subscription found".into()))?;
    let (remote_servers, _, _) = repository::usage_for_payer(&state, &user.user_id).await?;
    let change = validate_quantity_change(
        request.quantity,
        account.purchased_server_quantity,
        remote_servers,
    )?;
    if account.cancel_at_period_end {
        return Err(AppError::ValidationError(
            "This subscription is canceled. Reactivate it from the customer portal before changing server slots.".into(),
        ));
    }
    let Some(subscription_id) = account.polar_subscription_id.clone() else {
        return Err(AppError::ValidationError(
            "Your subscription is not fully set up yet".into(),
        ));
    };
    // Increasing invoices the prorated difference right away; decreasing
    // only credits the next invoice — no immediate refund for slots you stop
    // using mid-period.
    let proration_mode = match change {
        QuantityChange::Increase => "invoice",
        QuantityChange::Decrease => "prorate",
    };
    let attempt_id = Uuid::now_v7().to_string();
    match polar::update_subscription_units(
        &state,
        &subscription_id,
        request.quantity,
        proration_mode,
    )
    .await
    {
        Ok(_) => {
            let now = Utc::now().timestamp_millis();
            repository::set_purchased_server_quantity(&state, &user.user_id, request.quantity, now)
                .await?;
            repository::record_quantity_sync_attempt(
                &state,
                &attempt_id,
                &user.user_id,
                &subscription_id,
                account.purchased_server_quantity,
                request.quantity,
                proration_mode,
                "success",
                None,
            )
            .await?;
            Ok(ok(()))
        }
        Err(error) => {
            repository::record_quantity_sync_attempt(
                &state,
                &attempt_id,
                &user.user_id,
                &subscription_id,
                account.purchased_server_quantity,
                request.quantity,
                proration_mode,
                "failed",
                Some(&error.to_string()),
            )
            .await?;
            Err(error)
        }
    }
}

pub async fn invoices_handler(
    State(state): State<SharedState>,
    Extension(user): Extension<AuthUser>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    require_cloud_customer(&state, &user)?;
    Ok(ok(repository::list_invoices(&state, &user.user_id).await?))
}

pub async fn invoice_pdf_handler(
    State(state): State<SharedState>,
    Extension(user): Extension<AuthUser>,
    Path(order_id): Path<String>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    require_cloud_customer(&state, &user)?;
    if !repository::invoice_belongs_to_user(&state, &order_id, &user.user_id).await? {
        return Err(AppError::ResourceNotFound);
    }
    Ok(ok(InvoicePdfResponse {
        url: polar::invoice_pdf_url(&state, &order_id).await?,
    }))
}

pub async fn portal_handler(
    State(state): State<SharedState>,
    Extension(user): Extension<AuthUser>,
    headers: HeaderMap,
) -> Result<impl axum::response::IntoResponse, AppError> {
    require_cloud_customer(&state, &user)?;
    let account = repository::get_account(&state, &user.user_id)
        .await?
        .ok_or_else(|| {
            AppError::ValidationError(
                "No Polar customer is associated with this account yet".into(),
            )
        })?;
    let customer_id = account.polar_customer_id.ok_or_else(|| {
        AppError::ValidationError("No Polar customer is associated with this account yet".into())
    })?;
    Ok(ok(PortalResponse {
        url: polar::customer_portal_url(&state, &customer_id, &return_base(&state, &headers).await)
            .await?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimated_total_multiplies_slot_price_by_quantity() {
        assert_eq!(estimated_total("300", 3), "900");
        assert_eq!(estimated_total("300", 0), "0");
    }

    #[test]
    fn estimated_total_falls_back_to_zero_on_unparseable_amounts() {
        assert_eq!(estimated_total("nope", 2), "0");
    }

    #[test]
    fn validate_quantity_change_rejects_below_one() {
        assert!(matches!(
            validate_quantity_change(0, 2, 0),
            Err(AppError::ValidationError(_))
        ));
    }

    #[test]
    fn validate_quantity_change_rejects_noop() {
        assert!(matches!(
            validate_quantity_change(2, 2, 1),
            Err(AppError::ValidationError(_))
        ));
    }

    #[test]
    fn validate_quantity_change_allows_increase() {
        assert_eq!(
            validate_quantity_change(3, 2, 1).unwrap(),
            QuantityChange::Increase
        );
    }

    #[test]
    fn validate_quantity_change_rejects_decrease_below_live_servers() {
        assert!(matches!(
            validate_quantity_change(1, 3, 2),
            Err(AppError::ValidationError(_))
        ));
    }

    #[test]
    fn validate_quantity_change_allows_decrease_down_to_live_servers() {
        assert_eq!(
            validate_quantity_change(2, 3, 2).unwrap(),
            QuantityChange::Decrease
        );
    }
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Result<&'a str, AppError> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .ok_or(AppError::WebhookVerificationFailed)
}

async fn process_webhook(
    state: &SharedState,
    headers: &HeaderMap,
    body: &Bytes,
) -> Result<(), AppError> {
    if !state.is_cloud {
        return Err(AppError::ResourceNotFound);
    }
    let config = state.polar.as_ref().ok_or(AppError::ResourceNotFound)?;
    let event_id = header(headers, "webhook-id")?;
    crate::billing::webhook::verify_signature(
        &config.webhook_secret,
        event_id,
        header(headers, "webhook-timestamp")?,
        header(headers, "webhook-signature")?,
        body,
    )?;
    let raw = std::str::from_utf8(body)
        .map_err(|_| AppError::ValidationError("Polar webhook body is not UTF-8".into()))?;
    let payload = serde_json::from_str(raw)
        .map_err(|_| AppError::ValidationError("Polar webhook body is not valid JSON".into()))?;
    crate::billing::webhook::process(state, event_id, payload, raw).await
}

/// Polar retries non-2xx deliveries (up to 10 times) and disables the
/// endpoint after 10 consecutive failures, so only a bad signature (403) or
/// a processing failure worth retrying (500) is reported as an error.
pub async fn webhook_handler(
    State(state): State<SharedState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let event_id = headers
        .get("webhook-id")
        .and_then(|value| value.to_str().ok());
    let event_type = serde_json::from_slice::<Value>(&body)
        .ok()
        .and_then(|value| {
            value
                .get("type")
                .and_then(Value::as_str)
                .map(str::to_string)
        });
    let delivery_id = Uuid::now_v7().to_string();

    // Every outcome except "billing not enabled here" is recorded, so a wrong
    // secret or a failing projection shows up in /debug/billing, not only logs.
    let (response_status, error_message) = match process_webhook(&state, &headers, &body).await {
        Ok(()) => (StatusCode::ACCEPTED, None),
        Err(AppError::ResourceNotFound) => return StatusCode::NOT_FOUND.into_response(),
        Err(AppError::WebhookVerificationFailed) => {
            tracing::warn!(delivery_id = %delivery_id, event_id = ?event_id, event_type = ?event_type, "Polar webhook signature verification failed");
            (
                StatusCode::FORBIDDEN,
                Some("Signature verification failed: check POLAR_WEBHOOK_SECRET and POLAR_ENVIRONMENT".to_string()),
            )
        }
        Err(error) => {
            tracing::error!(delivery_id = %delivery_id, event_id = ?event_id, event_type = ?event_type, %error, "Polar webhook processing failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Some(error.to_string().chars().take(500).collect()),
            )
        }
    };

    tracing::info!(
        delivery_id = %delivery_id,
        headers = %crate::billing::audit::headers(&headers),
        event_id = ?event_id,
        event_type = ?event_type,
        "Polar webhook received"
    );

    let request_body = String::from_utf8_lossy(&body).into_owned();
    match repository::record_webhook_delivery(
        &state,
        &delivery_id,
        event_id,
        event_type.as_deref(),
        &headers,
        &request_body,
    )
    .await
    {
        Ok(()) => {
            if let Err(error) = repository::complete_webhook_delivery(
                &state,
                &delivery_id,
                i64::from(response_status.as_u16()),
                "",
                error_message.as_deref(),
            )
            .await
            {
                tracing::error!(%error, delivery_id = %delivery_id, "billing webhook audit logging failed");
            }
        }
        Err(error) => {
            tracing::error!(%error, delivery_id = %delivery_id, "billing webhook audit logging failed");
        }
    }

    response_status.into_response()
}
