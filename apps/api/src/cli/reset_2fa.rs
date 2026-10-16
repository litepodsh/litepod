use sqlx::SqlitePool;

use crate::cli::OpsError;

/// Disable TOTP for `email` and wipe the recovery-code table. The user's
/// next login will not prompt for a second factor; they can re-enroll
/// from the account settings page.
///
/// TOTP secrets in the database are AES-256-GCM ciphertext keyed by
/// `ENCRYPTION_KEY`. We never read them here — going straight to
/// `totp_secret = NULL, totp_enabled = 0` skips the decrypt path entirely,
/// which is what we want on the offline recovery path.
pub async fn run(pool: &SqlitePool, email: &str) -> Result<(), OpsError> {
    let row = sqlx::query("SELECT id, email FROM users WHERE email = ?1")
        .bind(email)
        .fetch_optional(pool)
        .await?;
    let Some(row) = row else {
        return Err(OpsError::NotFound(format!("no account with email {email}")));
    };
    let user_id: String = sqlx::Row::try_get(&row, 0)
        .map_err(|error| OpsError::Invalid(format!("user row malformed: {error}")))?;
    let stored_email: String = sqlx::Row::try_get(&row, 1)
        .map_err(|error| OpsError::Invalid(format!("user row malformed: {error}")))?;

    let updated =
        sqlx::query("UPDATE users SET totp_secret = NULL, totp_enabled = 0 WHERE id = ?1")
            .bind(&user_id)
            .execute(pool)
            .await?
            .rows_affected();
    if updated == 0 {
        return Err(OpsError::Invalid(
            "2FA was not reset (row vanished mid-flight)".into(),
        ));
    }

    let removed = sqlx::query("DELETE FROM recovery_codes WHERE user_id = ?1")
        .bind(&user_id)
        .execute(pool)
        .await?
        .rows_affected();

    println!(
        "2FA reset for {stored_email} (user_id={user_id}); {removed} recovery code(s) cleared"
    );
    Ok(())
}
