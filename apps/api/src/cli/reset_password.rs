use sqlx::SqlitePool;

use crate::cli::OpsError;
use crate::utils::security;

pub async fn run(pool: &SqlitePool, email: &str, new_password: &str) -> Result<(), OpsError> {
    if new_password.len() < 8 {
        return Err(OpsError::Invalid(
            "password must be at least 8 characters".into(),
        ));
    }

    // Same lookup pattern as `git::repositories::user::get_user_by_email`,
    // kept inline so the CLI does not pull the full `SharedState` (which
    // needs Caddy/Dragonfly/Podman URLs that an offline tool does not
    // care about). One round-trip, then we own the user id and can
    // update + revoke without a second lookup.
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

    let hashed = security::hash_password(new_password.to_string());

    let updated = sqlx::query("UPDATE users SET hashed_password = ?1 WHERE id = ?2")
        .bind(&hashed)
        .bind(&user_id)
        .execute(pool)
        .await?
        .rows_affected();
    if updated == 0 {
        // The user existed a moment ago; this is a real DB error.
        return Err(OpsError::Invalid(
            "password was not updated (row vanished mid-flight)".into(),
        ));
    }

    // Revoke every session: if a password reset is happening, the
    // existing session is no longer trusted. Same call site as the HTTP
    // `password_reset::confirm` path (`api/src/users/password_reset.rs`).
    let revoked = sqlx::query("DELETE FROM sessions WHERE user_id = ?1")
        .bind(&user_id)
        .execute(pool)
        .await?
        .rows_affected();

    println!("password reset for {stored_email} (user_id={user_id}); {revoked} session(s) revoked");
    Ok(())
}
