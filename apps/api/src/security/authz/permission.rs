use std::collections::BTreeSet;

macro_rules! permissions {
    ($($variant:ident => $name:literal, org_only = $org_only:literal;)*) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum Permission { $($variant),* }

        impl Permission {
            pub const ALL: &'static [Permission] = &[$(Permission::$variant),*];

            pub fn as_str(self) -> &'static str {
                match self { $(Permission::$variant => $name),* }
            }

            /// Only meaningful at organization scope. A grant of one of these in
            /// a project or environment assignment is ignored by the evaluator
            /// and rejected by the management API (spec 2).
            pub fn org_only(self) -> bool {
                match self { $(Permission::$variant => $org_only),* }
            }

            pub fn parse(name: &str) -> Option<Permission> {
                Self::ALL.iter().copied().find(|p| p.as_str() == name)
            }
        }
    };
}

permissions! {
    OrgRead => "org.read", org_only = true;
    OrgUpdate => "org.update", org_only = true;
    OrgMembersManage => "org.members.manage", org_only = true;
    OrgRolesManage => "org.roles.manage", org_only = true;
    OrgSsoManage => "org.sso.manage", org_only = true;
    OrgAuditRead => "org.audit.read", org_only = true;
    OrgTokensManage => "org.tokens.manage", org_only = true;
    AssistantUse => "assistant.use", org_only = true;
    ProjectRead => "project.read", org_only = false;
    ProjectCreate => "project.create", org_only = true;
    ProjectUpdate => "project.update", org_only = false;
    ProjectDelete => "project.delete", org_only = false;
    EnvCreate => "env.create", org_only = false;
    EnvUpdate => "env.update", org_only = false;
    EnvDelete => "env.delete", org_only = false;
    AppRead => "app.read", org_only = false;
    AppCreate => "app.create", org_only = false;
    AppUpdate => "app.update", org_only = false;
    AppDelete => "app.delete", org_only = false;
    AppDeploy => "app.deploy", org_only = false;
    AppLogsRead => "app.logs.read", org_only = false;
    AppExec => "app.exec", org_only = false;
    AppEnvRead => "app.env.read", org_only = false;
    AppEnvUpdate => "app.env.update", org_only = false;
    AppDomainsManage => "app.domains.manage", org_only = false;
    BackupRead => "backup.read", org_only = false;
    BackupManage => "backup.manage", org_only = false;
    BackupRestore => "backup.restore", org_only = false;
    BackupTargetsManage => "backup.targets.manage", org_only = true;
    ServerRead => "server.read", org_only = true;
    ServerManage => "server.manage", org_only = true;
    ServerTerminal => "server.terminal", org_only = true;
    GitProviderManage => "git_provider.manage", org_only = true;
    NotificationManage => "notification.manage", org_only = true;
    ScheduledTaskManage => "scheduled_task.manage", org_only = true;
}

use Permission::*;

/// Base role id 4, seeded by `0052_rbac.sql`: in the organization, nothing else.
const MEMBER: &[Permission] = &[OrgRead];

/// Everything a `ROLE_VIEWER` (3) reaches today through `OrgRead` / `AppRead`.
/// Viewers list env vars masked; revealing them is `AppEnvRead` (Editor).
const VIEWER_EXTRA: &[Permission] = &[ProjectRead, AppRead, AppLogsRead, ServerRead, BackupRead];

/// Everything a `ROLE_EDITOR` (2) reaches today through `OrgEdit` / `AppEdit` /
/// `AppDeploy` (plus the Editor-only `?reveal=true` on env vars), on top of Viewer.
const EDITOR_EXTRA: &[Permission] = &[
    AppEnvRead,
    ProjectCreate,
    ProjectUpdate,
    ProjectDelete,
    EnvCreate,
    EnvUpdate,
    EnvDelete,
    AppCreate,
    AppUpdate,
    AppDelete,
    AppDeploy,
    AppEnvUpdate,
    AppDomainsManage,
    BackupManage,
    BackupRestore,
    GitProviderManage,
    AssistantUse,
];

/// Permission set of a system role, by `roles.id`. Admin-level is "all", so
/// adding a permission to the catalog grants it to Admin and SysAdmin with no
/// migration — system roles live in code precisely so the catalog can grow.
pub fn system_role_permissions(role_id: i64) -> BTreeSet<Permission> {
    match role_id {
        0 | 1 => Permission::ALL.iter().copied().collect(),
        2 => MEMBER
            .iter()
            .chain(VIEWER_EXTRA)
            .chain(EDITOR_EXTRA)
            .copied()
            .collect(),
        3 => MEMBER.iter().chain(VIEWER_EXTRA).copied().collect(),
        4 => MEMBER.iter().copied().collect(),
        _ => BTreeSet::new(),
    }
}

/// Catalog permissions a token may name directly as a scope: Editor level, so
/// a granular token can never reach further than a `write` + `deploy` one.
pub fn token_grantable() -> BTreeSet<Permission> {
    system_role_permissions(2)
}

/// The most a personal access token can act with, before intersecting with
/// the owner's own permissions. Two kinds of scope, freely mixed:
/// - legacy presets (`token_scope_allows`): any of them gives Viewer level,
///   `write` reaches Editor, `deploy` adds only `app.deploy`;
/// - catalog names from [`token_grantable`] (`app.deploy`, `backup.read`, …),
///   each granting exactly itself. A token with only these gets nothing else
///   beyond `org.read`, the membership marker every org route assumes.
pub fn token_ceiling(scopes: &[String]) -> BTreeSet<Permission> {
    let legacy = |name: &str| scopes.iter().any(|s| s == name);
    let mut ceiling = if legacy("read") || legacy("write") || legacy("deploy") {
        system_role_permissions(3)
    } else {
        system_role_permissions(4)
    };
    if legacy("write") {
        ceiling.extend(system_role_permissions(2));
    }
    if legacy("deploy") {
        ceiling.insert(AppDeploy);
    }
    let grantable = token_grantable();
    ceiling.extend(
        scopes
            .iter()
            .filter_map(|s| Permission::parse(s))
            .filter(|p| grantable.contains(p)),
    );
    ceiling
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_permission_round_trips_through_its_name() {
        for p in Permission::ALL {
            assert_eq!(Permission::parse(p.as_str()), Some(*p), "{}", p.as_str());
        }
        assert_eq!(Permission::parse("app.nope"), None);
    }

    #[test]
    fn names_are_unique() {
        let names: BTreeSet<_> = Permission::ALL.iter().map(|p| p.as_str()).collect();
        assert_eq!(names.len(), Permission::ALL.len());
    }

    #[test]
    fn system_roles_nest() {
        let member = system_role_permissions(4);
        let viewer = system_role_permissions(3);
        let editor = system_role_permissions(2);
        let admin = system_role_permissions(1);
        assert!(member.is_subset(&viewer));
        assert!(viewer.is_subset(&editor));
        assert!(editor.is_subset(&admin));
        assert_eq!(admin.len(), Permission::ALL.len());
        assert_eq!(system_role_permissions(0), admin);
        assert!(system_role_permissions(99).is_empty());
    }

    /// Locks today's behavior: these routes are Admin-only (`AppAdmin` /
    /// `OrgAdmin`) before the migration, so Editor must not gain them.
    #[test]
    fn editor_does_not_gain_admin_only_powers() {
        let editor = system_role_permissions(2);
        for p in [
            AppExec,
            ServerManage,
            ServerTerminal,
            NotificationManage,
            ScheduledTaskManage,
            BackupTargetsManage,
            OrgUpdate,
            OrgMembersManage,
            OrgAuditRead,
        ] {
            assert!(!editor.contains(&p), "{}", p.as_str());
        }
    }

    #[test]
    fn viewer_keeps_todays_reads() {
        let viewer = system_role_permissions(3);
        for p in [
            OrgRead,
            ProjectRead,
            AppRead,
            AppLogsRead,
            ServerRead,
            BackupRead,
        ] {
            assert!(viewer.contains(&p), "{}", p.as_str());
        }
    }

    /// `get_environment_handler` lists masked vars at Viewer level but
    /// `?reveal=true` requires Editor (`role_id > ROLE_EDITOR` is refused).
    #[test]
    fn revealing_secrets_is_editor_level() {
        assert!(!system_role_permissions(3).contains(&AppEnvRead));
        assert!(system_role_permissions(2).contains(&AppEnvRead));
    }

    #[test]
    fn tokens_never_reach_admin_only_permissions() {
        let all_scopes: Vec<String> = ["read", "write", "deploy"].map(String::from).to_vec();
        let ceiling = token_ceiling(&all_scopes);
        assert!(ceiling.is_subset(&system_role_permissions(2)));
        assert!(!ceiling.contains(&AppExec));
    }

    #[test]
    fn a_deploy_token_can_deploy_but_not_update() {
        let ceiling = token_ceiling(&["deploy".to_string()]);
        assert!(ceiling.contains(&AppDeploy));
        assert!(ceiling.contains(&AppRead));
        assert!(!ceiling.contains(&AppUpdate));
    }

    #[test]
    fn a_granular_token_gets_only_what_it_names() {
        let ceiling = token_ceiling(&["app.deploy".to_string(), "app.read".to_string()]);
        assert_eq!(ceiling, BTreeSet::from([OrgRead, AppRead, AppDeploy]));
    }

    #[test]
    fn a_granular_token_cannot_name_admin_permissions() {
        let ceiling = token_ceiling(&["app.exec".to_string(), "org.members.manage".to_string()]);
        assert_eq!(ceiling, BTreeSet::from([OrgRead]));
    }
}
