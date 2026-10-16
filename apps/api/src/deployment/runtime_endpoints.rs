//! Durable, server-scoped loopback endpoints for remote application ingress.
//!
//! A remote endpoint is not a public Podman port. It is the stable
//! `127.0.0.1:<host_port>` target used by a server-local Caddy instance or by
//! litepod's private relay. Local applications deliberately have no row here.

use chrono::Utc;

use crate::{dtos::response::AppError, state::state::SharedState};

pub const REMOTE_PORT_MIN: i64 = 42_000;
pub const REMOTE_PORT_MAX: i64 = 42_999;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeEndpoint {
    pub application_id: String,
    pub server_id: String,
    pub host_port: u16,
    pub container_port: u16,
}

fn row(row: &sqlx::sqlite::SqliteRow) -> Result<RuntimeEndpoint, AppError> {
    use sqlx::Row;
    let host_port: i64 = row
        .try_get(2)
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    let container_port: i64 = row
        .try_get(3)
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    Ok(RuntimeEndpoint {
        application_id: row
            .try_get(0)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?,
        server_id: row
            .try_get(1)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?,
        host_port: u16::try_from(host_port).map_err(|_| {
            AppError::DatabaseError("Invalid persisted remote endpoint port".into())
        })?,
        container_port: u16::try_from(container_port).map_err(|_| {
            AppError::DatabaseError("Invalid persisted remote container port".into())
        })?,
    })
}

pub async fn get(
    state: &SharedState,
    application_id: &str,
) -> Result<Option<RuntimeEndpoint>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "SELECT application_id, server_id, host_port, container_port FROM application_runtime_endpoints WHERE application_id = ?1",
    )
    .bind(application_id)
    .fetch_optional(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?
    .map(|record| row(&record))
    .transpose()
}

/// Same as `get`, but for an application with more than one tunnel (a remote
/// Compose app with several publicly routed services) - picks the one row for
/// this exact `server_id` and `container_port` instead of an arbitrary row.
pub async fn get_for_service(
    state: &SharedState,
    application_id: &str,
    server_id: &str,
    container_port: u16,
) -> Result<Option<RuntimeEndpoint>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "SELECT application_id, server_id, host_port, container_port FROM application_runtime_endpoints WHERE application_id = ?1 AND server_id = ?2 AND container_port = ?3",
    )
    .bind(application_id)
    .bind(server_id)
    .bind(container_port as i64)
    .fetch_optional(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?
    .map(|record| row(&record))
    .transpose()
}

/// Removes endpoint rows left behind when a non-Compose application's listener
/// port changes. Compose intentionally keeps one row per service port.
pub async fn remove_stale_for_application(
    state: &SharedState,
    application_id: &str,
    current_container_port: u16,
) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "DELETE FROM application_runtime_endpoints WHERE application_id = ?1 AND container_port != ?2",
    )
    .bind(application_id)
    .bind(current_container_port as i64)
    .execute(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    Ok(())
}

/// Returns no endpoint for a Local application. For a remote application this
/// allocates the first available member of the closed litepod range and persists
/// it before Podman is called. A concurrent winner simply causes a retry with
/// a newly-read allocation set; there is no fall back to local Podman.
pub async fn ensure_remote(
    state: &SharedState,
    application_id: &str,
    container_port: u16,
) -> Result<Option<RuntimeEndpoint>, AppError> {
    if container_port == 0 {
        return Err(AppError::ValidationError(
            "Remote container port must be between 1 and 65535".into(),
        ));
    }

    use sqlx::Row;
    let conn = crate::git::repositories::db::get_conn(state).await;
    // A group-app replica resolves its server from the ambient fan-out context;
    // a single-server app resolves it from `applications.server_id`.
    let server_id = match crate::servers::replica::current() {
        Some(server_id) => server_id,
        None => {
            let placement = sqlx::query("SELECT server_id FROM applications WHERE id = ?1")
                .bind(application_id)
                .fetch_optional(&conn)
                .await
                .map_err(|error| AppError::DatabaseError(error.to_string()))?
                .ok_or(AppError::ApplicationNotFound)?;
            placement
                .try_get::<Option<String>, _>(0)
                .map_err(|error| AppError::DatabaseError(error.to_string()))?
                .ok_or_else(|| {
                    AppError::ValidationError(
                        "This application targets a server group; deploy through a replica".into(),
                    )
                })?
        }
    };
    let placement = sqlx::query("SELECT kind, state FROM servers WHERE id = ?1")
        .bind(&server_id)
        .fetch_optional(&conn)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?
        .ok_or(AppError::ApplicationNotFound)?;
    let kind: String = placement
        .try_get(0)
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    let server_state: String = placement
        .try_get(1)
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;

    if kind == "local" {
        return Ok(None);
    }
    if kind != "ssh" || server_state != "ready" {
        return Err(AppError::ResourceInUse(format!(
            "The selected server is unavailable for remote deployment"
        )));
    }

    // container_port is now part of the row's identity (an app can hold one
    // tunnel per distinct service/port), so a service whose port changed just
    // gets a new row - the stale row for its old port is left as an orphaned
    // tunnel, same lazy-cleanup tradeoff the single app-level row already had.
    if let Some(existing) =
        get_for_service(state, application_id, &server_id, container_port).await?
    {
        return Ok(Some(existing));
    }

    // The unique server/port index is the final authority. Retrying lets two
    // deployment workers safely race without ever assigning a shared socket.
    for _ in REMOTE_PORT_MIN..=REMOTE_PORT_MAX {
        let taken_rows =
            sqlx::query("SELECT host_port FROM application_runtime_endpoints WHERE server_id = ?1")
                .bind(server_id.clone())
                .fetch_all(&conn)
                .await
                .map_err(|error| AppError::DatabaseError(error.to_string()))?;
        let mut taken = std::collections::HashSet::new();
        for row in &taken_rows {
            taken.insert(
                row.try_get::<i64, _>(0)
                    .map_err(|error| AppError::DatabaseError(error.to_string()))?,
            );
        }
        let Some(host_port) =
            (REMOTE_PORT_MIN..=REMOTE_PORT_MAX).find(|port| !taken.contains(port))
        else {
            return Err(AppError::ValidationError(format!(
                "No remote loopback port is available on the selected server ({}-{})",
                REMOTE_PORT_MIN, REMOTE_PORT_MAX
            )));
        };
        let now = Utc::now().timestamp_millis();
        match sqlx::query(
            "INSERT INTO application_runtime_endpoints (application_id, server_id, host_port, container_port, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
        )
        .bind(application_id)
        .bind(server_id.clone())
        .bind(host_port)
        .bind(container_port as i64)
        .bind(now)
        .execute(&conn)
        .await
        {
            Ok(_) => return get_for_service(state, application_id, &server_id, container_port).await,
            Err(error) if error.to_string().to_ascii_lowercase().contains("unique") => continue,
            Err(error) => return Err(AppError::DatabaseError(error.to_string())),
        }
    }

    Err(AppError::ValidationError(
        "Could not allocate a unique remote loopback port; retry the deployment".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_port_range_is_private_and_bounded() {
        assert!(REMOTE_PORT_MIN >= 1024);
        assert!(REMOTE_PORT_MAX < 65_536);
        assert!(REMOTE_PORT_MIN <= 42_123 && 42_123 <= REMOTE_PORT_MAX);
    }
}
