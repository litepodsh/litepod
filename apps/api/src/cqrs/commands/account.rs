use crate::dtos::account::{TotpEnabledResponse, TotpSetupResponse};
use crate::dtos::response::AppError;
use crate::git::repositories::{self, account as account_repo};
use crate::state::state::SharedState;
use crate::utils::{security, totp};

/// Change the current user's password.
/// Change the current user's password and evict every other session.
///
/// `keep_token` is the caller's own session, which survives; everything else is
/// deleted. Changing a password is how someone responds to believing it leaked,
/// and leaving the attacker's existing session alive makes that response useless.
pub async fn change_password(
    state: &SharedState,
    user_id: &str,
    current_password: &str,
    new_password: &str,
    confirm_password: &str,
    keep_token: Option<&str>,
) -> Result<(), AppError> {
    if new_password != confirm_password {
        return Err(AppError::ValidationError("Passwords do not match".into()));
    }

    let user_row = repositories::user::get_user_by_id(state, user_id)
        .await
        .ok_or(AppError::UserNotFound)?;

    if !security::verify_password(&user_row.hashed_password, current_password.as_bytes()) {
        return Err(AppError::ValidationError(
            "Current password is incorrect".into(),
        ));
    }

    let hashed = security::hash_password(new_password.to_string());
    crate::git::repositories::db::retry_on_stale_snapshot(|| async {
        let conn = crate::git::repositories::db::get_conn(state).await;
        sqlx::query("UPDATE users SET hashed_password = ?1 WHERE id = ?2")
            .bind(hashed.clone())
            .bind(user_id)
            .execute(&conn)
            .await
            .map_err(Into::into)
    })
    .await?;

    if let Err(error) =
        repositories::session::revoke_all_for_user_except(state, user_id, keep_token).await
    {
        // The password is already changed; refusing now would be worse than
        // reporting it. Loud, because a stale session surviving matters.
        tracing::error!(%error, %user_id, "password changed but other sessions were not revoked");
    }

    Ok(())
}

/// Generate a TOTP secret and store it (but don't enable yet).
pub async fn totp_setup(
    state: &SharedState,
    user_id: &str,
    email: &str,
) -> Result<TotpSetupResponse, AppError> {
    if account_repo::is_totp_enabled(state, user_id).await? {
        return Err(AppError::TotpAlreadyEnabled);
    }

    let secret = totp::generate_secret();
    let otpauth_url = totp::generate_otpauth_url(&secret, email, "litepod");

    // Encrypt the secret before storing
    let key = security::get_encryption_key();
    let encrypted =
        security::encrypt_data(&secret, &key).map_err(|e| AppError::InternalError(e))?;

    account_repo::set_totp_secret(state, user_id, &encrypted).await?;

    Ok(TotpSetupResponse {
        secret,
        otpauth_url,
    })
}

/// Verify a TOTP code and enable TOTP for the user.
/// Returns the plaintext recovery codes (one-time show).
pub async fn totp_verify_and_enable(
    state: &SharedState,
    user_id: &str,
    code: &str,
) -> Result<TotpEnabledResponse, AppError> {
    if account_repo::is_totp_enabled(state, user_id).await? {
        return Err(AppError::TotpAlreadyEnabled);
    }

    let encrypted_secret = account_repo::get_totp_secret(state, user_id)
        .await
        .ok_or(AppError::TotpSetupNotStarted)?;

    let key = security::get_encryption_key();
    let secret = security::decrypt_data(&encrypted_secret, &key)
        .map_err(|_| AppError::InternalError("Failed to decrypt TOTP secret".into()))?;

    // Verify with strict tolerance (exact current window)
    if !totp::verify_code_strict(&secret, code) {
        return Err(AppError::TotpInvalidCode);
    }

    // Enable TOTP
    account_repo::set_totp_enabled(state, user_id, true).await?;

    // Delete any old recovery codes
    let _ = account_repo::delete_recovery_codes(state, user_id).await;

    // Generate new recovery codes
    let recovery_codes = generate_recovery_codes();
    account_repo::save_recovery_codes(state, user_id, &recovery_codes).await?;

    Ok(TotpEnabledResponse {
        enabled: true,
        recovery_codes,
    })
}

/// Decrypt the user's stored TOTP secret.
pub(crate) async fn totp_secret_for(
    state: &SharedState,
    user_id: &str,
) -> Result<String, AppError> {
    let encrypted = account_repo::get_totp_secret(state, user_id)
        .await
        .ok_or(AppError::TotpNotEnabled)?;
    security::decrypt_data(&encrypted, &security::get_encryption_key())
        .map_err(|_| AppError::InternalError("Failed to decrypt TOTP secret".into()))
}

/// Accepts either a live TOTP code or one of the user's unused recovery codes,
/// spending whichever matched.
///
/// The single place second factors are checked, so login, disabling TOTP and any
/// future step-up all get the same guarantees: the TOTP path burns the code
/// against replay (`totp::verify_code_once`) and the recovery path claims the row
/// atomically by digest (`account_repo::claim_recovery_code`).
///
/// Returns which factor matched, `"totp"` or `"recovery_code"`, for the audit log.
pub async fn verify_second_factor(
    state: &SharedState,
    user_id: &str,
    code: &str,
) -> Result<&'static str, AppError> {
    let secret = totp_secret_for(state, user_id).await?;
    if totp::verify_code_once(state, user_id, &secret, code).await {
        return Ok("totp");
    }
    if account_repo::claim_recovery_code(state, user_id, code).await? {
        return Ok("recovery_code");
    }
    Err(AppError::TotpInvalidCode)
}

/// Disable TOTP. Accepts either a valid TOTP code or a valid recovery code.
pub async fn totp_disable(state: &SharedState, user_id: &str, code: &str) -> Result<(), AppError> {
    if !account_repo::is_totp_enabled(state, user_id).await? {
        return Err(AppError::TotpNotEnabled);
    }

    verify_second_factor(state, user_id, code).await?;

    // Disable TOTP and clear recovery codes
    account_repo::set_totp_enabled(state, user_id, false).await?;

    account_repo::delete_recovery_codes(state, user_id).await?;

    // Clear the secret
    account_repo::set_totp_secret(state, user_id, "").await?;

    Ok(())
}

/// Regenerate recovery codes (invalidates old ones). Requires TOTP verification.
pub async fn regenerate_recovery_codes(
    state: &SharedState,
    user_id: &str,
    totp_code: &str,
) -> Result<TotpEnabledResponse, AppError> {
    if !account_repo::is_totp_enabled(state, user_id).await? {
        return Err(AppError::TotpNotEnabled);
    }

    // A live TOTP code only — a recovery code must not be able to mint a fresh
    // set of recovery codes, or one leaked code becomes permanent access.
    let secret = totp_secret_for(state, user_id).await?;
    if !totp::verify_code_once(state, user_id, &secret, totp_code).await {
        return Err(AppError::TotpInvalidCode);
    }

    // Delete old codes
    account_repo::delete_recovery_codes(state, user_id).await?;

    // Generate new codes
    let recovery_codes = generate_recovery_codes();
    account_repo::save_recovery_codes(state, user_id, &recovery_codes).await?;

    Ok(TotpEnabledResponse {
        enabled: true,
        recovery_codes,
    })
}

/// Revoke every session for the user, including the current one. MCP agent
/// grants go too: their tokens and their session records close together.
/// Returns the number of browser sessions revoked.
pub async fn revoke_all_sessions(state: &SharedState, user_id: &str) -> Result<u64, AppError> {
    let _ = crate::git::repositories::oauth::revoke_all_grants_for_user(state, user_id)
        .await
        .map_err(AppError::DatabaseError)?;
    crate::git::repositories::db::retry_on_stale_snapshot(|| async {
        let conn = crate::git::repositories::db::get_conn(state).await;
        sqlx::query("DELETE FROM sessions WHERE user_id = ?1")
            .bind(user_id)
            .execute(&conn)
            .await
            .map(|result| result.rows_affected())
            .map_err(Into::into)
    })
    .await
}

/// Revoke every session for the user except the one making the request. MCP
/// agent grants are never the current session, so they all go as well.
/// Returns the number of browser sessions revoked.
pub async fn revoke_other_sessions(
    state: &SharedState,
    user_id: &str,
    keep_token: &str,
) -> Result<u64, AppError> {
    let _ = crate::git::repositories::oauth::revoke_all_grants_for_user(state, user_id)
        .await
        .map_err(AppError::DatabaseError)?;
    repositories::session::revoke_all_for_user_except(state, user_id, Some(keep_token))
        .await
        .map_err(Into::into)
}

/// Deactivate a user while preserving their data for a future verified reactivation.
/// The state change and session revocation share one transaction so an account cannot
/// remain usable if the request succeeds.
pub async fn deactivate_account(
    state: &SharedState,
    user_id: &str,
    current_password: &str,
) -> Result<(), AppError> {
    let user_row = repositories::user::get_user_by_id(state, user_id)
        .await
        .ok_or(AppError::UserNotFound)?;

    if !security::verify_password(&user_row.hashed_password, current_password.as_bytes()) {
        return Err(AppError::ValidationError(
            "Current password is incorrect".into(),
        ));
    }

    crate::git::repositories::db::retry_on_stale_snapshot(|| async {
        let conn = crate::git::repositories::db::get_conn(state).await;
        let mut tx = conn.begin().await?;
        sqlx::query(
            "UPDATE users SET status_id = 2, deactivated_at = datetime('now') WHERE id = ?1 AND status_id = 1",
        )
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM sessions WHERE user_id = ?1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    })
    .await?;

    Ok(())
}

/// Generate 8 random recovery codes (8 chars each, alphanumeric).
fn generate_recovery_codes() -> Vec<String> {
    use rand::RngExt;
    let charset: Vec<char> = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789".chars().collect();
    let mut codes = Vec::new();
    for _ in 0..8 {
        let code: String = (0..8)
            .map(|_| {
                let idx = rand::rng().random_range(0..charset.len());
                charset[idx]
            })
            .collect();
        codes.push(code);
    }
    codes
}
