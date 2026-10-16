use axum::{
    Extension, Json,
    extract::{Path, State},
    response::IntoResponse,
};

use crate::cqrs::commands::invitation;
use crate::dtos::invitation::CreateInvitationRequest;
use crate::dtos::response::{AppError, created, ok};
use crate::git::repositories::user;
use crate::security::middleware::auth::AuthUser;
use crate::state::state::SharedState;
use crate::{cqrs::queries::organization as organization_q, git::repositories::organization};

async fn require_sysadmin(state: &SharedState, auth_user: &AuthUser) -> Result<(), AppError> {
    if !user::is_sysadmin(state, &auth_user.user_id).await {
        return Err(AppError::InsufficientPermissions);
    }
    Ok(())
}

#[utoipa::path(
    get,
    path = "/api/v1/invitations",
    responses((status = 200, description = "List all invitations")),
    tag = "users"
)]
pub async fn list_handler(
    State(state): State<SharedState>,
    Extension(auth_user): Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    if state.is_cloud {
        Ok(ok(invitation::list_invitations_for_creator(
            &state,
            &auth_user.user_id,
        )
        .await?))
    } else {
        require_sysadmin(&state, &auth_user).await?;
        Ok(ok(invitation::list_invitations(&state).await?))
    }
}

#[utoipa::path(
    post,
    path = "/api/v1/invitations",
    request_body = CreateInvitationRequest,
    responses((status = 201, description = "Pending user and invitation created")),
    tag = "users"
)]
pub async fn create_handler(
    State(state): State<SharedState>,
    Extension(auth_user): Extension<AuthUser>,
    Json(req): Json<CreateInvitationRequest>,
) -> Result<impl IntoResponse, AppError> {
    // Invitations are unlimited under the base + per-server billing model —
    // only remote servers are billed, so no capacity/RBAC-downgrade check
    // here anymore.
    if !state.is_cloud {
        require_sysadmin(&state, &auth_user).await?;
    }
    let invitation = invitation::create_invitation(&state, &auth_user.user_id, &req).await?;
    let _ = crate::security::security_events::record(
        &state,
        None,
        &auth_user,
        "invitation.created",
        Some(&invitation.invitation.email),
    )
    .await;
    Ok(created(invitation))
}

#[utoipa::path(
    post,
    path = "/api/v1/invitations/{id}/revoke",
    responses((status = 200, description = "Invitation revoked")),
    tag = "users"
)]
pub async fn revoke_handler(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Extension(auth_user): Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    if !state.is_cloud {
        require_sysadmin(&state, &auth_user).await?;
    }
    invitation::revoke_invitation(&state, &auth_user.user_id, &id).await?;
    let _ = crate::security::security_events::record(
        &state,
        None,
        &auth_user,
        "invitation.revoked",
        Some(&id),
    )
    .await;
    Ok(ok(()))
}

#[utoipa::path(
    post,
    path = "/api/v1/invitations/{id}/regenerate",
    responses((status = 200, description = "Invitation link regenerated")),
    tag = "users"
)]
pub async fn regenerate_handler(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Extension(auth_user): Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    if !state.is_cloud {
        require_sysadmin(&state, &auth_user).await?;
    }
    let regenerated = invitation::regenerate_invitation(&state, &auth_user.user_id, &id).await?;
    let _ = crate::security::security_events::record(
        &state,
        None,
        &auth_user,
        "invitation.regenerated",
        Some(&id),
    )
    .await;
    Ok(ok(regenerated))
}

#[utoipa::path(
    get,
    path = "/api/v1/invitations/organizations",
    responses((status = 200, description = "Organizations available for an invitation")),
    tag = "users"
)]
pub async fn eligible_organizations_handler(
    State(state): State<SharedState>,
    Extension(auth_user): Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    let organizations = if state.is_cloud {
        organization::list_admin_organizations_for_user(&state, &auth_user.user_id)
            .await
            .map_err(AppError::DatabaseError)?
            .into_iter()
            .map(|row| crate::dtos::organization::OrganizationDto {
                id: row.id,
                name: row.name,
                slug: row.slug,
                description: row.description,
                icon: row.icon,
                icon_bg: row.icon_bg,
                plan: row.plan,
                is_root: row.is_root,
                audit_logs_enabled: row.audit_logs_enabled,
                assistant_enabled: row.assistant_enabled,
                mcp_enabled: row.mcp_enabled,
                audit_logs_retention_days: row.audit_logs_retention_days,
                created_at: row.created_at,
            })
            .collect()
    } else {
        require_sysadmin(&state, &auth_user).await?;
        organization_q::list_all_organizations(&state).await?
    };
    Ok(ok(organizations))
}

#[utoipa::path(
    get,
    path = "/api/v1/invitations/{token}",
    responses((status = 200, description = "Valid invitation details")),
    tag = "auth"
)]
pub async fn inspect_handler(
    State(state): State<SharedState>,
    Path(token): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    Ok(ok(invitation::inspect_invitation(&state, &token).await?))
}

#[cfg(test)]
mod security_tests {
    use super::*;
    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode},
        routing::post,
    };
    use tower::ServiceExt;

    #[tokio::test]
    async fn body_organizations_require_authority() {
        for cloud in [true, false] {
            let (state, _path) = crate::testing::test_state_for(cloud).await;
            // A real administrator of A must still be refused when the nested
            // body asks to grant membership in B. The whole request is atomic.
            sqlx::query("UPDATE organization_members SET role_id = 1 WHERE user_id = 'viewer' AND organization_id = 'organization-a'")
                .execute(&state.db).await.unwrap();
            let router = Router::new()
                .route("/invitations", post(create_handler))
                .layer(Extension(crate::testing::auth_user("viewer")))
                .with_state(state.clone());
            let response = router.clone().oneshot(Request::post("/invitations")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"username":"invitee","email":"invitee@example.com","organizations":[{"organization_id":"organization-a","role_id":3},{"organization_id":"organization-b","role_id":3}]}"#)).unwrap()).await.unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
            let count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM users WHERE email = 'invitee@example.com'",
            )
            .fetch_one(&state.db)
            .await
            .unwrap();
            assert_eq!(count, 0, "denied invitation left a user behind");
            if cloud {
                let response = router.oneshot(Request::post("/invitations")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"username":"invitee","email":"invitee@example.com","organizations":[{"organization_id":"organization-a","role_id":3}]}"#)).unwrap()).await.unwrap();
                assert_eq!(response.status(), StatusCode::CREATED);
            }
        }
    }
}
