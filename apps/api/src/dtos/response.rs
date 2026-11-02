use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
pub struct ApiResponse<T: Serialize> {
    pub error: Option<String>,
    pub data: Option<T>,
}

#[derive(Debug)]
pub enum AppError {
    InvalidCredentials,
    UserNotFound,
    OrganizationNotFound,
    ProjectNotFound,
    EnvironmentNotFound,
    ApplicationNotFound,
    ResourceNotFound,
    ProviderNotFound,
    #[allow(dead_code)] // surfaced by the deploy-guard once the queue backpressure lands.
    DeploymentInProgress(String),
    WebhookVerificationFailed,
    InvitationNotFound,
    InvitationRevoked,
    InvitationAlreadyAccepted,
    RegistrationClosed,
    NotOrganizationMember,
    InsufficientPermissions,
    AlreadyExists,
    ResourceInUse(String),
    #[allow(dead_code)] // org-member invite flow is staged; error variant reserved.
    AlreadyMember,
    InvitationExpired,
    #[allow(dead_code)] // emitted by the future middleware-session expiry path.
    SessionExpired,
    TotpAlreadyEnabled,
    TotpNotEnabled,
    TotpSetupNotStarted,
    TotpInvalidCode,
    #[allow(dead_code)] // consumed by the recovery-code retry path planned for next release.
    TotpRecoveryCodeInvalid,
    #[allow(dead_code)] // inner detail is intentionally not surfaced to clients.
    CatalogUnavailable(String),
    ValidationError(String),
    DatabaseError(String),
    InternalError(String),
    TurnstileFailed,
    /// Password login refused: the user's organization enforces single sign-on.
    SsoRequired,
    /// The organization requires two-factor authentication and this account has
    /// none. Account routes stay open so it can be set up.
    TwoFactorRequired,
    /// Initial setup carries the wrong (or no) install token, so the first
    /// administrator cannot be claimed by just anyone who reaches the endpoint.
    SetupTokenInvalid,
}

/// Machine-readable marker the UI keys on to send the user to 2FA setup.
pub const TWO_FACTOR_REQUIRED_CODE: &str = "two_factor_required";

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppError::InvalidCredentials => write!(f, "Invalid credentials"),
            AppError::UserNotFound => write!(f, "User not found"),
            AppError::OrganizationNotFound => write!(f, "Organization not found"),
            AppError::ProjectNotFound => write!(f, "Project not found"),
            AppError::EnvironmentNotFound => write!(f, "Environment not found"),
            AppError::ApplicationNotFound => write!(f, "Application not found"),
            AppError::ResourceNotFound => write!(f, "Resource not found"),
            AppError::ProviderNotFound => write!(f, "Git provider not found"),
            AppError::DeploymentInProgress(id) => write!(f, "Deployment {} already running", id),
            AppError::WebhookVerificationFailed => write!(f, "Webhook verification failed"),
            AppError::InvitationNotFound => write!(f, "Invitation not found"),
            AppError::InvitationRevoked => write!(f, "Invitation has been revoked"),
            AppError::InvitationAlreadyAccepted => {
                write!(f, "Invitation has already been accepted")
            }
            AppError::RegistrationClosed => {
                write!(f, "Registration is available by invitation only")
            }
            AppError::NotOrganizationMember => write!(f, "Not a member of this organization"),
            AppError::InsufficientPermissions => write!(f, "Insufficient permissions"),
            AppError::AlreadyExists => write!(f, "Resource already exists"),
            AppError::SsoRequired => write!(
                f,
                "Your organization requires single sign-on. Use Continue with SSO."
            ),
            AppError::TwoFactorRequired => write!(
                f,
                "This organization requires two-factor authentication. Set it up on your account to continue."
            ),
            AppError::ResourceInUse(msg) => write!(f, "{}", msg),
            AppError::AlreadyMember => write!(f, "User is already a member of this organization"),
            AppError::InvitationExpired => write!(f, "Invitation has expired"),
            AppError::SessionExpired => write!(f, "Session has expired"),
            AppError::TotpAlreadyEnabled => write!(f, "TOTP is already enabled"),
            AppError::TotpNotEnabled => write!(f, "TOTP is not enabled"),
            AppError::TotpSetupNotStarted => write!(f, "TOTP setup has not been started"),
            AppError::TotpInvalidCode => write!(f, "Invalid TOTP code"),
            AppError::TotpRecoveryCodeInvalid => write!(f, "Invalid or already used recovery code"),
            AppError::CatalogUnavailable(_) => write!(f, "Template catalog is unavailable"),
            AppError::ValidationError(msg) => write!(f, "{}", msg),
            AppError::DatabaseError(msg) => write!(f, "Database error: {}", msg),
            AppError::InternalError(msg) => write!(f, "Internal error: {}", msg),
            AppError::TurnstileFailed => write!(f, "Captcha verification failed"),
            AppError::SetupTokenInvalid => write!(f, "Invalid setup token"),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = match &self {
            AppError::InvalidCredentials => StatusCode::UNAUTHORIZED,
            AppError::UserNotFound => StatusCode::NOT_FOUND,
            AppError::OrganizationNotFound => StatusCode::NOT_FOUND,
            AppError::ProjectNotFound => StatusCode::NOT_FOUND,
            AppError::EnvironmentNotFound => StatusCode::NOT_FOUND,
            AppError::ApplicationNotFound => StatusCode::NOT_FOUND,
            AppError::ResourceNotFound => StatusCode::NOT_FOUND,
            AppError::ProviderNotFound => StatusCode::NOT_FOUND,
            AppError::DeploymentInProgress(_) => StatusCode::CONFLICT,
            AppError::WebhookVerificationFailed => StatusCode::UNAUTHORIZED,
            AppError::InvitationNotFound => StatusCode::NOT_FOUND,
            AppError::InvitationRevoked => StatusCode::GONE,
            AppError::InvitationAlreadyAccepted => StatusCode::GONE,
            AppError::RegistrationClosed => StatusCode::FORBIDDEN,
            AppError::NotOrganizationMember => StatusCode::FORBIDDEN,
            AppError::InsufficientPermissions => StatusCode::FORBIDDEN,
            AppError::AlreadyExists => StatusCode::CONFLICT,
            AppError::ResourceInUse(_) => StatusCode::CONFLICT,
            AppError::AlreadyMember => StatusCode::CONFLICT,
            AppError::InvitationExpired => StatusCode::GONE,
            AppError::SessionExpired => StatusCode::UNAUTHORIZED,
            AppError::TotpAlreadyEnabled => StatusCode::CONFLICT,
            AppError::TotpNotEnabled => StatusCode::CONFLICT,
            AppError::TotpSetupNotStarted => StatusCode::BAD_REQUEST,
            AppError::TotpInvalidCode => StatusCode::BAD_REQUEST,
            AppError::TotpRecoveryCodeInvalid => StatusCode::BAD_REQUEST,
            AppError::CatalogUnavailable(_) => StatusCode::BAD_GATEWAY,
            AppError::ValidationError(_) => StatusCode::UNPROCESSABLE_ENTITY,
            AppError::DatabaseError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            AppError::InternalError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            AppError::TurnstileFailed => StatusCode::FORBIDDEN,
            AppError::SsoRequired => StatusCode::FORBIDDEN,
            AppError::TwoFactorRequired => StatusCode::FORBIDDEN,
            AppError::SetupTokenInvalid => StatusCode::FORBIDDEN,
        };

        if status == StatusCode::INTERNAL_SERVER_ERROR {
            tracing::error!(error = %self, "request failed with an internal server error");
        }

        // The one error with a `code` beside the message: a 403 the UI must
        // tell apart from "you may not see this".
        if matches!(self, AppError::TwoFactorRequired) {
            return (
                status,
                Json(serde_json::json!({
                    "error": self.to_string(),
                    "data": null,
                    "code": TWO_FACTOR_REQUIRED_CODE,
                })),
            )
                .into_response();
        }

        let body = Json(ApiResponse::<()> {
            error: Some(match &self {
                AppError::DatabaseError(_) | AppError::InternalError(_) => {
                    "An internal error occurred. Please try again later.".to_owned()
                }
                _ => self.to_string(),
            }),
            data: None,
        });

        (status, body).into_response()
    }
}

impl From<sqlx::Error> for AppError {
    fn from(error: sqlx::Error) -> Self {
        AppError::DatabaseError(error.to_string())
    }
}

impl From<String> for AppError {
    fn from(error: String) -> Self {
        AppError::DatabaseError(error)
    }
}

pub fn ok<T: Serialize>(data: T) -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(ApiResponse {
            error: None,
            data: Some(data),
        }),
    )
}

pub fn created<T: Serialize>(data: T) -> impl IntoResponse {
    (
        StatusCode::CREATED,
        Json(ApiResponse {
            error: None,
            data: Some(data),
        }),
    )
}

pub fn accepted<T: Serialize>(data: T) -> impl IntoResponse {
    (
        StatusCode::ACCEPTED,
        Json(ApiResponse {
            error: None,
            data: Some(data),
        }),
    )
}

#[cfg(test)]
mod security_tests {
    use super::*;

    #[tokio::test]
    async fn internal_details_never_enter_http_responses() {
        for error in [
            AppError::DatabaseError("SQL error at /private/database".into()),
            AppError::InternalError("upstream credential: secret".into()),
        ] {
            let response = error.into_response();
            assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
            let body = axum::body::to_bytes(response.into_body(), 4096)
                .await
                .unwrap();
            let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(
                value["error"],
                "An internal error occurred. Please try again later."
            );
        }
        let response = AppError::TwoFactorRequired.into_response();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["code"], TWO_FACTOR_REQUIRED_CODE);
        assert!(value["data"].is_null());

        let response = AppError::ValidationError("Name is required".into()).into_response();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        assert!(String::from_utf8_lossy(&body).contains("Name is required"));
    }
}
