use crate::dtos::organization::{OrganizationDto, OrganizationMemberDto};
use crate::dtos::response::AppError;
use crate::git::repositories::{organization, organization_member};
use crate::state::state::SharedState;

pub async fn list_my_organizations(
    state: &SharedState,
    user_id: &str,
) -> Result<Vec<OrganizationDto>, AppError> {
    let rows = organization::list_organizations_for_user(state, user_id)
        .await
        .map_err(|e| AppError::DatabaseError(e))?;

    Ok(rows
        .into_iter()
        .map(|r| OrganizationDto {
            id: r.id,
            name: r.name,
            slug: r.slug,
            description: r.description,
            icon: r.icon,
            icon_bg: r.icon_bg,
            plan: r.plan,
            is_root: r.is_root,
            audit_logs_enabled: r.audit_logs_enabled,
            assistant_enabled: r.assistant_enabled,
            mcp_enabled: r.mcp_enabled,
            audit_logs_retention_days: r.audit_logs_retention_days,
            created_at: r.created_at,
        })
        .collect())
}

pub async fn list_all_organizations(state: &SharedState) -> Result<Vec<OrganizationDto>, AppError> {
    let rows = organization::list_all_organizations(state)
        .await
        .map_err(AppError::DatabaseError)?;
    Ok(rows
        .into_iter()
        .map(|r| OrganizationDto {
            id: r.id,
            name: r.name,
            slug: r.slug,
            description: r.description,
            icon: r.icon,
            icon_bg: r.icon_bg,
            plan: r.plan,
            is_root: r.is_root,
            audit_logs_enabled: r.audit_logs_enabled,
            assistant_enabled: r.assistant_enabled,
            mcp_enabled: r.mcp_enabled,
            audit_logs_retention_days: r.audit_logs_retention_days,
            created_at: r.created_at,
        })
        .collect())
}

pub async fn get_organization(
    state: &SharedState,
    organization_id: &str,
) -> Result<OrganizationDto, AppError> {
    let row = organization::get_organization_by_id(state, organization_id)
        .await
        .ok_or(AppError::OrganizationNotFound)?;

    Ok(OrganizationDto {
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
}

pub async fn list_members(
    state: &SharedState,
    organization_id: &str,
) -> Result<Vec<OrganizationMemberDto>, AppError> {
    let rows = organization_member::list_members(state, organization_id)
        .await
        .map_err(|e| AppError::DatabaseError(e))?;

    Ok(rows
        .into_iter()
        .map(|r| OrganizationMemberDto {
            user_id: r.user_id,
            email: r.email,
            name: r.name,
            role_id: r.role_id,
            role_name: r.role_name,
            joined_at: r.joined_at,
            disabled: r.disabled,
        })
        .collect())
}
