//! Ambient "which replica am I deploying to" context for group fan-out.
//!
//! A group app fans out across servers by running the ordinary single-server
//! pipeline once per member. Rather than thread a `server_id` through the ~25
//! call sites that resolve the application's server, they all read this
//! task-local: when a fan-out worker sets it, `ServerRuntime::for_application`
//! and `runtime_endpoints::ensure_remote` resolve to that member instead of
//! `applications.server_id`. Single-server apps never set it and behave exactly
//! as before.

use std::future::Future;

use sqlx::Row;

use crate::{dtos::response::AppError, state::state::SharedState};

tokio::task_local! {
    static REPLICA_SERVER: String;
}

/// The replica server the current task is deploying to, if any.
pub fn current() -> Option<String> {
    REPLICA_SERVER.try_with(|id| id.clone()).ok()
}

/// Suffixes a litepod-local scratch path (clone, staging dir) with the active
/// replica, so replicas of one run deploying in parallel never share it.
pub fn scoped_path(path: std::path::PathBuf) -> std::path::PathBuf {
    match current() {
        Some(server_id) => path.join(format!("replica-{server_id}")),
        None => path,
    }
}

/// Runs `future` with `server_id` as the active replica target, restoring the
/// previous (absent) context afterwards.
pub async fn for_replica<T>(server_id: &str, future: impl Future<Output = T>) -> T {
    REPLICA_SERVER.scope(server_id.to_string(), future).await
}

/// The server an application's endpoint/ingress code should target: the ambient
/// replica during fan-out, otherwise the application's single `server_id`.
/// `None` means a group app is being asked for a single target outside a fan-out
/// (e.g. domain reconciliation before its per-domain pin is set).
pub async fn target_server_id(
    state: &SharedState,
    application_id: &str,
) -> Result<Option<String>, AppError> {
    if let Some(server_id) = current() {
        return Ok(Some(server_id));
    }
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query("SELECT server_id FROM applications WHERE id = ?1")
        .bind(application_id)
        .fetch_optional(&conn)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?
        .ok_or(AppError::ApplicationNotFound)?;
    row.try_get::<Option<String>, _>(0)
        .map_err(|error| AppError::DatabaseError(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn seed_group_app(state: &SharedState) {
        sqlx::query(
            "INSERT INTO server_groups (id, organization_id, name, created_at, updated_at) VALUES ('group-1', 'organization-a', 'dev', 1, 1)",
        )
        .execute(&state.db)
        .await
        .unwrap();
        for id in ["organization-a-server-a", "organization-a-server-b"] {
            sqlx::query(
                "INSERT INTO servers (id, kind, name, organization_id, ingress_mode, state, ssh_host, ssh_port, ssh_user, created_at, updated_at) VALUES (?1, 'ssh', ?1, 'organization-a', 'public_edge', 'ready', '198.51.100.1', 22, 'litepod', 1, 1)",
            )
            .bind(id)
            .execute(&state.db)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO server_group_members (group_id, server_id, created_at) VALUES ('group-1', ?1, 1)",
            )
            .bind(id)
            .execute(&state.db)
            .await
            .unwrap();
        }
        sqlx::query(
            "INSERT INTO projects (id, organization_id, name, created_at, updated_at) VALUES ('organization-a-project', 'organization-a', 'p', 1, 1)",
        )
        .execute(&state.db)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO environments (id, project_id, name, created_at, updated_at) VALUES ('organization-a-environment', 'organization-a-project', 'prod', 1, 1)",
        )
        .execute(&state.db)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO applications (id, environment_id, name, server_group_id, created_at, updated_at) VALUES ('app-1', 'organization-a-environment', 'app', 'group-1', 1, 1)",
        )
        .execute(&state.db)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn for_replica_scopes_the_ambient_target() {
        let (state, path) = crate::testing::test_state().await;
        seed_group_app(&state).await;

        assert_eq!(target_server_id(&state, "app-1").await.unwrap(), None);

        let resolved = for_replica("organization-a-server-a", target_server_id(&state, "app-1"))
            .await
            .unwrap();
        assert_eq!(resolved, Some("organization-a-server-a".to_string()));

        assert_eq!(target_server_id(&state, "app-1").await.unwrap(), None);

        drop(state);
        crate::testing::cleanup_database(&path);
    }
}
