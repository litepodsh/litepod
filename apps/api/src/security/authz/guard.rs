//! The rules every access-control write goes through.
//! Handlers stay thin: they resolve, call these, write, and call
//! [`ensure_admin_remains`] before committing.

use std::collections::BTreeSet;

use sqlx::SqliteConnection;

use super::permission::{Permission, system_role_permissions};
use super::set::{PermissionSet, Target};
use crate::dtos::response::AppError;

fn db(e: impl ToString) -> AppError {
    AppError::DatabaseError(e.to_string())
}

/// A role as the API names it: `"1"`..`"4"` are the system roles in code,
/// anything else is a custom role id in `rbac_roles`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RoleRef {
    System(i64),
    Custom(String),
}

impl RoleRef {
    pub fn parse(id: &str) -> RoleRef {
        match id.parse::<i64>() {
            Ok(n @ 1..=4) => RoleRef::System(n),
            _ => RoleRef::Custom(id.to_string()),
        }
    }

    pub fn is_custom(&self) -> bool {
        matches!(self, RoleRef::Custom(_))
    }
}

pub fn system_role_name(role_id: i64) -> &'static str {
    match role_id {
        1 => "Admin",
        2 => "Editor",
        3 => "Viewer",
        4 => "Member",
        _ => "Unknown",
    }
}

/// The permissions a role grants. A custom role must belong to `organization_id`
/// (404 otherwise, so ids from other orgs are indistinguishable from missing).
pub async fn role_permissions(
    conn: &mut SqliteConnection,
    organization_id: &str,
    role: &RoleRef,
) -> Result<BTreeSet<Permission>, AppError> {
    match role {
        RoleRef::System(id) => Ok(system_role_permissions(*id)),
        RoleRef::Custom(id) => {
            let exists: Option<i64> = sqlx::query_scalar(
                "SELECT 1 FROM rbac_roles WHERE id = ?1 AND organization_id = ?2",
            )
            .bind(id)
            .bind(organization_id)
            .fetch_optional(&mut *conn)
            .await
            .map_err(db)?;
            if exists.is_none() {
                return Err(AppError::ResourceNotFound);
            }
            let names: Vec<String> = sqlx::query_scalar(
                "SELECT permission FROM rbac_role_permissions WHERE role_id = ?1",
            )
            .bind(id)
            .fetch_all(&mut *conn)
            .await
            .map_err(db)?;
            // Names no longer in the catalog grant nothing (the evaluator skips them too).
            Ok(names.iter().filter_map(|n| Permission::parse(n)).collect())
        }
    }
}

/// Rule 2: every name must be in the compile-time catalog.
pub fn parse_catalog(names: &[String]) -> Result<BTreeSet<Permission>, AppError> {
    names
        .iter()
        .map(|n| {
            Permission::parse(n)
                .ok_or_else(|| AppError::ValidationError(format!("Unknown permission: {n}")))
        })
        .collect()
}

/// Rule 3: a custom role holding an org-only permission cannot be assigned
/// below org scope. System roles can: the evaluator ignores their org-only part
/// there, which is what "Editor on project X" means.
pub fn ensure_scope_fits(
    perms: &BTreeSet<Permission>,
    scope_type: &str,
    role: &RoleRef,
) -> Result<(), AppError> {
    if scope_type == "org" || !role.is_custom() {
        return Ok(());
    }
    match perms.iter().find(|p| p.org_only()) {
        Some(p) => Err(AppError::ValidationError(format!(
            "{} is organization-wide and cannot be granted at {scope_type} scope",
            p.as_str()
        ))),
        None => Ok(()),
    }
}

/// Rule 4 (anti-escalation): the caller must already hold, at `target`, every
/// permission it is about to hand out (or take away). Org-only permissions are
/// skipped below org scope, matching what such a grant actually gives. A
/// SysAdmin's set holds everything, so it passes without a special case.
pub fn ensure_can_grant(
    caller: &PermissionSet,
    perms: &BTreeSet<Permission>,
    target: Target,
) -> Result<(), AppError> {
    let at_org = matches!(target, Target::Org);
    let missing = perms
        .iter()
        .filter(|p| at_org || !p.org_only())
        .find(|p| !caller.allows(**p, clone_target(&target)));
    match missing {
        Some(_) => Err(AppError::InsufficientPermissions),
        None => Ok(()),
    }
}

fn clone_target<'a>(target: &Target<'a>) -> Target<'a> {
    match target {
        Target::Org => Target::Org,
        Target::Project(p) => Target::Project(p),
        Target::Environment {
            project_id,
            environment_id,
        } => Target::Environment {
            project_id,
            environment_id,
        },
    }
}

/// Rule 5 (last admin): after the write, someone in the org must still hold
/// both `org.members.manage` and `org.roles.manage` org-wide — through base
/// role Admin, an org-scope Admin assignment, or a custom role with both, each
/// directly or via a group. Run on the write's transaction, before commit.
pub async fn ensure_admin_remains(
    conn: &mut SqliteConnection,
    organization_id: &str,
) -> Result<(), AppError> {
    let admins: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM organization_members m
          WHERE m.organization_id = ?1
            AND (m.role_id = 1 OR EXISTS (
                SELECT 1 FROM rbac_assignments a
                 WHERE a.organization_id = ?1 AND a.scope_type = 'org'
                   AND ((a.principal_type = 'user' AND a.principal_id = m.user_id)
                     OR (a.principal_type = 'group' AND a.principal_id IN
                           (SELECT group_id FROM rbac_group_members WHERE user_id = m.user_id)))
                   AND (a.system_role_id = 1
                     OR (SELECT COUNT(*) FROM rbac_role_permissions rp
                          WHERE rp.role_id = a.custom_role_id
                            AND rp.permission IN ('org.members.manage', 'org.roles.manage')) = 2)))",
    )
    .bind(organization_id)
    .fetch_one(&mut *conn)
    .await
    .map_err(db)?;
    if admins == 0 {
        return Err(AppError::ResourceInUse(
            "This change would leave the organization without an administrator".into(),
        ));
    }
    Ok(())
}

/// Commits when an administrator remains, otherwise rolls back right away.
/// A dropped transaction only rolls back lazily, and the write lock it still
/// holds makes the next writer fail with SQLITE_BUSY.
pub async fn commit_if_admin_remains(
    mut tx: sqlx::Transaction<'_, sqlx::Sqlite>,
    organization_id: &str,
) -> Result<(), AppError> {
    match ensure_admin_remains(&mut tx, organization_id).await {
        Ok(()) => tx.commit().await.map_err(db),
        Err(e) => {
            tx.rollback().await.map_err(db)?;
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{assign, cleanup_database, set_base_role, test_state};

    fn set(role_id: i64) -> PermissionSet {
        PermissionSet::for_base_role(role_id)
    }

    #[test]
    fn role_ids_one_to_four_are_system_roles() {
        assert_eq!(RoleRef::parse("1"), RoleRef::System(1));
        assert_eq!(RoleRef::parse("4"), RoleRef::System(4));
        assert!(RoleRef::parse("0").is_custom(), "SysAdmin is not grantable");
        assert!(RoleRef::parse("0192-uuid").is_custom());
    }

    #[test]
    fn unknown_permissions_are_rejected() {
        let err = parse_catalog(&["app.deploy".into(), "app.fly".into()]).unwrap_err();
        assert!(matches!(err, AppError::ValidationError(m) if m.contains("app.fly")));
    }

    #[test]
    fn a_custom_org_only_permission_does_not_fit_a_project() {
        let perms = parse_catalog(&["server.manage".into()]).unwrap();
        let custom = RoleRef::Custom("r".into());
        assert!(ensure_scope_fits(&perms, "project", &custom).is_err());
        assert!(ensure_scope_fits(&perms, "org", &custom).is_ok());
        assert!(
            ensure_scope_fits(&system_role_permissions(1), "project", &RoleRef::System(1)).is_ok()
        );
    }

    #[test]
    fn an_editor_cannot_grant_admin_but_an_admin_can() {
        let admin = system_role_permissions(1);
        assert!(matches!(
            ensure_can_grant(&set(2), &admin, Target::Org),
            Err(AppError::InsufficientPermissions)
        ));
        assert!(ensure_can_grant(&set(1), &admin, Target::Org).is_ok());
        assert!(
            ensure_can_grant(&set(0), &admin, Target::Org).is_ok(),
            "SysAdmin"
        );
    }

    #[test]
    fn an_editor_can_grant_admin_on_a_project_minus_the_org_part() {
        // Admin at project scope = Editor + app.exec there; an Editor lacks app.exec.
        let admin = system_role_permissions(1);
        assert!(ensure_can_grant(&set(2), &admin, Target::Project("p")).is_err());
        assert!(
            ensure_can_grant(&set(2), &system_role_permissions(2), Target::Project("p")).is_ok()
        );
    }

    #[tokio::test]
    async fn the_last_admin_cannot_go_but_a_group_admin_counts() {
        let (state, path) = test_state().await;
        // organization-a has no Admin in the fixture; make editor the only one.
        set_base_role(&state, "organization-a", "editor", 1).await;
        let mut conn = state.db.acquire().await.unwrap();
        ensure_admin_remains(&mut conn, "organization-a")
            .await
            .unwrap();

        set_base_role(&state, "organization-a", "editor", 2).await;
        assert!(matches!(
            ensure_admin_remains(&mut conn, "organization-a").await,
            Err(AppError::ResourceInUse(_))
        ));

        for sql in [
            "INSERT INTO rbac_groups (id, organization_id, name) VALUES ('g', 'organization-a', 'ops')",
            "INSERT INTO rbac_group_members (group_id, user_id) VALUES ('g', 'viewer')",
            "INSERT INTO rbac_assignments (id, organization_id, principal_type, principal_id, system_role_id, scope_type, scope_id) \
             VALUES ('a', 'organization-a', 'group', 'g', 1, 'org', 'organization-a')",
        ] {
            sqlx::query(sql).execute(&state.db).await.unwrap();
        }
        ensure_admin_remains(&mut conn, "organization-a")
            .await
            .unwrap();

        // A project-scope Admin is not an org administrator.
        sqlx::query("DELETE FROM rbac_assignments")
            .execute(&state.db)
            .await
            .unwrap();
        assign(
            &state,
            "organization-a",
            "viewer",
            1,
            "project",
            "organization-a-project",
        )
        .await;
        assert!(
            ensure_admin_remains(&mut conn, "organization-a")
                .await
                .is_err()
        );
        cleanup_database(&path);
    }
}
