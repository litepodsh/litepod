//! The scope types organization-scoped handlers receive.
//!
//! Every organization-scoped route used to depend on the author remembering two
//! lines: `require_organization_*` for the role, then `check_application` (or an
//! `*_for_organization` lookup) to tie the child id back to that organization.
//! Where someone forgot, nothing caught it — that omission is the whole of OWASP
//! A01 in this codebase, and the design that allows it is A04.
//!
//! The types below invert that. `OrgScope` and `AppScope` are only constructible
//! by the extractors in `security::authz::extract`, and those always run
//! membership, ownership and permission checks. A handler that takes one has
//! already been checked; a handler that doesn't take one has no
//! `organization_id` to misuse.
//!
//! # Using them
//!
//! ```ignore
//! pub async fn compose_content_get_handler(
//!     State(state): State<SharedState>,
//!     RequireApp(app, _): RequireApp<perm::AppRead>,
//! ) -> Result<impl IntoResponse, AppError> {
//!     let store = ComposeFileStore::new(&state.data_dir, &app.application_id);
//! ```
//!
//! Two rules:
//!
//! * These are `FromRequestParts` extractors, so they must come **before** any
//!   body extractor (`Json<T>`), which axum requires to be the last argument.
//!   The compiler enforces this.
//! * They read the path without consuming it, so a handler that needs ids these
//!   don't cover (`domain_id`, `container_name`, `exec_id`) still takes its own
//!   `Path<...>` alongside and ignores the leading segments.
//!
//! # Status codes
//!
//! Membership first (403 `NotOrganizationMember`), ownership second (404
//! `ProjectNotFound` / `EnvironmentNotFound` / `ApplicationNotFound`), permission
//! last (403 `InsufficientPermissions`). A missing child is deliberately a 404
//! rather than a 403, so the response doesn't confirm that the id exists.

use std::sync::Arc;

use axum::extract::{FromRequestParts, RawPathParams};
use axum::http::request::Parts;

use crate::dtos::response::AppError;
use crate::security::authz::PermissionSet;
use crate::security::middleware::auth::AuthUser;

/// Proof that `user` may act inside `organization_id`.
///
/// The guarantee is about routes, not about the type: the only way a *request*
/// produces one of these is through the extractors in `security::authz::extract`,
/// which always run the check. The fields are public because handlers have to
/// read them, and `crate::testing` builds one directly to exercise handler logic
/// without a router — that shortcut proves nothing about authorization, which is
/// why authorization is tested through a real router.
#[derive(Clone, Debug)]
pub struct OrgScope {
    pub organization_id: String,
    pub user: AuthUser,
    /// The caller's base role in the organization (`organization_members.role_id`;
    /// 0 is SysAdmin, 4 is Member). Decisions belong to `permissions`, not this.
    pub role_id: i64,
    /// What the caller may do in this organization: base role plus scoped
    /// assignments, capped by the token's scopes when authenticated by token.
    pub permissions: Arc<PermissionSet>,
}

/// Proof of [`OrgScope`] plus proof that the application in the path really
/// belongs to that organization.
#[derive(Clone, Debug)]
pub struct AppScope {
    pub organization_id: String,
    pub project_id: String,
    pub environment_id: String,
    pub application_id: String,
    /// Snapshot of the application's name at the time the scope was built —
    /// audit log rows use it as `target_name` so the user-visible trail stays
    /// meaningful even after the application is renamed or deleted.
    pub application_name: String,
    pub user: AuthUser,
    /// Mirrors [`OrgScope::role_id`].
    #[allow(dead_code)]
    pub role_id: i64,
    /// Mirrors [`OrgScope::permissions`].
    pub permissions: Arc<PermissionSet>,
}

/// Reads one path parameter by name instead of deserializing the whole tuple,
/// so this works for any route shape as long as the segment is named.
///
/// `RawPathParams` reads `parts.extensions` without taking anything out of it,
/// which is what lets the handler extract `Path<...>` afterwards.
pub(crate) async fn path_param(parts: &mut Parts, name: &str) -> Result<String, AppError> {
    // A route that doesn't declare this segment is a wiring mistake, not a
    // client error — 500, and never a silent pass.
    let missing = || AppError::InternalError(format!("route has no {name} path parameter"));
    let params = RawPathParams::from_request_parts(parts, &())
        .await
        .map_err(|_| missing())?;
    params
        .iter()
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value.to_string())
        .ok_or_else(missing)
}

#[cfg(test)]
mod tests {
    use axum::body::{Body, to_bytes};
    use axum::extract::Request;
    use axum::http::StatusCode;
    use axum::middleware::Next;
    use axum::routing::get;
    use axum::{Router, response::Response};
    use tower::ServiceExt;

    use super::*;
    use crate::security::authz::extract::{RequireApp, RequireOrg, perm};
    use crate::security::middleware::auth::TokenScopes;
    use crate::state::state::SharedState;
    use crate::testing::{
        auth_user, cleanup_database, seed_application, sysadmin_user, test_state,
    };

    // These were the legacy `OrgRead`/`OrgEdit`/`AppRead`/`AppEdit`/`AppDeploy`
    // tenancy tests. They now drive the `security::authz::extract` extractor
    // with the permission each legacy extractor stood for, so the same
    // guarantees stay pinned after the legacy code is gone.
    type OrgRead = RequireOrg<perm::OrgRead>;
    type OrgEdit = RequireOrg<perm::ProjectCreate>;
    type AppRead = RequireApp<perm::AppRead>;
    type AppEdit = RequireApp<perm::AppUpdate>;
    type AppDeploy = RequireApp<perm::AppDeploy>;

    /// A one-route router carrying the extractor under test, with `user` injected
    /// the way `auth_middleware` would inject it. Requests go through the real
    /// axum stack, so path matching and extractor ordering are exercised too.
    fn probe<E>(
        state: SharedState,
        user: AuthUser,
        path: &str,
        handler: fn(E) -> ProbeFuture,
    ) -> Router
    where
        E: FromRequestParts<SharedState> + Send + 'static,
    {
        Router::new()
            .route(path, get(handler))
            .layer(axum::middleware::from_fn(
                move |mut request: Request, next: Next| {
                    let user = user.clone();
                    async move {
                        request.extensions_mut().insert(user);
                        next.run(request).await
                    }
                },
            ))
            .with_state(state)
    }

    type ProbeFuture = std::pin::Pin<Box<dyn Future<Output = String> + Send>>;

    fn org_probe(RequireOrg(org, _): OrgRead) -> ProbeFuture {
        Box::pin(async move { format!("{}:{}", org.organization_id, org.role_id) })
    }
    fn org_edit_probe(RequireOrg(org, _): OrgEdit) -> ProbeFuture {
        Box::pin(async move { format!("{}:{}", org.organization_id, org.role_id) })
    }
    fn app_probe(RequireApp(app, _): AppRead) -> ProbeFuture {
        Box::pin(async move { app.application_id })
    }
    fn app_edit_probe(RequireApp(app, _): AppEdit) -> ProbeFuture {
        Box::pin(async move { app.application_id })
    }
    fn app_deploy_probe(RequireApp(app, _): AppDeploy) -> ProbeFuture {
        Box::pin(async move { app.application_id })
    }

    /// Same as [`probe`], but also injects `TokenScopes` — what
    /// `auth_middleware` does for a request authenticated by personal access
    /// token instead of the session cookie.
    fn probe_with_scopes<E>(
        state: SharedState,
        user: AuthUser,
        scopes: &[&str],
        path: &str,
        handler: fn(E) -> ProbeFuture,
    ) -> Router
    where
        E: FromRequestParts<SharedState> + Send + 'static,
    {
        let scopes = TokenScopes(scopes.iter().map(|s| s.to_string()).collect());
        Router::new()
            .route(path, get(handler))
            .layer(axum::middleware::from_fn(
                move |mut request: Request, next: Next| {
                    let user = user.clone();
                    let scopes = scopes.clone();
                    async move {
                        request.extensions_mut().insert(user);
                        request.extensions_mut().insert(scopes);
                        next.run(request).await
                    }
                },
            ))
            .with_state(state)
    }

    const ORG_PATH: &str = "/api/v1/organizations/{organization_id}/probe";
    const APP_PATH: &str = "/api/v1/organizations/{organization_id}/projects/{project_id}\
                            /environments/{environment_id}/applications/{application_id}/probe";

    async fn parts(response: Response) -> (StatusCode, String) {
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, String::from_utf8_lossy(&bytes).to_string())
    }

    fn get_request(uri: &str) -> Request<Body> {
        Request::builder().uri(uri).body(Body::empty()).unwrap()
    }

    #[tokio::test]
    async fn a_non_member_is_refused() {
        let (state, path) = test_state().await;
        let router = probe(state, auth_user("outsider"), ORG_PATH, org_probe);
        let (status, body) = parts(
            router
                .oneshot(get_request("/api/v1/organizations/organization-a/probe"))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(body.contains("Not a member of this organization"), "{body}");
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn a_member_of_another_organization_is_refused() {
        // `viewer` is a real member — of organization-a. Naming organization-b in
        // the URL must not carry that membership across.
        let (state, path) = test_state().await;
        let router = probe(state, auth_user("viewer"), ORG_PATH, org_probe);
        let (status, body) = parts(
            router
                .oneshot(get_request("/api/v1/organizations/organization-b/probe"))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(body.contains("Not a member of this organization"), "{body}");
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn a_viewer_is_refused_an_edit_scope() {
        let (state, path) = test_state().await;
        let router = probe(state, auth_user("viewer"), ORG_PATH, org_edit_probe);
        let (status, body) = parts(
            router
                .oneshot(get_request("/api/v1/organizations/organization-a/probe"))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(body.contains("Insufficient permissions"), "{body}");
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn a_viewer_passes_a_read_scope_and_keeps_its_role() {
        let (state, path) = test_state().await;
        let router = probe(state, auth_user("viewer"), ORG_PATH, org_probe);
        let (status, body) = parts(
            router
                .oneshot(get_request("/api/v1/organizations/organization-a/probe"))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, "organization-a:3");
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn a_sysadmin_outranks_membership() {
        let (state, path) = test_state().await;
        let router = probe(state, sysadmin_user("outsider"), ORG_PATH, org_edit_probe);
        let (status, body) = parts(
            router
                .oneshot(get_request("/api/v1/organizations/organization-b/probe"))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body, "organization-b:0",
            "sysadmin should resolve to role 0"
        );
        cleanup_database(&path);
    }

    /// Locks down a past bug: the check used to read `is_sysadmin && !is_cloud`,
    /// so on cloud a SysAdmin could not reach an organization it wasn't a member
    /// of. `organization_role` has no edition split now; this makes sure one does
    /// not come back.
    #[tokio::test]
    async fn a_sysadmin_outranks_membership_on_cloud_too() {
        let (state, path) = crate::testing::test_state_for(true).await;
        let router = probe(state, sysadmin_user("outsider"), ORG_PATH, org_edit_probe);
        let (status, body) = parts(
            router
                .oneshot(get_request("/api/v1/organizations/organization-a/probe"))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, "organization-a:0");
        cleanup_database(&path);
    }

    /// On cloud the `local` server is the control plane host: only a SysAdmin may
    /// reach it, any other member gets a 404. Remote servers and self-hosted are untouched.
    #[tokio::test]
    async fn cloud_local_server_routes_are_sysadmin_only() {
        const SERVER_PATH: &str = "/api/v1/organizations/{organization_id}/servers/{server_id}";
        let status = |cloud: bool, user: AuthUser, server_id: &'static str| async move {
            let (state, path) = crate::testing::test_state_for(cloud).await;
            let router = probe(state, user, SERVER_PATH, org_probe);
            let response = router
                .oneshot(get_request(&format!(
                    "/api/v1/organizations/organization-a/servers/{server_id}"
                )))
                .await
                .unwrap();
            cleanup_database(&path);
            response.status()
        };
        assert_eq!(
            status(true, auth_user("viewer"), "local").await,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            status(true, sysadmin_user("outsider"), "local").await,
            StatusCode::OK
        );
        assert_eq!(
            status(true, auth_user("viewer"), "ssh-a").await,
            StatusCode::OK
        );
        assert_eq!(
            status(false, auth_user("viewer"), "local").await,
            StatusCode::OK
        );
    }

    /// On self-hosted the `local` server is shared by every organization, so its
    /// host-wide routes (console, exec, volumes, Caddy, maintenance) are
    /// SysAdmin-only too. Only the read-only allowlist stays open to members.
    #[tokio::test]
    async fn self_hosted_local_server_host_routes_are_sysadmin_only() {
        let status = |route: &'static str, user: AuthUser, server_id: &'static str| async move {
            let (state, path) = crate::testing::test_state_for(false).await;
            let router = probe(
                state,
                user,
                &format!("/api/v1/organizations/{{organization_id}}/servers/{{server_id}}{route}"),
                org_probe,
            );
            let response = router
                .oneshot(get_request(&format!(
                    "/api/v1/organizations/organization-a/servers/{server_id}{route}"
                )))
                .await
                .unwrap();
            cleanup_database(&path);
            response.status()
        };
        for route in [
            "/podman/overview",
            "/podman/resources/containers",
            "/podman/volume/data/workspace",
            "/caddy/config",
            "/terminal-sessions",
        ] {
            assert_eq!(
                status(route, auth_user("viewer"), "local").await,
                StatusCode::NOT_FOUND,
                "{route}"
            );
            assert_eq!(
                status(route, sysadmin_user("outsider"), "local").await,
                StatusCode::OK,
                "{route}"
            );
            assert_eq!(
                status(route, auth_user("viewer"), "ssh-a").await,
                StatusCode::OK,
                "{route}"
            );
        }
        for route in ["", "/applications", "/ports", "/podman/metrics/history"] {
            assert_eq!(
                status(route, auth_user("viewer"), "local").await,
                StatusCode::OK,
                "{route}"
            );
        }
    }

    #[tokio::test]
    async fn an_application_from_another_organization_is_not_found() {
        // The whole shape of the seven critical bugs: a real role in the
        // organization the caller named, combined with a child id from elsewhere.
        // 404 rather than 403 on purpose — the answer must not confirm the id exists.
        let (state, path) = test_state().await;
        let (_, _, foreign_application) = seed_application(&state, "organization-b").await;
        let router = probe(state, auth_user("viewer"), APP_PATH, app_probe);
        let uri = format!(
            "/api/v1/organizations/organization-a/projects/organization-b-project\
             /environments/organization-b-environment/applications/{foreign_application}/probe"
        );
        let (status, body) = parts(router.oneshot(get_request(&uri)).await.unwrap()).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(body.contains("Application not found"), "{body}");
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn an_application_in_the_callers_organization_is_reachable() {
        let (state, path) = test_state().await;
        let (project, environment, application) = seed_application(&state, "organization-a").await;
        let router = probe(state, auth_user("viewer"), APP_PATH, app_probe);
        let uri = format!(
            "/api/v1/organizations/organization-a/projects/{project}\
             /environments/{environment}/applications/{application}/probe"
        );
        let (status, body) = parts(router.oneshot(get_request(&uri)).await.unwrap()).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, application);
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn a_route_without_an_organization_segment_fails_loudly() {
        // Wiring mistake, not a client error: it must be a 500, never a silent pass.
        let (state, path) = test_state().await;
        let router = probe(state, auth_user("editor"), "/api/v1/probe", org_probe);
        let (status, _) = parts(router.oneshot(get_request("/api/v1/probe")).await.unwrap()).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        cleanup_database(&path);
    }

    // ── Personal access token scopes ────────────────────────────────────────

    #[tokio::test]
    async fn a_read_scoped_token_cannot_reach_an_edit_route() {
        // `editor` really is Editor here — the token caps it down to `read`.
        let (state, path) = test_state().await;
        let router = probe_with_scopes(
            state,
            auth_user("editor"),
            &["read"],
            ORG_PATH,
            org_edit_probe,
        );
        let (status, _) = parts(
            router
                .oneshot(get_request("/api/v1/organizations/organization-a/probe"))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn a_write_scoped_token_reaches_an_edit_route() {
        let (state, path) = test_state().await;
        let router = probe_with_scopes(
            state,
            auth_user("editor"),
            &["write"],
            ORG_PATH,
            org_edit_probe,
        );
        let (status, _) = parts(
            router
                .oneshot(get_request("/api/v1/organizations/organization-a/probe"))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn a_deploy_scoped_token_can_deploy_but_not_edit() {
        let (state, path) = test_state().await;
        let (project, environment, application) = seed_application(&state, "organization-a").await;
        let uri = format!(
            "/api/v1/organizations/organization-a/projects/{project}\
             /environments/{environment}/applications/{application}/probe"
        );

        let deploy_router = probe_with_scopes(
            state.clone(),
            auth_user("editor"),
            &["deploy"],
            APP_PATH,
            app_deploy_probe,
        );
        let (status, _) = parts(deploy_router.oneshot(get_request(&uri)).await.unwrap()).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "deploy scope should reach AppDeploy"
        );

        let edit_router = probe_with_scopes(
            state,
            auth_user("editor"),
            &["deploy"],
            APP_PATH,
            app_edit_probe,
        );
        let (status, _) = parts(edit_router.oneshot(get_request(&uri)).await.unwrap()).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "deploy scope alone should not reach AppEdit"
        );

        cleanup_database(&path);
    }

    #[tokio::test]
    async fn losing_organization_membership_stops_a_token_even_with_full_scope() {
        // A token's scope only narrows access — it never substitutes for
        // membership. `outsider` has none in organization-a, so a `write`
        // scope still gets refused for the same reason a session would.
        let (state, path) = test_state().await;
        let router = probe_with_scopes(
            state,
            auth_user("outsider"),
            &["read", "write", "deploy"],
            ORG_PATH,
            org_probe,
        );
        let (status, _) = parts(
            router
                .oneshot(get_request("/api/v1/organizations/organization-a/probe"))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        cleanup_database(&path);
    }
}
