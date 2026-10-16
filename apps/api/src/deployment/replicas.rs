//! Per-server deployment state for group-targeted applications. A group app is
//! one logical application deployed as an independent replica on every member
//! server; this table records the per-replica outcome so the UI can show
//! "2 of 3 running" instead of a single binary status.

use chrono::Utc;

use crate::{dtos::response::AppError, state::state::SharedState};

pub const STATUS_NOT_DEPLOYED: &str = "not_deployed";
pub const STATUS_DEPLOYING: &str = "deploying";
pub const STATUS_RUNNING: &str = "running";
pub const STATUS_FAILED: &str = "failed";
pub const STATUS_STOPPED: &str = "stopped";

#[derive(Debug, Clone)]
pub struct ApplicationReplicaRecord {
    pub server_id: String,
    pub status: String,
    pub last_error: Option<String>,
    pub deployed_at: Option<i64>,
}

fn row(record: &sqlx::sqlite::SqliteRow) -> Result<ApplicationReplicaRecord, AppError> {
    use sqlx::Row;
    Ok(ApplicationReplicaRecord {
        server_id: record
            .try_get(0)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?,
        status: record
            .try_get(1)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?,
        last_error: record
            .try_get(2)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?,
        deployed_at: record
            .try_get(3)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?,
    })
}

/// Makes replica rows match the application's current target servers: inserts a
/// `not_deployed` row for every new member and removes rows whose server left
/// the target (membership or the app's own target changed). Idempotent.
pub async fn reconcile(
    state: &SharedState,
    application_id: &str,
    server_ids: &[String],
) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let mut tx = conn
        .begin()
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    let now = Utc::now().timestamp_millis();
    for server_id in server_ids {
        sqlx::query(
            "INSERT OR IGNORE INTO application_replicas (application_id, server_id, status, container_name, updated_at) VALUES (?1, ?2, 'not_deployed', '', ?3)",
        )
        .bind(application_id)
        .bind(server_id)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    }
    if server_ids.is_empty() {
        sqlx::query("DELETE FROM application_replicas WHERE application_id = ?1")
            .bind(application_id)
            .execute(&mut *tx)
            .await
            .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    } else {
        let placeholders = (0..server_ids.len())
            .map(|index| format!("?{}", index + 2))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "DELETE FROM application_replicas WHERE application_id = ?1 AND server_id NOT IN ({placeholders})"
        );
        let mut query = sqlx::query(&sql).bind(application_id);
        for server_id in server_ids {
            query = query.bind(server_id);
        }
        query
            .execute(&mut *tx)
            .await
            .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    }
    tx.commit()
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    Ok(())
}

pub async fn set_status(
    state: &SharedState,
    application_id: &str,
    server_id: &str,
    status: &str,
    last_error: Option<&str>,
) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let now = Utc::now().timestamp_millis();
    sqlx::query(
        "UPDATE application_replicas SET status = ?1, last_error = ?2, deployed_at = CASE WHEN ?1 = 'running' THEN ?3 ELSE deployed_at END, updated_at = ?3 WHERE application_id = ?4 AND server_id = ?5",
    )
    .bind(status)
    .bind(last_error)
    .bind(now)
    .bind(application_id)
    .bind(server_id)
    .execute(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    Ok(())
}

pub async fn list(
    state: &SharedState,
    application_id: &str,
) -> Result<Vec<ApplicationReplicaRecord>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(
        "SELECT server_id, status, last_error, deployed_at FROM application_replicas WHERE application_id = ?1 ORDER BY server_id",
    )
    .bind(application_id)
    .fetch_all(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    rows.iter().map(row).collect()
}

/// Aggregate status for an application with replicas: `running` when at least
/// one replica runs, `failed` when none run and at least one failed, otherwise
/// `not_deployed`.
pub async fn aggregate(
    state: &SharedState,
    application_id: &str,
) -> Result<Option<String>, AppError> {
    let replicas = list(state, application_id).await?;
    if replicas.is_empty() {
        return Ok(None);
    }
    if replicas
        .iter()
        .any(|replica| replica.status == STATUS_RUNNING)
    {
        return Ok(Some(STATUS_RUNNING.to_string()));
    }
    if replicas
        .iter()
        .any(|replica| matches!(replica.status.as_str(), STATUS_FAILED))
    {
        return Ok(Some(STATUS_FAILED.to_string()));
    }
    if replicas
        .iter()
        .any(|replica| replica.status == STATUS_DEPLOYING)
    {
        return Ok(Some(STATUS_DEPLOYING.to_string()));
    }
    if replicas
        .iter()
        .any(|replica| replica.status == STATUS_STOPPED)
    {
        return Ok(Some(STATUS_STOPPED.to_string()));
    }
    Ok(Some(STATUS_NOT_DEPLOYED.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reconcile_inserts_and_prunes_replicas() {
        let (state, path) = crate::testing::test_state().await;
        sqlx::query(
            "INSERT INTO servers (id, kind, name, organization_id, ingress_mode, state, ssh_host, ssh_port, ssh_user, created_at, updated_at) VALUES ('srv-a', 'ssh', 'a', 'organization-a', 'public_edge', 'ready', '198.51.100.1', 22, 'litepod', 1, 1)",
        )
        .execute(&state.db).await.unwrap();
        sqlx::query(
            "INSERT INTO servers (id, kind, name, organization_id, ingress_mode, state, ssh_host, ssh_port, ssh_user, created_at, updated_at) VALUES ('srv-b', 'ssh', 'b', 'organization-a', 'public_edge', 'ready', '198.51.100.2', 22, 'litepod', 1, 1)",
        )
        .execute(&state.db).await.unwrap();
        sqlx::query(
            "INSERT INTO projects (id, organization_id, name, created_at, updated_at) VALUES ('organization-a-project', 'organization-a', 'p', 1, 1)",
        )
        .execute(&state.db).await.unwrap();
        sqlx::query(
            "INSERT INTO environments (id, project_id, name, created_at, updated_at) VALUES ('organization-a-environment', 'organization-a-project', 'prod', 1, 1)",
        )
        .execute(&state.db).await.unwrap();
        sqlx::query(
            "INSERT INTO applications (id, environment_id, name, server_id, created_at, updated_at) VALUES ('app-1', 'organization-a-environment', 'app', 'srv-a', 1, 1)",
        )
        .execute(&state.db).await.unwrap();

        reconcile(&state, "app-1", &["srv-a".to_string(), "srv-b".to_string()])
            .await
            .unwrap();
        assert_eq!(list(&state, "app-1").await.unwrap().len(), 2);

        reconcile(&state, "app-1", &["srv-a".to_string()])
            .await
            .unwrap();
        let replicas = list(&state, "app-1").await.unwrap();
        assert_eq!(replicas.len(), 1);
        assert_eq!(replicas[0].server_id, "srv-a");

        drop(state);
        crate::testing::cleanup_database(&path);
    }

    #[tokio::test]
    async fn aggregate_prefers_running_then_failed() {
        let (state, path) = crate::testing::test_state().await;
        sqlx::query(
            "INSERT INTO servers (id, kind, name, organization_id, ingress_mode, state, ssh_host, ssh_port, ssh_user, created_at, updated_at) VALUES ('srv-a', 'ssh', 'a', 'organization-a', 'public_edge', 'ready', '198.51.100.1', 22, 'litepod', 1, 1)",
        )
        .execute(&state.db).await.unwrap();
        sqlx::query(
            "INSERT INTO projects (id, organization_id, name, created_at, updated_at) VALUES ('organization-a-project', 'organization-a', 'p', 1, 1)",
        )
        .execute(&state.db).await.unwrap();
        sqlx::query(
            "INSERT INTO environments (id, project_id, name, created_at, updated_at) VALUES ('organization-a-environment', 'organization-a-project', 'prod', 1, 1)",
        )
        .execute(&state.db).await.unwrap();
        sqlx::query(
            "INSERT INTO applications (id, environment_id, name, server_id, created_at, updated_at) VALUES ('app-1', 'organization-a-environment', 'app', 'srv-a', 1, 1)",
        )
        .execute(&state.db).await.unwrap();

        reconcile(&state, "app-1", &["srv-a".to_string()])
            .await
            .unwrap();
        assert_eq!(
            aggregate(&state, "app-1").await.unwrap(),
            Some(STATUS_NOT_DEPLOYED.to_string())
        );

        set_status(&state, "app-1", "srv-a", STATUS_FAILED, Some("boom"))
            .await
            .unwrap();
        assert_eq!(
            aggregate(&state, "app-1").await.unwrap(),
            Some(STATUS_FAILED.to_string())
        );

        set_status(&state, "app-1", "srv-a", STATUS_RUNNING, None)
            .await
            .unwrap();
        assert_eq!(
            aggregate(&state, "app-1").await.unwrap(),
            Some(STATUS_RUNNING.to_string())
        );

        drop(state);
        crate::testing::cleanup_database(&path);
    }
}
