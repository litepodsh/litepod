//! Access-control management:
//! custom roles, local groups, scoped assignments, the permission catalog and a
//! member's effective permissions. Every rule lives in `security::authz::guard`;
//! these handlers resolve ids, call it, write in one transaction and audit.

use std::collections::BTreeSet;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{Row, SqliteConnection};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::dtos::response::{AppError, created, ok};
use crate::git::repositories::audit_log;
use crate::security::authz::extract::{RequireOrg, perm};
use crate::security::authz::guard::{
    RoleRef, commit_if_admin_remains, ensure_can_grant, ensure_scope_fits, parse_catalog,
    role_permissions, system_role_name,
};
use crate::security::authz::permission::system_role_permissions;
use crate::security::authz::set::{PermissionSetDto, load};
use crate::security::authz::{Permission, PermissionSet, Target};
use crate::security::middleware::auth::AuthUser;
use crate::security::middleware::org_scope::OrgScope;
use crate::state::state::SharedState;

const SYSTEM_ROLE_DESCRIPTIONS: [(i64, &str); 4] = [
    (1, "Everything in the organization"),
    (2, "Create, change and deploy projects and applications"),
    (3, "Read-only access"),
    (
        4,
        "Organization membership only; access comes from assignments",
    ),
];

fn db(e: impl ToString) -> AppError {
    let message = e.to_string();
    if message.to_ascii_lowercase().contains("unique constraint") {
        AppError::AlreadyExists
    } else {
        AppError::DatabaseError(message)
    }
}

fn names(perms: &BTreeSet<Permission>) -> Vec<String> {
    perms.iter().map(|p| p.as_str().to_string()).collect()
}

async fn audit(
    state: &SharedState,
    org: &OrgScope,
    action: &str,
    target_type: &str,
    target_id: &str,
    target_name: &str,
    metadata: serde_json::Value,
) {
    audit_log::record(
        state,
        &org.organization_id,
        &org.user,
        action,
        Some(target_type),
        Some(target_id),
        Some(target_name),
        Some(&metadata.to_string()),
        None,
        None,
        None,
    )
    .await
    .ok();
}

// ── Scopes ──────────────────────────────────────────────────────────────────

/// A scope proven to belong to the organization.
struct Scope {
    scope_type: String,
    scope_id: String,
    /// Owning project, for an environment scope.
    project_id: Option<String>,
}

impl Scope {
    fn target(&self) -> Target<'_> {
        match (self.scope_type.as_str(), &self.project_id) {
            ("project", _) => Target::Project(&self.scope_id),
            ("environment", Some(project_id)) => Target::Environment {
                project_id,
                environment_id: &self.scope_id,
            },
            _ => Target::Org,
        }
    }
}

/// 404 for a scope id that is not in this organization, so ids from other
/// tenants are indistinguishable from missing ones.
async fn resolve_scope(
    conn: &mut SqliteConnection,
    organization_id: &str,
    scope_type: &str,
    scope_id: &str,
) -> Result<Scope, AppError> {
    let project_id = match scope_type {
        "org" if scope_id == organization_id => None,
        "org" => return Err(AppError::OrganizationNotFound),
        "project" => {
            let found: Option<i64> =
                sqlx::query_scalar("SELECT 1 FROM projects WHERE id = ?1 AND organization_id = ?2")
                    .bind(scope_id)
                    .bind(organization_id)
                    .fetch_optional(&mut *conn)
                    .await
                    .map_err(db)?;
            found.ok_or(AppError::ProjectNotFound)?;
            None
        }
        "environment" => Some(
            sqlx::query_scalar::<_, String>(
                "SELECT e.project_id FROM environments e JOIN projects p ON p.id = e.project_id
                  WHERE e.id = ?1 AND p.organization_id = ?2",
            )
            .bind(scope_id)
            .bind(organization_id)
            .fetch_optional(&mut *conn)
            .await
            .map_err(db)?
            .ok_or(AppError::EnvironmentNotFound)?,
        ),
        other => {
            return Err(AppError::ValidationError(format!(
                "scope_type must be org, project or environment, not {other}"
            )));
        }
    };
    Ok(Scope {
        scope_type: scope_type.to_string(),
        scope_id: scope_id.to_string(),
        project_id,
    })
}

/// Rule 4 for one assignment: the caller must be able to grant `role` at
/// `scope`. Used for creating and for removing (and, through a group, for
/// changing who holds it).
async fn ensure_can_grant_assignment(
    conn: &mut SqliteConnection,
    org: &OrgScope,
    role: &RoleRef,
    scope: &Scope,
) -> Result<BTreeSet<Permission>, AppError> {
    let perms = role_permissions(conn, &org.organization_id, role).await?;
    ensure_can_grant(&org.permissions, &perms, scope.target())?;
    Ok(perms)
}

/// Every assignment a group holds, as (role, scope), for rule 4 on group
/// membership changes and deletion.
async fn ensure_can_grant_group(
    conn: &mut SqliteConnection,
    org: &OrgScope,
    group_id: &str,
) -> Result<(), AppError> {
    let rows = sqlx::query(
        "SELECT system_role_id, custom_role_id, scope_type, scope_id FROM rbac_assignments
          WHERE organization_id = ?1 AND principal_type = 'group' AND principal_id = ?2",
    )
    .bind(&org.organization_id)
    .bind(group_id)
    .fetch_all(&mut *conn)
    .await
    .map_err(db)?;
    for row in rows {
        let role = role_ref(row.get(0), row.get(1));
        let scope_type: String = row.get(2);
        let scope_id: String = row.get(3);
        let scope = resolve_scope(conn, &org.organization_id, &scope_type, &scope_id).await?;
        ensure_can_grant_assignment(conn, org, &role, &scope).await?;
    }
    Ok(())
}

fn role_ref(system_role_id: Option<i64>, custom_role_id: Option<String>) -> RoleRef {
    match (system_role_id, custom_role_id) {
        (Some(id), _) => RoleRef::System(id),
        (None, Some(id)) => RoleRef::Custom(id),
        (None, None) => RoleRef::Custom(String::new()),
    }
}

// ── Catalog ─────────────────────────────────────────────────────────────────

#[derive(Serialize, ToSchema)]
pub struct CatalogEntryDto {
    pub name: String,
    /// Grantable only at organization scope.
    pub org_only: bool,
}

#[utoipa::path(
    get, path = "/api/v1/organizations/{organization_id}/permissions/catalog",
    params(("organization_id" = String, Path, description = "Organization UUID")),
    responses((status = 200, description = "Every permission a role can hold", body = [CatalogEntryDto])),
    tag = "access"
)]
pub async fn catalog_handler(
    RequireOrg(_org, _): RequireOrg<perm::OrgRead>,
) -> Result<impl IntoResponse, AppError> {
    let entries: Vec<CatalogEntryDto> = Permission::ALL
        .iter()
        .map(|p| CatalogEntryDto {
            name: p.as_str().to_string(),
            org_only: p.org_only(),
        })
        .collect();
    Ok(ok(entries))
}

// ── Roles ───────────────────────────────────────────────────────────────────

#[derive(Serialize, ToSchema)]
pub struct RoleDto {
    /// `"1"`..`"4"` for system roles, a UUID for custom ones.
    pub id: String,
    pub name: String,
    pub description: String,
    pub system: bool,
    pub permissions: Vec<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct RoleInput {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub permissions: Vec<String>,
}

#[derive(Deserialize, IntoParams)]
pub struct DeleteRoleQuery {
    /// Also delete the role's assignments.
    #[serde(default)]
    pub force: bool,
}

async fn list_custom_roles(
    conn: &mut SqliteConnection,
    organization_id: &str,
    only: Option<&str>,
) -> Result<Vec<RoleDto>, AppError> {
    let rows = sqlx::query(
        "SELECT r.id, r.name, r.description, rp.permission FROM rbac_roles r
           LEFT JOIN rbac_role_permissions rp ON rp.role_id = r.id
          WHERE r.organization_id = ?1 AND (?2 IS NULL OR r.id = ?2)
          ORDER BY r.name, rp.permission",
    )
    .bind(organization_id)
    .bind(only)
    .fetch_all(&mut *conn)
    .await
    .map_err(db)?;
    let mut roles: Vec<RoleDto> = Vec::new();
    for row in rows {
        let id: String = row.get(0);
        if roles.last().is_none_or(|r| r.id != id) {
            roles.push(RoleDto {
                id,
                name: row.get(1),
                description: row.get(2),
                system: false,
                permissions: Vec::new(),
            });
        }
        if let Some(p) = row.get::<Option<String>, _>(3) {
            roles.last_mut().unwrap().permissions.push(p);
        }
    }
    Ok(roles)
}

/// The custom role `role_id` of this organization, or 404; system ids → 409.
async fn custom_role(
    conn: &mut SqliteConnection,
    organization_id: &str,
    role_id: &str,
) -> Result<RoleDto, AppError> {
    if !RoleRef::parse(role_id).is_custom() {
        return Err(AppError::ResourceInUse(
            "System roles cannot be changed or deleted".into(),
        ));
    }
    list_custom_roles(conn, organization_id, Some(role_id))
        .await?
        .pop()
        .ok_or(AppError::ResourceNotFound)
}

fn validated_name(name: &str) -> Result<String, AppError> {
    let name = name.trim();
    if name.is_empty() || name.len() > 64 {
        return Err(AppError::ValidationError(
            "Name must be 1 to 64 characters".into(),
        ));
    }
    Ok(name.to_string())
}

async fn insert_permissions(
    conn: &mut SqliteConnection,
    role_id: &str,
    perms: &BTreeSet<Permission>,
) -> Result<(), AppError> {
    for p in perms {
        sqlx::query("INSERT INTO rbac_role_permissions (role_id, permission) VALUES (?1, ?2)")
            .bind(role_id)
            .bind(p.as_str())
            .execute(&mut *conn)
            .await
            .map_err(db)?;
    }
    Ok(())
}

#[utoipa::path(
    get, path = "/api/v1/organizations/{organization_id}/roles",
    params(("organization_id" = String, Path, description = "Organization UUID")),
    responses((status = 200, description = "System and custom roles", body = [RoleDto])),
    tag = "access"
)]
pub async fn list_roles_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgRead>,
) -> Result<impl IntoResponse, AppError> {
    let mut roles: Vec<RoleDto> = SYSTEM_ROLE_DESCRIPTIONS
        .iter()
        .map(|(id, description)| RoleDto {
            id: id.to_string(),
            name: system_role_name(*id).to_string(),
            description: description.to_string(),
            system: true,
            permissions: names(&system_role_permissions(*id)),
        })
        .collect();
    let mut conn = state.db.acquire().await.map_err(db)?;
    roles.extend(list_custom_roles(&mut conn, &org.organization_id, None).await?);
    Ok(ok(roles))
}

#[utoipa::path(
    post, path = "/api/v1/organizations/{organization_id}/roles",
    params(("organization_id" = String, Path, description = "Organization UUID")),
    request_body = RoleInput,
    responses(
        (status = 201, description = "Role created", body = RoleDto),
        (status = 403, description = "The role holds a permission the caller lacks"),
        (status = 409, description = "Name taken"),
        (status = 422, description = "Unknown permission or bad name"),
    ),
    tag = "access"
)]
pub async fn create_role_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgRolesManage>,
    Json(input): Json<RoleInput>,
) -> Result<impl IntoResponse, AppError> {
    let name = validated_name(&input.name)?;
    let perms = parse_catalog(&input.permissions)?;
    ensure_can_grant(&org.permissions, &perms, Target::Org)?;

    let id = Uuid::now_v7().to_string();
    let mut tx = state.db.begin().await.map_err(db)?;
    sqlx::query(
        "INSERT INTO rbac_roles (id, organization_id, name, description) VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(&id)
    .bind(&org.organization_id)
    .bind(&name)
    .bind(input.description.trim())
    .execute(&mut *tx)
    .await
    .map_err(db)?;
    insert_permissions(&mut tx, &id, &perms).await?;
    tx.commit().await.map_err(db)?;

    audit(
        &state,
        &org,
        "role.created",
        "role",
        &id,
        &name,
        json!({ "permissions": names(&perms) }),
    )
    .await;
    Ok(created(RoleDto {
        id,
        name,
        description: input.description.trim().to_string(),
        system: false,
        permissions: names(&perms),
    }))
}

#[utoipa::path(
    patch, path = "/api/v1/organizations/{organization_id}/roles/{role_id}",
    params(
        ("organization_id" = String, Path, description = "Organization UUID"),
        ("role_id" = String, Path, description = "Custom role id"),
    ),
    request_body = RoleInput,
    responses(
        (status = 200, description = "Role replaced", body = RoleDto),
        (status = 403, description = "Old or new permissions exceed the caller's"),
        (status = 409, description = "System role, name taken, or last administrator"),
        (status = 422, description = "Unknown permission, or org-only permission on a role assigned below org scope"),
    ),
    tag = "access"
)]
pub async fn update_role_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgRolesManage>,
    Path((_organization_id, role_id)): Path<(String, String)>,
    Json(input): Json<RoleInput>,
) -> Result<impl IntoResponse, AppError> {
    let name = validated_name(&input.name)?;
    let perms = parse_catalog(&input.permissions)?;
    let mut tx = state.db.begin().await.map_err(db)?;
    let before = custom_role(&mut tx, &org.organization_id, &role_id).await?;
    let old = parse_catalog(&before.permissions).unwrap_or_default();
    // Editing a role changes what its holders can do, in both directions.
    ensure_can_grant(&org.permissions, &old, Target::Org)?;
    ensure_can_grant(&org.permissions, &perms, Target::Org)?;

    let scoped: Option<String> = sqlx::query_scalar(
        "SELECT scope_type FROM rbac_assignments WHERE custom_role_id = ?1 AND scope_type <> 'org' LIMIT 1",
    )
    .bind(&role_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(db)?;
    if let Some(scope_type) = scoped {
        ensure_scope_fits(&perms, &scope_type, &RoleRef::Custom(role_id.clone()))?;
    }

    sqlx::query("UPDATE rbac_roles SET name = ?1, description = ?2 WHERE id = ?3")
        .bind(&name)
        .bind(input.description.trim())
        .bind(&role_id)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    sqlx::query("DELETE FROM rbac_role_permissions WHERE role_id = ?1")
        .bind(&role_id)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    insert_permissions(&mut tx, &role_id, &perms).await?;
    commit_if_admin_remains(tx, &org.organization_id).await?;

    audit(
        &state,
        &org,
        "role.updated",
        "role",
        &role_id,
        &name,
        json!({ "before": before.permissions, "after": names(&perms), "previous_name": before.name }),
    )
    .await;
    Ok(ok(RoleDto {
        id: role_id,
        name,
        description: input.description.trim().to_string(),
        system: false,
        permissions: names(&perms),
    }))
}

#[utoipa::path(
    delete, path = "/api/v1/organizations/{organization_id}/roles/{role_id}",
    params(
        ("organization_id" = String, Path, description = "Organization UUID"),
        ("role_id" = String, Path, description = "Custom role id"),
        DeleteRoleQuery,
    ),
    responses(
        (status = 204, description = "Role deleted"),
        (status = 409, description = "System role, role in use without force, or last administrator"),
    ),
    tag = "access"
)]
pub async fn delete_role_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgRolesManage>,
    Path((_organization_id, role_id)): Path<(String, String)>,
    Query(query): Query<DeleteRoleQuery>,
) -> Result<impl IntoResponse, AppError> {
    let mut tx = state.db.begin().await.map_err(db)?;
    let role = custom_role(&mut tx, &org.organization_id, &role_id).await?;
    let perms = parse_catalog(&role.permissions).unwrap_or_default();
    ensure_can_grant(&org.permissions, &perms, Target::Org)?;

    let uses: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM rbac_assignments WHERE custom_role_id = ?1")
            .bind(&role_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(db)?;
    if uses > 0 && !query.force {
        return Err(AppError::ResourceInUse(format!(
            "Role is assigned {uses} time(s); delete with force=true to remove those assignments"
        )));
    }
    // Assignments go with it (ON DELETE CASCADE).
    sqlx::query("DELETE FROM rbac_roles WHERE id = ?1")
        .bind(&role_id)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    commit_if_admin_remains(tx, &org.organization_id).await?;

    audit(
        &state,
        &org,
        "role.deleted",
        "role",
        &role_id,
        &role.name,
        json!({ "permissions": role.permissions, "assignments_removed": uses }),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

// ── Groups ──────────────────────────────────────────────────────────────────

#[derive(Serialize, ToSchema)]
pub struct GroupMemberDto {
    pub user_id: String,
    pub name: String,
    pub email: String,
}

#[derive(Serialize, ToSchema)]
pub struct GroupDto {
    pub id: String,
    pub name: String,
    /// `local`, or `idp` / `scim` when an identity provider owns it.
    pub source: String,
    pub members: Vec<GroupMemberDto>,
}

#[derive(Deserialize, ToSchema)]
pub struct GroupInput {
    pub name: String,
}

#[derive(Deserialize, ToSchema)]
pub struct GroupMembersInput {
    pub user_ids: Vec<String>,
}

async fn list_groups(
    conn: &mut SqliteConnection,
    organization_id: &str,
    only: Option<&str>,
) -> Result<Vec<GroupDto>, AppError> {
    let rows = sqlx::query(
        "SELECT g.id, g.name, g.source, u.id, u.name, u.email FROM rbac_groups g
           LEFT JOIN rbac_group_members gm ON gm.group_id = g.id
           LEFT JOIN users u ON u.id = gm.user_id
          WHERE g.organization_id = ?1 AND (?2 IS NULL OR g.id = ?2)
          ORDER BY g.name, u.name",
    )
    .bind(organization_id)
    .bind(only)
    .fetch_all(&mut *conn)
    .await
    .map_err(db)?;
    let mut groups: Vec<GroupDto> = Vec::new();
    for row in rows {
        let id: String = row.get(0);
        if groups.last().is_none_or(|g| g.id != id) {
            groups.push(GroupDto {
                id,
                name: row.get(1),
                source: row.get(2),
                members: Vec::new(),
            });
        }
        if let Some(user_id) = row.get::<Option<String>, _>(3) {
            groups.last_mut().unwrap().members.push(GroupMemberDto {
                user_id,
                name: row.get(4),
                email: row.get(5),
            });
        }
    }
    Ok(groups)
}

/// The group, or 404. `writable` additionally refuses (409) groups an identity
/// provider owns (rule 7); their assignments stay editable.
async fn group(
    conn: &mut SqliteConnection,
    organization_id: &str,
    group_id: &str,
    writable: bool,
) -> Result<GroupDto, AppError> {
    let group = list_groups(conn, organization_id, Some(group_id))
        .await?
        .pop()
        .ok_or(AppError::ResourceNotFound)?;
    if writable && group.source != "local" {
        return Err(AppError::ResourceInUse(format!(
            "This group is managed by your identity provider ({})",
            group.source
        )));
    }
    Ok(group)
}

#[utoipa::path(
    get, path = "/api/v1/organizations/{organization_id}/groups",
    params(("organization_id" = String, Path, description = "Organization UUID")),
    responses((status = 200, description = "Groups with their members", body = [GroupDto])),
    tag = "access"
)]
pub async fn list_groups_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgRead>,
) -> Result<impl IntoResponse, AppError> {
    let mut conn = state.db.acquire().await.map_err(db)?;
    Ok(ok(list_groups(&mut conn, &org.organization_id, None).await?))
}

#[utoipa::path(
    get, path = "/api/v1/organizations/{organization_id}/groups/{group_id}",
    params(
        ("organization_id" = String, Path, description = "Organization UUID"),
        ("group_id" = String, Path, description = "Group id"),
    ),
    responses((status = 200, description = "One group", body = GroupDto)),
    tag = "access"
)]
pub async fn get_group_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgRead>,
    Path((_organization_id, group_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let mut conn = state.db.acquire().await.map_err(db)?;
    Ok(ok(
        group(&mut conn, &org.organization_id, &group_id, false).await?
    ))
}

#[utoipa::path(
    post, path = "/api/v1/organizations/{organization_id}/groups",
    params(("organization_id" = String, Path, description = "Organization UUID")),
    request_body = GroupInput,
    responses((status = 201, description = "Group created", body = GroupDto)),
    tag = "access"
)]
pub async fn create_group_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgMembersManage>,
    Json(input): Json<GroupInput>,
) -> Result<impl IntoResponse, AppError> {
    let name = validated_name(&input.name)?;
    let id = Uuid::now_v7().to_string();
    sqlx::query("INSERT INTO rbac_groups (id, organization_id, name) VALUES (?1, ?2, ?3)")
        .bind(&id)
        .bind(&org.organization_id)
        .bind(&name)
        .execute(&state.db)
        .await
        .map_err(db)?;
    audit(
        &state,
        &org,
        "group.created",
        "group",
        &id,
        &name,
        json!({}),
    )
    .await;
    Ok(created(GroupDto {
        id,
        name,
        source: "local".into(),
        members: Vec::new(),
    }))
}

#[utoipa::path(
    patch, path = "/api/v1/organizations/{organization_id}/groups/{group_id}",
    params(
        ("organization_id" = String, Path, description = "Organization UUID"),
        ("group_id" = String, Path, description = "Group id"),
    ),
    request_body = GroupInput,
    responses((status = 200, description = "Group renamed", body = GroupDto)),
    tag = "access"
)]
pub async fn update_group_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgMembersManage>,
    Path((_organization_id, group_id)): Path<(String, String)>,
    Json(input): Json<GroupInput>,
) -> Result<impl IntoResponse, AppError> {
    let name = validated_name(&input.name)?;
    let mut conn = state.db.acquire().await.map_err(db)?;
    let mut current = group(&mut conn, &org.organization_id, &group_id, true).await?;
    sqlx::query("UPDATE rbac_groups SET name = ?1 WHERE id = ?2")
        .bind(&name)
        .bind(&group_id)
        .execute(&mut *conn)
        .await
        .map_err(db)?;
    audit(
        &state,
        &org,
        "group.updated",
        "group",
        &group_id,
        &name,
        json!({ "previous_name": current.name }),
    )
    .await;
    current.name = name;
    Ok(ok(current))
}

#[utoipa::path(
    delete, path = "/api/v1/organizations/{organization_id}/groups/{group_id}",
    params(
        ("organization_id" = String, Path, description = "Organization UUID"),
        ("group_id" = String, Path, description = "Group id"),
    ),
    responses(
        (status = 204, description = "Group and its assignments deleted"),
        (status = 409, description = "Managed by an identity provider, or last administrator"),
    ),
    tag = "access"
)]
pub async fn delete_group_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgMembersManage>,
    Path((_organization_id, group_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let mut tx = state.db.begin().await.map_err(db)?;
    let current = group(&mut tx, &org.organization_id, &group_id, true).await?;
    ensure_can_grant_group(&mut tx, &org, &group_id).await?;
    // The trigger on rbac_groups removes the group's assignments.
    sqlx::query("DELETE FROM rbac_groups WHERE id = ?1")
        .bind(&group_id)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    commit_if_admin_remains(tx, &org.organization_id).await?;
    audit(
        &state,
        &org,
        "group.deleted",
        "group",
        &group_id,
        &current.name,
        json!({ "members": current.members.iter().map(|m| &m.user_id).collect::<Vec<_>>() }),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    put, path = "/api/v1/organizations/{organization_id}/groups/{group_id}/members",
    params(
        ("organization_id" = String, Path, description = "Organization UUID"),
        ("group_id" = String, Path, description = "Group id"),
    ),
    request_body = GroupMembersInput,
    responses(
        (status = 200, description = "Members replaced", body = GroupDto),
        (status = 403, description = "The group holds a grant the caller could not give"),
        (status = 409, description = "Managed by an identity provider, or last administrator"),
        (status = 422, description = "A user is not a member of the organization"),
    ),
    tag = "access"
)]
pub async fn set_group_members_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgMembersManage>,
    Path((_organization_id, group_id)): Path<(String, String)>,
    Json(input): Json<GroupMembersInput>,
) -> Result<impl IntoResponse, AppError> {
    let user_ids: BTreeSet<String> = input.user_ids.into_iter().collect();
    let mut tx = state.db.begin().await.map_err(db)?;
    let before = group(&mut tx, &org.organization_id, &group_id, true).await?;
    // Adding a member hands them every grant of the group; removing one takes it away.
    ensure_can_grant_group(&mut tx, &org, &group_id).await?;

    sqlx::query("DELETE FROM rbac_group_members WHERE group_id = ?1")
        .bind(&group_id)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    for user_id in &user_ids {
        let inserted = sqlx::query(
            "INSERT INTO rbac_group_members (group_id, user_id)
             SELECT ?1, user_id FROM organization_members WHERE organization_id = ?2 AND user_id = ?3",
        )
        .bind(&group_id)
        .bind(&org.organization_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
        if inserted.rows_affected() == 0 {
            tx.rollback().await.map_err(db)?;
            return Err(AppError::ValidationError(format!(
                "{user_id} is not a member of this organization"
            )));
        }
    }
    let after = group(&mut tx, &org.organization_id, &group_id, false).await?;
    commit_if_admin_remains(tx, &org.organization_id).await?;

    audit(
        &state,
        &org,
        "group.members_changed",
        "group",
        &group_id,
        &after.name,
        json!({
            "before": before.members.iter().map(|m| &m.user_id).collect::<Vec<_>>(),
            "after": after.members.iter().map(|m| &m.user_id).collect::<Vec<_>>(),
        }),
    )
    .await;
    Ok(ok(after))
}

// ── Assignments ─────────────────────────────────────────────────────────────

#[derive(Serialize, ToSchema)]
pub struct AssignmentDto {
    pub id: String,
    /// `user` or `group`.
    pub principal_type: String,
    pub principal_id: String,
    pub principal_name: String,
    /// Same id space as `RoleDto.id`.
    pub role_id: String,
    pub role_name: String,
    /// `org`, `project` or `environment`.
    pub scope_type: String,
    pub scope_id: String,
    /// Organization, project, or "project / environment" name.
    pub scope_name: String,
    /// Owning project of an environment scope, so the UI can show inheritance.
    pub project_id: Option<String>,
    /// Set in effective permissions when the grant reaches the user through a group.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub via_group: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct AssignmentInput {
    pub principal_type: String,
    pub principal_id: String,
    pub role_id: String,
    pub scope_type: String,
    pub scope_id: String,
}

#[derive(Deserialize, IntoParams)]
pub struct AssignmentFilter {
    pub scope_type: Option<String>,
    pub scope_id: Option<String>,
    /// Only assignments reaching this user, directly or through a group.
    #[serde(skip)]
    pub user_id: Option<String>,
}

async fn list_assignments(
    conn: &mut SqliteConnection,
    organization_id: &str,
    filter: &AssignmentFilter,
    only: Option<&str>,
) -> Result<Vec<AssignmentDto>, AppError> {
    let rows = sqlx::query(
        "SELECT a.id, a.principal_type, a.principal_id,
                COALESCE(u.name, g.name, a.principal_id),
                a.system_role_id, a.custom_role_id, r.name,
                a.scope_type, a.scope_id, e.project_id,
                CASE WHEN a.principal_type = 'group' THEN g.name END,
                CASE a.scope_type WHEN 'org' THEN o.name WHEN 'project' THEN p.name
                     ELSE ep.name || ' / ' || e.name END
           FROM rbac_assignments a
           JOIN organizations o ON o.id = a.organization_id
           LEFT JOIN projects p ON a.scope_type = 'project' AND p.id = a.scope_id
           LEFT JOIN users u ON a.principal_type = 'user' AND u.id = a.principal_id
           LEFT JOIN rbac_groups g ON a.principal_type = 'group' AND g.id = a.principal_id
           LEFT JOIN rbac_roles r ON r.id = a.custom_role_id
           LEFT JOIN environments e ON a.scope_type = 'environment' AND e.id = a.scope_id
           LEFT JOIN projects ep ON ep.id = e.project_id
          WHERE a.organization_id = ?1
            AND (?2 IS NULL OR a.scope_type = ?2)
            AND (?3 IS NULL OR a.scope_id = ?3)
            AND (?4 IS NULL OR a.id = ?4)
            AND (?5 IS NULL
              OR (a.principal_type = 'user' AND a.principal_id = ?5)
              OR (a.principal_type = 'group' AND a.principal_id IN
                    (SELECT group_id FROM rbac_group_members WHERE user_id = ?5)))
          ORDER BY a.created_at",
    )
    .bind(organization_id)
    .bind(&filter.scope_type)
    .bind(&filter.scope_id)
    .bind(only)
    .bind(&filter.user_id)
    .fetch_all(&mut *conn)
    .await
    .map_err(db)?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let system: Option<i64> = row.get(4);
            let custom: Option<String> = row.get(5);
            AssignmentDto {
                id: row.get(0),
                principal_type: row.get(1),
                principal_id: row.get(2),
                principal_name: row.get(3),
                role_id: system
                    .map(|id| id.to_string())
                    .or(custom)
                    .unwrap_or_default(),
                role_name: system
                    .map(|id| system_role_name(id).to_string())
                    .or(row.get(6))
                    .unwrap_or_default(),
                scope_type: row.get(7),
                scope_id: row.get(8),
                scope_name: row.get::<Option<String>, _>(11).unwrap_or_default(),
                project_id: row.get(9),
                via_group: filter.user_id.as_ref().and(row.get(10)),
            }
        })
        .collect())
}

#[utoipa::path(
    get, path = "/api/v1/organizations/{organization_id}/assignments",
    params(("organization_id" = String, Path, description = "Organization UUID"), AssignmentFilter),
    responses((status = 200, description = "Assignments, optionally at one scope", body = [AssignmentDto])),
    tag = "access"
)]
pub async fn list_assignments_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgRead>,
    Query(filter): Query<AssignmentFilter>,
) -> Result<impl IntoResponse, AppError> {
    let mut conn = state.db.acquire().await.map_err(db)?;
    Ok(ok(list_assignments(
        &mut conn,
        &org.organization_id,
        &filter,
        None,
    )
    .await?))
}

#[utoipa::path(
    post, path = "/api/v1/organizations/{organization_id}/assignments",
    params(("organization_id" = String, Path, description = "Organization UUID")),
    request_body = AssignmentInput,
    responses(
        (status = 201, description = "Assignment created", body = AssignmentDto),
        (status = 403, description = "The caller cannot grant this role at this scope"),
        (status = 404, description = "Scope or role not in this organization"),
        (status = 409, description = "Already assigned"),
        (status = 422, description = "Bad principal, Member role, or org-only permissions below org scope"),
    ),
    tag = "access"
)]
pub async fn create_assignment_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgMembersManage>,
    Json(input): Json<AssignmentInput>,
) -> Result<impl IntoResponse, AppError> {
    let role = RoleRef::parse(&input.role_id);
    if role == RoleRef::System(4) {
        return Err(AppError::ValidationError(
            "Member is a base role; set it on the member instead".into(),
        ));
    }
    let mut tx = state.db.begin().await.map_err(db)?;
    let scope = resolve_scope(
        &mut tx,
        &org.organization_id,
        &input.scope_type,
        &input.scope_id,
    )
    .await?;

    let principal_sql = match input.principal_type.as_str() {
        "user" => "SELECT 1 FROM organization_members WHERE organization_id = ?1 AND user_id = ?2",
        "group" => "SELECT 1 FROM rbac_groups WHERE organization_id = ?1 AND id = ?2",
        _ => {
            return Err(AppError::ValidationError(
                "principal_type must be user or group".into(),
            ));
        }
    };
    let principal: Option<i64> = sqlx::query_scalar(principal_sql)
        .bind(&org.organization_id)
        .bind(&input.principal_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db)?;
    if principal.is_none() {
        return Err(AppError::ValidationError(format!(
            "{} is not a {} of this organization",
            input.principal_id,
            if input.principal_type == "user" {
                "member"
            } else {
                "group"
            }
        )));
    }

    let perms = ensure_can_grant_assignment(&mut tx, &org, &role, &scope).await?;
    ensure_scope_fits(&perms, &scope.scope_type, &role)?;

    let id = Uuid::now_v7().to_string();
    let (system_role_id, custom_role_id) = match &role {
        RoleRef::System(n) => (Some(*n), None),
        RoleRef::Custom(c) => (None, Some(c.clone())),
    };
    sqlx::query(
        "INSERT INTO rbac_assignments
            (id, organization_id, principal_type, principal_id, system_role_id, custom_role_id, scope_type, scope_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )
    .bind(&id)
    .bind(&org.organization_id)
    .bind(&input.principal_type)
    .bind(&input.principal_id)
    .bind(system_role_id)
    .bind(custom_role_id)
    .bind(&scope.scope_type)
    .bind(&scope.scope_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| match db(e) {
        AppError::AlreadyExists => {
            AppError::ResourceInUse("They already have this role at this scope".into())
        }
        other => other,
    })?;
    let created_row = list_assignments(
        &mut tx,
        &org.organization_id,
        &AssignmentFilter {
            scope_type: None,
            scope_id: None,
            user_id: None,
        },
        Some(&id),
    )
    .await?
    .pop()
    .ok_or(AppError::ResourceNotFound)?;
    tx.commit().await.map_err(db)?;

    audit(
        &state,
        &org,
        "assignment.created",
        "assignment",
        &id,
        &created_row.principal_name,
        assignment_metadata(&created_row),
    )
    .await;
    Ok(created(created_row))
}

fn assignment_metadata(a: &AssignmentDto) -> serde_json::Value {
    json!({
        "principal_type": a.principal_type,
        "principal_id": a.principal_id,
        "role_id": a.role_id,
        "role_name": a.role_name,
        "scope_type": a.scope_type,
        "scope_id": a.scope_id,
    })
}

#[utoipa::path(
    delete, path = "/api/v1/organizations/{organization_id}/assignments/{assignment_id}",
    params(
        ("organization_id" = String, Path, description = "Organization UUID"),
        ("assignment_id" = String, Path, description = "Assignment id"),
    ),
    responses(
        (status = 204, description = "Assignment removed"),
        (status = 403, description = "The caller could not have granted it"),
        (status = 409, description = "Last administrator"),
    ),
    tag = "access"
)]
pub async fn delete_assignment_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgMembersManage>,
    Path((_organization_id, assignment_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let mut tx = state.db.begin().await.map_err(db)?;
    let assignment = list_assignments(
        &mut tx,
        &org.organization_id,
        &AssignmentFilter {
            scope_type: None,
            scope_id: None,
            user_id: None,
        },
        Some(&assignment_id),
    )
    .await?
    .pop()
    .ok_or(AppError::ResourceNotFound)?;
    let scope = resolve_scope(
        &mut tx,
        &org.organization_id,
        &assignment.scope_type,
        &assignment.scope_id,
    )
    .await?;
    ensure_can_grant_assignment(&mut tx, &org, &RoleRef::parse(&assignment.role_id), &scope)
        .await?;
    sqlx::query("DELETE FROM rbac_assignments WHERE id = ?1")
        .bind(&assignment_id)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    commit_if_admin_remains(tx, &org.organization_id).await?;

    audit(
        &state,
        &org,
        "assignment.deleted",
        "assignment",
        &assignment_id,
        &assignment.principal_name,
        assignment_metadata(&assignment),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

// ── Effective permissions ───────────────────────────────────────────────────

#[derive(Serialize, ToSchema)]
pub struct EffectivePermissionsDto {
    pub base_role_id: i64,
    pub base_role_name: String,
    pub permissions: PermissionSetDto,
    /// Every assignment reaching the user; `via_group` set when through a group.
    pub assignments: Vec<AssignmentDto>,
}

#[utoipa::path(
    get, path = "/api/v1/organizations/{organization_id}/members/{user_id}/effective-permissions",
    params(
        ("organization_id" = String, Path, description = "Organization UUID"),
        ("user_id" = String, Path, description = "Member user id"),
    ),
    responses(
        (status = 200, description = "What the member can do and where it comes from", body = EffectivePermissionsDto),
        (status = 403, description = "Needs org.members.manage unless asking about yourself"),
        (status = 404, description = "Not a member"),
    ),
    tag = "access"
)]
pub async fn effective_permissions_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgRead>,
    Path((_organization_id, user_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    if user_id != org.user.user_id
        && !org
            .permissions
            .allows(Permission::OrgMembersManage, Target::Org)
    {
        return Err(AppError::InsufficientPermissions);
    }
    let mut conn = state.db.acquire().await.map_err(db)?;
    let row = sqlx::query(
        "SELECT m.role_id, u.name, u.email FROM organization_members m JOIN users u ON u.id = m.user_id
          WHERE m.organization_id = ?1 AND m.user_id = ?2",
    )
    .bind(&org.organization_id)
    .bind(&user_id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(db)?
    .ok_or(AppError::ResourceNotFound)?;
    let base_role_id: i64 = row.get(0);
    // Evaluated as a plain member: a global SysAdmin flag is not an org grant.
    let member = AuthUser {
        user_id: user_id.clone(),
        email: row.get(2),
        name: row.get(1),
        is_sysadmin: false,
        token_id: None,
        token_name: None,
    };
    let set: PermissionSet = load(&state, &member, &org.organization_id, base_role_id).await?;
    let assignments = list_assignments(
        &mut conn,
        &org.organization_id,
        &AssignmentFilter {
            scope_type: None,
            scope_id: None,
            user_id: Some(user_id),
        },
        None,
    )
    .await?;
    Ok(ok(EffectivePermissionsDto {
        base_role_id,
        base_role_name: system_role_name(base_role_id).to_string(),
        permissions: set.to_dto(),
        assignments,
    }))
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::{Body, to_bytes};
    use axum::extract::Request;
    use axum::http::{Method, StatusCode};
    use axum::middleware::Next;
    use axum::routing::{delete, get, patch, put};
    use tower::ServiceExt;

    use super::*;
    use crate::testing::{
        auth_user, cleanup_database, seed_application, set_base_role, test_state,
    };

    fn router(state: SharedState, user: AuthUser) -> Router {
        let base = "/api/v1/organizations/{organization_id}";
        Router::new()
            .route(&format!("{base}/permissions/catalog"), get(catalog_handler))
            .route(
                &format!("{base}/roles"),
                get(list_roles_handler).post(create_role_handler),
            )
            .route(
                &format!("{base}/roles/{{role_id}}"),
                patch(update_role_handler).delete(delete_role_handler),
            )
            .route(
                &format!("{base}/groups"),
                get(list_groups_handler).post(create_group_handler),
            )
            .route(
                &format!("{base}/groups/{{group_id}}"),
                get(get_group_handler)
                    .patch(update_group_handler)
                    .delete(delete_group_handler),
            )
            .route(
                &format!("{base}/groups/{{group_id}}/members"),
                put(set_group_members_handler),
            )
            .route(
                &format!("{base}/assignments"),
                get(list_assignments_handler).post(create_assignment_handler),
            )
            .route(
                &format!("{base}/assignments/{{assignment_id}}"),
                delete(delete_assignment_handler),
            )
            .route(
                &format!("{base}/members/{{user_id}}/effective-permissions"),
                get(effective_permissions_handler),
            )
            .layer(axum::middleware::from_fn(
                move |mut req: Request, next: Next| {
                    let user = user.clone();
                    async move {
                        req.extensions_mut().insert(user);
                        next.run(req).await
                    }
                },
            ))
            .with_state(state)
    }

    async fn call(
        state: &SharedState,
        user: &str,
        method: Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> (StatusCode, serde_json::Value) {
        let request = Request::builder()
            .method(method)
            .uri(format!("/api/v1/organizations/organization-a{path}"))
            .header("content-type", "application/json")
            .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
            .unwrap();
        let response = router(state.clone(), auth_user(user))
            .oneshot(request)
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, json)
    }

    async fn exec(state: &SharedState, sql: &str) {
        sqlx::query(sql).execute(&state.db).await.unwrap();
    }

    fn assign_body(
        principal: &str,
        role: &str,
        scope_type: &str,
        scope_id: &str,
    ) -> serde_json::Value {
        json!({"principal_type": "user", "principal_id": principal, "role_id": role,
               "scope_type": scope_type, "scope_id": scope_id})
    }

    /// `editor` becomes organization-a's only Admin; audit logging is on.
    async fn setup() -> (SharedState, std::path::PathBuf) {
        let (state, path) = test_state().await;
        set_base_role(&state, "organization-a", "editor", 1).await;
        exec(&state, "UPDATE organizations SET audit_logs_enabled = 1").await;
        (state, path)
    }

    /// `viewer` gets a custom org-scope role `mgr` holding `perms`.
    async fn grant_viewer(state: &SharedState, perms: &[&str]) {
        exec(state, "INSERT INTO rbac_roles (id, organization_id, name) VALUES ('mgr', 'organization-a', 'manager')").await;
        for p in perms {
            exec(
                state,
                &format!(
                    "INSERT INTO rbac_role_permissions (role_id, permission) VALUES ('mgr', '{p}')"
                ),
            )
            .await;
        }
        exec(
            state,
            "INSERT INTO rbac_assignments (id, organization_id, principal_type, principal_id, custom_role_id, scope_type, scope_id) \
             VALUES ('mgr-a', 'organization-a', 'user', 'viewer', 'mgr', 'org', 'organization-a')",
        )
        .await;
    }

    async fn audited(state: &SharedState, action: &str) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM audit_logs WHERE action = ?1")
            .bind(action)
            .fetch_one(&state.db)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn roles_lifecycle_with_rules_and_audit() {
        let (state, path) = setup().await;
        let (status, roles) = call(&state, "viewer", Method::GET, "/roles", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            roles["data"].as_array().unwrap().len(),
            4,
            "system roles only"
        );

        let (status, body) = call(
            &state,
            "editor",
            Method::POST,
            "/roles",
            Some(json!({"name": "deployer", "permissions": ["app.read", "app.deploy"]})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        let role_id = body["data"]["id"].as_str().unwrap().to_string();
        assert_eq!(audited(&state, "role.created").await, 1);

        let (status, _) = call(
            &state,
            "editor",
            Method::POST,
            "/roles",
            Some(json!({"name": "x", "permissions": ["app.fly"]})),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "rule 2");

        let (status, _) = call(
            &state,
            "editor",
            Method::POST,
            "/roles",
            Some(json!({"name": "deployer", "permissions": []})),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "name taken");

        let (status, _) = call(
            &state,
            "editor",
            Method::PATCH,
            "/roles/2",
            Some(json!({"name": "Editor", "permissions": []})),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "rule 1");

        let (status, _) = call(
            &state,
            "editor",
            Method::POST,
            "/assignments",
            Some(assign_body(
                "viewer",
                &role_id,
                "project",
                "organization-a-project",
            )),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "no such project yet");
        seed_application(&state, "organization-a").await;
        let (status, _) = call(
            &state,
            "editor",
            Method::POST,
            "/assignments",
            Some(assign_body(
                "viewer",
                &role_id,
                "project",
                "organization-a-project",
            )),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);

        let (status, _) = call(
            &state,
            "editor",
            Method::PATCH,
            &format!("/roles/{role_id}"),
            Some(json!({"name": "deployer", "permissions": ["app.deploy", "server.manage"]})),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "rule 3 on edit");

        let (status, _) = call(
            &state,
            "editor",
            Method::DELETE,
            &format!("/roles/{role_id}"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "rule 6");
        let (status, _) = call(
            &state,
            "editor",
            Method::DELETE,
            &format!("/roles/{role_id}?force=true"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM rbac_assignments")
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert_eq!(left, 0, "force removes the role's assignments");
        assert_eq!(audited(&state, "role.deleted").await, 1);
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn a_role_manager_cannot_create_a_role_beyond_its_own_permissions() {
        let (state, path) = setup().await;
        grant_viewer(&state, &["org.read", "org.roles.manage", "app.read"]).await;
        let (status, _) = call(
            &state,
            "viewer",
            Method::POST,
            "/roles",
            Some(json!({"name": "ops", "permissions": ["org.sso.manage"]})),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, _) = call(
            &state,
            "viewer",
            Method::POST,
            "/roles",
            Some(json!({"name": "reader", "permissions": ["app.read"]})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn assignments_enforce_scope_principal_and_escalation() {
        let (state, path) = setup().await;
        let (project, _, _) = seed_application(&state, "organization-a").await;
        let (project_b, _, _) = seed_application(&state, "organization-b").await;
        grant_viewer(&state, &["org.read", "org.members.manage"]).await;
        let post = |body| Some(body);

        // Holding org.members.manage is not enough to hand out Admin or Editor.
        let (status, _) = call(
            &state,
            "viewer",
            Method::POST,
            "/assignments",
            post(assign_body("viewer", "1", "org", "organization-a")),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, _) = call(
            &state,
            "viewer",
            Method::POST,
            "/assignments",
            post(assign_body("viewer", "2", "project", &project)),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        let (status, _) = call(
            &state,
            "editor",
            Method::POST,
            "/assignments",
            post(assign_body("viewer", "2", "project", &project_b)),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "another org's project");
        let (status, _) = call(
            &state,
            "editor",
            Method::POST,
            "/assignments",
            post(assign_body("outsider", "2", "project", &project)),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "not a member");
        let (status, _) = call(
            &state,
            "editor",
            Method::POST,
            "/assignments",
            post(assign_body("viewer", "4", "project", &project)),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "Member is a base role"
        );
        let (status, _) = call(
            &state,
            "editor",
            Method::POST,
            "/assignments",
            post(assign_body("viewer", "mgr", "project", &project)),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "org-only role below org scope"
        );

        // Granted to someone else, so `viewer` holds nothing it came from.
        let (status, body) = call(
            &state,
            "editor",
            Method::POST,
            "/assignments",
            post(assign_body("editor", "2", "project", &project)),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        assert_eq!(body["data"]["role_name"], "Editor");
        assert_eq!(
            body["data"]["scope_name"],
            project.as_str(),
            "fixture names = ids"
        );
        let id = body["data"]["id"].as_str().unwrap().to_string();
        let (status, _) = call(
            &state,
            "editor",
            Method::POST,
            "/assignments",
            post(assign_body("editor", "2", "project", &project)),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "duplicate");

        let (status, list) = call(
            &state,
            "viewer",
            Method::GET,
            &format!("/assignments?scope_type=project&scope_id={project}"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(list["data"].as_array().unwrap().len(), 1);

        // Removing a grant you could not have given is refused too.
        let (status, _) = call(
            &state,
            "viewer",
            Method::DELETE,
            &format!("/assignments/{id}"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, _) = call(
            &state,
            "editor",
            Method::DELETE,
            &format!("/assignments/{id}"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert_eq!(audited(&state, "assignment.created").await, 1);
        assert_eq!(audited(&state, "assignment.deleted").await, 1);
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn groups_rules_and_the_last_admin() {
        let (state, path) = setup().await;
        let (status, body) = call(
            &state,
            "editor",
            Method::POST,
            "/groups",
            Some(json!({"name": "ops"})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let group_id = body["data"]["id"].as_str().unwrap().to_string();
        let members = format!("/groups/{group_id}/members");

        let (status, _) = call(
            &state,
            "editor",
            Method::PUT,
            &members,
            Some(json!({"user_ids": ["outsider"]})),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let (status, body) = call(
            &state,
            "editor",
            Method::PUT,
            &members,
            Some(json!({"user_ids": ["viewer", "editor"]})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["members"].as_array().unwrap().len(), 2);

        // Make the group the only path to Admin.
        let (status, _) = call(
            &state,
            "editor",
            Method::POST,
            "/assignments",
            Some(
                json!({"principal_type": "group", "principal_id": group_id, "role_id": "1",
                        "scope_type": "org", "scope_id": "organization-a"}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        set_base_role(&state, "organization-a", "editor", 2).await;

        let (status, _) = call(
            &state,
            "editor",
            Method::DELETE,
            &format!("/groups/{group_id}"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "last admin via group");
        let (status, _) = call(
            &state,
            "editor",
            Method::PUT,
            &members,
            Some(json!({"user_ids": []})),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "emptying the admin group");

        exec(
            &state,
            &format!("UPDATE rbac_groups SET source = 'scim' WHERE id = '{group_id}'"),
        )
        .await;
        let (status, _) = call(
            &state,
            "editor",
            Method::PATCH,
            &format!("/groups/{group_id}"),
            Some(json!({"name": "renamed"})),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "rule 7");
        assert_eq!(audited(&state, "group.members_changed").await, 1);
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn effective_permissions_are_self_or_manager_only() {
        let (state, path) = setup().await;
        let (project, _, _) = seed_application(&state, "organization-a").await;
        crate::testing::assign(&state, "organization-a", "viewer", 2, "project", &project).await;

        let (status, body) = call(
            &state,
            "viewer",
            Method::GET,
            "/members/viewer/effective-permissions",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["base_role_name"], "Viewer");
        assert!(
            body["data"]["permissions"]["projects"][&project]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p == "app.deploy")
        );
        assert_eq!(body["data"]["assignments"].as_array().unwrap().len(), 1);

        let (status, _) = call(
            &state,
            "viewer",
            Method::GET,
            "/members/editor/effective-permissions",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, _) = call(
            &state,
            "editor",
            Method::GET,
            "/members/outsider/effective-permissions",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        cleanup_database(&path);
    }
}
