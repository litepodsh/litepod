use chrono::Utc;
use sqlx::Row;

use crate::{dtos::response::AppError, state::state::SharedState};

use super::repository::{self, ServerRecord};

#[derive(Debug, Clone)]
pub struct ServerGroupRecord {
    pub id: String,
    pub name: String,
    pub description: String,
    pub created_at: i64,
    pub updated_at: i64,
}

const SELECT_GROUP: &str = "id, name, description, created_at, updated_at";

fn row(record: &sqlx::sqlite::SqliteRow) -> Result<ServerGroupRecord, AppError> {
    let read = |index: usize| -> Result<String, AppError> {
        record
            .try_get(index)
            .map_err(|error| AppError::DatabaseError(error.to_string()))
    };
    Ok(ServerGroupRecord {
        id: read(0)?,
        name: read(1)?,
        description: read(2)?,
        created_at: record
            .try_get(3)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?,
        updated_at: record
            .try_get(4)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?,
    })
}

pub async fn list(
    state: &SharedState,
    organization_id: &str,
) -> Result<Vec<ServerGroupRecord>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(&format!(
        "SELECT {SELECT_GROUP} FROM server_groups WHERE organization_id = ?1 ORDER BY name"
    ))
    .bind(organization_id)
    .fetch_all(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    rows.iter().map(row).collect()
}

pub async fn get(state: &SharedState, id: &str) -> Result<Option<ServerGroupRecord>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(&format!(
        "SELECT {SELECT_GROUP} FROM server_groups WHERE id = ?1"
    ))
    .bind(id)
    .fetch_optional(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?
    .map(|record| row(&record))
    .transpose()
}

pub async fn count_members(state: &SharedState, group_id: &str) -> Result<u64, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let count: i64 = sqlx::query("SELECT COUNT(*) FROM server_group_members WHERE group_id = ?1")
        .bind(group_id)
        .fetch_optional(&conn)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?
        .ok_or_else(|| AppError::DatabaseError("Could not count group members".into()))?
        .try_get(0)
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    Ok(u64::try_from(count).unwrap_or(0))
}

pub async fn get_for_organization(
    state: &SharedState,
    id: &str,
    organization_id: &str,
) -> Result<Option<ServerGroupRecord>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(&format!(
        "SELECT {SELECT_GROUP} FROM server_groups WHERE id = ?1 AND organization_id = ?2"
    ))
    .bind(id)
    .bind(organization_id)
    .fetch_optional(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?
    .map(|record| row(&record))
    .transpose()
}

pub async fn insert(
    state: &SharedState,
    id: &str,
    organization_id: &str,
    name: &str,
    description: &str,
) -> Result<ServerGroupRecord, AppError> {
    let now = Utc::now().timestamp_millis();
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "INSERT INTO server_groups (id, organization_id, name, description, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
    )
    .bind(id)
    .bind(organization_id)
    .bind(name)
    .bind(description)
    .bind(now)
    .execute(&conn)
    .await
    .map_err(|error| {
        let message = error.to_string();
        if message.to_ascii_lowercase().contains("unique") {
            AppError::AlreadyExists
        } else {
            AppError::DatabaseError(message)
        }
    })?;
    get_for_organization(state, id, organization_id)
        .await?
        .ok_or(AppError::ResourceNotFound)
}

pub async fn update(
    state: &SharedState,
    id: &str,
    organization_id: &str,
    name: &str,
    description: &str,
) -> Result<ServerGroupRecord, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE server_groups SET name = ?1, description = ?2, updated_at = ?3 WHERE id = ?4 AND organization_id = ?5",
    )
    .bind(name)
    .bind(description)
    .bind(Utc::now().timestamp_millis())
    .bind(id)
    .bind(organization_id)
    .execute(&conn)
    .await
    .map_err(|error| {
        let message = error.to_string();
        if message.to_ascii_lowercase().contains("unique") {
            AppError::AlreadyExists
        } else {
            AppError::DatabaseError(message)
        }
    })?;
    get_for_organization(state, id, organization_id)
        .await?
        .ok_or(AppError::ResourceNotFound)
}

pub async fn delete(state: &SharedState, id: &str, organization_id: &str) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("DELETE FROM server_group_members WHERE group_id = ?1")
        .bind(id)
        .execute(&conn)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    sqlx::query("DELETE FROM server_groups WHERE id = ?1 AND organization_id = ?2")
        .bind(id)
        .bind(organization_id)
        .execute(&conn)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    Ok(())
}

pub async fn list_members(
    state: &SharedState,
    group_id: &str,
) -> Result<Vec<ServerRecord>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(
        "SELECT s.id FROM server_group_members m JOIN servers s ON s.id = m.server_id WHERE m.group_id = ?1 ORDER BY s.name",
    )
    .bind(group_id)
    .fetch_all(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    let mut members = Vec::new();
    for record in &rows {
        let id: String = record
            .try_get(0)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?;
        members.push(
            repository::get(state, &id)
                .await?
                .ok_or(AppError::ResourceNotFound)?,
        );
    }
    Ok(members)
}

/// Replaces the membership of `group_id` with `server_ids`, atomically. Every
/// id must be a remote (`kind = 'ssh'`) server owned by `organization_id`;
/// the shared `local` server and foreign servers are refused here because
/// SQLite cannot express that check across a foreign key.
pub async fn replace_members(
    state: &SharedState,
    group_id: &str,
    organization_id: &str,
    server_ids: &[String],
) -> Result<(), AppError> {
    for server_id in server_ids {
        let server = repository::get_for_organization(state, server_id, organization_id)
            .await?
            .ok_or_else(|| {
                AppError::ValidationError(format!(
                    "Server {server_id} does not exist in this organization"
                ))
            })?;
        if server.kind != "ssh" {
            return Err(AppError::ValidationError(format!(
                "The local server cannot belong to a server group"
            )));
        }
    }

    let new_ids: std::collections::HashSet<&String> = server_ids.iter().collect();
    for current in list_members(state, group_id).await? {
        if new_ids.contains(&current.id) {
            continue;
        }
        let running: i64 = sqlx::query(
            "SELECT COUNT(*) FROM application_replicas r
             JOIN applications a ON a.id = r.application_id
             WHERE a.server_group_id = ?1 AND r.server_id = ?2 AND r.status IN ('running', 'deploying')",
        )
        .bind(group_id)
        .bind(&current.id)
        .fetch_optional(&state.db)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?
        .ok_or_else(|| AppError::DatabaseError("Could not count running replicas".into()))?
        .try_get::<i64, _>(0)
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
        if running > 0 {
            return Err(AppError::ResourceInUse(format!(
                "Server '{}' still runs a deployed replica and cannot be removed from the group",
                current.name
            )));
        }
    }

    let mut tx = state
        .db
        .begin()
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    sqlx::query("DELETE FROM server_group_members WHERE group_id = ?1")
        .bind(group_id)
        .execute(&mut *tx)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    let now = Utc::now().timestamp_millis();
    for server_id in server_ids {
        sqlx::query(
            "INSERT INTO server_group_members (group_id, server_id, created_at) VALUES (?1, ?2, ?3)",
        )
        .bind(group_id)
        .bind(server_id)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    }
    tx.commit()
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    Ok(())
}

/// The servers an application deploys to: its single `server_id`, or every
/// remote member of its `server_group_id`. Every deployment code path must
/// derive its target through this resolver, never from `applications.server_id`
/// directly, so group apps fan out and single-server apps keep the old shape.
pub async fn resolve_target_servers(
    state: &SharedState,
    application_id: &str,
) -> Result<Vec<ServerRecord>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let record = sqlx::query("SELECT server_id, server_group_id FROM applications WHERE id = ?1")
        .bind(application_id)
        .fetch_optional(&conn)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?
        .ok_or(AppError::ApplicationNotFound)?;
    let server_id: Option<String> = record
        .try_get(0)
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    let group_id: Option<String> = record
        .try_get(1)
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;

    if let Some(server_id) = server_id {
        return Ok(vec![
            repository::get(state, &server_id)
                .await?
                .ok_or(AppError::ResourceNotFound)?,
        ]);
    }

    let group_id = group_id.ok_or(AppError::ApplicationNotFound)?;
    let rows = sqlx::query(
        "SELECT m.server_id FROM server_group_members m JOIN servers s ON s.id = m.server_id WHERE m.group_id = ?1 AND s.kind = 'ssh' ORDER BY s.name",
    )
    .bind(&group_id)
    .fetch_all(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    let mut servers = Vec::new();
    for row in &rows {
        let id: String = row
            .try_get(0)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?;
        servers.push(
            repository::get(state, &id)
                .await?
                .ok_or(AppError::ResourceNotFound)?,
        );
    }
    Ok(servers)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn seed_group(state: &SharedState, organization_id: &str) -> (String, String) {
        let group_id = format!("{organization_id}-group");
        sqlx::query(
            "INSERT INTO server_groups (id, organization_id, name, description, created_at, updated_at) VALUES (?1, ?2, 'dev', '', 1, 1)",
        )
        .bind(&group_id)
        .bind(organization_id)
        .execute(&state.db)
        .await
        .unwrap();
        let remote_a = format!("{organization_id}-server-a");
        let remote_b = format!("{organization_id}-server-b");
        for id in [&remote_a, &remote_b] {
            sqlx::query(
                "INSERT INTO servers (id, kind, name, organization_id, ingress_mode, state, ssh_host, ssh_port, ssh_user, created_at, updated_at) VALUES (?1, 'ssh', ?1, ?2, 'public_edge', 'ready', '198.51.100.1', 22, 'litepod', 1, 1)",
            )
            .bind(id)
            .bind(organization_id)
            .execute(&state.db)
            .await
            .unwrap();
        }
        (group_id, remote_a)
    }

    #[tokio::test]
    async fn resolve_target_servers_returns_group_members_ordered() {
        let (state, path) = crate::testing::test_state().await;
        let (group_id, _remote_a) = seed_group(&state, "organization-a").await;

        sqlx::query(
            "INSERT INTO server_group_members (group_id, server_id, created_at) VALUES (?1, ?2, 1), (?1, ?3, 1)",
        )
        .bind(&group_id)
        .bind(format!("organization-a-server-a"))
        .bind(format!("organization-a-server-b"))
        .execute(&state.db)
        .await
        .unwrap();
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
            "INSERT INTO applications (id, environment_id, name, server_group_id, created_at, updated_at) VALUES ('organization-a-app', 'organization-a-environment', 'app', ?1, 1, 1)",
        )
        .bind(&group_id)
        .execute(&state.db)
        .await
        .unwrap();

        let servers = resolve_target_servers(&state, "organization-a-app")
            .await
            .unwrap();
        let ids: Vec<String> = servers.iter().map(|s| s.id.clone()).collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&"organization-a-server-a".to_string()));
        assert!(ids.contains(&"organization-a-server-b".to_string()));
        assert!(ids.iter().all(|id| id != "local"));

        drop(state);
        crate::testing::cleanup_database(&path);
    }

    #[tokio::test]
    async fn replace_members_rejects_local_and_foreign_servers() {
        let (state, path) = crate::testing::test_state().await;
        let (group_id, _) = seed_group(&state, "organization-a").await;

        let err = replace_members(&state, &group_id, "organization-a", &["local".to_string()])
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::ValidationError(_)));

        // A server owned by organization-b is foreign to organization-a.
        sqlx::query(
            "INSERT INTO servers (id, kind, name, organization_id, ingress_mode, state, ssh_host, ssh_port, ssh_user, created_at, updated_at) VALUES ('organization-b-server-x', 'ssh', 'x', 'organization-b', 'public_edge', 'ready', '198.51.100.2', 22, 'litepod', 1, 1)",
        )
        .execute(&state.db)
        .await
        .unwrap();
        let err = replace_members(
            &state,
            &group_id,
            "organization-a",
            &["organization-b-server-x".to_string()],
        )
        .await
        .unwrap_err();
        assert!(matches!(err, AppError::ValidationError(_)));

        drop(state);
        crate::testing::cleanup_database(&path);
    }
}
