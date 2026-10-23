use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Full account profile returned by GET /api/v1/account
#[derive(Serialize, ToSchema)]
pub struct AccountProfileResponse {
    pub user_id: String,
    pub name: String,
    pub email: String,
    pub totp_enabled: bool,
    pub recovery_codes_remaining: i64,
}

/// Request body for PATCH /api/v1/account
#[derive(Deserialize, ToSchema)]
pub struct UpdateAccountRequest {
    pub name: String,
}

/// Request body for DELETE /api/v1/account.
#[derive(Deserialize, ToSchema)]
pub struct DeactivateAccountRequest {
    pub current_password: String,
    pub confirmation: String,
}

/// Request body for POST /api/v1/account/change-password
#[derive(Deserialize, ToSchema)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
    pub confirm_password: String,
}

/// Response for POST /api/v1/account/totp/setup
#[derive(Serialize, ToSchema)]
pub struct TotpSetupResponse {
    pub secret: String,
    pub otpauth_url: String,
}

/// Request body for POST /api/v1/account/totp/verify
#[derive(Deserialize, ToSchema)]
pub struct TotpVerifyRequest {
    pub code: String,
}

/// Response for POST /api/v1/account/totp/verify and POST /api/v1/account/recovery-codes
#[derive(Serialize, ToSchema)]
pub struct TotpEnabledResponse {
    pub enabled: bool,
    pub recovery_codes: Vec<String>,
}

/// Request for POST /api/v1/account/totp/disable
#[derive(Deserialize, ToSchema)]
pub struct TotpDisableRequest {
    pub code: String,
}

/// Request for POST /api/v1/account/recovery-codes
#[derive(Deserialize, ToSchema)]
pub struct RegenerateCodesRequest {
    pub code: String,
}

/// Session info returned by GET /api/v1/account/sessions.
/// `kind` is "device" for a signed-in browser or "mcp" for an agent connected
/// over MCP OAuth; the MCP-only fields stay `None` on device sessions.
#[derive(Serialize, ToSchema)]
pub struct SessionDto {
    pub id: String,
    pub created_at: String,
    pub expires_at: String,
    pub is_current: bool,
    pub user_agent: Option<String>,
    pub kind: String,
    pub client_name: Option<String>,
    pub agent: Option<String>,
    /// When an MCP grant was revoked or expired. `None` while it is open.
    pub closed_at: Option<String>,
}
