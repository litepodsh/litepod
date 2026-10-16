use crate::dtos::response::AppError;
use crate::git::repositories::role;
use crate::security::middleware::auth::AuthUser;
use crate::state::state::SharedState;

/// How a user should be named in something a human reads outside the app — a
/// notification, for instance. Falls back to the email when no display name is
/// set, which is the case for accounts created through an invite.
pub fn actor_label(user: &AuthUser) -> String {
    let name = user.name.trim();
    if name.is_empty() {
        user.email.clone()
    } else {
        name.to_string()
    }
}

/// The pseudo role id a SysAdmin evaluates as. Access decisions use
/// `security::authz`, not role ids.
pub const ROLE_SYSADMIN: i64 = 0;

/// The caller's effective role inside `organization_id`.
///
/// A SysAdmin outranks membership everywhere, cloud included — that is the
/// whole point of the flag, and splitting the rule by edition is what let the
/// old `is_sysadmin && !is_cloud` check drift out of sync with the rest.
///
/// These helpers take `&AuthUser` rather than a bare user id on purpose: a
/// handler that only has the id cannot call them, so it cannot accidentally
/// perform a membership lookup that skips the SysAdmin bypass.
pub async fn organization_role(
    state: &SharedState,
    organization_id: &str,
    user: &AuthUser,
) -> Result<i64, AppError> {
    if user.is_sysadmin {
        return Ok(ROLE_SYSADMIN);
    }
    role::get_organization_member_role(state, organization_id, &user.user_id)
        .await
        .map(|role| role.id)
        .ok_or(AppError::NotOrganizationMember)
}

/// Rejects unless the application really hangs off this organization.
///
/// The role helpers above only prove the caller belongs to the organization they
/// named in the URL — which is always true, because the caller picks it. This is
/// the other half: the lookup is a four-table join keyed on all four ids
/// (`repositories::project::get_application`), so a valid role can never be
/// paired with a child id from somewhere else.
///
/// Lives here rather than in `handler::deployments` because
/// `middleware::org_scope` needs it and shouldn't depend on a handler module.
/// `handler::deployments` re-exports it, so its existing callers are unaffected.
/// Returns the application row rather than just `()` so callers that already
/// paid for this lookup can use it — `AppScope` reads `name` to stamp on audit
/// log entries and any future handler that needs name + id skips its own query.
pub async fn check_application(
    state: &SharedState,
    organization_id: &str,
    project_id: &str,
    environment_id: &str,
    application_id: &str,
) -> Result<crate::git::repositories::project::ApplicationRow, AppError> {
    crate::git::repositories::project::get_application(
        state,
        organization_id,
        project_id,
        environment_id,
        application_id,
    )
    .await
    .map_err(AppError::DatabaseError)?
    .ok_or(AppError::ApplicationNotFound)
}

/// Rejects a request authenticated by personal access token. Used only by the
/// handlers that manage tokens themselves (`handler::personal_access_tokens`):
/// a token must never be usable to mint or revoke tokens, its own included, so
/// that path stays session-only regardless of how permissive its scopes are.
pub fn require_session_auth(user: &AuthUser) -> Result<(), AppError> {
    if user.token_id.is_some() {
        return Err(AppError::InsufficientPermissions);
    }
    Ok(())
}
