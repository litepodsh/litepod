//! Permission extractors. Same guarantee as `middleware::org_scope`: a scope is
//! only constructible here, and constructing it always runs membership,
//! ownership and permission checks, in that order.

use std::marker::PhantomData;
use std::sync::Arc;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;

use super::Permission;
use super::set::{PermissionSet, Target, load};
use crate::dtos::response::AppError;
use crate::handler::common::{check_application, organization_role};
use crate::security::middleware::auth::{AuthUser, TokenScopes};
use crate::security::middleware::org_scope::{AppScope, OrgScope, path_param};
use crate::state::state::SharedState;

pub trait Perm: Send + Sync + 'static {
    const PERMISSION: Permission;
    /// Also require every permission of this system role at org scope. For
    /// features that act on the caller's behalf across the organization, so a
    /// narrow custom grant can't be widened through them.
    const ALSO_COVERS_ROLE: Option<i64> = None;
}

pub mod perm {
    use super::{Perm, Permission};
    macro_rules! markers {
        ($($name:ident),* $(,)?) => {
            $(
                // Some markers (org.roles/sso/tokens.manage) are only taken by
                // routes from the access-control and SSO specs, not built yet.
                #[allow(dead_code)]
                pub struct $name;
                impl Perm for $name {
                    const PERMISSION: Permission = Permission::$name;
                }
            )*
        };
    }
    markers!(
        OrgRead,
        OrgUpdate,
        OrgMembersManage,
        OrgRolesManage,
        OrgSsoManage,
        OrgAuditRead,
        OrgTokensManage,
        ProjectRead,
        ProjectCreate,
        ProjectUpdate,
        ProjectDelete,
        EnvCreate,
        EnvUpdate,
        EnvDelete,
        AppRead,
        AppCreate,
        AppUpdate,
        AppDelete,
        AppDeploy,
        AppLogsRead,
        AppExec,
        AppEnvRead,
        AppEnvUpdate,
        AppDomainsManage,
        BackupRead,
        BackupManage,
        BackupRestore,
        BackupTargetsManage,
        ServerRead,
        ServerManage,
        ServerTerminal,
        GitProviderManage,
        NotificationManage,
        ScheduledTaskManage,
    );

    /// The assistant deploys, edits and reads on the caller's behalf anywhere in
    /// the organization, so using it needs Editor-level access org-wide — what
    /// it needed before RBAC (`OrgEdit`). Granting `assistant.use` alone in a
    /// custom role is not enough.
    pub struct AssistantUse;
    impl Perm for AssistantUse {
        const PERMISSION: Permission = Permission::AssistantUse;
        const ALSO_COVERS_ROLE: Option<i64> = Some(2);
    }
}

#[derive(Clone, Debug)]
pub struct ProjectScope {
    pub organization_id: String,
    pub project_id: String,
    pub user: AuthUser,
    pub permissions: Arc<PermissionSet>,
}

#[derive(Clone, Debug)]
#[allow(dead_code)] // `permissions` is for handlers that branch further; none does yet.
pub struct EnvScope {
    pub organization_id: String,
    pub project_id: String,
    pub environment_id: String,
    pub user: AuthUser,
    pub permissions: Arc<PermissionSet>,
}

pub struct RequireOrg<P>(pub OrgScope, pub PhantomData<P>);
pub struct RequireProject<P>(pub ProjectScope, pub PhantomData<P>);
pub struct RequireEnv<P>(pub EnvScope, pub PhantomData<P>);
pub struct RequireApp<P>(pub AppScope, pub PhantomData<P>);

/// Read-only `/servers/local/...` routes a self-hosted non-SysAdmin may still
/// reach: the server card, the caller's own applications and ports on it, and
/// host-level metrics. Each handler filters to the caller's organization or
/// returns host-wide numbers only; nothing here can reach a container, volume
/// or Caddy route. Deny by default: a new local route stays SysAdmin-only until
/// it is listed here.
fn shared_local_read(parts: &Parts) -> bool {
    const SUFFIXES: [&str; 4] = [
        "/servers/local",
        "/servers/local/applications",
        "/servers/local/ports",
        "/servers/local/podman/metrics/history",
    ];
    parts.method == axum::http::Method::GET
        && SUFFIXES
            .iter()
            .any(|suffix| parts.uri.path().ends_with(suffix))
}

/// Membership + permission set. Never checks a permission itself.
async fn base(parts: &mut Parts, state: &SharedState) -> Result<OrgScope, AppError> {
    // `auth_middleware` puts this here. Absent means the route was wired outside
    // the authenticated group.
    let user = parts
        .extensions
        .get::<AuthUser>()
        .cloned()
        .ok_or(AppError::InvalidCredentials)?;
    let organization_id = path_param(parts, "organization_id").await?;
    // Not a member → 403 NotOrganizationMember, before anything else.
    let role_id = organization_role(state, &organization_id, &user).await?;
    // An organization that requires 2FA refuses a member who has none. After the
    // membership check so a stranger learns nothing about the organization's
    // settings; the SysAdmin is the break-glass and is exempt. Every
    // organization route comes through here, so password, SSO, OAuth and token
    // callers are all covered. Account routes are not organization-scoped,
    // which is what keeps 2FA setup reachable for the refused member.
    if !user.is_sysadmin
        && crate::git::repositories::organization::requires_two_factor(state, &organization_id)
            .await
            .map_err(AppError::DatabaseError)?
        && !crate::git::repositories::account::is_totp_enabled(state, &user.user_id).await?
    {
        return Err(AppError::TwoFactorRequired);
    }
    // The `local` server is the control plane's own host on cloud, and on
    // self-hosted it is shared by every organization (`organization_id IS NULL`):
    // its console, exec, volume, Caddy and maintenance routes act on the whole
    // host, across tenants. Every `/servers/{server_id}/...` route is a 404 for it
    // unless the caller is the SysAdmin (database role plus
    // `CLOUD_SYSADMIN_USER_ID` when set). Self-hosted keeps the few read-only
    // routes the UI needs to show apps placed on it — see `shared_local_read`.
    if !user.is_sysadmin
        && path_param(parts, "server_id").await.ok().as_deref() == Some("local")
        && (state.is_cloud || !shared_local_read(parts))
    {
        return Err(AppError::ResourceNotFound);
    }
    let mut set = load(state, &user, &organization_id, role_id).await?;
    if let Some(TokenScopes(scopes)) = parts.extensions.get::<TokenScopes>() {
        set.cap_to_token(scopes);
    }
    Ok(OrgScope {
        organization_id,
        user,
        role_id,
        permissions: Arc::new(set),
    })
}

fn require(set: &PermissionSet, p: Permission, target: Target) -> Result<(), AppError> {
    if set.allows(p, target) {
        Ok(())
    } else {
        Err(AppError::InsufficientPermissions)
    }
}

async fn project_of(
    parts: &mut Parts,
    state: &SharedState,
    org: &OrgScope,
) -> Result<String, AppError> {
    let project_id = path_param(parts, "project_id").await?;
    crate::git::repositories::project::get_project(state, &org.organization_id, &project_id)
        .await
        .map_err(AppError::DatabaseError)?
        .ok_or(AppError::ProjectNotFound)?;
    Ok(project_id)
}

async fn environment_of(
    parts: &mut Parts,
    state: &SharedState,
    org: &OrgScope,
    project_id: &str,
) -> Result<String, AppError> {
    let environment_id = path_param(parts, "environment_id").await?;
    crate::git::repositories::project::get_environment(
        state,
        &org.organization_id,
        project_id,
        &environment_id,
    )
    .await
    .map_err(AppError::DatabaseError)?
    .ok_or(AppError::EnvironmentNotFound)?;
    Ok(environment_id)
}

impl<P: Perm> FromRequestParts<SharedState> for RequireOrg<P> {
    type Rejection = AppError;
    async fn from_request_parts(parts: &mut Parts, state: &SharedState) -> Result<Self, AppError> {
        let org = base(parts, state).await?;
        require(&org.permissions, P::PERMISSION, Target::Org)?;
        if let Some(role_id) = P::ALSO_COVERS_ROLE
            && !org.permissions.covers_role_at_org(role_id)
        {
            return Err(AppError::InsufficientPermissions);
        }
        Ok(Self(org, PhantomData))
    }
}

impl<P: Perm> FromRequestParts<SharedState> for RequireProject<P> {
    type Rejection = AppError;
    async fn from_request_parts(parts: &mut Parts, state: &SharedState) -> Result<Self, AppError> {
        let org = base(parts, state).await?;
        let project_id = project_of(parts, state, &org).await?;
        require(
            &org.permissions,
            P::PERMISSION,
            Target::Project(&project_id),
        )?;
        Ok(Self(
            ProjectScope {
                organization_id: org.organization_id,
                project_id,
                user: org.user,
                permissions: org.permissions,
            },
            PhantomData,
        ))
    }
}

impl<P: Perm> FromRequestParts<SharedState> for RequireEnv<P> {
    type Rejection = AppError;
    async fn from_request_parts(parts: &mut Parts, state: &SharedState) -> Result<Self, AppError> {
        let org = base(parts, state).await?;
        let project_id = project_of(parts, state, &org).await?;
        let environment_id = environment_of(parts, state, &org, &project_id).await?;
        require(
            &org.permissions,
            P::PERMISSION,
            Target::Environment {
                project_id: &project_id,
                environment_id: &environment_id,
            },
        )?;
        Ok(Self(
            EnvScope {
                organization_id: org.organization_id,
                project_id,
                environment_id,
                user: org.user,
                permissions: org.permissions,
            },
            PhantomData,
        ))
    }
}

impl<P: Perm> FromRequestParts<SharedState> for RequireApp<P> {
    type Rejection = AppError;
    async fn from_request_parts(parts: &mut Parts, state: &SharedState) -> Result<Self, AppError> {
        let org = base(parts, state).await?;
        let project_id = path_param(parts, "project_id").await?;
        let environment_id = path_param(parts, "environment_id").await?;
        let application_id = path_param(parts, "application_id").await?;
        // Four-id join: a foreign application is a 404, never a 403.
        let application = check_application(
            state,
            &org.organization_id,
            &project_id,
            &environment_id,
            &application_id,
        )
        .await?;
        require(
            &org.permissions,
            P::PERMISSION,
            Target::Environment {
                project_id: &project_id,
                environment_id: &environment_id,
            },
        )?;
        Ok(Self(
            AppScope {
                organization_id: org.organization_id,
                project_id,
                environment_id,
                application_id,
                application_name: application.name,
                user: org.user,
                role_id: org.role_id,
                permissions: org.permissions,
            },
            PhantomData,
        ))
    }
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
    use crate::testing::{
        assign, auth_user, cleanup_database, seed_application, set_base_role, test_state,
    };

    type ProbeFuture = std::pin::Pin<Box<dyn Future<Output = String> + Send>>;

    /// A one-route router carrying the extractor under test, with `user` (and
    /// optionally token scopes) injected the way `auth_middleware` would.
    fn probe_with<E>(
        state: SharedState,
        user: AuthUser,
        scopes: Option<&[&str]>,
        path: &str,
        handler: fn(E) -> ProbeFuture,
    ) -> Router
    where
        E: FromRequestParts<SharedState> + Send + 'static,
    {
        let scopes = scopes.map(|s| TokenScopes(s.iter().map(|s| s.to_string()).collect()));
        Router::new()
            .route(path, get(handler))
            .layer(axum::middleware::from_fn(
                move |mut request: Request, next: Next| {
                    let user = user.clone();
                    let scopes = scopes.clone();
                    async move {
                        request.extensions_mut().insert(user);
                        if let Some(scopes) = scopes {
                            request.extensions_mut().insert(scopes);
                        }
                        next.run(request).await
                    }
                },
            ))
            .with_state(state)
    }

    fn probe<E>(
        state: SharedState,
        user: AuthUser,
        path: &str,
        handler: fn(E) -> ProbeFuture,
    ) -> Router
    where
        E: FromRequestParts<SharedState> + Send + 'static,
    {
        probe_with(state, user, None, path, handler)
    }

    const ORG_PATH: &str = "/api/v1/organizations/{organization_id}/probe";
    const PROJECT_PATH: &str =
        "/api/v1/organizations/{organization_id}/projects/{project_id}/probe";
    const APP_PATH: &str = "/api/v1/organizations/{organization_id}/projects/{project_id}\
                            /environments/{environment_id}/applications/{application_id}/probe";

    async fn status_of(router: Router, uri: &str) -> (StatusCode, String) {
        let response: Response = router
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, String::from_utf8_lossy(&bytes).to_string())
    }

    fn app_uri(org: &str, p: &str, e: &str, a: &str) -> String {
        format!("/api/v1/organizations/{org}/projects/{p}/environments/{e}/applications/{a}/probe")
    }

    fn deploy_probe(RequireApp(app, _): RequireApp<perm::AppDeploy>) -> ProbeFuture {
        Box::pin(async move { app.application_id })
    }
    fn exec_probe(RequireApp(app, _): RequireApp<perm::AppExec>) -> ProbeFuture {
        Box::pin(async move { app.application_id })
    }
    fn project_update_probe(
        RequireProject(p, _): RequireProject<perm::ProjectUpdate>,
    ) -> ProbeFuture {
        Box::pin(async move { p.project_id })
    }
    fn org_read_probe(RequireOrg(o, _): RequireOrg<perm::OrgRead>) -> ProbeFuture {
        Box::pin(async move { o.organization_id })
    }
    fn server_manage_probe(RequireOrg(o, _): RequireOrg<perm::ServerManage>) -> ProbeFuture {
        Box::pin(async move { o.organization_id })
    }
    fn members_manage_probe(RequireOrg(o, _): RequireOrg<perm::OrgMembersManage>) -> ProbeFuture {
        Box::pin(async move { o.organization_id })
    }

    #[tokio::test]
    async fn editor_base_role_deploys_but_cannot_exec() {
        let (state, path) = test_state().await;
        let (p, e, a) = seed_application(&state, "organization-a").await;
        let uri = app_uri("organization-a", &p, &e, &a);
        let ok = probe(state.clone(), auth_user("editor"), APP_PATH, deploy_probe);
        assert_eq!(status_of(ok, &uri).await.0, StatusCode::OK);
        let no = probe(state, auth_user("editor"), APP_PATH, exec_probe);
        assert_eq!(status_of(no, &uri).await.0, StatusCode::FORBIDDEN);
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn a_member_with_a_project_grant_deploys_only_there() {
        let (state, path) = test_state().await;
        let (p, e, a) = seed_application(&state, "organization-a").await;
        set_base_role(&state, "organization-a", "viewer", 4).await;
        assign(&state, "organization-a", "viewer", 2, "project", &p).await;

        let router = probe(state.clone(), auth_user("viewer"), APP_PATH, deploy_probe);
        let uri = app_uri("organization-a", &p, &e, &a);
        assert_eq!(status_of(router, &uri).await.0, StatusCode::OK);

        // Second project in the same org: no grant there.
        sqlx::query(
            "INSERT INTO projects (id, organization_id, name, created_at, updated_at) VALUES ('p2', 'organization-a', 'p2', 0, 0)",
        )
        .execute(&state.db)
        .await
        .unwrap();
        let router = probe(
            state,
            auth_user("viewer"),
            PROJECT_PATH,
            project_update_probe,
        );
        let (status, _) = status_of(
            router,
            "/api/v1/organizations/organization-a/projects/p2/probe",
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn a_foreign_project_is_not_found_before_permission_is_checked() {
        let (state, path) = test_state().await;
        let (foreign, _, _) = seed_application(&state, "organization-b").await;
        let router = probe(
            state,
            auth_user("viewer"),
            PROJECT_PATH,
            project_update_probe,
        );
        let uri = format!("/api/v1/organizations/organization-a/projects/{foreign}/probe");
        assert_eq!(status_of(router, &uri).await.0, StatusCode::NOT_FOUND);
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn a_non_member_is_refused_with_the_membership_error() {
        let (state, path) = test_state().await;
        let router = probe(state, auth_user("outsider"), ORG_PATH, org_read_probe);
        let (status, body) = status_of(router, "/api/v1/organizations/organization-a/probe").await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(body.contains("Not a member of this organization"), "{body}");
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn a_deploy_token_reaches_deploy_but_not_exec_even_for_admin_grant() {
        let (state, path) = test_state().await;
        let (p, e, a) = seed_application(&state, "organization-a").await;
        assign(&state, "organization-a", "editor", 1, "project", &p).await;
        let uri = app_uri("organization-a", &p, &e, &a);
        let ok = probe_with(
            state.clone(),
            auth_user("editor"),
            Some(&["deploy"]),
            APP_PATH,
            deploy_probe,
        );
        assert_eq!(status_of(ok, &uri).await.0, StatusCode::OK);
        let no = probe_with(
            state,
            auth_user("editor"),
            Some(&["read", "write", "deploy"]),
            APP_PATH,
            exec_probe,
        );
        assert_eq!(status_of(no, &uri).await.0, StatusCode::FORBIDDEN);
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn a_project_admin_grant_opens_exec_but_project_editor_does_not() {
        let (state, path) = test_state().await;
        let (p, e, a) = seed_application(&state, "organization-a").await;
        set_base_role(&state, "organization-a", "viewer", 4).await;
        assign(&state, "organization-a", "viewer", 2, "project", &p).await;
        let uri = app_uri("organization-a", &p, &e, &a);
        let no = probe(state.clone(), auth_user("viewer"), APP_PATH, exec_probe);
        assert_eq!(status_of(no, &uri).await.0, StatusCode::FORBIDDEN);
        assign(&state, "organization-a", "viewer", 1, "environment", &e).await;
        let ok = probe(state, auth_user("viewer"), APP_PATH, exec_probe);
        assert_eq!(status_of(ok, &uri).await.0, StatusCode::OK);
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn a_project_admin_cannot_manage_servers() {
        let (state, path) = test_state().await;
        let (p, _, _) = seed_application(&state, "organization-a").await;
        set_base_role(&state, "organization-a", "viewer", 4).await;
        assign(&state, "organization-a", "viewer", 1, "project", &p).await;
        let router = probe(state, auth_user("viewer"), ORG_PATH, server_manage_probe);
        let (status, _) = status_of(router, "/api/v1/organizations/organization-a/probe").await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn a_custom_org_role_can_manage_members_but_an_editor_cannot() {
        let (state, path) = test_state().await;
        let uri = "/api/v1/organizations/organization-a/probe";

        let editor = probe(
            state.clone(),
            auth_user("editor"),
            ORG_PATH,
            members_manage_probe,
        );
        assert_eq!(status_of(editor, uri).await.0, StatusCode::FORBIDDEN);

        set_base_role(&state, "organization-a", "viewer", 4).await;
        for sql in [
            "INSERT INTO rbac_roles (id, organization_id, name) VALUES ('hr', 'organization-a', 'people-ops')",
            "INSERT INTO rbac_role_permissions (role_id, permission) VALUES ('hr', 'org.members.manage')",
            "INSERT INTO rbac_assignments (id, organization_id, principal_type, principal_id, custom_role_id, scope_type, scope_id) \
             VALUES ('a', 'organization-a', 'user', 'viewer', 'hr', 'org', 'organization-a')",
        ] {
            sqlx::query(sql).execute(&state.db).await.unwrap();
        }
        let custom = probe(state, auth_user("viewer"), ORG_PATH, members_manage_probe);
        assert_eq!(status_of(custom, uri).await.0, StatusCode::OK);
        cleanup_database(&path);
    }

    fn assistant_probe(RequireOrg(o, _): RequireOrg<perm::AssistantUse>) -> ProbeFuture {
        Box::pin(async move { o.organization_id })
    }

    #[tokio::test]
    async fn assistant_use_alone_cannot_widen_a_narrow_grant() {
        let (state, path) = test_state().await;
        let uri = "/api/v1/organizations/organization-a/probe";
        set_base_role(&state, "organization-a", "viewer", 4).await;
        for sql in [
            "INSERT INTO rbac_roles (id, organization_id, name) VALUES ('ai', 'organization-a', 'ai-only')",
            "INSERT INTO rbac_role_permissions (role_id, permission) VALUES ('ai', 'assistant.use')",
            "INSERT INTO rbac_assignments (id, organization_id, principal_type, principal_id, custom_role_id, scope_type, scope_id) \
             VALUES ('a', 'organization-a', 'user', 'viewer', 'ai', 'org', 'organization-a')",
        ] {
            sqlx::query(sql).execute(&state.db).await.unwrap();
        }
        let narrow = probe(
            state.clone(),
            auth_user("viewer"),
            ORG_PATH,
            assistant_probe,
        );
        assert_eq!(status_of(narrow, uri).await.0, StatusCode::FORBIDDEN);

        let editor = probe(state, auth_user("editor"), ORG_PATH, assistant_probe);
        assert_eq!(status_of(editor, uri).await.0, StatusCode::OK);
        cleanup_database(&path);
    }

    #[test]
    fn there_is_one_marker_per_permission() {
        // If this fails, a Permission variant was added without a perm:: marker.
        use perm::*;
        let markers = [
            OrgRead::PERMISSION,
            OrgUpdate::PERMISSION,
            OrgMembersManage::PERMISSION,
            OrgRolesManage::PERMISSION,
            OrgSsoManage::PERMISSION,
            OrgAuditRead::PERMISSION,
            OrgTokensManage::PERMISSION,
            AssistantUse::PERMISSION,
            ProjectRead::PERMISSION,
            ProjectCreate::PERMISSION,
            ProjectUpdate::PERMISSION,
            ProjectDelete::PERMISSION,
            EnvCreate::PERMISSION,
            EnvUpdate::PERMISSION,
            EnvDelete::PERMISSION,
            AppRead::PERMISSION,
            AppCreate::PERMISSION,
            AppUpdate::PERMISSION,
            AppDelete::PERMISSION,
            AppDeploy::PERMISSION,
            AppLogsRead::PERMISSION,
            AppExec::PERMISSION,
            AppEnvRead::PERMISSION,
            AppEnvUpdate::PERMISSION,
            AppDomainsManage::PERMISSION,
            BackupRead::PERMISSION,
            BackupManage::PERMISSION,
            BackupRestore::PERMISSION,
            BackupTargetsManage::PERMISSION,
            ServerRead::PERMISSION,
            ServerManage::PERMISSION,
            ServerTerminal::PERMISSION,
            GitProviderManage::PERMISSION,
            NotificationManage::PERMISSION,
            ScheduledTaskManage::PERMISSION,
        ];
        let distinct: std::collections::BTreeSet<_> = markers.iter().collect();
        assert_eq!(distinct.len(), Permission::ALL.len());
    }

    // ── Organization-wide 2FA requirement ───────────────────────────────────

    async fn require_two_factor(state: &SharedState, organization_id: &str) {
        crate::git::repositories::organization::set_require_two_factor(
            state,
            organization_id,
            true,
        )
        .await
        .unwrap();
    }

    async fn enable_totp(state: &SharedState, user_id: &str) {
        sqlx::query("UPDATE users SET totp_enabled = 1 WHERE id = ?1")
            .bind(user_id)
            .execute(&state.db)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn a_member_without_2fa_is_refused_when_the_organization_requires_it() {
        let (state, path) = test_state().await;
        require_two_factor(&state, "organization-a").await;

        let router = probe(state, auth_user("viewer"), ORG_PATH, org_read_probe);
        let (status, body) = status_of(router, "/api/v1/organizations/organization-a/probe").await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(body.contains("two-factor"), "{body}");
        assert!(body.contains(crate::dtos::response::TWO_FACTOR_REQUIRED_CODE), "{body}");
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn a_member_with_2fa_passes_when_the_organization_requires_it() {
        let (state, path) = test_state().await;
        require_two_factor(&state, "organization-a").await;
        enable_totp(&state, "viewer").await;

        let router = probe(state, auth_user("viewer"), ORG_PATH, org_read_probe);
        let (status, _) = status_of(router, "/api/v1/organizations/organization-a/probe").await;
        assert_eq!(status, StatusCode::OK);
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn the_requirement_binds_only_the_organization_that_set_it() {
        // `editor` is a member of both organizations and has no 2FA.
        // organization-a requires it; organization-b does not.
        let (state, path) = test_state().await;
        require_two_factor(&state, "organization-a").await;

        let uri = |org: &str| format!("/api/v1/organizations/{org}/probe");
        let router = probe(state.clone(), auth_user("editor"), ORG_PATH, org_read_probe);
        let (status, _) = status_of(router, &uri("organization-a")).await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        let router = probe(state, auth_user("editor"), ORG_PATH, org_read_probe);
        let (status, _) = status_of(router, &uri("organization-b")).await;
        assert_eq!(status, StatusCode::OK);
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn the_sysadmin_is_exempt_from_the_requirement() {
        let (state, path) = test_state().await;
        require_two_factor(&state, "organization-a").await;

        let router = probe(
            state,
            crate::testing::sysadmin_user("outsider"),
            ORG_PATH,
            org_read_probe,
        );
        let (status, _) = status_of(router, "/api/v1/organizations/organization-a/probe").await;
        assert_eq!(status, StatusCode::OK);
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn a_non_member_still_gets_the_membership_error_not_the_2fa_one() {
        // A stranger must learn nothing about the organization's settings.
        let (state, path) = test_state().await;
        require_two_factor(&state, "organization-a").await;

        let router = probe(state, auth_user("outsider"), ORG_PATH, org_read_probe);
        let (status, body) = status_of(router, "/api/v1/organizations/organization-a/probe").await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(body.contains("Not a member"), "{body}");
        cleanup_database(&path);
    }
}
