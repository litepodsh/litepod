//! Read-only terminal session history. Gated by the same permission that
//! opens the terminal: `app.exec` for an application, `server.terminal` for a
//! server (host shell rows on a `local` server only for whoever may open it).

use axum::{
    extract::{Path, Query, State},
    response::IntoResponse,
};
use serde::Deserialize;

use crate::cqrs::queries::terminal_session::{SessionScope, list_sessions};
use crate::dtos::response::{AppError, ok};
use crate::security::authz::extract::{RequireApp, RequireOrg, perm};
use crate::state::state::SharedState;

#[derive(Deserialize)]
pub struct TerminalSessionsQuery {
    /// Default 20, clamped to 1..=100.
    pub limit: Option<i64>,
    /// Keyset cursor: the `id` of the last session from the previous page.
    pub before: Option<String>,
}

#[utoipa::path(
    get,
    path = "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/terminal-sessions",
    responses((status = 200, description = "Container exec sessions, newest first")),
    tag = "applications"
)]
pub async fn list_application_sessions_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppExec>,
    Query(q): Query<TerminalSessionsQuery>,
) -> Result<impl IntoResponse, AppError> {
    let sessions = list_sessions(
        &state,
        &app.organization_id,
        SessionScope::Application(&app.application_id),
        q.limit,
        q.before.as_deref(),
    )
    .await?;
    Ok(ok(sessions))
}

#[utoipa::path(
    get,
    path = "/api/v1/organizations/{organization_id}/servers/{server_id}/terminal-sessions",
    responses((status = 200, description = "Host shell and container exec sessions, newest first")),
    tag = "servers"
)]
pub async fn list_server_sessions_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerTerminal>,
    Path((_organization_id, server_id)): Path<(String, String)>,
    Query(q): Query<TerminalSessionsQuery>,
) -> Result<impl IntoResponse, AppError> {
    let server =
        crate::servers::repository::get_for_server_route(&state, &server_id, &org.organization_id)
            .await?
            .ok_or(AppError::ResourceNotFound)?;
    // Mirrors `podman::terminal::host::authorize_terminal`.
    let include_host = server.kind != "local" || org.user.is_sysadmin;
    let sessions = list_sessions(
        &state,
        &org.organization_id,
        SessionScope::Server {
            id: &server.id,
            include_host,
        },
        q.limit,
        q.before.as_deref(),
    )
    .await?;
    Ok(ok(sessions))
}

#[cfg(test)]
mod tests {
    use axum::body::{Body, to_bytes};
    use axum::extract::Request;
    use axum::http::StatusCode;
    use axum::middleware::Next;
    use axum::{Router, routing::get};
    use tower::ServiceExt;

    use super::*;
    use crate::testing::{
        auth_user, cleanup_database, seed_application, set_base_role, test_state,
    };

    const APP: &str = "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/terminal-sessions";
    const SERVER: &str =
        "/api/v1/organizations/{organization_id}/servers/{server_id}/terminal-sessions";

    async fn status(state: &SharedState, user: &str, uri: &str) -> StatusCode {
        let user = auth_user(user);
        let router = Router::new()
            .route(APP, get(list_application_sessions_handler))
            .route(SERVER, get(list_server_sessions_handler))
            .layer(axum::middleware::from_fn(
                move |mut req: Request, next: Next| {
                    let user = user.clone();
                    async move {
                        req.extensions_mut().insert(user);
                        next.run(req).await
                    }
                },
            ))
            .with_state(state.clone());
        let response = router
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        to_bytes(response.into_body(), usize::MAX).await.unwrap();
        status
    }

    #[tokio::test]
    async fn history_needs_the_permission_that_opens_the_terminal() {
        let (state, path) = test_state().await;
        let (p, e, a) = seed_application(&state, "organization-a").await;
        let app_uri = format!(
            "/api/v1/organizations/organization-a/projects/{p}/environments/{e}/applications/{a}/terminal-sessions"
        );
        let server_uri =
            "/api/v1/organizations/organization-a/servers/organization-a-server/terminal-sessions";

        // Editor: no app.exec, no server.terminal.
        assert_eq!(
            status(&state, "editor", &app_uri).await,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            status(&state, "editor", server_uri).await,
            StatusCode::FORBIDDEN
        );

        set_base_role(&state, "organization-a", "editor", 1).await;
        assert_eq!(status(&state, "editor", &app_uri).await, StatusCode::OK);
        assert_eq!(status(&state, "editor", server_uri).await, StatusCode::OK);
        // A server of another organization is not found, not listed.
        let foreign =
            "/api/v1/organizations/organization-a/servers/organization-b-server/terminal-sessions";
        seed_application(&state, "organization-b").await;
        assert_eq!(
            status(&state, "editor", foreign).await,
            StatusCode::NOT_FOUND
        );
        cleanup_database(&path);
    }
}
