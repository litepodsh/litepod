use chrono::{Duration, Utc};
use serde::Serialize;

use crate::{dtos::response::AppError, state::state::SharedState};

/// Litepod Cloud is now a single unified plan: base fee + a per-remote-server
/// unit price, with no numeric caps on organizations/users. This replaces the
/// old `Entitlement` object parsed from Polar product `custom_data`.
#[derive(Clone, Debug, Serialize)]
pub struct SubscriptionStatus {
    pub status: String,
    pub product_id: Option<String>,
    pub server_price_id: Option<String>,
    pub purchased_server_quantity: i64,
    pub period_ends_at: Option<i64>,
    pub grace_until: Option<i64>,
    pub in_grace: bool,
    /// Canceled in the portal: still usable until `period_ends_at`.
    pub cancel_at_period_end: bool,
}

impl SubscriptionStatus {
    fn from_account_row(row: &crate::billing::repository::BillingAccountRow) -> Self {
        let now = Utc::now().timestamp_millis();
        let in_grace = row.grace_until.is_some_and(|until| until > now);
        // Backward compatibility for accounts written before transaction
        // events were normalized to subscription lifecycle statuses.
        let status = match row.status.as_str() {
            "completed" | "paid" => "active",
            _ => &row.status,
        }
        .to_string();
        Self {
            status,
            product_id: row.polar_product_id.clone(),
            server_price_id: row.per_server_price_id.clone(),
            purchased_server_quantity: row.purchased_server_quantity,
            period_ends_at: row.period_ends_at,
            grace_until: row.grace_until,
            in_grace,
            cancel_at_period_end: row.cancel_at_period_end,
        }
    }
}

pub async fn status_for_payer(
    state: &SharedState,
    payer_id: &str,
) -> Result<SubscriptionStatus, AppError> {
    let row = crate::billing::repository::get_account(state, payer_id).await?;
    let Some(mut row) = row else {
        return Ok(SubscriptionStatus {
            status: "inactive".into(),
            product_id: None,
            server_price_id: None,
            purchased_server_quantity: 0,
            period_ends_at: None,
            grace_until: None,
            in_grace: false,
            cancel_at_period_end: false,
        });
    };
    if matches!(row.status.as_str(), "completed" | "paid") {
        crate::billing::repository::normalize_legacy_transaction_account_status(state, payer_id)
            .await?;
        row.status = "active".into();
    }
    Ok(SubscriptionStatus::from_account_row(&row))
}

const GRACE_PERIOD_DAYS: i64 = 14;

pub fn grace_deadline(ends_at: i64) -> i64 {
    ends_at + Duration::days(GRACE_PERIOD_DAYS).num_milliseconds()
}

/// Gates any action that should require a paying customer (currently: adding
/// a remote server). Organizations and users are unlimited and unchecked.
pub async fn require_active_subscription(
    state: &SharedState,
    payer_id: &str,
) -> Result<(), AppError> {
    let status = status_for_payer(state, payer_id).await?;
    if status.in_grace {
        return Err(AppError::ValidationError(
            "Your subscription is in its grace period; renew to add paid resources.".into(),
        ));
    }
    if !matches!(status.status.as_str(), "active" | "trialing") {
        return Err(AppError::ValidationError(
            "An active subscription is required for this action".into(),
        ));
    }
    Ok(())
}

/// A payer can create a remote server only while the live count is below
/// what they've purchased — `purchased_server_quantity` is a hard cap, not
/// a metered "bill whatever you use" quantity.
fn has_server_capacity(remote_servers: i64, purchased_server_quantity: i64) -> bool {
    remote_servers < purchased_server_quantity
}

pub fn requires_server_entitlement(is_cloud: bool, is_sysadmin: bool) -> bool {
    is_cloud && !is_sysadmin
}

/// Resolves the payer for `organization_id`, requires an active
/// subscription, and requires the live remote-server count to still be
/// under the purchased quantity. Returns the payer ID for callers that need
/// it (none currently do after the purchase-cap rewrite, but keeping the
/// return type avoids reshaping every caller if a future need appears).
pub async fn require_active_subscription_for_server(
    state: &SharedState,
    organization_id: &str,
) -> Result<String, AppError> {
    let payer_id =
        crate::billing::repository::payer_for_organization(state, organization_id).await?;
    require_active_subscription(state, &payer_id).await?;
    let status = status_for_payer(state, &payer_id).await?;
    let (remote_servers, _, _) =
        crate::billing::repository::usage_for_payer(state, &payer_id).await?;
    if !has_server_capacity(remote_servers, status.purchased_server_quantity) {
        return Err(AppError::ValidationError(format!(
            "You've used all {} purchased server slot(s). Buy more from the Billing page.",
            status.purchased_server_quantity
        )));
    }
    Ok(payer_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(
        status: &str,
        grace_until: Option<i64>,
    ) -> crate::billing::repository::BillingAccountRow {
        crate::billing::repository::BillingAccountRow {
            user_id: "u1".into(),
            polar_customer_id: None,
            polar_subscription_id: None,
            polar_product_id: Some("pro_litepod".into()),
            polar_price_id: Some("pri_per_server".into()),
            base_price_id: Some("pri_per_server".into()),
            per_server_price_id: Some("pri_per_server".into()),
            status: status.into(),
            period_starts_at: None,
            period_ends_at: Some(1_800_000_000_000),
            grace_until,
            entitlement_json: "{}".into(),
            purchased_server_quantity: 2,
            quantity_synced_at: None,
            quantity_sync_error: None,
            cancel_at_period_end: false,
        }
    }

    #[test]
    fn subscription_status_keeps_period_end_price_ids_and_quantity() {
        let status = SubscriptionStatus::from_account_row(&row("active", None));
        assert_eq!(status.product_id.as_deref(), Some("pro_litepod"));
        assert_eq!(status.server_price_id.as_deref(), Some("pri_per_server"));
        assert_eq!(status.purchased_server_quantity, 2);
        assert_eq!(status.period_ends_at, Some(1_800_000_000_000));
    }

    #[test]
    fn completed_and_paid_normalize_to_active() {
        assert_eq!(
            SubscriptionStatus::from_account_row(&row("completed", None)).status,
            "active"
        );
        assert_eq!(
            SubscriptionStatus::from_account_row(&row("paid", None)).status,
            "active"
        );
    }

    #[test]
    fn grace_until_in_the_future_marks_in_grace() {
        let future = Utc::now().timestamp_millis() + 60_000;
        assert!(SubscriptionStatus::from_account_row(&row("canceled", Some(future))).in_grace);
    }

    #[test]
    fn has_server_capacity_true_when_under_purchased_quantity() {
        assert!(has_server_capacity(1, 3));
    }

    #[test]
    fn has_server_capacity_false_when_at_purchased_quantity() {
        assert!(!has_server_capacity(3, 3));
    }

    #[test]
    fn has_server_capacity_false_when_over_purchased_quantity() {
        assert!(!has_server_capacity(4, 3));
    }

    #[test]
    fn sysadmin_does_not_need_cloud_server_entitlement() {
        assert!(!requires_server_entitlement(true, true));
    }

    #[test]
    fn regular_cloud_user_needs_server_entitlement() {
        assert!(requires_server_entitlement(true, false));
    }

    #[test]
    fn standard_mode_does_not_need_cloud_server_entitlement() {
        assert!(!requires_server_entitlement(false, false));
    }
}
