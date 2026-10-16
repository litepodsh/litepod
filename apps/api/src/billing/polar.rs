use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{dtos::response::AppError, state::state::SharedState};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PriceRole {
    PerServer,
}

impl PriceRole {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "per_server" => Some(Self::PerServer),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct CatalogTrialPeriod {
    pub interval: String,
    pub frequency: u32,
    pub requires_payment_method: bool,
}

/// One active Polar unit-based product, tagged by its role in the plan.
/// Litepod Cloud has one per-server product for each billing interval; a
/// Polar checkout takes product IDs, so `id` is the product ID.
#[derive(Clone, Debug, Serialize)]
pub struct PlanPrice {
    pub id: String,
    pub role: PriceRole,
    pub interval: String,
    pub amount: String,
    pub currency_code: String,
    pub trial_period: Option<CatalogTrialPeriod>,
}

#[derive(Deserialize)]
struct PolarList {
    items: Vec<Value>,
    pagination: PolarPagination,
}
#[derive(Deserialize)]
struct PolarPagination {
    max_page: u32,
}

fn config(state: &SharedState) -> Result<&crate::billing::config::PolarConfig, AppError> {
    state
        .polar
        .as_ref()
        .ok_or_else(|| AppError::ValidationError("Polar billing is not configured".into()))
}

/// Pins the request and response shapes this client parses.
const API_VERSION: &str = "2027-01";

fn url(state: &SharedState, path: &str) -> Result<String, AppError> {
    Ok(format!(
        "{}/v1{path}",
        config(state)?.environment.base_url()
    ))
}

/// Sends a request and returns the JSON body, or a validation error carrying
/// Polar's status. `action` names the operation for logs and messages.
async fn send(
    state: &SharedState,
    request: reqwest::RequestBuilder,
    action: &str,
) -> Result<Value, AppError> {
    let config = config(state)?;
    let response = request
        .bearer_auth(&config.access_token)
        .header("Polar-Version", API_VERSION)
        .send()
        .await
        .map_err(|error| AppError::InternalError(format!("Polar {action} failed: {error}")))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        tracing::warn!(environment = ?config.environment, action, %status, %body, "Polar API request failed");
        return Err(AppError::ValidationError(format!(
            "Polar could not {action}: {status}"
        )));
    }
    response.json().await.map_err(|error| {
        AppError::InternalError(format!("Polar returned invalid JSON for {action}: {error}"))
    })
}

async fn get(state: &SharedState, path: &str, action: &str) -> Result<Value, AppError> {
    send(state, reqwest::Client::new().get(url(state, path)?), action).await
}

/// Walks every page of a Polar list endpoint. `path` must already carry a
/// query string.
async fn list_all(state: &SharedState, path: &str, action: &str) -> Result<Vec<Value>, AppError> {
    let mut items = Vec::new();
    let mut page = 1;
    loop {
        let list: PolarList = serde_json::from_value(
            get(state, &format!("{path}&limit=100&page={page}"), action).await?,
        )
        .map_err(|e| AppError::InternalError(format!("Polar {action} response: {e}")))?;
        items.extend(list.items);
        if page >= list.pagination.max_page {
            return Ok(items);
        }
        page += 1;
    }
}

fn trial_period_from_product(product: &Value) -> Option<CatalogTrialPeriod> {
    Some(CatalogTrialPeriod {
        interval: product.get("trial_interval")?.as_str()?.to_string(),
        frequency: u32::try_from(product.get("trial_interval_count")?.as_u64()?).ok()?,
        requires_payment_method: true,
    })
}

/// Per-unit amount in minor units and the currency of a product's
/// unit-based price. A product can carry one price per currency; USD wins.
// shortcut: reads the first volume tier only, upgrade if tiered rates are ever configured.
fn unit_price(product: &Value) -> Option<(i64, String)> {
    let prices: Vec<&Value> = product
        .get("prices")?
        .as_array()?
        .iter()
        .filter(|price| {
            price.get("amount_type").and_then(Value::as_str) == Some("unit_based")
                && price.get("is_archived").and_then(Value::as_bool) != Some(true)
        })
        .collect();
    let price = prices
        .iter()
        .find(|price| price.get("price_currency").and_then(Value::as_str) == Some("usd"))
        .or_else(|| prices.first())?;
    // `unit_amount` is a decimal string of cents in 2027-01.
    let amount = price.pointer("/tiers/tiers/0/unit_amount")?;
    let amount = amount
        .as_str()
        .and_then(|value| value.parse::<f64>().ok())
        .or_else(|| amount.as_f64())?
        .round() as i64;
    let currency = price.get("price_currency")?.as_str()?.to_uppercase();
    Some((amount, currency))
}

fn plan_price(product: &Value) -> Option<PlanPrice> {
    let id = product.get("id").and_then(Value::as_str)?;
    let Some(role) = product
        .pointer("/metadata/litepod_price_role")
        .and_then(Value::as_str)
        .and_then(PriceRole::parse)
    else {
        tracing::warn!(product_id = id, metadata = ?product.get("metadata"), "Ignoring Polar product without a valid litepod_price_role");
        return None;
    };
    let interval = product
        .get("recurring_interval")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let interval_count = product
        .get("recurring_interval_count")
        .and_then(Value::as_i64)
        .unwrap_or(1);
    if !matches!(interval, "month" | "year") || interval_count != 1 {
        tracing::warn!(
            product_id = id,
            interval,
            interval_count,
            "Ignoring Polar product with an unsupported billing interval"
        );
        return None;
    }
    let Some((amount, currency_code)) = unit_price(product) else {
        tracing::warn!(
            product_id = id,
            "Ignoring Polar product without a unit-based price"
        );
        return None;
    };
    Some(PlanPrice {
        id: id.to_string(),
        role,
        interval: interval.to_string(),
        amount: amount.to_string(),
        currency_code,
        trial_period: trial_period_from_product(product),
    })
}

/// Lists every active Litepod per-server product (month and year), tagged by
/// the product-level `metadata.litepod_price_role`.
pub async fn active_plan_prices(state: &SharedState) -> Result<Vec<PlanPrice>, AppError> {
    let products = list_all(
        state,
        "/products/?is_archived=false&is_recurring=true",
        "list products",
    )
    .await?;
    // Catalog data only (no customer data), so it is safe to log in full.
    tracing::info!(
        products = %serde_json::to_string_pretty(&products).unwrap_or_default(),
        "Polar products received"
    );
    Ok(products.iter().filter_map(plan_price).collect())
}

/// Finds the per-server price for a billing interval.
pub fn server_price<'a>(prices: &'a [PlanPrice], interval: &str) -> Option<&'a PlanPrice> {
    prices
        .iter()
        .find(|price| price.role == PriceRole::PerServer && price.interval == interval)
}

/// Lists subscriptions for the cloud SysAdmin reconciliation flow; ordinary
/// webhook handling never needs to poll Polar.
pub async fn subscriptions_for_reconciliation(state: &SharedState) -> Result<Vec<Value>, AppError> {
    list_all(
        state,
        "/subscriptions/?sorting=-started_at",
        "list subscriptions",
    )
    .await
}

/// Fetches a single subscription's current state straight from Polar: the
/// pull side of reconciliation, for linked accounts whose status may have
/// drifted because a webhook never arrived (e.g. no reachable local tunnel).
/// The response has the same shape as a `subscription.*` webhook's `data`.
pub async fn get_subscription(
    state: &SharedState,
    subscription_id: &str,
) -> Result<Value, AppError> {
    get(
        state,
        &format!("/subscriptions/{subscription_id}"),
        "fetch subscription",
    )
    .await
}

/// Sets a subscription's unit (server slot) count. `proration_behavior` is one of Polar's
/// `invoice` (charge the difference now) or `prorate` (settle on the next
/// invoice).
pub async fn update_subscription_units(
    state: &SharedState,
    subscription_id: &str,
    units: i64,
    proration_behavior: &str,
) -> Result<Value, AppError> {
    let request = reqwest::Client::new()
        .patch(url(state, &format!("/subscriptions/{subscription_id}"))?)
        .json(&json!({"units": units, "proration_behavior": proration_behavior}));
    send(state, request, "update this subscription's server quantity").await
}

/// Starts a hosted checkout for the per-server product and returns its URL.
/// `external_customer_id` ties the resulting Polar customer to the Litepod
/// user, so webhooks resolve the payer without guessing.
pub async fn create_checkout(
    state: &SharedState,
    user_id: &str,
    customer_email: &str,
    customer_name: &str,
    product_id: &str,
    units: i64,
    return_base: &str,
) -> Result<String, AppError> {
    let prices = active_plan_prices(state).await?;
    if !prices
        .iter()
        .any(|price| price.id == product_id && price.role == PriceRole::PerServer)
    {
        return Err(AppError::ValidationError(
            "That Polar product is not an active litepod plan".into(),
        ));
    }
    let base = return_base.trim_end_matches('/');
    let binding = Uuid::now_v7().to_string();
    let request = reqwest::Client::new()
        .post(url(state, "/checkouts/")?)
        .json(&json!({
            "products": [product_id],
            "units": units,
            "external_customer_id": user_id,
            // Prefills the checkout form; the buyer can still change them.
            "customer_email": customer_email,
            "customer_name": (!customer_name.trim().is_empty()).then_some(customer_name),
            "metadata": {"litepod_billing_binding": binding},
            "success_url": format!("{base}/billing?checkout=success&checkout_id={{CHECKOUT_ID}}"),
            "return_url": format!("{base}/billing"),
        }));
    let payload = send(state, request, "create checkout").await?;
    let (Some(checkout_id), Some(checkout_url)) = (
        payload.get("id").and_then(Value::as_str),
        payload.get("url").and_then(Value::as_str),
    ) else {
        return Err(AppError::InternalError(
            "Polar checkout response omitted id or url".into(),
        ));
    };
    crate::billing::repository::create_checkout_binding(
        state,
        &binding,
        user_id,
        product_id,
        checkout_id,
    )
    .await?;
    Ok(checkout_url.to_string())
}

pub async fn customer_portal_url(
    state: &SharedState,
    customer_id: &str,
    return_base: &str,
) -> Result<String, AppError> {
    let request = reqwest::Client::new()
        .post(url(state, "/customer-sessions/")?)
        .json(&json!({
            "customer_id": customer_id,
            "return_url": format!("{}/billing", return_base.trim_end_matches('/')),
        }));
    let payload = send(state, request, "open the customer portal").await?;
    payload
        .get("customer_portal_url")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| AppError::InternalError("Polar portal response omitted the URL".into()))
}

/// Gets an order's invoice URL. Polar generates invoices asynchronously and a
/// `POST` regenerates an existing one, so fetch first and only generate (then
/// poll briefly) when the invoice does not exist yet.
pub async fn invoice_pdf_url(state: &SharedState, order_id: &str) -> Result<String, AppError> {
    let path = format!("/orders/{order_id}/invoice");
    let fetch = || async { get(state, &path, "fetch the invoice").await.ok() };
    let mut payload = fetch().await;
    if payload.is_none() {
        send(
            state,
            reqwest::Client::new().post(url(state, &path)?),
            "generate the invoice",
        )
        .await?;
        for _ in 0..5 {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            payload = fetch().await;
            if payload.is_some() {
                break;
            }
        }
    }
    payload
        .as_ref()
        .and_then(|payload| payload.get("url"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| {
            AppError::ValidationError(
                "Polar is still generating this invoice. Try again in a few seconds.".into(),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn product(metadata: Value, interval: &str) -> Value {
        json!({
            "id": "prod_1",
            "metadata": metadata,
            "recurring_interval": interval,
            "recurring_interval_count": 1,
            "trial_interval": "day",
            "trial_interval_count": 14,
            "prices": [
                {
                    "amount_type": "unit_based",
                    "price_currency": "eur",
                    "tiers": {"type": "volume", "tiers": [{"bound": null, "unit_amount": "0"}]}
                },
                {
                    "amount_type": "unit_based",
                    "price_currency": "usd",
                    "tiers": {"type": "volume", "tiers": [{"bound": 9999, "unit_amount": "300"}]}
                }
            ]
        })
    }

    #[test]
    fn parses_a_unit_based_per_server_product_in_usd() {
        let price = plan_price(&product(
            json!({"litepod_price_role": "per_server"}),
            "month",
        ))
        .unwrap();
        assert_eq!(price.id, "prod_1");
        assert_eq!(price.interval, "month");
        assert_eq!(price.amount, "300");
        assert_eq!(price.currency_code, "USD");
        let trial = price.trial_period.unwrap();
        assert_eq!((trial.interval.as_str(), trial.frequency), ("day", 14));
    }

    #[test]
    fn ignores_products_without_role_or_supported_interval() {
        assert!(plan_price(&product(json!({}), "month")).is_none());
        assert!(
            plan_price(&product(
                json!({"litepod_price_role": "per_server"}),
                "week"
            ))
            .is_none()
        );
    }

    #[test]
    fn price_role_parses_known_roles_only() {
        assert_eq!(PriceRole::parse("per_server"), Some(PriceRole::PerServer));
        assert_eq!(PriceRole::parse("base"), None);
    }

    #[test]
    fn server_price_matches_by_interval() {
        let prices = vec![PlanPrice {
            id: "prod_m".into(),
            role: PriceRole::PerServer,
            interval: "month".into(),
            amount: "300".into(),
            currency_code: "USD".into(),
            trial_period: None,
        }];
        assert_eq!(server_price(&prices, "month").unwrap().id, "prod_m");
        assert!(server_price(&prices, "year").is_none());
    }
}
