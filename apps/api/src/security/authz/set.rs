use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde::Serialize;
use sqlx::Row;

use super::permission::{Permission, system_role_permissions, token_ceiling};
use crate::dtos::response::AppError;
use crate::security::middleware::auth::AuthUser;
use crate::state::state::SharedState;

pub enum Target<'a> {
    Org,
    Project(&'a str),
    Environment {
        project_id: &'a str,
        environment_id: &'a str,
    },
}

pub enum Visible {
    All,
    Only(HashSet<String>),
}

impl Visible {
    pub fn contains(&self, id: &str) -> bool {
        match self {
            Visible::All => true,
            Visible::Only(ids) => ids.contains(id),
        }
    }
}

/// What one user may do inside one organization, for the life of one request.
#[derive(Clone, Debug, Default)]
pub struct PermissionSet {
    org: BTreeSet<Permission>,
    projects: HashMap<String, BTreeSet<Permission>>,
    /// environment_id → (project_id, permissions)
    environments: HashMap<String, (String, BTreeSet<Permission>)>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct PermissionSetDto {
    pub org: Vec<String>,
    pub projects: BTreeMap<String, Vec<String>>,
    pub environments: BTreeMap<String, Vec<String>>,
}

fn names(set: &BTreeSet<Permission>) -> Vec<String> {
    set.iter().map(|p| p.as_str().to_string()).collect()
}

fn db(e: impl ToString) -> AppError {
    AppError::DatabaseError(e.to_string())
}

impl PermissionSet {
    /// Base role only, no assignments. Used by the legacy extractors, which
    /// still decide by role and never consult assignments.
    pub fn for_base_role(role_id: i64) -> Self {
        Self {
            org: system_role_permissions(role_id),
            ..Self::default()
        }
    }

    pub fn allows(&self, p: Permission, target: Target) -> bool {
        if self.org.contains(&p) {
            return true;
        }
        if p.org_only() {
            return false;
        }
        let in_project = |id: &str| self.projects.get(id).is_some_and(|s| s.contains(&p));
        match target {
            Target::Org => false,
            // Reading a project is also allowed when only one of its
            // environments was granted, so a listed project can be opened.
            Target::Project(project_id) => {
                in_project(project_id)
                    || (p == Permission::ProjectRead
                        && self
                            .environments
                            .values()
                            .any(|(owner, s)| owner == project_id && s.contains(&p)))
            }
            Target::Environment {
                project_id,
                environment_id,
            } => {
                in_project(project_id)
                    || self
                        .environments
                        .get(environment_id)
                        .is_some_and(|(owner, s)| owner == project_id && s.contains(&p))
            }
        }
    }

    /// Whether the caller holds every permission of `role_id` organization-wide.
    /// For features that act on the caller's behalf across the whole org (the
    /// assistant), where a narrower grant would let the feature exceed its user.
    pub fn covers_role_at_org(&self, role_id: i64) -> bool {
        system_role_permissions(role_id).is_subset(&self.org)
    }

    pub fn cap_to_token(&mut self, scopes: &[String]) {
        let ceiling = token_ceiling(scopes);
        self.org.retain(|p| ceiling.contains(p));
        for set in self.projects.values_mut() {
            set.retain(|p| ceiling.contains(p));
        }
        for (_, set) in self.environments.values_mut() {
            set.retain(|p| ceiling.contains(p));
        }
    }

    /// Projects where `p` holds anywhere (project itself or one of its envs).
    pub fn visible_projects(&self, p: Permission) -> Visible {
        if self.org.contains(&p) {
            return Visible::All;
        }
        let mut ids: HashSet<String> = self
            .projects
            .iter()
            .filter(|(_, s)| s.contains(&p))
            .map(|(id, _)| id.clone())
            .collect();
        ids.extend(
            self.environments
                .values()
                .filter(|(_, s)| s.contains(&p))
                .map(|(project_id, _)| project_id.clone()),
        );
        Visible::Only(ids)
    }

    pub fn visible_environments(&self, p: Permission, project_id: &str) -> Visible {
        if self.org.contains(&p)
            || self
                .projects
                .get(project_id)
                .is_some_and(|s| s.contains(&p))
        {
            return Visible::All;
        }
        Visible::Only(
            self.environments
                .iter()
                .filter(|(_, (owner, s))| owner == project_id && s.contains(&p))
                .map(|(id, _)| id.clone())
                .collect(),
        )
    }

    pub fn to_dto(&self) -> PermissionSetDto {
        PermissionSetDto {
            org: names(&self.org),
            projects: self
                .projects
                .iter()
                .map(|(k, v)| (k.clone(), names(v)))
                .collect(),
            environments: self
                .environments
                .iter()
                .map(|(k, (_, v))| (k.clone(), names(v)))
                .collect(),
        }
    }

    fn grant(
        &mut self,
        scope_type: &str,
        scope_id: String,
        project_of_env: Option<String>,
        perms: BTreeSet<Permission>,
    ) {
        if scope_type == "org" {
            self.org.extend(perms);
            return;
        }
        let scoped = perms.into_iter().filter(|p| !p.org_only());
        match scope_type {
            "project" => self.projects.entry(scope_id).or_default().extend(scoped),
            "environment" => {
                // An env whose row is gone has no project_id; skip it.
                let Some(project_id) = project_of_env else {
                    return;
                };
                self.environments
                    .entry(scope_id)
                    .or_insert_with(|| (project_id, BTreeSet::new()))
                    .1
                    .extend(scoped);
            }
            _ => {}
        }
    }
}

/// Base role (already resolved by the caller, so membership has been checked)
/// plus every assignment that reaches `user` in `organization_id`, directly or
/// through a group. One query.
pub async fn load(
    state: &SharedState,
    user: &AuthUser,
    organization_id: &str,
    base_role_id: i64,
) -> Result<PermissionSet, AppError> {
    let mut set = PermissionSet::for_base_role(base_role_id);
    if user.is_sysadmin {
        return Ok(set); // role 0 = everything; nothing to add.
    }
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(
        "SELECT a.scope_type, a.scope_id, e.project_id, a.system_role_id, rp.permission
           FROM rbac_assignments a
           LEFT JOIN environments e ON a.scope_type = 'environment' AND e.id = a.scope_id
           LEFT JOIN rbac_role_permissions rp ON rp.role_id = a.custom_role_id
          WHERE a.organization_id = ?1
            AND ((a.principal_type = 'user' AND a.principal_id = ?2)
              OR (a.principal_type = 'group' AND a.principal_id IN
                    (SELECT group_id FROM rbac_group_members WHERE user_id = ?2)))",
    )
    .bind(organization_id)
    .bind(&user.user_id)
    .fetch_all(&conn)
    .await
    .map_err(db)?;

    for row in rows {
        let scope_type: String = row.try_get(0).map_err(db)?;
        let scope_id: String = row.try_get(1).map_err(db)?;
        let project_of_env: Option<String> = row.try_get(2).map_err(db)?;
        let system_role_id: Option<i64> = row.try_get(3).map_err(db)?;
        let permission: Option<String> = row.try_get(4).map_err(db)?;

        let perms = match (system_role_id, permission) {
            (Some(role_id), _) => system_role_permissions(role_id),
            (None, Some(name)) => match Permission::parse(&name) {
                Some(p) => BTreeSet::from([p]),
                None => {
                    // A catalog entry was removed after this row was written.
                    tracing::warn!(permission = %name, "ignoring unknown permission in rbac_role_permissions");
                    continue;
                }
            },
            (None, None) => continue, // custom role with no permissions
        };
        set.grant(&scope_type, scope_id, project_of_env, perms);
    }
    Ok(set)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{
        assign, auth_user, cleanup_database, seed_application, set_base_role, sysadmin_user,
        test_state,
    };

    #[test]
    fn a_base_role_alone_reproduces_todays_access() {
        let set = PermissionSet::for_base_role(2);
        assert!(set.allows(Permission::AppDeploy, Target::Project("any")));
        assert!(!set.allows(Permission::AppExec, Target::Org));
    }

    #[tokio::test]
    async fn a_project_grant_reaches_its_environments_but_not_siblings() {
        let (state, path) = test_state().await;
        let (project, environment, _) = seed_application(&state, "organization-a").await;
        set_base_role(&state, "organization-a", "viewer", 4).await;
        assign(&state, "organization-a", "viewer", 2, "project", &project).await;

        let set = load(&state, &auth_user("viewer"), "organization-a", 4)
            .await
            .unwrap();
        assert!(set.allows(Permission::AppDeploy, Target::Project(&project)));
        assert!(set.allows(
            Permission::AppDeploy,
            Target::Environment {
                project_id: &project,
                environment_id: &environment
            }
        ));
        assert!(!set.allows(Permission::AppDeploy, Target::Project("other-project")));
        assert!(!set.allows(Permission::AppDeploy, Target::Org));
        assert!(
            set.allows(Permission::OrgRead, Target::Org),
            "Member keeps org.read"
        );
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn an_environment_grant_does_not_reach_the_project() {
        let (state, path) = test_state().await;
        let (project, environment, _) = seed_application(&state, "organization-a").await;
        set_base_role(&state, "organization-a", "viewer", 4).await;
        assign(
            &state,
            "organization-a",
            "viewer",
            3,
            "environment",
            &environment,
        )
        .await;

        let set = load(&state, &auth_user("viewer"), "organization-a", 4)
            .await
            .unwrap();
        assert!(set.allows(
            Permission::AppRead,
            Target::Environment {
                project_id: &project,
                environment_id: &environment
            }
        ));
        assert!(!set.allows(Permission::AppRead, Target::Project(&project)));
        assert!(
            set.visible_projects(Permission::ProjectRead)
                .contains(&project),
            "the project holding a granted env must be listable"
        );
        assert!(
            set.allows(Permission::ProjectRead, Target::Project(&project)),
            "...and openable"
        );
        assert!(!set.allows(Permission::ProjectUpdate, Target::Project(&project)));
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn org_only_permissions_are_ignored_at_project_scope() {
        let (state, path) = test_state().await;
        let (project, _, _) = seed_application(&state, "organization-a").await;
        set_base_role(&state, "organization-a", "viewer", 4).await;
        assign(&state, "organization-a", "viewer", 1, "project", &project).await;

        let set = load(&state, &auth_user("viewer"), "organization-a", 4)
            .await
            .unwrap();
        assert!(set.allows(Permission::AppExec, Target::Project(&project)));
        assert!(!set.allows(Permission::ServerManage, Target::Org));
        assert!(!set.allows(Permission::OrgMembersManage, Target::Org));
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn an_org_grant_includes_org_only_permissions() {
        let (state, path) = test_state().await;
        set_base_role(&state, "organization-a", "viewer", 4).await;
        assign(
            &state,
            "organization-a",
            "viewer",
            1,
            "org",
            "organization-a",
        )
        .await;

        let set = load(&state, &auth_user("viewer"), "organization-a", 4)
            .await
            .unwrap();
        assert!(set.allows(Permission::ServerManage, Target::Org));
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn assignments_in_another_org_do_not_leak() {
        let (state, path) = test_state().await;
        let (project_b, _, _) = seed_application(&state, "organization-b").await;
        assign(&state, "organization-b", "editor", 1, "project", &project_b).await;

        let set = load(&state, &auth_user("editor"), "organization-a", 2)
            .await
            .unwrap();
        assert!(!set.allows(Permission::AppExec, Target::Project(&project_b)));
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn a_custom_role_via_a_group_grants_its_permissions() {
        let (state, path) = test_state().await;
        let (project, _, _) = seed_application(&state, "organization-a").await;
        set_base_role(&state, "organization-a", "viewer", 4).await;
        for sql in [
            "INSERT INTO rbac_roles (id, organization_id, name) VALUES ('r', 'organization-a', 'deployer')",
            "INSERT INTO rbac_role_permissions (role_id, permission) VALUES ('r', 'app.deploy'), ('r', 'app.not-a-thing')",
            "INSERT INTO rbac_groups (id, organization_id, name) VALUES ('g', 'organization-a', 'devs')",
            "INSERT INTO rbac_group_members (group_id, user_id) VALUES ('g', 'viewer')",
        ] {
            sqlx::query(sql).execute(&state.db).await.unwrap();
        }
        sqlx::query(
            "INSERT INTO rbac_assignments (id, organization_id, principal_type, principal_id, custom_role_id, scope_type, scope_id) \
             VALUES ('a', 'organization-a', 'group', 'g', 'r', 'project', ?1)",
        )
        .bind(&project)
        .execute(&state.db)
        .await
        .unwrap();

        let set = load(&state, &auth_user("viewer"), "organization-a", 4)
            .await
            .unwrap();
        assert!(set.allows(Permission::AppDeploy, Target::Project(&project)));
        assert!(!set.allows(Permission::AppUpdate, Target::Project(&project)));
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn a_token_caps_even_a_sysadmin() {
        let (state, path) = test_state().await;
        let mut set = load(&state, &sysadmin_user("outsider"), "organization-a", 0)
            .await
            .unwrap();
        assert!(set.allows(Permission::AppExec, Target::Org));
        set.cap_to_token(&["write".to_string()]);
        assert!(!set.allows(Permission::AppExec, Target::Org));
        assert!(set.allows(Permission::AppUpdate, Target::Org));
        cleanup_database(&path);
    }
}
