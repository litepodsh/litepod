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
use crate::handler::common::require_cloud_sysadmin;
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
