use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Deserialize, ToSchema)]
pub struct RegisterRequest {
    pub username: String,
    pub email: String,
    pub password: String,
    pub confirm_password: String,
    /// Cloudflare Turnstile token from the widget. Required (and verified) only on the
    /// cloud path; self-hosted installs never send it.
    #[serde(default)]
    pub turnstile_token: Option<String>,
    /// Ticked "I accept the Terms, Acceptable Use Policy and Privacy Policy".
    /// Required on cloud (`users::legal`); ignored on self-hosted.
    #[serde(default)]
    pub accept_terms: bool,
}

#[derive(Deserialize, ToSchema)]
pub struct SetupRequest {
    pub name: String,
    pub email: String,
    pub password: String,
    pub confirm_password: String,
}

#[derive(Serialize, ToSchema)]
pub struct SetupCheckResponse {
    pub setup_required: bool,
    pub user_count: i64,
}

#[derive(Serialize, ToSchema)]
pub struct RegistrationStatusResponse {
    pub setup_required: bool,
    pub public_registration_open: bool,
    pub is_cloud: bool,
}

#[derive(Deserialize, ToSchema)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
    /// Cloudflare Turnstile token. Only required once `login_ban_middleware`'s failure
    /// counter for this IP crosses `LOGIN_CAPTCHA_THRESHOLD` (cloud only).
    #[serde(default)]
    pub turnstile_token: Option<String>,
}

/// 401/403 body for `login_handler`, in place of the generic `ApiResponse` error shape, so
/// the frontend knows whether to render the Turnstile widget before the user retries.
#[derive(Serialize, ToSchema)]
pub struct LoginErrorResponse {
    pub error: String,
    pub captcha_required: bool,
}

/// Returned by `login_handler` when the password was right but the account has
/// TOTP enabled. Carries no session cookie: the caller must present a code to
/// `POST /api/v1/auth/login/totp` before anything is issued.
#[derive(Serialize, ToSchema)]
pub struct TotpChallengeResponse {
    pub totp_required: bool,
    pub challenge_token: String,
}

#[derive(Deserialize, ToSchema)]
pub struct CompleteTotpLoginRequest {
    pub challenge_token: String,
    /// A six-digit authenticator code, or one of the account's recovery codes.
    pub code: String,
}

#[derive(Deserialize, ToSchema)]
pub struct PasswordResetRequest {
    pub email: String,
}

#[derive(Deserialize, ToSchema)]
pub struct PasswordResetConfirmRequest {
    pub token: String,
    pub password: String,
    pub confirm_password: String,
}

#[derive(Serialize, ToSchema)]
pub struct AuthResponse {
    pub user: UserClaims,
}

#[derive(Serialize, ToSchema)]
pub struct EmailVerificationPendingResponse {
    pub email: String,
    pub accepted_deliveries: u8,
    pub resend_available_at: i64,
    pub delivery_failed: bool,
    pub reactivating: bool,
}
#[derive(Deserialize, ToSchema)]
pub struct ConfirmEmailVerificationRequest {
    pub email: String,
    pub code: String,
}
#[derive(Deserialize, ToSchema)]
pub struct ResendEmailVerificationRequest {
    pub email: String,
}
#[derive(Serialize, ToSchema)]
pub struct ConfirmEmailVerificationResponse {
    pub verified: bool,
    pub reactivated: bool,
}

#[derive(Serialize, ToSchema, Clone)]
pub struct UserClaims {
    pub user_id: String,
    pub email: String,
    pub name: String,
    pub is_sysadmin: bool,
}

#[derive(Deserialize, ToSchema)]
#[allow(dead_code)] // refresh flow is staged behind a feature flag; DTO is published but not consumed yet.
pub struct RefreshRequest {
    pub refresh_token: String,
}

#[derive(Serialize, ToSchema)]
pub struct MeResponse {
    pub user_id: String,
    pub email: String,
    pub name: String,
    pub is_sysadmin: bool,
    pub created_at: String,
}
