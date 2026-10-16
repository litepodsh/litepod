//! Acceptance of the cloud legal documents (Terms of Service, Acceptable Use
//! Policy, Privacy Policy).
//!
//! The sign-up form has a checkbox, but the form is not the boundary: a cloud
//! account cannot be created through the API without `accept_terms: true`, and
//! every acceptance is stored with the version, time, IP and user agent so it
//! can be shown later. Self-hosted installs have no hosted service to agree
//! to, so none of this applies there.

use chrono::Utc;
use uuid::Uuid;

use crate::dtos::response::AppError;
use crate::state::state::SharedState;

/// The "Last updated" date of the legal pages in the UI. Bump both together
/// when a document changes materially, so new acceptances name the new text.
pub const LEGAL_VERSION: &str = "2026-10-07";

/// Who accepted from where, captured by the handler that saw the request.
pub struct Acceptance {
    pub ip: Option<String>,
    pub user_agent: Option<String>,
}

/// Refuses a cloud sign-up that did not accept the legal documents.
pub fn require_accepted(is_cloud: bool, accepted: bool) -> Result<(), AppError> {
    if is_cloud && !accepted {
        return Err(AppError::ValidationError(
            "You must accept the Terms of Service, Acceptable Use Policy and Privacy Policy to create an account"
                .into(),
        ));
    }
    Ok(())
}

/// Stores one acceptance of the current version.
pub async fn record(
    state: &SharedState,
    user_id: &str,
    acceptance: &Acceptance,
) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "INSERT INTO legal_acceptances (id, user_id, version, accepted_at, ip, user_agent) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )
    .bind(Uuid::now_v7().to_string())
    .bind(user_id)
    .bind(LEGAL_VERSION)
    .bind(Utc::now().timestamp_millis())
    .bind(acceptance.ip.as_deref())
    .bind(acceptance.user_agent.as_deref())
    .execute(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_cloud_requires_acceptance() {
        assert!(require_accepted(true, false).is_err());
        assert!(require_accepted(true, true).is_ok());
        assert!(require_accepted(false, false).is_ok());
    }

    #[tokio::test]
    async fn an_acceptance_keeps_version_time_and_origin() {
        let (state, path) = crate::testing::test_state_for(true).await;
        let acceptance = Acceptance {
            ip: Some("203.0.113.7".into()),
            user_agent: Some("test-agent".into()),
        };
        record(&state, "editor", &acceptance).await.unwrap();
        let (version, ip, agent): (String, String, String) = sqlx::query_as(
            "SELECT version, ip, user_agent FROM legal_acceptances WHERE user_id = 'editor'",
        )
        .fetch_one(&state.db)
        .await
        .unwrap();
        assert_eq!(version, LEGAL_VERSION);
        assert_eq!(ip, "203.0.113.7");
        assert_eq!(agent, "test-agent");
        crate::testing::cleanup_database(&path);
    }
}
