use sqlx::SqlitePool;

use crate::cli::OpsError;

pub async fn run(pool: &SqlitePool, current_email: &str, new_email: &str) -> Result<(), OpsError> {
    if !new_email.contains('@') || new_email.len() < 3 {
        return Err(OpsError::Invalid(format!(
            "{new_email:?} is not a valid email"
        )));
    }
    if new_email == current_email {
        return Err(OpsError::Invalid(
            "new email is the same as the current one".into(),
        ));
    }

    let row = sqlx::query("SELECT id, email FROM users WHERE email = ?1")
        .bind(current_email)
        .fetch_optional(pool)
        .await?;
    let Some(row) = row else {
        return Err(OpsError::NotFound(format!(
            "no account with email {current_email}"
        )));
    };
    let user_id: String = sqlx::Row::try_get(&row, 0)
        .map_err(|error| OpsError::Invalid(format!("user row malformed: {error}")))?;

    // Re-check inside the UPDATE: `users.email` is `UNIQUE`, so a race
    // with a concurrent `INSERT` will return `SqliteError::Unique` here
    // and we surface a friendly error instead of a raw DB message.
    let updated = sqlx::query("UPDATE users SET email = ?1 WHERE id = ?2 AND email = ?3")
        .bind(new_email)
        .bind(&user_id)
        .bind(current_email)
        .execute(pool)
        .await;

    match updated {
        Ok(result) if result.rows_affected() == 0 => Err(OpsError::Invalid(
            "email was not changed (row vanished mid-flight)".into(),
        )),
        Ok(_) => {
            // Email is the address we send password resets to. Revoke
            // sessions so the operator is forced to log in again and
            // re-verify the new address is reachable.
            let revoked = sqlx::query("DELETE FROM sessions WHERE user_id = ?1")
                .bind(&user_id)
                .execute(pool)
                .await?
                .rows_affected();
            println!(
                "email changed: {current_email} -> {new_email} (user_id={user_id}); {revoked} session(s) revoked"
            );
            Ok(())
        }
        Err(sqlx::Error::Database(database_error))
            if database_error.message().to_uppercase().contains("UNIQUE") =>
        {
            Err(OpsError::Invalid(format!(
                "{new_email} is already in use by another account"
            )))
        }
        Err(error) => Err(OpsError::Db(error)),
    }
}
