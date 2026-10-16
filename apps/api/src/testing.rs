//! Shared test fixture: a migrated database with two organizations and three
//! users, which is what any tenancy test needs before it can prove anything.
//!
//! Seeded state:
//!
//! | user       | organization-a | organization-b |
//! |------------|----------------|----------------|
//! | `editor`   | Editor (2)     | Editor (2)     |
//! | `viewer`   | Viewer (3)     | —              |
//! | `outsider` | —              | —              |
//!
//! `viewer` is the cross-tenant probe (a real member of exactly one of the two),
//! `outsider` the no-membership probe. `editor` belongs to both on purpose, so a
//! test can show that holding a role somewhere is not the same as holding it
//! here.
//!
//! Lifted out of `handler::projects`, which grew it first; that module and
//! `middleware::org_scope` now share it rather than keeping two copies in sync.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use uuid::Uuid;

use crate::state::state::SharedState;
use crate::{migrations, security::middleware::auth::AuthUser, state::state::AppState};

/// A non-sysadmin `AuthUser` for one of the seeded ids.
pub fn auth_user(user_id: &str) -> AuthUser {
    AuthUser {
        user_id: user_id.to_owned(),
        email: format!("{user_id}@example.com"),
        name: user_id.to_owned(),
        is_sysadmin: false,
        token_id: None,
        token_name: None,
    }
}

/// Same, but with the global SysAdmin flag — which outranks membership
/// everywhere (`handler::common::organization_role`).
pub fn sysadmin_user(user_id: &str) -> AuthUser {
    AuthUser {
        is_sysadmin: true,
        ..auth_user(user_id)
    }
}

/// Opens (creating if needed) a pooled sqlite connection at `path`, with the
/// same connect options production boot uses. The single bootstrap point every
/// test fixture in this codebase builds its database through, so there is
/// exactly one sqlx-pool-construction idiom, not one per file.
pub async fn sqlite_pool_at(path: &Path) -> sqlx::SqlitePool {
    let connect_options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .foreign_keys(true);
    SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(connect_options)
        .await
        .expect("failed to open test sqlite database")
}

pub async fn test_state() -> (SharedState, PathBuf) {
    test_state_for(false).await
}

pub async fn test_state_for(is_cloud: bool) -> (SharedState, PathBuf) {
    test_state_configured(is_cloud, String::new(), PathBuf::new()).await
}

/// `test_state` with a public IP (for generated sslip.io domains) and a data
/// directory (for the Compose file store). Callers own `data_dir` cleanup.
pub async fn test_state_with_network(public_ip: &str, data_dir: PathBuf) -> (SharedState, PathBuf) {
    test_state_configured(false, public_ip.to_owned(), data_dir).await
}

async fn test_state_configured(
    is_cloud: bool,
    public_ip: String,
    data_dir: PathBuf,
) -> (SharedState, PathBuf) {
    let path = std::env::temp_dir().join(format!("litepod-project-handlers-{}.db", Uuid::now_v7()));
    let db = sqlite_pool_at(&path).await;
    migrations::run(&db).await;

    let state = Arc::new(AppState {
        started_at: chrono::Local::now(),
        db,
        caddy_url: String::new(),
        caddy_admin_url: String::new(),
        caddyfile_path: PathBuf::new(),
        caddy_server_upstream: String::new(),
        podman_socket: String::new(),
        pending_execs: Mutex::new(HashMap::new()),
        data_dir,
        data_volume: None,
        host_data_dir: PathBuf::new(),
        podman_network: String::new(),
        public_url: String::new(),
        oauth_states: Mutex::new(HashMap::new()),
        public_ip,
        is_cloud,
        polar: None,
        dev_mode: false,
        port_range: (0, 0),
        reserved_ports: HashSet::new(),
        volume_access: Default::default(),
        podman_host: Default::default(),
        queue_runtime: Default::default(),
        version: String::new(),
        git_sha: String::new(),
        image_repo: String::new(),
        image_tag: String::new(),
        update_channel: Default::default(),
        version_manifest_url: String::new(),
        update_interval_secs: 0,
        container_name: String::new(),
        caddy_container: String::new(),
        stack_env_file: None,
        update_state: Default::default(),
        backup_lock: Default::default(),
        rate_limiter: tokio::sync::RwLock::new(
            crate::security::rate_limit::RateLimiterState::from_config(Default::default()),
        ),
        ephemeral: Default::default(),
    });

    for user_id in ["editor", "viewer", "outsider"] {
        sqlx::query("INSERT INTO users (id, email, hashed_password, name) VALUES (?1, ?2, ?3, ?4)")
            .bind(user_id)
            .bind(format!("{user_id}@example.com"))
            .bind("hash")
            .bind(user_id)
            .execute(&state.db)
            .await
            .unwrap();
    }
    for (organization_id, slug) in [
        ("organization-a", "organization-a"),
        ("organization-b", "organization-b"),
    ] {
        sqlx::query(
            "INSERT INTO organizations (id, name, slug, created_by) VALUES (?1, ?2, ?3, ?4)",
        )
        .bind(organization_id)
        .bind(organization_id)
        .bind(slug)
        .bind("editor")
        .execute(&state.db)
        .await
        .unwrap();
    }
    for (organization_id, user_id, role_id) in [
        ("organization-a", "editor", 2_i64),
        ("organization-b", "editor", 2_i64),
        ("organization-a", "viewer", 3_i64),
    ] {
        sqlx::query(
            "INSERT INTO organization_members (organization_id, user_id, role_id) VALUES (?1, ?2, ?3)",
        )
        .bind(organization_id)
        .bind(user_id)
        .bind(role_id)
        .execute(&state.db)
        .await
        .unwrap();
    }
    (state, path)
}

/// Inserts `project` / `environment` / `application` rows owned by
/// `organization_id`, returning the ids. Ids are derived from the organization
/// so a test reads unambiguously — `organization-b-app` under an
/// `organization-a` URL is visibly the cross-tenant case.
pub async fn seed_application(
    state: &SharedState,
    organization_id: &str,
) -> (String, String, String) {
    let project_id = format!("{organization_id}-project");
    let environment_id = format!("{organization_id}-environment");
    let application_id = format!("{organization_id}-app");
    let server_id = format!("{organization_id}-server");

    // `servers` has a CHECK requiring an ssh row to carry host/port/user.
    sqlx::query(
        "INSERT INTO servers \
         (id, kind, name, organization_id, ingress_mode, state, ssh_host, ssh_port, ssh_user, created_at, updated_at) \
         VALUES (?1, 'ssh', ?1, ?2, 'central', 'ready', '198.51.100.1', 22, 'litepod', 0, 0)",
    )
    .bind(server_id.clone())
    .bind(organization_id)
    .execute(&state.db)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO projects (id, organization_id, name, created_at, updated_at) VALUES (?1, ?2, ?1, 0, 0)",
    )
    .bind(project_id.clone())
    .bind(organization_id)
    .execute(&state.db)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO environments (id, project_id, name, created_at, updated_at) VALUES (?1, ?2, ?1, 0, 0)",
    )
    .bind(environment_id.clone())
    .bind(project_id.clone())
    .execute(&state.db)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO applications (id, environment_id, name, server_id, created_at, updated_at) \
         VALUES (?1, ?2, ?1, ?3, 0, 0)",
    )
    .bind(application_id.clone())
    .bind(environment_id.clone())
    .bind(server_id)
    .execute(&state.db)
    .await
    .unwrap();

    (project_id, environment_id, application_id)
}

/// Grants `system_role_id` (1/2/3) to `user_id` at a scope. Test-only shortcut
/// for what the spec-2 management API will do.
pub async fn assign(
    state: &SharedState,
    organization_id: &str,
    user_id: &str,
    system_role_id: i64,
    scope_type: &str,
    scope_id: &str,
) {
    sqlx::query(
        "INSERT INTO rbac_assignments (id, organization_id, principal_type, principal_id, system_role_id, scope_type, scope_id) \
         VALUES (?1, ?2, 'user', ?3, ?4, ?5, ?6)",
    )
    .bind(Uuid::now_v7().to_string())
    .bind(organization_id)
    .bind(user_id)
    .bind(system_role_id)
    .bind(scope_type)
    .bind(scope_id)
    .execute(&state.db)
    .await
    .unwrap();
}

/// Rewrites a member's org-wide base role (e.g. to 4 = Member).
pub async fn set_base_role(
    state: &SharedState,
    organization_id: &str,
    user_id: &str,
    role_id: i64,
) {
    sqlx::query(
        "UPDATE organization_members SET role_id = ?1 WHERE organization_id = ?2 AND user_id = ?3",
    )
    .bind(role_id)
    .bind(organization_id)
    .bind(user_id)
    .execute(&state.db)
    .await
    .unwrap();
}

/// Builds an [`OrgScope`] directly, for handler tests that want to exercise what
/// a handler does *after* authorization.
///
/// It deliberately performs no check — that is the point. Authorization itself is
/// tested in `middleware::org_scope`, through a real router, where the extractor
/// actually runs. A handler test that used this to assert a 403 would be asserting
/// against its own fixture.
pub fn granted(
    organization_id: &str,
    user_id: &str,
    role_id: i64,
) -> crate::security::middleware::org_scope::OrgScope {
    crate::security::middleware::org_scope::OrgScope {
        organization_id: organization_id.to_owned(),
        user: auth_user(user_id),
        role_id,
        permissions: std::sync::Arc::new(crate::security::authz::PermissionSet::for_base_role(
            role_id,
        )),
    }
}

/// [`granted`] for an environment-scoped handler. Same caveat: no check runs.
pub fn granted_env(
    organization_id: &str,
    project_id: &str,
    environment_id: &str,
    user_id: &str,
    role_id: i64,
) -> crate::security::authz::extract::EnvScope {
    let org = granted(organization_id, user_id, role_id);
    crate::security::authz::extract::EnvScope {
        organization_id: org.organization_id,
        project_id: project_id.to_owned(),
        environment_id: environment_id.to_owned(),
        user: org.user,
        permissions: org.permissions,
    }
}

pub fn cleanup_database(path: &Path) {
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{}", path.display(), suffix));
    }
}
