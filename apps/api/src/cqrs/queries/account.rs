use crate::dtos::account::{AccountProfileResponse, SessionDto};
use crate::dtos::response::AppError;
use crate::git::repositories::{self, account as account_repo};
use crate::state::state::SharedState;

/// Get the current user's account profile.
pub async fn profile(
    state: &SharedState,
    user_id: &str,
) -> Result<AccountProfileResponse, AppError> {
    let user_row = repositories::user::get_user_by_id(state, user_id)
        .await
        .ok_or(AppError::UserNotFound)?;

    let totp_enabled = account_repo::is_totp_enabled(state, user_id).await?;
    let recovery_codes_remaining = account_repo::count_unused_recovery_codes(state, user_id).await;

    Ok(AccountProfileResponse {
        user_id: user_row.id,
        name: user_row.name,
        email: user_row.email,
        totp_enabled,
        recovery_codes_remaining,
    })
}

/// List active sessions for the current user.
pub async fn sessions(
    state: &SharedState,
    user_id: &str,
    current_session_token: &str,
) -> Result<Vec<SessionDto>, AppError> {
    let rows = account_repo::list_sessions(state, user_id).await;
    // Rows carry digests, so the cookie has to be digested to spot the current one.
    let current_digest = crate::utils::security::digest_token(current_session_token);
    let sessions: Vec<SessionDto> = rows
        .into_iter()
        .map(
            |(id, token_digest, created_at, expires_at, user_agent)| SessionDto {
                id,
                created_at,
                expires_at,
                is_current: token_digest == current_digest,
                user_agent,
            },
        )
        .collect();
    Ok(sessions)
}
