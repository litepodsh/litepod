use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};

use crate::cqrs::queries::user as user_queries;
use crate::dtos::response::{AppError, ok};
use crate::dtos::user::{SetUserDisabledRequest, UpdateUserRequest};
use crate::git::repositories::user;
use crate::handler::common::{require_cloud_sysadmin, require_session_auth};
use crate::security::middleware::auth::AuthUser;
use crate::state::state::SharedState;

/// The Admin portal's own view of the accounts on the instance.
///
/// Unlike `/api/v1/users`, which every edition's SysAdmin can call, this one
/// is the portal's list: it exists only in the cloud edition, where the portal
/// does. See `handler::common::require_cloud_sysadmin` for the gate.
#[utoipa::path(
    get,
    path = "/api/v1/admin/users",
    responses(
        (status = 200, description = "Every account on the instance (cloud SysAdmin only)"),
    ),
    tag = "users"
)]
pub async fn admin_list_users_handler(
    State(state): State<SharedState>,
    Extension(auth_user): Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    require_cloud_sysadmin(&state, &auth_user).await?;

    let users = user_queries::list_users(&state).await?;
    Ok(ok(users))
}

#[utoipa::path(
    get,
    path = "/api/v1/users",
    responses(
        (status = 200, description = "List of all users (SysAdmin only)"),
    ),
    tag = "users"
)]
pub async fn list_users_handler(
    State(state): State<SharedState>,
    Extension(auth_user): Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    if !user::is_sysadmin(&state, &auth_user.user_id).await {
        return Err(AppError::InsufficientPermissions);
    }

    let users = user_queries::list_users(&state).await?;
    Ok(ok(users))
}

#[utoipa::path(
    get,
    path = "/api/v1/users/{user_id}",
    responses(
        (status = 200, description = "User found"),
        (status = 404, description = "User not found"),
    ),
    tag = "users"
)]
pub async fn get_user_handler(
    State(state): State<SharedState>,
    Path(user_id): Path<String>,
    Extension(auth_user): Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    if auth_user.user_id != user_id && !user::is_sysadmin(&state, &auth_user.user_id).await {
        return Err(AppError::InsufficientPermissions);
    }

    let u = user_queries::get_user(&state, &user_id).await?;
    Ok(ok(u))
}

#[utoipa::path(
    patch,
    path = "/api/v1/users/{user_id}",
    request_body = UpdateUserRequest,
    responses(
        (status = 200, description = "User updated"),
    ),
    tag = "users"
)]
pub async fn update_user_handler(
    State(state): State<SharedState>,
    Path(user_id): Path<String>,
    Extension(auth_user): Extension<AuthUser>,
    Json(req): Json<UpdateUserRequest>,
) -> Result<impl IntoResponse, AppError> {
    if auth_user.user_id != user_id && !user::is_sysadmin(&state, &auth_user.user_id).await {
        return Err(AppError::InsufficientPermissions);
    }

    if let Some(ref new_name) = req.name {
        let current = user_queries::get_user(&state, &user_id).await?;
        if new_name != &current.name && user::get_user_by_name(&state, new_name).await.is_some() {
            return Err(AppError::AlreadyExists);
        }
    }

    user::update_user(&state, &user_id, req.name.as_deref())
        .await
        .map_err(|e| AppError::DatabaseError(e))?;

    let _ = crate::security::security_events::record(
        &state,
        Some(&user_id),
        &auth_user,
        "user.updated",
        req.name.as_deref(),
    )
    .await;

    let u = user_queries::get_user(&state, &user_id).await?;
    Ok(ok(u))
}

#[utoipa::path(
    delete,
    path = "/api/v1/users/{user_id}",
    responses(
        (status = 200, description = "User deleted"),
    ),
    tag = "users"
)]
pub async fn delete_user_handler(
    State(state): State<SharedState>,
    Path(user_id): Path<String>,
    Extension(auth_user): Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    if auth_user.user_id != user_id && !user::is_sysadmin(&state, &auth_user.user_id).await {
        return Err(AppError::InsufficientPermissions);
    }

    // Protect SysAdmin users from deletion
    if user::is_sysadmin(&state, &user_id).await {
        return Err(AppError::InsufficientPermissions);
    }

    user::delete_user(&state, &user_id)
        .await
        .map_err(|e| AppError::DatabaseError(e))?;

    let _ = crate::security::security_events::record(
        &state,
        Some(&user_id),
        &auth_user,
        "user.deleted",
        None,
    )
    .await;

    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    put,
    path = "/api/v1/users/{user_id}/disabled",
    request_body = SetUserDisabledRequest,
    responses(
        (status = 204, description = "User disabled or re-enabled (SysAdmin only)"),
    ),
    tag = "users"
)]
pub async fn set_user_disabled_handler(
    State(state): State<SharedState>,
    Path(user_id): Path<String>,
    Extension(auth_user): Extension<AuthUser>,
    Json(req): Json<SetUserDisabledRequest>,
) -> Result<impl IntoResponse, AppError> {
    if !user::is_sysadmin(&state, &auth_user.user_id).await {
        return Err(AppError::InsufficientPermissions);
    }
    apply_disabled(&state, &auth_user, &user_id, req.disabled).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The Admin portal's own toggle. The portal only exists in the cloud edition,
/// so this is the cloud SysAdmin's route to lock an account out; the generic
/// `/api/v1/users/{id}/disabled` stays available to every edition's SysAdmin.
#[utoipa::path(
    put,
    path = "/api/v1/admin/users/{user_id}/disabled",
    request_body = SetUserDisabledRequest,
    responses(
        (status = 204, description = "User disabled or re-enabled (cloud SysAdmin only)"),
    ),
    tag = "users"
)]
pub async fn admin_set_user_disabled_handler(
    State(state): State<SharedState>,
    Path(user_id): Path<String>,
    Extension(auth_user): Extension<AuthUser>,
    Json(req): Json<SetUserDisabledRequest>,
) -> Result<impl IntoResponse, AppError> {
    require_cloud_sysadmin(&state, &auth_user).await?;
    apply_disabled(&state, &auth_user, &user_id, req.disabled).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The agents this user approved over MCP OAuth, open or closed. Session-only,
/// like everything else that manages tokens.
#[utoipa::path(
    get,
    path = "/api/v1/admin/users/{user_id}/mcp-connections",
    params(("user_id" = String, Path, description = "User identifier")),
    responses(
        (status = 200, description = "The user's MCP connections (cloud SysAdmin only)"),
    ),
    tag = "users"
)]
pub async fn admin_list_mcp_connections_handler(
    State(state): State<SharedState>,
    Path(user_id): Path<String>,
    Extension(auth_user): Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    require_cloud_sysadmin(&state, &auth_user).await?;
    require_session_auth(&auth_user)?;
    Ok(ok(user_queries::mcp_connections(&state, &user_id).await?))
}

/// Cuts one agent's access now: every token of the grant and its session record.
#[utoipa::path(
    delete,
    path = "/api/v1/admin/users/{user_id}/mcp-connections/{connection_id}",
    params(
        ("user_id" = String, Path, description = "User identifier"),
        ("connection_id" = String, Path, description = "Connection identifier"),
    ),
    responses(
        (status = 204, description = "Connection revoked"),
        (status = 404, description = "Not one of this user's connections"),
    ),
    tag = "users"
)]
pub async fn admin_revoke_mcp_connection_handler(
    State(state): State<SharedState>,
    Path((user_id, connection_id)): Path<(String, String)>,
    Extension(auth_user): Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    use crate::git::repositories::oauth;

    require_cloud_sysadmin(&state, &auth_user).await?;
    require_session_auth(&auth_user)?;
    let code_digest = oauth::take_grant(&state, &connection_id, &user_id)
        .await
        .map_err(AppError::DatabaseError)?
        .ok_or(AppError::ResourceNotFound)?;
    oauth::revoke_by_code(&state, &code_digest)
        .await
        .map_err(AppError::DatabaseError)?;
    let _ = crate::security::security_events::record(
        &state,
        Some(&user_id),
        &auth_user,
        "session.revoked",
        Some(&connection_id),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

/// The shared body of both disable routes: the checks that must hold no matter
/// which door the caller came through, then the write and its audit record.
async fn apply_disabled(
    state: &SharedState,
    auth_user: &AuthUser,
    user_id: &str,
    disabled: bool,
) -> Result<(), AppError> {
    // A SysAdmin can't lock out themselves or another SysAdmin.
    if auth_user.user_id == user_id || user::is_sysadmin(state, user_id).await {
        return Err(AppError::InsufficientPermissions);
    }

    if !user::set_disabled(state, user_id, disabled)
        .await
        .map_err(AppError::DatabaseError)?
    {
        return Err(AppError::ValidationError(
            "Only active users can be disabled, and only disabled users re-enabled".into(),
        ));
    }

    let _ = crate::security::security_events::record(
        state,
        Some(user_id),
        auth_user,
        if disabled { "user.disabled" } else { "user.enabled" },
        None,
    )
    .await;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{cleanup_database, sysadmin_user, test_state_for};
    use axum::response::IntoResponse;

    async fn seed_grant(state: &SharedState, user_id: &str, name: &str) -> String {
        use crate::git::repositories::oauth;

        crate::testing::seed_mcp_connection(state, "organization-a", user_id, name).await;
        oauth::create_client(state, name, name, "[]").await.unwrap();
        oauth::create_refresh(
            state,
            &oauth::NewRefresh {
                token: name,
                client_id: name,
                user_id,
                organization_id: "organization-a",
                access_token_id: name,
                code_digest: name,
                expires_at: "2999-01-01 00:00:00",
            },
        )
        .await
        .unwrap();
        oauth::record_session(
            state,
            name,
            name,
            user_id,
            "organization-a",
            name,
            "claude",
            "2999-01-01 00:00:00",
        )
        .await
        .unwrap();
        oauth::list_sessions(state, user_id)
            .await
            .unwrap()
            .into_iter()
            .find(|row| row.client_name == name)
            .unwrap()
            .id
    }

    fn status_of<T: IntoResponse>(result: Result<T, AppError>) -> StatusCode {
        match result {
            Ok(response) => response.into_response().status(),
            Err(error) => error.into_response().status(),
        }
    }

    /// Same gate as the rest of the portal, and a token never gets through it:
    /// a stolen one must not be able to cut other people's connections.
    #[tokio::test]
    async fn only_a_signed_in_cloud_sysadmin_reaches_mcp_connections() {
        let call = async |state: &SharedState, actor: AuthUser| {
            status_of(
                admin_list_mcp_connections_handler(
                    State(state.clone()),
                    Path("viewer".to_owned()),
                    Extension(actor),
                )
                .await,
            )
        };

        let (state, path) = test_state_for(false).await;
        assert_eq!(
            call(&state, sysadmin_user("editor")).await,
            StatusCode::NOT_FOUND
        );
        cleanup_database(&path);

        let (state, path) = test_state_for(true).await;
        crate::git::repositories::user::make_sysadmin(&state, "editor")
            .await
            .unwrap();
        assert_eq!(call(&state, sysadmin_user("editor")).await, StatusCode::OK);
        assert_eq!(
            call(&state, crate::testing::auth_user("viewer")).await,
            StatusCode::FORBIDDEN
        );
        let mut with_token = sysadmin_user("editor");
        with_token.token_id = Some("token".to_owned());
        assert_eq!(call(&state, with_token).await, StatusCode::FORBIDDEN);
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn revoking_a_connection_cuts_only_that_users_agent() {
        use crate::git::repositories::{oauth, personal_access_token as tokens};

        let (state, path) = test_state_for(true).await;
        crate::git::repositories::user::make_sysadmin(&state, "editor")
            .await
            .unwrap();
        let target = seed_grant(&state, "viewer", "target").await;
        let bystander = seed_grant(&state, "viewer", "bystander").await;
        let foreign = seed_grant(&state, "editor", "foreign").await;
        tokens::touch_last_used(&state, "target").await;
        assert_eq!(
            oauth::count_open_sessions(&state).await.unwrap()["viewer"],
            2
        );

        let listed = user_queries::mcp_connections(&state, "viewer")
            .await
            .unwrap();
        let row = listed.iter().find(|row| row.id == target).unwrap();
        assert_eq!(row.organization_name, "organization-a");
        assert!(row.last_used_at.is_some(), "the grant's token was used");

        let revoke = async |user: &str, connection: &str| {
            status_of(
                admin_revoke_mcp_connection_handler(
                    State(state.clone()),
                    Path((user.to_owned(), connection.to_owned())),
                    Extension(sysadmin_user("editor")),
                )
                .await,
            )
        };
        assert_eq!(revoke("viewer", &foreign).await, StatusCode::NOT_FOUND);
        assert!(
            tokens::find_active_by_id(&state, "foreign")
                .await
                .unwrap()
                .is_some()
        );

        assert_eq!(revoke("viewer", &target).await, StatusCode::NO_CONTENT);
        assert!(
            tokens::find_active_by_id(&state, "target")
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            tokens::find_active_by_id(&state, "bystander")
                .await
                .unwrap()
                .is_some()
        );
        assert_eq!(
            oauth::count_open_sessions(&state).await.unwrap()["viewer"],
            1
        );
        let listed = user_queries::mcp_connections(&state, "viewer")
            .await
            .unwrap();
        let closed = listed.iter().find(|row| row.id == target).unwrap();
        assert!(closed.closed_at.is_some(), "kept as history, marked closed");
        assert!(
            listed
                .iter()
                .find(|row| row.id == bystander)
                .unwrap()
                .closed_at
                .is_none()
        );
        cleanup_database(&path);
    }

    /// The Admin portal is the cloud SysAdmin's alone: off-cloud the surface
    /// does not exist (404), and on-cloud holding an account is not enough
    /// (403). Asserted on the handler, because that is where the gate lives.
    #[tokio::test]
    async fn the_admin_portal_answers_only_the_cloud_sysadmin() {
        for (is_cloud, expected_admin) in [
            (true, StatusCode::OK),
            (false, StatusCode::NOT_FOUND),
        ] {
            let (state, path) = test_state_for(is_cloud).await;
            crate::git::repositories::user::make_sysadmin(&state, "editor")
                .await
                .unwrap();

            let admin = admin_list_users_handler(
                State(state.clone()),
                Extension(sysadmin_user("editor")),
            )
            .await;
            let admin_status = match admin {
                Ok(response) => response.into_response().status(),
                Err(error) => error.into_response().status(),
            };
            assert_eq!(
                admin_status, expected_admin,
                "the sysadmin's own answer must depend on the edition (is_cloud={is_cloud})"
            );

            let plain = admin_list_users_handler(
                State(state.clone()),
                Extension(crate::testing::auth_user("viewer")),
            )
            .await;
            let plain_status = match plain {
                Ok(response) => response.into_response().status(),
                Err(error) => error.into_response().status(),
            };
            // On-cloud the portal exists, so a plain account is told it may not
            // in; off-cloud the portal does not exist at all, so nobody is told
            // anything different from the 404 the SysAdmin gets.
            let expected_plain = if is_cloud {
                StatusCode::FORBIDDEN
            } else {
                StatusCode::NOT_FOUND
            };
            assert_eq!(
                plain_status, expected_plain,
                "merely holding an account must not open the portal (is_cloud={is_cloud})"
            );

            cleanup_database(&path);
        }
    }
}
