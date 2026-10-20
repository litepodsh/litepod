use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use chrono::{Duration, NaiveDate, Utc};
use uuid::Uuid;

use crate::dtos::personal_access_token::{
    CreateTokenRequest, CreatedTokenDto, MAX_EXPIRY_DAYS, PersonalAccessTokenDto, VALID_SCOPES,
};
use crate::dtos::response::{AppError, created, ok};
use crate::git::repositories::personal_access_token as tokens;
use crate::handler::common::require_session_auth;
use crate::security::authz::Permission;
use crate::security::authz::permission::token_grantable;
use crate::security::middleware::auth::AuthUser;
use crate::state::state::SharedState;

fn validate_scopes(scopes: &[String]) -> Result<(), AppError> {
    if scopes.is_empty() {
        return Err(AppError::ValidationError(
            "At least one scope is required".into(),
        ));
    }
    let grantable = token_grantable();
    let valid = |s: &str| {
        VALID_SCOPES.contains(&s) || Permission::parse(s).is_some_and(|p| grantable.contains(&p))
    };
    if let Some(bad) = scopes.iter().find(|s| !valid(s)) {
        return Err(AppError::ValidationError(format!("Invalid scope: {bad}")));
    }
    Ok(())
}

/// Validates the requested expiry and turns it into the `%Y-%m-%d %H:%M:%S`
/// string the DB column and `auth_middleware`'s comparison both expect —
/// end-of-day, so the token stays valid through the whole date the user picked.
fn expiry_string(date: NaiveDate) -> Result<String, AppError> {
    let today = Utc::now().naive_utc().date();
    if date <= today {
        return Err(AppError::ValidationError(
            "Expiration must be in the future".into(),
        ));
    }
    if date > today + Duration::days(MAX_EXPIRY_DAYS) {
        return Err(AppError::ValidationError(format!(
            "Expiration cannot be more than {MAX_EXPIRY_DAYS} days out"
        )));
    }
    Ok(date
        .and_hms_opt(23, 59, 59)
        .expect("valid time")
        .format("%Y-%m-%d %H:%M:%S")
        .to_string())
}

fn to_dto(row: tokens::TokenListItem) -> PersonalAccessTokenDto {
    PersonalAccessTokenDto {
        id: row.id,
        name: row.name,
        token_prefix: row.token_prefix,
        scopes: serde_json::from_str(&row.scopes).unwrap_or_default(),
        expires_at: row.expires_at,
        last_used_at: row.last_used_at,
        created_at: row.created_at,
    }
}

/// GET /api/v1/account/tokens
#[utoipa::path(
    get, path = "/api/v1/account/tokens", tag = "account",
    responses((status = 200, description = "Personal access tokens"), (status = 401, description = "Unauthorized"))
)]
pub async fn list_tokens_handler(
    State(state): State<SharedState>,
    Extension(auth_user): Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    require_session_auth(&auth_user)?;
    let rows = tokens::list_for_user(&state, &auth_user.user_id)
        .await
        .map_err(AppError::DatabaseError)?;
    Ok(ok(rows.into_iter().map(to_dto).collect::<Vec<_>>()))
}

/// GET /api/v1/account/tokens/permissions — catalog names a token may take as
/// scopes, in catalog order.
#[utoipa::path(
    get, path = "/api/v1/account/tokens/permissions", tag = "account",
    responses((status = 200, description = "Permissions grantable to a token"), (status = 401, description = "Unauthorized"))
)]
pub async fn grantable_permissions_handler(
    Extension(auth_user): Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    require_session_auth(&auth_user)?;
    let grantable = token_grantable();
    Ok(ok(Permission::ALL
        .iter()
        .filter(|p| grantable.contains(p))
        .map(|p| p.as_str())
        .collect::<Vec<_>>()))
}

/// POST /api/v1/account/tokens
#[utoipa::path(
    post, path = "/api/v1/account/tokens", tag = "account", request_body = CreateTokenRequest,
    responses((status = 201, description = "Token created — value shown once"), (status = 401, description = "Unauthorized"))
)]
pub async fn create_token_handler(
    State(state): State<SharedState>,
    Extension(auth_user): Extension<AuthUser>,
    Json(req): Json<CreateTokenRequest>,
) -> Result<impl IntoResponse, AppError> {
    require_session_auth(&auth_user)?;
    let name = req.name.trim();
    if name.is_empty() {
        return Err(AppError::ValidationError("Name is required".into()));
    }
    validate_scopes(&req.scopes)?;
    let expires_at = req.expires_at.map(expiry_string).transpose()?;

    let id = Uuid::now_v7().to_string();
    let raw_token = crate::utils::security::generate_token();
    let token_prefix = format!("lp_{}", &raw_token[..10]);
    let scopes_json =
        serde_json::to_string(&req.scopes).map_err(|e| AppError::InternalError(e.to_string()))?;

    tokens::create(
        &state,
        &id,
        &auth_user.user_id,
        name,
        &raw_token,
        &token_prefix,
        &scopes_json,
        expires_at.as_deref(),
        None,
    )
    .await
    .map_err(AppError::DatabaseError)?;

    tracing::info!(user_id = %auth_user.user_id, token_id = %id, name, "personal_access_token.created");
    let _ = crate::security::security_events::record(
        &state,
        Some(&auth_user.user_id),
        &auth_user,
        "token.created",
        Some(name),
    )
    .await;

    Ok(created(CreatedTokenDto {
        token_info: PersonalAccessTokenDto {
            id,
            name: name.to_string(),
            token_prefix,
            scopes: req.scopes,
            expires_at,
            last_used_at: None,
            created_at: Utc::now()
                .naive_utc()
                .format("%Y-%m-%d %H:%M:%S")
                .to_string(),
        },
        token: raw_token,
    }))
}

/// DELETE /api/v1/account/tokens/{id}
#[utoipa::path(
    delete, path = "/api/v1/account/tokens/{id}", tag = "account",
    params(("id" = String, Path, description = "Token identifier")),
    responses((status = 204, description = "Token revoked"), (status = 404, description = "Not found"), (status = 401, description = "Unauthorized"))
)]
pub async fn revoke_token_handler(
    State(state): State<SharedState>,
    Extension(auth_user): Extension<AuthUser>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    require_session_auth(&auth_user)?;
    let affected = tokens::revoke_for_user(&state, &id, &auth_user.user_id)
        .await
        .map_err(AppError::DatabaseError)?;
    if affected == 0 {
        return Err(AppError::ResourceNotFound);
    }
    tracing::info!(user_id = %auth_user.user_id, token_id = %id, "personal_access_token.revoked");
    let _ = crate::security::security_events::record(
        &state,
        Some(&auth_user.user_id),
        &auth_user,
        "token.revoked",
        Some(&id),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

/// DELETE /api/v1/account/tokens — revoke every personal access token for the
/// current user.
#[utoipa::path(
    delete, path = "/api/v1/account/tokens", tag = "account",
    responses((status = 204, description = "All tokens revoked"), (status = 401, description = "Unauthorized"))
)]
pub async fn revoke_all_tokens_handler(
    State(state): State<SharedState>,
    Extension(auth_user): Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    require_session_auth(&auth_user)?;
    let affected = tokens::revoke_all_for_user(&state, &auth_user.user_id)
        .await
        .map_err(AppError::DatabaseError)?;
    tracing::info!(user_id = %auth_user.user_id, count = affected, "personal_access_token.revoked_all");
    let _ = crate::security::security_events::record(
        &state,
        Some(&auth_user.user_id),
        &auth_user,
        "tokens.revoked_all",
        Some(&affected.to_string()),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use axum::{body::to_bytes, response::IntoResponse};

    use super::*;
    use crate::testing::{auth_user, cleanup_database, test_state};

    fn token_of(user_id: &str) -> AuthUser {
        AuthUser {
            token_id: Some("token-abc".into()),
            token_name: Some("ci".into()),
            ..auth_user(user_id)
        }
    }

    fn request(scopes: &[&str], expires_at: Option<NaiveDate>) -> CreateTokenRequest {
        CreateTokenRequest {
            name: "ci".into(),
            scopes: scopes.iter().map(|s| s.to_string()).collect(),
            expires_at,
        }
    }

    #[tokio::test]
    async fn creating_returns_the_raw_token_once_and_listing_never_does() {
        let (state, path) = test_state().await;
        let created = create_token_handler(
            State(state.clone()),
            Extension(auth_user("editor")),
            Json(request(&["read", "write"], None)),
        )
        .await
        .unwrap()
        .into_response();
        assert_eq!(created.status(), StatusCode::CREATED);
        let body = to_bytes(created.into_body(), usize::MAX).await.unwrap();
        let created: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let raw_token = created["data"]["token"].as_str().unwrap().to_string();
        assert_eq!(
            raw_token.len(),
            64,
            "generate_token() is 256 bits, hex-encoded"
        );

        let listed = list_tokens_handler(State(state.clone()), Extension(auth_user("editor")))
            .await
            .unwrap()
            .into_response();
        let body = to_bytes(listed.into_body(), usize::MAX).await.unwrap();
        let listed = std::str::from_utf8(&body).unwrap();
        // The list DTO has no field to hold it, but assert on the wire format
        // too: the raw secret must never appear in a listing response.
        assert!(!listed.contains(&raw_token));

        drop(state);
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn revoking_someone_elses_token_is_not_found_not_forbidden() {
        let (state, path) = test_state().await;
        create_token_handler(
            State(state.clone()),
            Extension(auth_user("editor")),
            Json(request(&["read"], None)),
        )
        .await
        .unwrap();
        let listed = tokens::list_for_user(&state, "editor").await.unwrap();
        let id = listed[0].id.clone();

        let result = revoke_token_handler(
            State(state.clone()),
            Extension(auth_user("viewer")),
            Path(id.clone()),
        )
        .await;
        assert!(matches!(result, Err(AppError::ResourceNotFound)));

        let result = revoke_token_handler(
            State(state.clone()),
            Extension(auth_user("editor")),
            Path(id),
        )
        .await;
        assert!(result.is_ok());

        drop(state);
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn revoking_all_clears_only_the_callers_tokens() {
        let (state, path) = test_state().await;
        for _ in 0..2 {
            create_token_handler(
                State(state.clone()),
                Extension(auth_user("editor")),
                Json(request(&["read"], None)),
            )
            .await
            .unwrap();
        }
        create_token_handler(
            State(state.clone()),
            Extension(auth_user("viewer")),
            Json(request(&["read"], None)),
        )
        .await
        .unwrap();

        revoke_all_tokens_handler(State(state.clone()), Extension(auth_user("editor")))
            .await
            .unwrap();

        assert!(
            tokens::list_for_user(&state, "editor")
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            tokens::list_for_user(&state, "viewer").await.unwrap().len(),
            1
        );

        drop(state);
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn a_token_cannot_manage_tokens() {
        let (state, path) = test_state().await;
        let result = create_token_handler(
            State(state.clone()),
            Extension(token_of("editor")),
            Json(request(&["read"], None)),
        )
        .await;
        assert!(matches!(result, Err(AppError::InsufficientPermissions)));

        let result = list_tokens_handler(State(state.clone()), Extension(token_of("editor"))).await;
        assert!(matches!(result, Err(AppError::InsufficientPermissions)));

        let result = revoke_token_handler(
            State(state.clone()),
            Extension(token_of("editor")),
            Path("whatever".into()),
        )
        .await;
        assert!(matches!(result, Err(AppError::InsufficientPermissions)));

        let result =
            revoke_all_tokens_handler(State(state.clone()), Extension(token_of("editor"))).await;
        assert!(matches!(result, Err(AppError::InsufficientPermissions)));

        drop(state);
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn expiry_is_capped_at_one_year_and_must_be_future() {
        let (state, path) = test_state().await;
        let past = Utc::now().naive_utc().date() - Duration::days(1);
        let result = create_token_handler(
            State(state.clone()),
            Extension(auth_user("editor")),
            Json(request(&["read"], Some(past))),
        )
        .await;
        assert!(matches!(result, Err(AppError::ValidationError(_))));

        let too_far = Utc::now().naive_utc().date() + Duration::days(MAX_EXPIRY_DAYS + 1);
        let result = create_token_handler(
            State(state.clone()),
            Extension(auth_user("editor")),
            Json(request(&["read"], Some(too_far))),
        )
        .await;
        assert!(matches!(result, Err(AppError::ValidationError(_))));

        drop(state);
        cleanup_database(&path);
    }
}
