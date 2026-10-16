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
use crate::security::middleware::auth::AuthUser;
use crate::state::state::SharedState;

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
    // A SysAdmin can't lock out themselves or another SysAdmin.
    if auth_user.user_id == user_id || user::is_sysadmin(&state, &user_id).await {
        return Err(AppError::InsufficientPermissions);
    }

    if !user::set_disabled(&state, &user_id, req.disabled)
        .await
        .map_err(AppError::DatabaseError)?
    {
        return Err(AppError::ValidationError(
            "Only active users can be disabled, and only disabled users re-enabled".into(),
        ));
    }

    let _ = crate::security::security_events::record(
        &state,
        Some(&user_id),
        &auth_user,
        if req.disabled {
            "user.disabled"
        } else {
            "user.enabled"
        },
        None,
    )
    .await;

    Ok(StatusCode::NO_CONTENT)
}
