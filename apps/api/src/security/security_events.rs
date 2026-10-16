//! Append-only security event trail for account self-service and sysadmin
//! operations (migration `0044`).
//!
//! Complements `git::repositories::audit_log` for events with no clean
//! organization scope: logins, password changes, two-factor toggles, session
//! and access-value revocation, backups, restores, maintenance, certificate
//! inventory changes, infrastructure restarts and system updates. Like the
//! terminal session tables it is deliberate, write-only forensics — nothing
//! that can authenticate is ever stored.
//!
//! `record` never fails the caller's real operation: call sites swallow or
//! warn on the error, mirroring `audit_log::record`.

use crate::security::middleware::auth::AuthUser;
use crate::state::state::SharedState;
use uuid::Uuid;

/// Writes one security event row.
///
/// - `subject_user_id` is the account the event is *about* (`None` for pure
///   sysadmin actions such as a maintenance prune or a system update).
/// - `actor` is who performed it — a session or (never for account security:
///   those routes are session-only) a personal access token.
/// - `action` follows the `domain.verb` vocabulary (`auth.login`,
///   `password.changed`, `two_factor.enabled`, `backup.restored`,
///   `infrastructure.restarted`, …).
/// - `metadata` is a short string; never include credentials.
pub async fn record(
    state: &SharedState,
    subject_user_id: Option<&str>,
    actor: &AuthUser,
    action: &str,
    metadata: Option<&str>,
) -> Result<(), String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "INSERT INTO security_events (id, user_id, actor_user_id, actor_token_id, actor_token_name, action, metadata) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )
    .bind(Uuid::now_v7().to_string())
    .bind(subject_user_id)
    .bind(&actor.user_id)
    .bind(&actor.token_id)
    .bind(&actor.token_name)
    .bind(action)
    .bind(metadata)
    .execute(&conn)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Writes an event with no authenticated actor — used by failure paths such as
/// a wrong-password login, where there is no verified user to attribute. Never
/// guess a user id here: `actor_user_id` is a foreign key, so a fabricated id
/// would fail the insert and (worse) another real user's id would misattribute
/// the event.
pub async fn record_anonymous(
    state: &SharedState,
    action: &str,
    metadata: Option<&str>,
) -> Result<(), String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "INSERT INTO security_events (id, user_id, actor_user_id, actor_token_id, actor_token_name, action, metadata) VALUES (?1, NULL, NULL, NULL, NULL, ?2, ?3)",
    )
    .bind(Uuid::now_v7().to_string())
    .bind(action)
    .bind(metadata)
    .execute(&conn)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn security_events_schema_stores_lifecycle_only() {
        let migration = include_str!("../../migrations/0044_security_events.sql");
        for column in [
            "user_id",
            "actor_user_id",
            "actor_token_id",
            "action",
            "metadata",
            "created_at",
        ] {
            assert!(migration.contains(column), "missing audit column: {column}");
        }
        // Nothing that can authenticate may ever be persisted.
        for forbidden in [
            "password",
            "totp",
            "secret",
            "recovery",
            "token_value",
            "private_key",
        ] {
            assert!(
                !migration.contains(forbidden),
                "must not persist {forbidden}"
            );
        }
    }
}
