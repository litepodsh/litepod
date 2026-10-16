//! Locks the system roles to the access they had before RBAC, so the migration
//! is invisible to installs that only use Admin / Editor / Viewer.

use super::permission::{Permission, system_role_permissions};
use Permission::*;

/// Legacy ceiling that guarded each permission's routes before RBAC (from the
/// handler inventory in the plan). 3 = Viewer, 2 = Editor, 1 = Admin.
fn legacy_ceiling(p: Permission) -> i64 {
    match p {
        OrgRead | ProjectRead | AppRead | AppLogsRead | ServerRead | BackupRead => 3,
        ProjectCreate | ProjectUpdate | ProjectDelete | EnvCreate | EnvUpdate | EnvDelete
        | AppEnvRead | AppCreate | AppUpdate | AppDelete | AppDeploy | AppEnvUpdate
        | AppDomainsManage | BackupManage | BackupRestore | GitProviderManage | AssistantUse => 2,
        _ => 1,
    }
}

#[test]
fn system_roles_match_the_legacy_ceilings() {
    for role in [1_i64, 2, 3] {
        let set = system_role_permissions(role);
        for p in Permission::ALL {
            assert_eq!(
                set.contains(p),
                role <= legacy_ceiling(*p),
                "role {role} / {}",
                p.as_str()
            );
        }
    }
}
