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

/// Writes an event about `user_id` with nobody signed in to attribute it to,
/// such as an email confirmation, which is authenticated by its code alone.
pub async fn record_about(
    state: &SharedState,
    user_id: &str,
    action: &str,
    metadata: Option<&str>,
) -> Result<(), String> {
    insert(state, Some(user_id), None, action, metadata).await
}

/// Cloud SysAdmin's view of what happens inside organizations. Independent of
/// the organization's own audit log and its toggle. `actor` is `None` when no
/// user is behind it (a Git push, a deploy hook).
pub async fn record_for_organization(
    state: &SharedState,
    organization_id: &str,
    actor: Option<&AuthUser>,
    action: &str,
    target_name: Option<&str>,
) -> Result<(), String> {
    if !state.is_cloud {
        return Ok(());
    }
    let conn = crate::git::repositories::db::get_conn(state).await;
    let organization: Option<String> =
        sqlx::query_scalar("SELECT name FROM organizations WHERE id = ?1")
            .bind(organization_id)
            .fetch_optional(&conn)
            .await
            .map_err(|e| e.to_string())?;
    let metadata = [organization.as_deref(), target_name]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
    // An empty id is a non-user actor (SCIM), which `actor_user_id` cannot reference.
    let actor = actor.filter(|actor| !actor.user_id.is_empty());
    let subject = actor.map(|actor| actor.user_id.as_str());
    insert(state, subject, actor, action, Some(&metadata)).await
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

pub const DEFAULT_RETENTION_DAYS: i64 = 90;
pub const MAX_RETENTION_DAYS: i64 = 365;

pub async fn retention_days(state: &SharedState) -> Result<i64, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query_scalar("SELECT retention_days FROM security_event_settings WHERE id = 1")
        .fetch_one(&conn)
        .await
        .map_err(|e| e.to_string())
}

/// Stores the window and applies a shorter one right away instead of at the next daily purge.
pub async fn set_retention_days(state: &SharedState, days: i64) -> Result<(), String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("UPDATE security_event_settings SET retention_days = ?1 WHERE id = 1")
        .bind(days)
        .execute(&conn)
        .await
        .map_err(|e| e.to_string())?;
    purge_expired(state).await.map(|_| ())
}

pub async fn purge_expired(state: &SharedState) -> Result<u64, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let result = sqlx::query(
        "DELETE FROM security_events WHERE created_at < datetime('now', '-' || \
         (SELECT retention_days FROM security_event_settings WHERE id = 1) || ' days')",
    )
    .execute(&conn)
    .await
    .map_err(|e| e.to_string())?;
    Ok(result.rows_affected())
}

/// Purges at startup, then daily. Cloud only: the privacy policy promises this
/// window for Litepod Cloud, and self-hosted operators set their own rules.
pub fn spawn(state: SharedState) {
    if !state.is_cloud {
        return;
    }
    tokio::spawn(async move {
        let mut daily = tokio::time::interval(std::time::Duration::from_secs(24 * 60 * 60));
        loop {
            daily.tick().await;
            if let Err(error) = purge_expired(&state).await {
                tracing::warn!(%error, "security event purge failed");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{cleanup_database, test_state_for};

    async fn seed(state: &SharedState, id: &str, age_days: i64) {
        sqlx::query(
            "INSERT INTO security_events (id, action, created_at) \
             VALUES (?1, 'auth.login', datetime('now', '-' || ?2 || ' days'))",
        )
        .bind(id)
        .bind(age_days)
        .execute(&state.db)
        .await
        .unwrap();
    }

    async fn ids(state: &SharedState) -> Vec<String> {
        sqlx::query_scalar("SELECT id FROM security_events ORDER BY id")
            .fetch_all(&state.db)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn purge_follows_the_default_and_an_extended_window() {
        let (state, path) = test_state_for(true).await;
        assert_eq!(
            retention_days(&state).await.unwrap(),
            DEFAULT_RETENTION_DAYS
        );
        for (id, age) in [("a-89", 89), ("b-120", 120), ("c-400", 400)] {
            seed(&state, id, age).await;
        }
        set_retention_days(&state, 365).await.unwrap();
        assert_eq!(ids(&state).await, ["a-89", "b-120"]);
        set_retention_days(&state, 90).await.unwrap();
        assert_eq!(ids(&state).await, ["a-89"]);
        cleanup_database(&path);
    }

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
