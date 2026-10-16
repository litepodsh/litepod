//! What happens to an application when its server reboots.
//!
//! `desired_state` is the user's intent, kept apart from `status` (what Podman
//! reports), which the monitor overwrites after every reboot. A trigger from
//! migration 0039 writes `running`; only a user stop/remove writes `stopped`.

use serde::{Deserialize, Serialize};
use sqlx::Row;

use crate::{dtos::response::AppError, state::state::SharedState};

pub const POLICIES: [&str; 3] = ["restore", "always", "notify"];

#[derive(Debug, Serialize, Deserialize)]
pub struct BootPolicyDto {
    pub boot_policy: String,
}

pub async fn get_policy(
    state: &SharedState,
    application_id: &str,
) -> Result<BootPolicyDto, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query("SELECT boot_policy FROM applications WHERE id = ?1")
        .bind(application_id)
        .fetch_optional(&conn)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?
        .ok_or(AppError::ApplicationNotFound)?;
    Ok(BootPolicyDto {
        boot_policy: row
            .try_get(0)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?,
    })
}

pub async fn set_policy(
    state: &SharedState,
    application_id: &str,
    policy: &str,
) -> Result<BootPolicyDto, AppError> {
    if !POLICIES.contains(&policy) {
        return Err(AppError::ValidationError(format!(
            "Unknown boot policy '{policy}'"
        )));
    }
    let conn = crate::git::repositories::db::get_conn(state).await;
    let changed =
        sqlx::query("UPDATE applications SET boot_policy = ?1, updated_at = ?2 WHERE id = ?3")
            .bind(policy)
            .bind(chrono::Utc::now().timestamp_millis())
            .bind(application_id)
            .execute(&conn)
            .await
            .map_err(|error| AppError::DatabaseError(error.to_string()))?
            .rows_affected();
    if changed == 0 {
        return Err(AppError::ApplicationNotFound);
    }
    Ok(BootPolicyDto {
        boot_policy: policy.to_string(),
    })
}

/// Called at the top of every user stop/remove, before the containers go away, so a
/// stop that fails halfway still records what the user asked for.
pub async fn mark_stopped(state: &SharedState, application_id: &str) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE application_deployments SET desired_state = 'stopped' WHERE application_id = ?1",
    )
    .bind(application_id)
    .execute(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn seeded() -> (SharedState, std::path::PathBuf, String) {
        let (state, path) = crate::testing::test_state().await;
        let (_, _, application_id) =
            crate::testing::seed_application(&state, "organization-a").await;
        sqlx::query("INSERT INTO application_deployments (application_id, status, container_name, hostname, updated_at) VALUES (?1, 'running', 'app', 'app.test', 1)")
            .bind(&application_id)
            .execute(&state.db)
            .await
            .unwrap();
        (state, path, application_id)
    }

    async fn desired(state: &SharedState, application_id: &str) -> String {
        sqlx::query("SELECT desired_state FROM application_deployments WHERE application_id = ?1")
            .bind(application_id)
            .fetch_one(&state.db)
            .await
            .unwrap()
            .try_get(0)
            .unwrap()
    }

    async fn set_status(state: &SharedState, application_id: &str, status: &str) {
        sqlx::query("UPDATE application_deployments SET status = ?1 WHERE application_id = ?2")
            .bind(status)
            .bind(application_id)
            .execute(&state.db)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn policy_defaults_to_restore_and_round_trips() {
        let (state, path, application_id) = seeded().await;
        assert_eq!(
            get_policy(&state, &application_id)
                .await
                .unwrap()
                .boot_policy,
            "restore"
        );
        set_policy(&state, &application_id, "notify").await.unwrap();
        assert_eq!(
            get_policy(&state, &application_id)
                .await
                .unwrap()
                .boot_policy,
            "notify"
        );
        crate::testing::cleanup_database(&path);
    }

    #[tokio::test]
    async fn unknown_policy_and_unknown_app_are_rejected() {
        let (state, path, application_id) = seeded().await;
        assert!(matches!(
            set_policy(&state, &application_id, "sometimes").await,
            Err(AppError::ValidationError(_))
        ));
        assert!(matches!(
            set_policy(&state, "missing", "always").await,
            Err(AppError::ApplicationNotFound)
        ));
        assert!(matches!(
            get_policy(&state, "missing").await,
            Err(AppError::ApplicationNotFound)
        ));
        crate::testing::cleanup_database(&path);
    }

    #[tokio::test]
    async fn a_user_stop_is_remembered_and_a_restart_overrides_it() {
        let (state, path, application_id) = seeded().await;
        mark_stopped(&state, &application_id).await.unwrap();
        assert_eq!(desired(&state, &application_id).await, "stopped");
        set_status(&state, &application_id, "stopped").await;
        set_status(&state, &application_id, "running").await;
        assert_eq!(desired(&state, &application_id).await, "running");
        crate::testing::cleanup_database(&path);
    }
}
