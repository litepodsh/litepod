use axum::response::IntoResponse;

use crate::dtos::response::{AppError, ok};
use crate::security::authz::extract::{RequireOrg, perm};

/// The caller's effective permissions in one organization, so the UI can hide
/// actions the API would refuse. Cosmetic for the client; the API stays the
/// authority on every route.
#[utoipa::path(
    get,
    path = "/api/v1/organizations/{organization_id}/me/permissions",
    params(("organization_id" = String, Path, description = "Organization UUID")),
    responses((status = 200, description = "Caller's effective permissions", body = crate::security::authz::set::PermissionSetDto)),
    tag = "organizations"
)]
pub async fn my_permissions_handler(
    RequireOrg(org, _): RequireOrg<perm::OrgRead>,
) -> Result<impl IntoResponse, AppError> {
    Ok(ok(org.permissions.to_dto()))
}

#[cfg(test)]
mod tests {
    use axum::body::{Body, to_bytes};
    use axum::extract::Request;
    use axum::http::StatusCode;
    use axum::middleware::Next;
    use axum::{Router, routing::get};
    use tower::ServiceExt;

    use super::my_permissions_handler;
    use crate::testing::{
        assign, auth_user, cleanup_database, seed_application, set_base_role, test_state,
    };

    #[tokio::test]
    async fn my_permissions_lists_scoped_grants() {
        let (state, path) = test_state().await;
        let (project, _, _) = seed_application(&state, "organization-a").await;
        set_base_role(&state, "organization-a", "viewer", 4).await;
        assign(&state, "organization-a", "viewer", 2, "project", &project).await;

        let router = Router::new()
            .route(
                "/api/v1/organizations/{organization_id}/me/permissions",
                get(my_permissions_handler),
            )
            .layer(axum::middleware::from_fn(
                |mut req: Request, next: Next| async move {
                    req.extensions_mut().insert(auth_user("viewer"));
                    next.run(req).await
                },
            ))
            .with_state(state);
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/api/v1/organizations/organization-a/me/permissions")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = String::from_utf8(
            to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        assert!(body.contains(&project), "{body}");
        assert!(body.contains("app.deploy"), "{body}");
        assert!(!body.contains("server.manage"), "{body}");
        cleanup_database(&path);
    }
}
