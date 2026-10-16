use std::collections::HashSet;

use chrono::{Duration, Utc};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::dtos::auth::{AuthResponse, UserClaims};
use crate::dtos::invitation::{
    CreateInvitationRequest, CreatedInvitationDto, InvitationDetailsDto, InvitationDto,
    InvitationOrganizationDto,
};
use crate::dtos::response::AppError;
use crate::git::repositories::{invitation, organization_member, user};
use crate::state::state::SharedState;
use crate::utils::security;

pub fn new_invitation_token() -> String {
    Uuid::new_v4().simple().to_string()
}

pub fn hash_invitation_token(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn validate_create(req: &CreateInvitationRequest) -> Result<(), AppError> {
    if req.username.trim().is_empty() {
        return Err(AppError::ValidationError("Username is required".into()));
    }
    if req.email.trim().is_empty() || !req.email.contains('@') {
        return Err(AppError::ValidationError(
            "A valid email is required".into(),
        ));
    }
    if req.organizations.is_empty() {
        return Err(AppError::ValidationError(
            "At least one organization is required".into(),
        ));
    }
    let mut organization_ids = HashSet::new();
    for organization in &req.organizations {
        if organization.organization_id.trim().is_empty() {
            return Err(AppError::ValidationError("Organization is required".into()));
        }
        if !organization_ids.insert(organization.organization_id.as_str()) {
            return Err(AppError::ValidationError(
                "Each organization may appear once".into(),
            ));
        }
        if !(1..=4).contains(&organization.role_id) {
            return Err(AppError::ValidationError(
                "Organization role must be Admin, Editor, Viewer, or Member".into(),
            ));
        }
    }
    Ok(())
}

fn status(row: &invitation::InvitationRow) -> &'static str {
    if row.accepted_at.is_some() {
        "accepted"
    } else if row.revoked_at.is_some() {
        "revoked"
    } else if row.expires_at <= Utc::now().format("%Y-%m-%d %H:%M:%S").to_string() {
        "expired"
    } else {
        "pending"
    }
}

fn row_state_error(row: &invitation::InvitationRow) -> AppError {
    match status(row) {
        "accepted" => AppError::InvitationAlreadyAccepted,
        "revoked" => AppError::InvitationRevoked,
        "expired" => AppError::InvitationExpired,
        _ => AppError::InternalError("Invitation state was unexpectedly pending".into()),
    }
}

async fn to_dto(
    state: &SharedState,
    row: invitation::InvitationRow,
) -> Result<InvitationDto, AppError> {
    let organizations = invitation::list_organizations(state, &row.user_id)
        .await?
        .into_iter()
        .map(|organization| InvitationOrganizationDto {
            organization_id: organization.organization_id,
            organization_name: organization.organization_name,
            role_id: organization.role_id,
            role_name: organization.role_name,
        })
        .collect();
    let invitation_status = status(&row).to_string();
    Ok(InvitationDto {
        id: row.id,
        user_id: row.user_id,
        username: row.username,
        email: row.email,
        created_by: row.created_by,
        created_by_name: row.created_by_name,
        status: invitation_status,
        expires_at: row.expires_at,
        created_at: row.created_at,
        organizations,
    })
}

pub async fn create_invitation(
    state: &SharedState,
    created_by: &str,
    req: &CreateInvitationRequest,
) -> Result<CreatedInvitationDto, AppError> {
    validate_create(req)?;
    let email = req.email.trim().to_ascii_lowercase();
    let username = req.username.trim();
    if user::get_user_by_email(state, &email).await.is_some() {
        return Err(AppError::ValidationError("Email is already in use".into()));
    }
    if user::get_user_by_name(state, username).await.is_some() {
        return Err(AppError::ValidationError(
            "Username is already in use".into(),
        ));
    }

    let user_id = Uuid::now_v7().to_string();
    let invitation_id = Uuid::now_v7().to_string();
    let token = new_invitation_token();
    let token_hash = hash_invitation_token(&token);
    let random_password_hash = security::hash_password(Uuid::new_v4().to_string());
    crate::git::repositories::db::retry_on_stale_snapshot(|| async {
        let conn = crate::git::repositories::db::get_conn(state).await;
        let mut tx = conn.begin().await?;

        if state.is_cloud {
            let admin_organization_ids =
                organization_member::list_admin_organization_ids_on(&mut *tx, created_by)
                    .await?;
            if req.organizations.iter().any(|organization| {
                !admin_organization_ids.contains(&organization.organization_id)
            }) {
                return Err(AppError::InsufficientPermissions);
            }
        }

        sqlx::query(
            "INSERT INTO users (id, email, hashed_password, name, status_id) VALUES (?1, ?2, ?3, ?4, 3)",
        )
        .bind(user_id.as_str())
        .bind(email.as_str())
        .bind(random_password_hash.as_str())
        .bind(username)
        .execute(&mut *tx)
        .await?;

        for organization in &req.organizations {
            let exists = sqlx::query("SELECT 1 FROM organizations WHERE id = ?1")
                .bind(organization.organization_id.as_str())
                .fetch_optional(&mut *tx)
                .await?
                .is_some();
            if !exists {
                return Err(AppError::OrganizationNotFound);
            }
            sqlx::query(
                "INSERT INTO organization_members (organization_id, user_id, role_id) VALUES (?1, ?2, ?3)",
            )
            .bind(organization.organization_id.as_str())
            .bind(user_id.as_str())
            .bind(organization.role_id)
            .execute(&mut *tx)
            .await?;
        }

        invitation::insert(&mut *tx, &invitation_id, &user_id, created_by, &token_hash)
            .await?;
        tx.commit().await?;
        Ok(())
    })
    .await?;

    let row = invitation::find_by_id(state, &invitation_id)
        .await?
        .ok_or_else(|| {
            AppError::InternalError("Invitation was created but could not be read".into())
        })?;
    Ok(CreatedInvitationDto {
        invitation: to_dto(state, row).await?,
        token,
    })
}

pub async fn list_invitations(state: &SharedState) -> Result<Vec<InvitationDto>, AppError> {
    let rows = invitation::list(state).await?;
    let mut result = Vec::with_capacity(rows.len());
    for row in rows {
        result.push(to_dto(state, row).await?);
    }
    Ok(result)
}

pub async fn list_invitations_for_creator(
    state: &SharedState,
    created_by: &str,
) -> Result<Vec<InvitationDto>, AppError> {
    let rows = invitation::list_for_creator(state, created_by).await?;
    let mut result = Vec::with_capacity(rows.len());
    for row in rows {
        result.push(to_dto(state, row).await?);
    }
    Ok(result)
}

pub async fn inspect_invitation(
    state: &SharedState,
    token: &str,
) -> Result<InvitationDetailsDto, AppError> {
    let row = invitation::find_by_token_hash(state, &hash_invitation_token(token))
        .await?
        .ok_or(AppError::InvitationNotFound)?;
    let current_status = status(&row);
    if current_status != "pending" {
        return Err(row_state_error(&row));
    }
    Ok(InvitationDetailsDto {
        username: row.username,
        email: row.email,
        status: current_status.into(),
    })
}

pub async fn revoke_invitation(
    state: &SharedState,
    created_by: &str,
    id: &str,
) -> Result<(), AppError> {
    let row = if state.is_cloud {
        invitation::find_by_id_for_creator(state, id, created_by).await
    } else {
        invitation::find_by_id(state, id).await
    }?
    .ok_or(AppError::InvitationNotFound)?;
    if status(&row) != "pending" {
        return Err(row_state_error(&row));
    }
    let changed = if state.is_cloud {
        invitation::revoke_for_creator(state, id, created_by).await
    } else {
        invitation::revoke(state, id).await
    }?;
    if changed == 0 {
        return Err(AppError::InvitationAlreadyAccepted);
    }
    Ok(())
}

pub async fn regenerate_invitation(
    state: &SharedState,
    created_by: &str,
    id: &str,
) -> Result<CreatedInvitationDto, AppError> {
    let row = if state.is_cloud {
        invitation::find_by_id_for_creator(state, id, created_by).await
    } else {
        invitation::find_by_id(state, id).await
    }?
    .ok_or(AppError::InvitationNotFound)?;
    if row.accepted_at.is_some() {
        return Err(AppError::InvitationAlreadyAccepted);
    }
    let token = new_invitation_token();
    let changed = if state.is_cloud {
        invitation::regenerate_for_creator(state, id, created_by, &hash_invitation_token(&token))
            .await
    } else {
        invitation::regenerate(state, id, &hash_invitation_token(&token)).await
    }?;
    if changed == 0 {
        return Err(AppError::InvitationAlreadyAccepted);
    }
    let updated = invitation::find_by_id(state, id)
        .await?
        .ok_or(AppError::InvitationNotFound)?;
    Ok(CreatedInvitationDto {
        invitation: to_dto(state, updated).await?,
        token,
    })
}

pub async fn accept_invitation(
    state: &SharedState,
    token: &str,
    password: &str,
    user_agent: Option<&str>,
) -> Result<(AuthResponse, String), AppError> {
    let token_hash = hash_invitation_token(token);
    let password_hash = security::hash_password(password.to_string());
    let (user_id, email, username, session_token) =
        crate::git::repositories::db::retry_on_stale_snapshot(|| async {
            let conn = crate::git::repositories::db::get_conn(state).await;
            let mut tx = conn.begin().await?;
            let row = invitation::find_by_token_hash_on(&mut *tx, &token_hash)
                .await?
                .ok_or(AppError::InvitationNotFound)?;
            if status(&row) != "pending" {
                return Err(row_state_error(&row));
            }

            let changed = invitation::mark_accepted(&mut *tx, &row.id)
                .await?;
            if changed == 0 {
                return Err(AppError::InvitationAlreadyAccepted);
            }
            sqlx::query(
                "UPDATE users SET hashed_password = ?1, status_id = 1 WHERE id = ?2 AND status_id = 3",
            )
            .bind(password_hash.as_str())
            .bind(row.user_id.as_str())
            .execute(&mut *tx)
            .await?;

            let session_id = Uuid::now_v7().to_string();
            // Same rules as `cqrs::commands::auth::create_session`, which this
            // duplicates because it has to run inside the acceptance transaction:
            // 256 CSPRNG bits for the token, and only its digest is stored.
            let session_token = crate::utils::security::generate_token();
            let expires_at = (Utc::now() + Duration::days(3))
                .naive_utc()
                .format("%Y-%m-%d %H:%M:%S")
                .to_string();
            sqlx::query(
                "INSERT INTO sessions (id, user_id, token_digest, expires_at, user_agent) VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .bind(session_id)
            .bind(row.user_id.as_str())
            .bind(crate::utils::security::digest_token(&session_token))
            .bind(expires_at)
            .bind(user_agent)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;

            Ok((row.user_id, row.email, row.username, session_token))
        })
        .await?;

    Ok((
        AuthResponse {
            user: UserClaims {
                user_id,
                email,
                name: username,
                is_sysadmin: false,
            },
        },
        session_token,
    ))
}

#[cfg(test)]
mod tests {
    use super::{hash_invitation_token, new_invitation_token, validate_create};
    use crate::dtos::invitation::{CreateInvitationRequest, InvitationOrganizationRequest};
    use crate::dtos::response::AppError;

    #[test]
    fn invitation_request_rejects_duplicate_organizations() {
        let request = CreateInvitationRequest {
            username: "pat".into(),
            email: "pat@example.test".into(),
            organizations: vec![
                InvitationOrganizationRequest {
                    organization_id: "t1".into(),
                    role_id: 3,
                },
                InvitationOrganizationRequest {
                    organization_id: "t1".into(),
                    role_id: 2,
                },
            ],
        };
        assert!(matches!(
            validate_create(&request),
            Err(AppError::ValidationError(message)) if message == "Each organization may appear once"
        ));
    }

    #[test]
    fn tokens_are_url_safe_and_hash_deterministically() {
        let token = new_invitation_token();
        assert!(token.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(
            hash_invitation_token("token"),
            hash_invitation_token("token")
        );
        assert_ne!(
            hash_invitation_token("token"),
            hash_invitation_token("other")
        );
    }
}
