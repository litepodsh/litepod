use sqlx::Row;

use crate::{
    dtos::{response::AppError, terminal_session::TerminalSessionDto},
    git::repositories::db::get_conn,
    state::state::SharedState,
};

pub const DEFAULT_LIMIT: i64 = 20;
pub const MAX_LIMIT: i64 = 100;

pub enum SessionScope<'a> {
    /// Container exec sessions opened from the application terminal.
    Application(&'a str),
    /// Server console container exec sessions, plus host shell sessions when
    /// `include_host` (the caller could open that host shell).
    Server { id: &'a str, include_host: bool },
}

/// Newest first, keyset-paginated on the UUIDv7 `id` (`before` = last id of
/// the previous page). Always bounded by `organization_id`.
pub async fn list_sessions(
    state: &SharedState,
    organization_id: &str,
    scope: SessionScope<'_>,
    limit: Option<i64>,
    before: Option<&str>,
) -> Result<Vec<TerminalSessionDto>, AppError> {
    let (application_id, server_id, include_host) = match scope {
        SessionScope::Application(id) => (Some(id), None, false),
        SessionScope::Server { id, include_host } => (None, Some(id), include_host),
    };
    let limit = limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let conn = get_conn(state).await;
    let rows = sqlx::query(
        "SELECT s.id, s.kind, s.container_id, s.started_at, s.finished_at,
                CAST(ROUND((julianday(s.finished_at) - julianday(s.started_at)) * 86400) AS INTEGER),
                s.result, s.failure_reason, u.name, u.email, s.actor_token_name
         FROM (
             SELECT id, 'host' AS kind, NULL AS container_id, organization_id, actor_user_id,
                    actor_token_name, started_at, finished_at, result, failure_reason
             FROM remote_terminal_sessions
             WHERE ?4 = 1 AND server_id = ?3
             UNION ALL
             SELECT id, 'container', container_id, organization_id, actor_user_id,
                    actor_token_name, started_at, finished_at, result, failure_reason
             FROM container_exec_sessions
             WHERE application_id = ?2 OR server_id = ?3
         ) s
         LEFT JOIN users u ON u.id = s.actor_user_id
         WHERE s.organization_id = ?1 AND (?5 IS NULL OR s.id < ?5)
         ORDER BY s.id DESC
         LIMIT ?6",
    )
    .bind(organization_id)
    .bind(application_id)
    .bind(server_id)
    .bind(include_host)
    .bind(before)
    .bind(limit)
    .fetch_all(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    rows.iter()
        .map(|row| {
            Ok(TerminalSessionDto {
                id: row.try_get(0)?,
                kind: row.try_get(1)?,
                container_id: row.try_get(2)?,
                started_at: row.try_get(3)?,
                finished_at: row.try_get(4)?,
                duration_seconds: row.try_get(5)?,
                result: row.try_get(6)?,
                failure_reason: row.try_get(7)?,
                actor_name: row.try_get(8)?,
                actor_email: row.try_get(9)?,
                actor_token_name: row.try_get(10)?,
            })
        })
        .collect::<Result<_, sqlx::Error>>()
        .map_err(|error| AppError::DatabaseError(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{cleanup_database, seed_application, test_state};

    async fn exec(
        state: &SharedState,
        id: &str,
        org: &str,
        app: Option<&str>,
        server: Option<&str>,
    ) {
        sqlx::query(
            "INSERT INTO container_exec_sessions (id, organization_id, application_id, server_id, container_id, actor_user_id, started_at)
             VALUES (?1, ?2, ?3, ?4, 'web-1', 'editor', '2026-01-01 10:00:00')",
        )
        .bind(id)
        .bind(org)
        .bind(app)
        .bind(server)
        .execute(&state.db)
        .await
        .unwrap();
    }

    async fn host(state: &SharedState, id: &str, org: &str, server: &str) {
        sqlx::query(
            "INSERT INTO remote_terminal_sessions (id, organization_id, server_id, actor_token_id, actor_token_name, started_at, finished_at, result)
             VALUES (?1, ?2, ?3, 't1', 'ci token', '2026-01-01 10:00:00', '2026-01-01 10:01:30', 'closed')",
        )
        .bind(id)
        .bind(org)
        .bind(server)
        .execute(&state.db)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn lists_application_and_server_history_newest_first_scoped_to_org() {
        let (state, path) = test_state().await;
        let (_, _, app) = seed_application(&state, "organization-a").await;
        let (_, _, other_app) = seed_application(&state, "organization-b").await;
        let server = "organization-a-server";
        exec(&state, "01", "organization-a", Some(&app), None).await;
        exec(&state, "02", "organization-a", None, Some(server)).await;
        host(&state, "03", "organization-a", server).await;
        exec(&state, "04", "organization-a", Some(&app), None).await;
        exec(&state, "05", "organization-b", Some(&other_app), None).await;

        let ids =
            |rows: Vec<TerminalSessionDto>| rows.into_iter().map(|r| r.id).collect::<Vec<_>>();

        let app_rows = list_sessions(
            &state,
            "organization-a",
            SessionScope::Application(&app),
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            app_rows[0].actor_email.as_deref(),
            Some("editor@example.com")
        );
        assert_eq!(app_rows[0].kind, "container");
        assert_eq!(app_rows[0].duration_seconds, None);
        assert_eq!(ids(app_rows), ["04", "01"]);

        // Another org's application id never leaks across the organization bound.
        let foreign = list_sessions(
            &state,
            "organization-a",
            SessionScope::Application(&other_app),
            None,
            None,
        )
        .await
        .unwrap();
        assert!(foreign.is_empty());

        let with_host = SessionScope::Server {
            id: server,
            include_host: true,
        };
        let server_rows = list_sessions(&state, "organization-a", with_host, None, None)
            .await
            .unwrap();
        assert_eq!(server_rows[0].kind, "host");
        assert_eq!(server_rows[0].duration_seconds, Some(90));
        assert_eq!(server_rows[0].actor_token_name.as_deref(), Some("ci token"));
        assert_eq!(ids(server_rows), ["03", "02"]);

        let no_host = SessionScope::Server {
            id: server,
            include_host: false,
        };
        let rows = list_sessions(&state, "organization-a", no_host, None, None)
            .await
            .unwrap();
        assert_eq!(ids(rows), ["02"]);

        // Pagination: limit + keyset cursor; limit is clamped to at least 1.
        let page = list_sessions(
            &state,
            "organization-a",
            SessionScope::Application(&app),
            Some(1),
            None,
        )
        .await
        .unwrap();
        assert_eq!(ids(page), ["04"]);
        let next = list_sessions(
            &state,
            "organization-a",
            SessionScope::Application(&app),
            Some(0),
            Some("04"),
        )
        .await
        .unwrap();
        assert_eq!(ids(next), ["01"]);
        cleanup_database(&path);
    }
}
