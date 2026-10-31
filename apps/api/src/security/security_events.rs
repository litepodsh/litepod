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
//! Auth events are geolocated and checked for anomalies afterwards, on cloud
//! only (`login_anomaly`).
//!
//! `record` never fails the caller's real operation: call sites swallow or
//! warn on the error, mirroring `audit_log::record`.

use crate::security::middleware::auth::AuthUser;
use crate::security::middleware::request_context;
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
///
/// The source IP and user agent come from the current request, when there is one.
pub async fn record(
    state: &SharedState,
    subject_user_id: Option<&str>,
    actor: &AuthUser,
    action: &str,
    metadata: Option<&str>,
) -> Result<(), String> {
    insert(state, subject_user_id, Some(actor), action, metadata).await
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
    insert(state, None, None, action, metadata).await
}

async fn insert(
    state: &SharedState,
    subject_user_id: Option<&str>,
    actor: Option<&AuthUser>,
    action: &str,
    metadata: Option<&str>,
) -> Result<(), String> {
    let context = request_context::current();
    let source_ip = context
        .as_ref()
        .and_then(|context| context.source_ip)
        .map(|ip| ip.to_string());
    let user_agent = context.and_then(|context| context.user_agent);
    let id = Uuid::now_v7().to_string();
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "INSERT INTO security_events (id, user_id, actor_user_id, actor_token_id, actor_token_name, action, metadata, source_ip, user_agent) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
    )
    .bind(&id)
    .bind(subject_user_id)
    .bind(actor.map(|actor| &actor.user_id))
    .bind(actor.and_then(|actor| actor.token_id.as_ref()))
    .bind(actor.and_then(|actor| actor.token_name.as_ref()))
    .bind(action)
    .bind(metadata)
    .bind(&source_ip)
    .bind(&user_agent)
    .execute(&conn)
    .await
    .map_err(|e| e.to_string())?;
    // Off the request path: ip.guide can take seconds, and a login must not wait on it.
    // Cloud only — a self-hosted install never sends its users' IPs to a third party.
    if state.is_cloud
        && crate::security::login_anomaly::is_located(action)
        && let Some(ip) = source_ip
    {
        tokio::spawn(crate::security::login_anomaly::locate_and_check(
            state.clone(),
            id,
            ip,
        ));
    }
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
