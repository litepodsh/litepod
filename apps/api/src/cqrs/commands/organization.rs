use uuid::Uuid;

use crate::dtos::organization::{
    CreateOrganizationRequest, OrganizationDto, UpdateOrganizationRequest,
};
use crate::dtos::response::AppError;
use crate::git::repositories::{organization, organization_member};
use crate::security::authz::guard::{commit_if_admin_remains, ensure_can_grant};
use crate::security::authz::permission::system_role_permissions;
use crate::security::authz::{PermissionSet, Target};
use crate::state::state::SharedState;

pub async fn create_organization(
    state: &SharedState,
    user_id: &str,
    req: &CreateOrganizationRequest,
) -> Result<OrganizationDto, AppError> {
    if organization::get_organization_by_slug(state, &req.slug)
        .await
        .is_some()
    {
        return Err(AppError::AlreadyExists);
    }

    let id = Uuid::now_v7().to_string();
    let desc = req.description.as_deref().unwrap_or("");
    let icon = req
        .icon
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("shield");
    let icon_bg = req.icon_bg.as_deref().unwrap_or("");
    let plan = req
        .plan
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(&req.name);

    if state.is_cloud {
        crate::git::repositories::db::retry_on_stale_snapshot(|| async {
            let conn = crate::git::repositories::db::get_conn(state).await;
            let mut tx = conn.begin().await?;
            if organization::count_created_on(&mut *tx, user_id).await? >= 3 {
                return Err(AppError::ValidationError(
                    "Cloud users can create at most three organizations".into(),
                ));
            }
            organization::create_organization_on(
                &mut *tx, &id, &req.name, &req.slug, desc, icon, icon_bg, plan, user_id,
            )
            .await?;
            organization_member::add_member_on(&mut *tx, &id, user_id, 1).await?;
            tx.commit().await?;
            Ok(())
        })
        .await?;
    } else {
        organization::create_organization(
            state, &id, &req.name, &req.slug, desc, icon, icon_bg, plan, user_id,
        )
        .await
        .map_err(AppError::DatabaseError)?;

        organization_member::add_member(state, &id, user_id, 1)
            .await
            .map_err(AppError::DatabaseError)?;
    }

    let row = organization::get_organization_by_id(state, &id)
        .await
        .ok_or(AppError::InternalError(
            "Organization created but not found".into(),
        ))?;

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

pub async fn update_organization(
    state: &SharedState,
    organization_id: &str,
    req: &UpdateOrganizationRequest,
) -> Result<OrganizationDto, AppError> {
    let existing = organization::get_organization_by_id(state, organization_id)
        .await
        .ok_or(AppError::OrganizationNotFound)?;

    let name = req.name.as_deref().map(str::trim);
    if name.is_some_and(str::is_empty) {
        return Err(AppError::ValidationError(
            "Organization name is required".into(),
        ));
    }
    // Matches the migration's own CHECK (1..=365) — reject here so a bad value
    // 400s instead of surfacing as a raw constraint-violation 500.
    if req
        .audit_logs_retention_days
        .is_some_and(|days| !(1..=365).contains(&days))
    {
        return Err(AppError::ValidationError(
            "Audit log retention must be between 1 and 365 days".into(),
        ));
    }

    organization::update_organization(
        state,
        organization_id,
        name,
        req.description.as_deref(),
        req.icon.as_deref(),
        req.icon_bg.as_deref(),
        req.plan.as_deref(),
        req.audit_logs_enabled,
        req.audit_logs_retention_days,
        req.assistant_enabled,
    )
    .await
    .map_err(|e| AppError::DatabaseError(e))?;

    if let Some(enabled) = req.mcp_enabled {
        organization::set_mcp_enabled(state, organization_id, enabled)
            .await
            .map_err(AppError::DatabaseError)?;
    }

    let row = organization::get_organization_by_id(state, organization_id)
        .await
        .ok_or(AppError::OrganizationNotFound)?;

    Ok(OrganizationDto {
        id: row.id,
        name: row.name,
        slug: existing.slug,
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

pub async fn delete_organization(
    state: &SharedState,
    organization_id: &str,
) -> Result<(), AppError> {
    if let Some(organization) = organization::get_organization_by_id(state, organization_id).await {
        if organization.is_root {
            return Err(AppError::InsufficientPermissions);
        }
    }
    organization::delete_organization(state, organization_id)
        .await
        .map_err(|e| AppError::DatabaseError(e))
}

#[allow(dead_code)] // public command surface for the upcoming invite-accept flow.
pub async fn add_member(
    state: &SharedState,
    organization_id: &str,
    target_user_id: &str,
    role_id: i64,
) -> Result<(), AppError> {
    if organization_member::is_member(state, organization_id, target_user_id).await {
        return Err(AppError::AlreadyMember);
    }
    organization_member::add_member(state, organization_id, target_user_id, role_id)
        .await
        .map_err(|e| AppError::DatabaseError(e))
}

/// The member's base role, or `NotOrganizationMember`.
async fn member_role(
    conn: &mut sqlx::SqliteConnection,
    organization_id: &str,
    user_id: &str,
) -> Result<i64, AppError> {
    sqlx::query_scalar(
        "SELECT role_id FROM organization_members WHERE organization_id = ?1 AND user_id = ?2",
    )
    .bind(organization_id)
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?
    .ok_or(AppError::NotOrganizationMember)
}

/// Removing a member takes their base role away, so the caller must hold it
/// (anti-escalation) and an administrator must remain.
pub async fn remove_member(
    state: &SharedState,
    caller: &PermissionSet,
    organization_id: &str,
    target_user_id: &str,
) -> Result<(), AppError> {
    let mut tx = state
        .db
        .begin()
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let current = member_role(&mut tx, organization_id, target_user_id).await?;
    ensure_can_grant(caller, &system_role_permissions(current), Target::Org)?;
    // The rbac_on_member_leave trigger drops their assignments and group seats.
    sqlx::query("DELETE FROM organization_members WHERE organization_id = ?1 AND user_id = ?2")
        .bind(organization_id)
        .bind(target_user_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    commit_if_admin_remains(tx, organization_id).await
}

pub async fn update_member_role(
    state: &SharedState,
    caller: &PermissionSet,
    organization_id: &str,
    target_user_id: &str,
    role_id: i64,
) -> Result<(), AppError> {
    // Base roles span Admin..=Member — ROLE_SYSADMIN (0) is a global flag on the
    // user, not something a per-organization row can grant. Without this an Admin
    // could write an out-of-range role_id and corrupt the invariant every read of
    // this table relies on.
    if !(1..=4).contains(&role_id) {
        return Err(AppError::ValidationError(
            "role_id must be 1 (Admin), 2 (Editor), 3 (Viewer) or 4 (Member)".into(),
        ));
    }
    let mut tx = state
        .db
        .begin()
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let current = member_role(&mut tx, organization_id, target_user_id).await?;
    // Both directions: no promoting past yourself, no demoting someone above you.
    ensure_can_grant(caller, &system_role_permissions(current), Target::Org)?;
    ensure_can_grant(caller, &system_role_permissions(role_id), Target::Org)?;
    sqlx::query(
        "UPDATE organization_members SET role_id = ?1 WHERE organization_id = ?2 AND user_id = ?3",
    )
    .bind(role_id)
    .bind(organization_id)
    .bind(target_user_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    commit_if_admin_remains(tx, organization_id).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{cleanup_database, set_base_role, test_state};

    #[tokio::test]
    async fn base_role_changes_follow_the_access_rules() {
        let (state, path) = test_state().await;
        set_base_role(&state, "organization-a", "editor", 1).await;
        let admin = PermissionSet::for_base_role(1);
        let editor = PermissionSet::for_base_role(2);

        // Member (4) is now a valid base role.
        update_member_role(&state, &admin, "organization-a", "viewer", 4)
            .await
            .unwrap();
        // An Editor cannot promote to Admin, nor demote an Admin.
        assert!(matches!(
            update_member_role(&state, &editor, "organization-a", "viewer", 1).await,
            Err(AppError::InsufficientPermissions)
        ));
        assert!(matches!(
            remove_member(&state, &editor, "organization-a", "editor").await,
            Err(AppError::InsufficientPermissions)
        ));
        // The only Admin can neither step down nor be removed.
        assert!(matches!(
            update_member_role(&state, &admin, "organization-a", "editor", 2).await,
            Err(AppError::ResourceInUse(_))
        ));
        assert!(matches!(
            remove_member(&state, &admin, "organization-a", "editor").await,
            Err(AppError::ResourceInUse(_))
        ));
        assert!(matches!(
            update_member_role(&state, &admin, "organization-a", "viewer", 0).await,
            Err(AppError::ValidationError(_))
        ));
        // With a second Admin, the first may go.
        update_member_role(&state, &admin, "organization-a", "viewer", 1)
            .await
            .unwrap();
        remove_member(&state, &admin, "organization-a", "editor")
            .await
            .unwrap();
        cleanup_database(&path);
    }
}
