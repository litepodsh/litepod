use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::billing::{entitlement::SubscriptionStatus, polar::PlanPrice};

#[derive(Deserialize, ToSchema)]
pub struct CheckoutRequest {
    pub interval: String,
}

#[derive(Deserialize, ToSchema)]
pub struct UpdateServerQuantityRequest {
    pub quantity: i64,
}

#[derive(Serialize, ToSchema)]
pub struct CheckoutResponse {
    pub checkout_url: String,
}

/// Litepod Cloud is now a single plan: a base price plus a per-server price,
/// each available monthly and annually. There is no more per-tier product
/// list to choose between.
#[derive(Serialize)]
pub struct BillingPlanDto {
    pub product_id: String,
    pub name: String,
    pub description: String,
    pub prices: Vec<PlanPrice>,
}

/// Server-computed so the frontend never does currency math itself.
#[derive(Serialize)]
pub struct BillingEstimateDto {
    pub interval: String,
    pub server_amount: String,
    pub server_quantity: i64,
    pub currency_code: String,
    pub estimated_total: String,
}

#[derive(Serialize)]
pub struct BillingOverviewResponse {
    pub polar_environment: String,
    pub plan: BillingPlanDto,
    pub subscription: SubscriptionStatus,
    pub usage: BillingUsage,
    pub estimate: Option<BillingEstimateDto>,
}

#[derive(Serialize)]
pub struct BillingUsage {
    pub remote_servers: i64,
    pub organizations: i64,
    pub users: i64,
    pub purchased_server_quantity: i64,
}

#[derive(Serialize)]
pub struct PortalResponse {
    pub url: String,
}

#[derive(Serialize)]
pub struct InvoicePdfResponse {
    pub url: String,
}
