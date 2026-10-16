use sqlx::Row;

pub async fn run(pool: &sqlx::SqlitePool) {
    // Ensure tracking table exists (in case migration #1 hasn't run yet)
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS _migrations (name TEXT PRIMARY KEY, applied_at TEXT NOT NULL DEFAULT (datetime('now')))",
    )
    .execute(pool)
    .await
    .expect("[migrations] failed to create _migrations table");

    // Read applied migrations
    let rows = sqlx::query("SELECT name FROM _migrations ORDER BY name")
        .fetch_all(pool)
        .await
        .expect("[migrations] failed to query _migrations");

    let mut applied: std::collections::HashSet<String> = std::collections::HashSet::new();
    for row in &rows {
        let name: String = row.try_get(0).expect("[migrations] invalid row");
        applied.insert(name);
    }

    let files = files();

    for path in &files {
        let name = path.file_name().unwrap().to_str().unwrap().to_string();

        if applied.contains(&name) {
            tracing::info!("[migrations] skipping {} (already applied)", name);
            continue;
        }

        tracing::info!("[migrations] applying {}", name);
        let sql = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("[migrations] failed to read {}: {}", name, e));

        sqlx::raw_sql(&sql)
            .execute(pool)
            .await
            .unwrap_or_else(|e| panic!("[migrations] failed applying {}: {}", name, e));

        sqlx::query("INSERT INTO _migrations (name) VALUES (?1)")
            .bind(name.as_str())
            .execute(pool)
            .await
            .unwrap_or_else(|e| panic!("[migrations] failed to record {}: {}", name, e));

        tracing::info!("[migrations] applied {}", name);
    }

    tracing::info!("[migrations] done ({} files processed)", files.len());
}

/// Every migration this binary ships, sorted by file name.
pub fn files() -> Vec<std::path::PathBuf> {
    let migrations_dir = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/migrations"));
    let mut files: Vec<_> = std::fs::read_dir(migrations_dir)
        .expect("[migrations] failed to read migrations directory")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map_or(false, |ext| ext == "sql"))
        .collect();
    files.sort();
    files
}

/// File names of every migration this binary ships.
pub fn names() -> std::collections::HashSet<String> {
    files()
        .iter()
        .filter_map(|path| path.file_name()?.to_str().map(str::to_string))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::run;
    use sqlx::Row;

    async fn temp_db() -> (std::path::PathBuf, sqlx::SqlitePool) {
        let path = std::env::temp_dir().join(format!("litepod-mig-{}.db", uuid::Uuid::now_v7()));
        let pool = crate::testing::sqlite_pool_at(&path).await;
        (path, pool)
    }

    async fn cleanup_temp_db(path: &std::path::Path) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{}", path.display(), suffix));
        }
    }

    #[tokio::test]
    async fn init_schema_creates_all_expected_tables() {
        let (path, pool) = temp_db().await;
        run(&pool).await;

        let tables = sqlx::query("SELECT name FROM sqlite_master WHERE type = 'table'")
            .fetch_all(&pool)
            .await
            .unwrap();
        let table_names: Vec<String> = tables
            .iter()
            .map(|row| row.try_get::<String, _>(0).unwrap())
            .collect();

        for expected in [
            "status_type",
            "status",
            "roles",
            "users",
            "sessions",
            "user_global_roles",
            "recovery_codes",
            "_migrations",
            "organizations",
            "organization_members",
            "projects",
            "environments",
            "applications",
            "application_artifacts",
            "application_deployments",
            "application_environment_variables",
            "application_domains",
            "git_providers",
            "github_apps",
            "webhook_deliveries",
            "server_domain_settings",
            "application_deployment_runs",
            "application_deployment_run_logs",
            "deployment_jobs",
            "build_configuration",
            "application_compose_proxy_routes",
            "application_managed_volumes",
            "s3_config",
            "backup_history",
            "restore_history",
            "servers",
            "server_jobs",
            "scheduled_tasks",
            "billing_accounts",
            "billing_quantity_sync_attempts",
        ] {
            assert!(
                table_names.iter().any(|name| name == expected),
                "missing table {expected}"
            );
        }

        // Compared against the migrations directory rather than a hand-written
        // list, so adding a migration does not require editing this assertion.
        let migrations = sqlx::query("SELECT name FROM _migrations ORDER BY name")
            .fetch_all(&pool)
            .await
            .unwrap();
        let applied: Vec<String> = migrations
            .iter()
            .map(|row| row.try_get::<String, _>(0).unwrap())
            .collect();
        let mut expected: Vec<String> =
            std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/migrations"))
                .unwrap()
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .filter(|name| name.ends_with(".sql"))
                .collect();
        expected.sort();
        assert_eq!(applied, expected);

        let roles = sqlx::query("SELECT name FROM roles ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
        let role_names: Vec<String> = roles
            .iter()
            .map(|row| row.try_get::<String, _>(0).unwrap())
            .collect();
        assert_eq!(
            role_names,
            vec!["SysAdmin", "Admin", "Editor", "Viewer", "Member"]
        );

        let statuses = sqlx::query("SELECT name FROM status ORDER BY status_id")
            .fetch_all(&pool)
            .await
            .unwrap();
        let status_names: Vec<String> = statuses
            .iter()
            .map(|row| row.try_get::<String, _>(0).unwrap())
            .collect();
        assert_eq!(
            status_names,
            vec!["Active", "Inactive", "Pending", "Disabled"]
        );

        drop(pool);
        cleanup_temp_db(&path).await;
    }

    #[tokio::test]
    async fn deployment_queue_schema_and_seed_are_present() {
        let (path, pool) = temp_db().await;
        run(&pool).await;

        let tables = sqlx::query("SELECT name FROM sqlite_master WHERE type = 'table'")
            .fetch_all(&pool)
            .await
            .unwrap();
        let table_names: Vec<String> = tables
            .iter()
            .map(|row| row.try_get::<String, _>(0).unwrap())
            .collect();
        for expected in ["deployment_jobs", "build_configuration"] {
            assert!(
                table_names.iter().any(|name| name == expected),
                "missing table {expected}"
            );
        }

        let indexes = sqlx::query("SELECT name FROM sqlite_master WHERE type = 'index'")
            .fetch_all(&pool)
            .await
            .unwrap();
        let index_names: Vec<String> = indexes
            .iter()
            .map(|row| row.try_get::<String, _>(0).unwrap())
            .collect();
        for expected in [
            "idx_deployment_jobs_status_queued_at",
            "idx_deployment_jobs_application_status",
            "idx_deployment_jobs_run_id",
        ] {
            assert!(
                index_names.iter().any(|name| name == expected),
                "missing index {expected}"
            );
        }

        let configuration =
            sqlx::query("SELECT max_concurrent_builds FROM build_configuration WHERE id = 1")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(configuration.try_get::<i64, _>(0).unwrap(), 3);

        drop(pool);
        cleanup_temp_db(&path).await;
    }

    #[tokio::test]
    async fn project_hierarchy_enforces_cascades_and_one_production() {
        let (path, pool) = temp_db().await;
        run(&pool).await;

        sqlx::query("INSERT INTO users (id, email, hashed_password, name) VALUES (?1, ?2, ?3, ?4)")
            .bind("user-1")
            .bind("user@example.com")
            .bind("hash")
            .bind("User")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO organizations (id, name, slug, created_by) VALUES (?1, ?2, ?3, ?4)",
        )
        .bind("organization-1")
        .bind("Organization")
        .bind("organization")
        .bind("user-1")
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO projects (id, organization_id, name, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind("project-1")
        .bind("organization-1")
        .bind("Project")
        .bind(1_i64)
        .bind(1_i64)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO environments (id, project_id, name, is_production, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .bind("environment-1")
        .bind("project-1")
        .bind("prod")
        .bind(1_i64)
        .bind(1_i64)
        .bind(1_i64)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO applications (id, environment_id, name, server_id, created_at, updated_at) VALUES (?1, ?2, ?3, 'local', ?4, ?5)",
        )
        .bind("application-1")
        .bind("environment-1")
        .bind("Application")
        .bind(1_i64)
        .bind(1_i64)
        .execute(&pool)
        .await
        .unwrap();

        let duplicate_production = sqlx::query(
            "INSERT INTO environments (id, project_id, name, is_production, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .bind("environment-2")
        .bind("project-1")
        .bind("prod 2")
        .bind(1_i64)
        .bind(1_i64)
        .bind(1_i64)
        .execute(&pool)
        .await;
        assert!(duplicate_production.is_err());

        sqlx::query("DELETE FROM projects WHERE id = ?1")
            .bind("project-1")
            .execute(&pool)
            .await
            .unwrap();
        for table in ["environments", "applications"] {
            let row = sqlx::query(&format!("SELECT COUNT(*) FROM {table}"))
                .fetch_one(&pool)
                .await
                .unwrap();
            assert_eq!(
                row.try_get::<i64, _>(0).unwrap(),
                0,
                "{table} should cascade"
            );
        }

        drop(pool);
        cleanup_temp_db(&path).await;
    }

    #[tokio::test]
    async fn git_providers_backfill_static_deploys_and_enforce_rules() {
        let (path, pool) = temp_db().await;
        run(&pool).await;

        sqlx::query("INSERT INTO users (id, email, hashed_password, name) VALUES (?1, ?2, ?3, ?4)")
            .bind("user-1")
            .bind("user@example.com")
            .bind("hash")
            .bind("User")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO organizations (id, name, slug, created_by) VALUES (?1, ?2, ?3, ?4)",
        )
        .bind("organization-1")
        .bind("Organization")
        .bind("organization")
        .bind("user-1")
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO projects (id, organization_id, name, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)").bind("project-1").bind("organization-1").bind("Project").bind(1_i64).bind(1_i64).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO environments (id, project_id, name, is_production, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)").bind("environment-1").bind("project-1").bind("prod").bind(1_i64).bind(1_i64).bind(1_i64).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO applications (id, environment_id, name, server_id, created_at, updated_at) VALUES (?1, ?2, ?3, 'local', ?4, ?5)").bind("application-1").bind("environment-1").bind("Application").bind(1_i64).bind(1_i64).execute(&pool).await.unwrap();

        // deploy_type defaults to git for an application that doesn't set it.
        let row = sqlx::query("SELECT deploy_type FROM applications WHERE id = 'application-1'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(row.try_get::<String, _>(0).unwrap(), "git");

        // Provider insert + (organization_id, name) uniqueness.
        sqlx::query("INSERT INTO git_providers (id, organization_id, name, provider_type, host_url, username, password_encrypted, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)").bind("provider-1").bind("organization-1").bind("Work GitHub").bind("github").bind("https://github.com").bind("octo").bind("encrypted").bind(1_i64).bind(1_i64).execute(&pool).await.unwrap();
        let duplicate = sqlx::query("INSERT INTO git_providers (id, organization_id, name, provider_type, host_url, username, password_encrypted, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)").bind("provider-2").bind("organization-1").bind("Work GitHub").bind("gitlab").bind("https://gitlab.com").bind("octo").bind("encrypted").bind(1_i64).bind(1_i64).execute(&pool).await;
        assert!(duplicate.is_err(), "(organization_id, name) must be unique");

        // provider_type CHECK rejects unknown types.
        let bad_type = sqlx::query("INSERT INTO git_providers (id, organization_id, name, provider_type, host_url, username, password_encrypted, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)").bind("provider-3").bind("organization-1").bind("Other").bind("sourcehut").bind("").bind("octo").bind("encrypted").bind(1_i64).bind(1_i64).execute(&pool).await;
        assert!(bad_type.is_err(), "unknown provider_type rejected");

        // Application can reference a provider; deleting the provider nulls the reference.
        sqlx::query("UPDATE applications SET deploy_type = 'git', git_provider_id = 'provider-1', git_url = 'https://github.com/o/r.git', git_branch = 'main', dockerfile_path = './Dockerfile', container_port = 3000 WHERE id = 'application-1'").execute(&pool).await.unwrap();
        sqlx::query("DELETE FROM git_providers WHERE id = 'provider-1'")
            .execute(&pool)
            .await
            .unwrap();
        let row =
            sqlx::query("SELECT git_provider_id FROM applications WHERE id = 'application-1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(
            row.try_get::<Option<String>, _>(0).unwrap().is_none(),
            "provider delete nulls the FK"
        );

        // Organization delete cascades providers.
        sqlx::query("INSERT INTO git_providers (id, organization_id, name, provider_type, host_url, username, password_encrypted, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)").bind("provider-4").bind("organization-1").bind("Gitea").bind("gitea").bind("").bind("octo").bind("encrypted").bind(1_i64).bind(1_i64).execute(&pool).await.unwrap();
        sqlx::query("DELETE FROM organizations WHERE id = 'organization-1'")
            .execute(&pool)
            .await
            .unwrap();
        let row = sqlx::query("SELECT COUNT(*) FROM git_providers")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            row.try_get::<i64, _>(0).unwrap(),
            0,
            "providers cascade with organization"
        );

        drop(pool);
        cleanup_temp_db(&path).await;
    }

    #[tokio::test]
    async fn application_domains_default_and_enforce_uniqueness() {
        let (path, pool) = temp_db().await;
        run(&pool).await;

        sqlx::query("INSERT INTO users (id, email, hashed_password, name) VALUES (?1, ?2, ?3, ?4)")
            .bind("user-1")
            .bind("user@example.com")
            .bind("hash")
            .bind("User")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO organizations (id, name, slug, created_by) VALUES (?1, ?2, ?3, ?4)",
        )
        .bind("organization-1")
        .bind("Organization")
        .bind("organization")
        .bind("user-1")
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO projects (id, organization_id, name, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)").bind("project-1").bind("organization-1").bind("Project").bind(1_i64).bind(1_i64).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO environments (id, project_id, name, is_production, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)").bind("environment-1").bind("project-1").bind("prod").bind(1_i64).bind(1_i64).bind(1_i64).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO applications (id, environment_id, name, server_id, created_at, updated_at) VALUES (?1, ?2, ?3, 'local', ?4, ?5)").bind("application-1").bind("environment-1").bind("Application").bind(1_i64).bind(1_i64).execute(&pool).await.unwrap();

        // config_stale exists and defaults to 0
        sqlx::query("INSERT INTO application_deployments (application_id, status, container_name, hostname, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)").bind("application-1").bind("not_deployed").bind("litepod-project-prod-application-appli").bind("application-1.example.sslip.io").bind(1_i64).execute(&pool).await.unwrap();
        let row = sqlx::query(
            "SELECT config_stale FROM application_deployments WHERE application_id = ?1",
        )
        .bind("application-1")
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.try_get::<i64, _>(0).unwrap(), 0);

        // (host, port) uniqueness
        sqlx::query("INSERT INTO application_domains (id, application_id, host, port, https, is_primary, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)").bind("domain-1").bind("application-1").bind("localhost").bind(10_000_i64).bind(0_i64).bind(1_i64).bind(1_i64).bind(1_i64).execute(&pool).await.unwrap();
        let duplicate = sqlx::query("INSERT INTO application_domains (id, application_id, host, port, https, is_primary, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)").bind("domain-2").bind("application-1").bind("localhost").bind(10_000_i64).bind(0_i64).bind(0_i64).bind(1_i64).bind(1_i64).execute(&pool).await;
        assert!(duplicate.is_err(), "(host, port) must be unique");

        // one primary per application
        let duplicate_primary = sqlx::query("INSERT INTO application_domains (id, application_id, host, port, https, is_primary, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)").bind("domain-3").bind("application-1").bind("localhost").bind(10_001_i64).bind(0_i64).bind(1_i64).bind(1_i64).bind(1_i64).execute(&pool).await;
        assert!(
            duplicate_primary.is_err(),
            "only one primary domain per application"
        );

        // cascade on application delete
        sqlx::query("DELETE FROM applications WHERE id = ?1")
            .bind("application-1")
            .execute(&pool)
            .await
            .unwrap();
        let remaining = sqlx::query("SELECT COUNT(*) FROM application_domains")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            remaining.try_get::<i64, _>(0).unwrap(),
            0,
            "domains should cascade"
        );

        drop(pool);
        cleanup_temp_db(&path).await;
    }

    #[tokio::test]
    async fn deployment_run_status_enforces_vocabulary() {
        let (path, pool) = temp_db().await;
        run(&pool).await;

        sqlx::query("INSERT INTO users (id, email, hashed_password, name) VALUES ('user-1', 'user@example.com', 'hash', 'User')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO organizations (id, name, slug, created_by) VALUES ('organization-1', 'Organization', 'organization', 'user-1')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO projects (id, organization_id, name, created_at, updated_at) VALUES ('project-1', 'organization-1', 'Project', 1, 1)").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO environments (id, project_id, name, is_production, created_at, updated_at) VALUES ('environment-1', 'project-1', 'prod', 1, 1, 1)").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO applications (id, environment_id, name, server_id, created_at, updated_at) VALUES ('application-1', 'environment-1', 'Application', 'local', 1, 1)").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO application_deployment_runs (id, application_id, source_type, build_method, status, started_at) VALUES ('run-1', 'application-1', 'git', 'docker', 'succeeded', 1)").execute(&pool).await.unwrap();

        for status in ["building", "deploying", "succeeded", "failed", "canceled"] {
            sqlx::query("UPDATE application_deployment_runs SET status = ?1 WHERE id = 'run-1'")
                .bind(status)
                .execute(&pool)
                .await
                .unwrap_or_else(|error| panic!("status {status} should be allowed: {error}"));
        }
        assert!(
            sqlx::query(
                "UPDATE application_deployment_runs SET status = 'running' WHERE id = 'run-1'",
            )
            .execute(&pool)
            .await
            .is_err(),
            "'running' is the container's vocabulary and must be rejected"
        );

        drop(pool);
        cleanup_temp_db(&path).await;
    }

    #[tokio::test]
    async fn deployment_run_actor_allows_lifecycle_actions_and_nulls_deleted_actor() {
        let (path, pool) = temp_db().await;
        run(&pool).await;

        sqlx::query("INSERT INTO users (id, email, hashed_password, name) VALUES ('owner-1', 'owner@example.com', 'hash', 'Owner')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO users (id, email, hashed_password, name) VALUES ('operator-1', 'operator@example.com', 'hash', 'Operator')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO organizations (id, name, slug, created_by) VALUES ('organization-1', 'Organization', 'organization', 'owner-1')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO projects (id, organization_id, name, created_at, updated_at) VALUES ('project-1', 'organization-1', 'Project', 1, 1)").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO environments (id, project_id, name, is_production, created_at, updated_at) VALUES ('environment-1', 'project-1', 'prod', 1, 1, 1)").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO applications (id, environment_id, name, server_id, created_at, updated_at) VALUES ('application-1', 'environment-1', 'Application', 'local', 1, 1)").execute(&pool).await.unwrap();

        for (index, trigger) in [
            "manual_deploy",
            "manual_start",
            "manual_stop",
            "manual_remove",
        ]
        .iter()
        .enumerate()
        {
            sqlx::query(
                "INSERT INTO application_deployment_runs (id, application_id, source_type, build_method, status, started_at, triggered_by, created_by) VALUES (?1, 'application-1', 'compose', 'compose', 'succeeded', ?2, ?3, 'operator-1')",
            )
            .bind(format!("run-{index}"))
            .bind(index as i64)
            .bind(*trigger)
            .execute(&pool)
            .await
            .unwrap_or_else(|error| panic!("trigger {trigger} should be allowed: {error}"));
        }
        sqlx::query("DELETE FROM users WHERE id = 'operator-1'")
            .execute(&pool)
            .await
            .unwrap();
        let row = sqlx::query(
            "SELECT COUNT(*) FROM application_deployment_runs WHERE created_by IS NULL",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.try_get::<i64, _>(0).unwrap(), 4);
        assert!(
            sqlx::query(
                "UPDATE application_deployment_runs SET triggered_by = 'unknown' WHERE id = 'run-0'",
            )
            .execute(&pool)
            .await
            .is_err()
        );

        drop(pool);
        cleanup_temp_db(&path).await;
    }

    #[tokio::test]
    async fn notifications_enforce_vocabulary_and_cascade() {
        let (path, pool) = temp_db().await;
        run(&pool).await;

        sqlx::query("INSERT INTO users (id, email, hashed_password, name) VALUES ('u1', 'u@example.com', 'hash', 'U')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO organizations (id, name, slug, created_by, is_root) VALUES ('t1', 'Root', 'root', 'u1', 1)").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO notification_providers (id, organization_id, name, kind, config_encrypted, created_at, updated_at) VALUES ('p1', 't1', 'Alerts', 'discord', 'blob', 1, 1)").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO notification_subscriptions (provider_id, action) VALUES ('p1', 'app_deploy')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO notification_deliveries (id, provider_id, action, status, next_attempt_at, payload, created_at, updated_at) VALUES ('d1', 'p1', 'app_deploy', 'pending', 1, '{}', 1, 1)").execute(&pool).await.unwrap();

        // The vocabulary is enforced by the schema, not only by Rust.
        assert!(
            sqlx::query("INSERT INTO notification_providers (id, organization_id, name, kind, config_encrypted, created_at, updated_at) VALUES ('p2', 't1', 'Lark', 'lark', 'blob', 1, 1)")
                .execute(&pool)
                .await
                .is_err(),
            "kind CHECK should reject 'lark'"
        );
        assert!(
            sqlx::query("INSERT INTO notification_subscriptions (provider_id, action) VALUES ('p1', 'not_an_action')")
                .execute(&pool)
                .await
                .is_err(),
            "action CHECK should reject unknown actions"
        );
        // 'database_backup' joined the vocabulary with the per-database backup feature.
        sqlx::query("INSERT INTO notification_subscriptions (provider_id, action) VALUES ('p1', 'database_backup')")
            .execute(&pool)
            .await
            .expect("action CHECK should accept 'database_backup'");
        assert!(
            sqlx::query("INSERT INTO notification_providers (id, organization_id, name, kind, config_encrypted, created_at, updated_at) VALUES ('p3', 't1', 'Alerts', 'slack', 'blob', 1, 1)")
                .execute(&pool)
                .await
                .is_err(),
            "(organization_id, name) should be unique inside an organization"
        );

        // Deleting the organization must not leave orphan providers, subscriptions or
        // queued deliveries pointing at credentials that no longer exist.
        sqlx::query("DELETE FROM organizations WHERE id = 't1'")
            .execute(&pool)
            .await
            .unwrap();
        for table in [
            "notification_providers",
            "notification_subscriptions",
            "notification_deliveries",
        ] {
            let row = sqlx::query(&format!("SELECT COUNT(*) FROM {table}"))
                .fetch_one(&pool)
                .await
                .unwrap();
            let count: i64 = row.try_get(0).unwrap();
            assert_eq!(count, 0, "{table} should have cascaded");
        }

        drop(pool);
        cleanup_temp_db(&path).await;
    }

    #[tokio::test]
    async fn scheduled_tasks_schema_has_expected_tables_and_server_column() {
        let (path, pool) = temp_db().await;
        run(&pool).await;

        for table in [
            "scheduled_tasks",
            "scheduled_task_executions",
            "scheduled_task_execution_logs",
        ] {
            let row =
                sqlx::query("SELECT name FROM sqlite_master WHERE type = 'table' AND name = ?1")
                    .bind(table)
                    .fetch_optional(&pool)
                    .await
                    .unwrap();
            assert!(row.is_some(), "missing table {table}");
        }

        for table in ["scheduled_tasks", "scheduled_task_executions"] {
            let row =
                sqlx::query("SELECT name FROM pragma_table_info(?1) WHERE name = 'server_id'")
                    .bind(table)
                    .fetch_optional(&pool)
                    .await
                    .unwrap();
            assert!(
                row.is_some(),
                "table {table} is missing the server_id column"
            );
        }

        drop(pool);
        cleanup_temp_db(&path).await;
    }

    #[tokio::test]
    async fn invitations_enforce_token_uniqueness_and_cascade() {
        let (path, pool) = temp_db().await;
        run(&pool).await;

        sqlx::query("INSERT INTO users (id, email, hashed_password, name) VALUES ('admin', 'admin@example.test', 'hash', 'Admin')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO users (id, email, hashed_password, name, status_id) VALUES ('pending', 'pending@example.test', 'hash', 'Pending', 3)").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO invitations (id, user_id, created_by, token_hash, expires_at) VALUES ('i1', 'pending', 'admin', 'hash', datetime('now', '+7 days'))").execute(&pool).await.unwrap();
        assert!(
            sqlx::query("INSERT INTO invitations (id, user_id, created_by, token_hash, expires_at) VALUES ('i2', 'pending', 'admin', 'hash', datetime('now', '+7 days'))")
                .execute(&pool)
                .await
                .is_err()
        );

        sqlx::query("DELETE FROM users WHERE id = 'pending'")
            .execute(&pool)
            .await
            .unwrap();
        let row = sqlx::query("SELECT COUNT(*) FROM invitations")
            .fetch_one(&pool)
            .await
            .unwrap();
        let count: i64 = row.try_get(0).unwrap();
        assert_eq!(count, 0);

        drop(pool);
        cleanup_temp_db(&path).await;
    }

    #[tokio::test]
    async fn server_groups_schema_and_endpoint_widening_are_present() {
        let (path, pool) = temp_db().await;
        run(&pool).await;

        for table in [
            "server_groups",
            "server_group_members",
            "application_replicas",
        ] {
            let exists =
                sqlx::query("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1")
                    .bind(table)
                    .fetch_optional(&pool)
                    .await
                    .unwrap()
                    .is_some();
            assert!(exists, "missing table {table}");
        }

        let app_cols = sqlx::query("PRAGMA table_info(applications)")
            .fetch_all(&pool)
            .await
            .unwrap();
        let app_names: std::collections::HashSet<String> = app_cols
            .iter()
            .map(|r| r.try_get::<String, _>(1).unwrap())
            .collect();
        assert!(app_names.contains("server_group_id"));

        let domain_cols = sqlx::query("PRAGMA table_info(application_domains)")
            .fetch_all(&pool)
            .await
            .unwrap();
        let domain_names: std::collections::HashSet<String> = domain_cols
            .iter()
            .map(|r| r.try_get::<String, _>(1).unwrap())
            .collect();
        assert!(domain_names.contains("server_id"));

        // The rebuilt endpoint tables must key on (application_id, server_id, container_port).
        let runtime_pk = sqlx::query("PRAGMA table_info(application_runtime_endpoints)")
            .fetch_all(&pool)
            .await
            .unwrap();
        let runtime_pk_cols: Vec<String> = runtime_pk
            .iter()
            .filter(|r| r.try_get::<i64, _>(5).unwrap() > 0)
            .map(|r| r.try_get::<String, _>(1).unwrap())
            .collect();
        assert_eq!(
            runtime_pk_cols,
            vec!["application_id", "server_id", "container_port"]
        );

        drop(pool);
        cleanup_temp_db(&path).await;
    }

    #[tokio::test]
    async fn applications_require_exactly_one_target_and_group_endpoints_place() {
        let (path, pool) = temp_db().await;
        run(&pool).await;

        sqlx::query("INSERT INTO users (id, email, hashed_password, name) VALUES ('user-1', 'user@example.com', 'hash', 'User')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO organizations (id, name, slug, created_by) VALUES ('organization-1', 'Organization', 'organization', 'user-1')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO projects (id, organization_id, name, created_at, updated_at) VALUES ('project-1', 'organization-1', 'Project', 1, 1)").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO environments (id, project_id, name, is_production, created_at, updated_at) VALUES ('environment-1', 'project-1', 'prod', 1, 1, 1)").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO server_groups (id, organization_id, name, created_at, updated_at) VALUES ('group-1', 'organization-1', 'dev', 1, 1)").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO servers (id, kind, name, ssh_host, ssh_port, ssh_user, ingress_mode, state, created_at, updated_at) VALUES ('remote-1', 'ssh', 'Remote', 'remote.example.test', 22, 'litepod', 'public_edge', 'ready', 1, 1)").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO server_group_members (group_id, server_id, created_at) VALUES ('group-1', 'remote-1', 1)").execute(&pool).await.unwrap();

        assert!(
            sqlx::query("INSERT INTO applications (id, environment_id, name, created_at, updated_at) VALUES ('app-null', 'environment-1', 'App', 1, 1)")
                .execute(&pool).await.is_err(),
            "an application with no target must be rejected"
        );
        assert!(
            sqlx::query("INSERT INTO applications (id, environment_id, name, server_id, server_group_id, created_at, updated_at) VALUES ('app-both', 'environment-1', 'App', 'local', 'group-1', 1, 1)")
                .execute(&pool).await.is_err(),
            "an application may not target both a server and a group"
        );

        sqlx::query("INSERT INTO applications (id, environment_id, name, server_group_id, created_at, updated_at) VALUES ('app-group', 'environment-1', 'App', 'group-1', 1, 1)").execute(&pool).await.unwrap();

        // A group app places its runtime endpoint on a member remote server.
        sqlx::query("INSERT INTO application_runtime_endpoints (application_id, server_id, host_port, container_port, created_at, updated_at) VALUES ('app-group', 'remote-1', 42123, 3000, 1, 1)").execute(&pool).await.unwrap();
        assert!(
            sqlx::query("INSERT INTO application_runtime_endpoints (application_id, server_id, host_port, container_port, created_at, updated_at) VALUES ('app-group', 'local', 42124, 3001, 1, 1)")
                .execute(&pool).await.is_err(),
            "a group endpoint may only place on a member remote server"
        );

        drop(pool);
        cleanup_temp_db(&path).await;
    }

    #[tokio::test]
    async fn applications_have_catalog_columns() {
        let (path, pool) = temp_db().await;
        run(&pool).await;

        let rows = sqlx::query("PRAGMA table_info(applications)")
            .fetch_all(&pool)
            .await
            .unwrap();
        let names: std::collections::HashSet<String> = rows
            .iter()
            .map(|row| row.try_get::<String, _>(1).unwrap())
            .collect();
        for expected in [
            "catalog_template_id",
            "catalog_icon_url",
            "catalog_primary_port",
        ] {
            assert!(names.contains(expected), "missing column {expected}");
        }

        drop(pool);
        cleanup_temp_db(&path).await;
    }

    #[tokio::test]
    async fn pending_email_verification_deadline_is_queryable() {
        let (path, pool) = temp_db().await;
        run(&pool).await;

        sqlx::query(
            "INSERT INTO users (id, email, hashed_password, name, status_id, email_verification_expires_at)
             VALUES ('pending-email', 'pending@example.test', 'hash', 'Pending', 3, '2026-08-07 12:00:00')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let row = sqlx::query(
            "SELECT id FROM users WHERE status_id = 3 AND email_verification_expires_at <= ?1",
        )
        .bind("2026-08-07 12:00:00")
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.try_get::<String, _>(0).unwrap(), "pending-email");

        drop(pool);
        cleanup_temp_db(&path).await;
    }

    #[tokio::test]
    async fn servers_seed_local_server_and_enforce_placement_rules() {
        let (path, pool) = temp_db().await;
        run(&pool).await;

        let local =
            sqlx::query("SELECT id, kind, ingress_mode, state FROM servers WHERE id = 'local'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(local.try_get::<String, _>(0).unwrap(), "local");
        assert_eq!(local.try_get::<String, _>(1).unwrap(), "local");
        assert_eq!(local.try_get::<String, _>(2).unwrap(), "central");
        assert_eq!(local.try_get::<String, _>(3).unwrap(), "ready");

        sqlx::query("INSERT INTO users (id, email, hashed_password, name) VALUES ('user-1', 'user@example.com', 'hash', 'User')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO organizations (id, name, slug, created_by) VALUES ('organization-1', 'Organization', 'organization', 'user-1')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO projects (id, organization_id, name, created_at, updated_at) VALUES ('project-1', 'organization-1', 'Project', 1, 1)").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO environments (id, project_id, name, is_production, created_at, updated_at) VALUES ('environment-1', 'project-1', 'prod', 1, 1, 1)").execute(&pool).await.unwrap();
        assert!(
            sqlx::query("INSERT INTO applications (id, environment_id, name, created_at, updated_at) VALUES ('application-1', 'environment-1', 'Application', 1, 1)")
                .execute(&pool)
                .await
                .is_err(),
            "new applications must not silently fall back to Local"
        );
        sqlx::query("INSERT INTO applications (id, environment_id, name, server_id, created_at, updated_at) VALUES ('application-1', 'environment-1', 'Application', 'local', 1, 1)").execute(&pool).await.unwrap();

        let server_id: String =
            sqlx::query("SELECT server_id FROM applications WHERE id = 'application-1'")
                .fetch_one(&pool)
                .await
                .unwrap()
                .try_get(0)
                .unwrap();
        assert_eq!(server_id, "local");

        assert!(
            sqlx::query("INSERT INTO servers (id, kind, name, ingress_mode, state, created_at, updated_at) VALUES ('not-local', 'local', 'Other local', 'central', 'ready', 1, 1)")
                .execute(&pool)
                .await
                .is_err()
        );
        assert!(
            sqlx::query("INSERT INTO servers (id, kind, name, ingress_mode, state, created_at, updated_at) VALUES ('invalid-mode', 'ssh', 'Invalid', 'bad', 'ready', 1, 1)")
                .execute(&pool)
                .await
                .is_err()
        );

        sqlx::query("INSERT INTO servers (id, kind, name, ssh_host, ssh_port, ssh_user, ingress_mode, state, created_at, updated_at) VALUES ('remote-1', 'ssh', 'Remote', 'remote.example.test', 22, 'litepod', 'private_relay', 'ready', 1, 1)").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO applications (id, environment_id, name, server_id, created_at, updated_at) VALUES ('application-remote-1', 'environment-1', 'Remote application', 'remote-1', 1, 1)").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO server_jobs (id, server_id, kind, status, actor_user_id, phase, created_at) VALUES ('setup-1', 'remote-1', 'setup', 'queued', 'user-1', 'queued', 1)").execute(&pool).await.unwrap();
        assert!(
            sqlx::query("INSERT INTO server_jobs (id, server_id, kind, status, actor_user_id, phase, created_at) VALUES ('repair-1', 'remote-1', 'repair', 'queued', 'user-1', 'queued', 1)")
                .execute(&pool)
                .await
                .is_err(),
            "only one Setup or Repair job may be active for a server"
        );
        sqlx::query("INSERT INTO application_runtime_endpoints (application_id, server_id, host_port, container_port, created_at, updated_at) VALUES ('application-remote-1', 'remote-1', 42123, 3000, 1, 1)").execute(&pool).await.unwrap();
        assert!(
            sqlx::query("INSERT INTO application_runtime_endpoints (application_id, server_id, host_port, container_port, created_at, updated_at) VALUES ('application-1', 'local', 42123, 80, 1, 1)")
                .execute(&pool)
                .await
                .is_err(),
            "Local applications must retain their existing non-endpoint runtime"
        );
        assert!(
            sqlx::query("UPDATE application_runtime_endpoints SET server_id = 'local' WHERE application_id = 'application-remote-1'")
                .execute(&pool)
                .await
                .is_err(),
            "An endpoint may never move away from immutable application placement"
        );

        drop(pool);
        cleanup_temp_db(&path).await;
    }

    #[tokio::test]
    async fn servers_have_host_key_known_hosts_column() {
        let (path, pool) = temp_db().await;
        run(&pool).await;

        let rows = sqlx::query("PRAGMA table_info(servers)")
            .fetch_all(&pool)
            .await
            .unwrap();
        let names: std::collections::HashSet<String> = rows
            .iter()
            .map(|row| row.try_get::<String, _>(1).unwrap())
            .collect();
        assert!(names.contains("host_key_known_hosts"));

        drop(pool);
        cleanup_temp_db(&path).await;
    }

    #[tokio::test]
    async fn servers_carry_their_geolocation() {
        let (path, pool) = temp_db().await;
        run(&pool).await;

        let rows = sqlx::query("PRAGMA table_info(servers)")
            .fetch_all(&pool)
            .await
            .unwrap();
        let names: std::collections::HashSet<String> = rows
            .iter()
            .map(|row| row.try_get::<String, _>(1).unwrap())
            .collect();
        for column in [
            "latitude",
            "longitude",
            "geo_city",
            "geo_country",
            "geo_asn",
            "geo_timezone",
            "geo_resolved_at",
        ] {
            assert!(names.contains(column), "servers is missing {column}");
        }

        // The seeded local server starts unplaced, so the fleet map knows to look it up.
        let resolved_at: Option<i64> =
            sqlx::query("SELECT geo_resolved_at FROM servers WHERE id = 'local'")
                .fetch_one(&pool)
                .await
                .unwrap()
                .try_get(0)
                .unwrap();
        assert_eq!(resolved_at, None);

        drop(pool);
        cleanup_temp_db(&path).await;
    }

    #[tokio::test]
    async fn billing_webhook_deliveries_round_trip() {
        let (path, pool) = temp_db().await;
        run(&pool).await;

        sqlx::query("INSERT INTO billing_webhook_deliveries (id, request_headers_json, request_body) VALUES ('delivery-1', '{}', '{}')").execute(&pool).await.unwrap();
        let row = sqlx::query(
            "SELECT request_headers_json, request_body, response_status FROM billing_webhook_deliveries WHERE id = 'delivery-1'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.try_get::<String, _>(0).unwrap(), "{}");
        assert_eq!(row.try_get::<String, _>(1).unwrap(), "{}");
        assert_eq!(row.try_get::<Option<i64>, _>(2).unwrap(), None);

        drop(pool);
        cleanup_temp_db(&path).await;
    }

    #[tokio::test]
    async fn billing_reconciliation_tables_accept_rows() {
        let (path, pool) = temp_db().await;
        run(&pool).await;

        sqlx::query("INSERT INTO billing_external_records (id, polar_customer_id, subscription_status, state) VALUES ('external-1', 'ctm_1', 'active', 'unassigned_external')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO billing_reconciliation_runs (id, processed_count, candidate_count) VALUES ('run-1', 1, 2)").execute(&pool).await.unwrap();

        drop(pool);
        cleanup_temp_db(&path).await;
    }

    /// Coolify-style base + per-server billing: a subscription carries a base
    /// item (qty 1) and a per-server item whose quantity is the number of
    /// server slots the payer has purchased (not auto-derived from usage).
    #[tokio::test]
    async fn billing_accounts_support_purchased_server_quantity() {
        let (path, pool) = temp_db().await;
        run(&pool).await;

        sqlx::query("INSERT INTO users (id, email, hashed_password, name) VALUES ('payer-1', 'payer@example.test', 'hash', 'Payer')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO billing_accounts (user_id, polar_subscription_id, polar_product_id, base_price_id, per_server_price_id, status, purchased_server_quantity) VALUES ('payer-1', 'sub_1', 'pro_1', 'pri_base', 'pri_per_server', 'active', 2)").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO billing_quantity_sync_attempts (id, user_id, polar_subscription_id, previous_quantity, target_quantity, proration_mode, status) VALUES ('attempt-1', 'payer-1', 'sub_1', 1, 2, 'prorated_immediately', 'success')").execute(&pool).await.unwrap();
        assert!(
            sqlx::query("INSERT INTO billing_quantity_sync_attempts (id, user_id, polar_subscription_id, previous_quantity, target_quantity, proration_mode, status) VALUES ('attempt-2', 'payer-1', 'sub_1', 2, 1, 'prorated_next_billing_period', 'partial')")
                .execute(&pool)
                .await
                .is_err(),
            "status CHECK should reject anything outside success/failed"
        );

        let row = sqlx::query(
            "SELECT purchased_server_quantity FROM billing_accounts WHERE user_id = 'payer-1'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.try_get::<i64, _>(0).unwrap(), 2);

        drop(pool);
        cleanup_temp_db(&path).await;
    }

    #[tokio::test]
    async fn deployment_runs_gain_rollback_columns_and_accept_manual_rollback() {
        let (path, pool) = temp_db().await;
        run(&pool).await;

        sqlx::query("INSERT INTO users (id, email, hashed_password, name) VALUES ('user-1', 'user@example.com', 'hash', 'User')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO organizations (id, name, slug, created_by) VALUES ('organization-1', 'Organization', 'organization', 'user-1')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO projects (id, organization_id, name, created_at, updated_at) VALUES ('project-1', 'organization-1', 'Project', 1, 1)").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO environments (id, project_id, name, is_production, created_at, updated_at) VALUES ('environment-1', 'project-1', 'prod', 1, 1, 1)").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO applications (id, environment_id, name, server_id, created_at, updated_at) VALUES ('application-1', 'environment-1', 'Application', 'local', 1, 1)").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO application_artifacts (id, application_id, filename, kind, byte_size, sha256, relative_path, created_at) VALUES ('artifact-1', 'application-1', 'site.zip', 'zip', 10, 'sha', 'path', 1)").execute(&pool).await.unwrap();

        sqlx::query("INSERT INTO application_deployment_runs (id, application_id, source_type, build_method, status, started_at, triggered_by, artifact_id) VALUES ('run-1', 'application-1', 'static', 'static', 'succeeded', 1, 'manual_rollback', 'artifact-1')").execute(&pool).await.unwrap();
        let row = sqlx::query(
            "SELECT artifact_id, triggered_by FROM application_deployment_runs WHERE id = 'run-1'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.try_get::<String, _>(0).unwrap(), "artifact-1");
        assert_eq!(row.try_get::<String, _>(1).unwrap(), "manual_rollback");

        assert!(
            sqlx::query("INSERT INTO application_deployment_runs (id, application_id, source_type, build_method, status, started_at, triggered_by) VALUES ('run-2', 'application-1', 'static', 'static', 'succeeded', 1, 'unknown_trigger')").execute(&pool).await.is_err(),
            "triggered_by CHECK should still reject values outside its vocabulary"
        );

        drop(pool);
        cleanup_temp_db(&path).await;
    }

    /// The real risk of the 0017 migration: `application_deployment_runs` is
    /// the parent of two `ON DELETE CASCADE` children. Rebuilding it via
    /// rename+recreate+drop with `foreign_keys` left ON would cascade-delete
    /// every log line and queue job in the database the moment this ships to
    /// an existing install - `temp_db()` can't catch that because it always
    /// applies every migration to an empty table. This test instead applies
    /// only migrations through 0016, inserts real child rows, then applies
    /// 0017's actual file content and asserts the children survive.
    #[tokio::test]
    async fn migration_0017_rebuild_does_not_cascade_delete_existing_children() {
        let (path, pool) = temp_db().await;

        let migrations_dir =
            std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/migrations"));
        let mut files: Vec<_> = std::fs::read_dir(migrations_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map_or(false, |ext| ext == "sql"))
            .collect();
        files.sort();
        let (before_0017, from_0017): (Vec<_>, Vec<_>) = files
            .into_iter()
            .partition(|p| p.file_name().unwrap().to_str().unwrap() < "0017");
        for path in &before_0017 {
            let sql = std::fs::read_to_string(path).unwrap();
            sqlx::raw_sql(&sql).execute(&pool).await.unwrap();
        }

        sqlx::query("INSERT INTO users (id, email, hashed_password, name) VALUES ('user-1', 'user@example.com', 'hash', 'User')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO organizations (id, name, slug, created_by) VALUES ('organization-1', 'Organization', 'organization', 'user-1')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO projects (id, organization_id, name, created_at, updated_at) VALUES ('project-1', 'organization-1', 'Project', 1, 1)").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO environments (id, project_id, name, is_production, created_at, updated_at) VALUES ('environment-1', 'project-1', 'prod', 1, 1, 1)").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO applications (id, environment_id, name, server_id, created_at, updated_at) VALUES ('application-1', 'environment-1', 'Application', 'local', 1, 1)").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO application_deployment_runs (id, application_id, source_type, build_method, status, started_at, triggered_by) VALUES ('run-1', 'application-1', 'git', 'docker', 'succeeded', 1, 'manual')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO application_deployment_run_logs (id, run_id, sequence, level, message, created_at) VALUES ('log-1', 'run-1', 0, 'info', 'hi', 1)").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO deployment_jobs (id, application_id, project_id, environment_id, organization_id, run_id, source_type, source_snapshot, build_method, build_snapshot, status, queued_at) VALUES ('job-1', 'application-1', 'project-1', 'environment-1', 'organization-1', 'run-1', 'git', '{}', 'docker', '{}', 'succeeded', 1)").execute(&pool).await.unwrap();

        for path in &from_0017 {
            let sql = std::fs::read_to_string(path).unwrap();
            sqlx::raw_sql(&sql).execute(&pool).await.unwrap();
        }

        let run_count: i64 =
            sqlx::query("SELECT COUNT(*) FROM application_deployment_runs WHERE id = 'run-1'")
                .fetch_one(&pool)
                .await
                .unwrap()
                .try_get(0)
                .unwrap();
        let log_count: i64 = sqlx::query(
            "SELECT COUNT(*) FROM application_deployment_run_logs WHERE run_id = 'run-1'",
        )
        .fetch_one(&pool)
        .await
        .unwrap()
        .try_get(0)
        .unwrap();
        let job_count: i64 =
            sqlx::query("SELECT COUNT(*) FROM deployment_jobs WHERE run_id = 'run-1'")
                .fetch_one(&pool)
                .await
                .unwrap()
                .try_get(0)
                .unwrap();
        assert_eq!(run_count, 1, "the run itself must survive the rebuild");
        assert_eq!(
            log_count, 1,
            "cascade must not wipe existing log lines on upgrade"
        );
        assert_eq!(
            job_count, 1,
            "cascade must not wipe existing queue jobs on upgrade"
        );

        let violations = sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert!(
            violations.is_empty(),
            "foreign_keys must be back ON with no dangling references after the rebuild"
        );

        drop(pool);
        cleanup_temp_db(&path).await;
    }

    /// 0039 backfills `desired_state` from what already exists, so it must be tested
    /// against rows that predate it, not an empty schema.
    #[tokio::test]
    async fn migration_0039_backfills_intent_and_widens_subscriptions() {
        let (path, pool) = temp_db().await;
        let (before, from): (Vec<_>, Vec<_>) = super::files()
            .into_iter()
            .partition(|p| p.file_name().unwrap().to_str().unwrap() < "0039");
        for path in &before {
            sqlx::raw_sql(&std::fs::read_to_string(path).unwrap())
                .execute(&pool)
                .await
                .unwrap();
        }

        for sql in [
            "INSERT INTO users (id, email, hashed_password, name) VALUES ('user-1', 'user@example.com', 'hash', 'User')",
            "INSERT INTO organizations (id, name, slug, created_by) VALUES ('organization-1', 'Organization', 'organization', 'user-1')",
            "INSERT INTO projects (id, organization_id, name, created_at, updated_at) VALUES ('project-1', 'organization-1', 'Project', 1, 1)",
            "INSERT INTO environments (id, project_id, name, is_production, created_at, updated_at) VALUES ('environment-1', 'project-1', 'prod', 1, 1, 1)",
            "INSERT INTO applications (id, environment_id, name, server_id, created_at, updated_at) VALUES ('app-running', 'environment-1', 'running', 'local', 1, 1)",
            "INSERT INTO applications (id, environment_id, name, server_id, created_at, updated_at) VALUES ('app-user-stopped', 'environment-1', 'user-stopped', 'local', 1, 1)",
            "INSERT INTO applications (id, environment_id, name, server_id, created_at, updated_at) VALUES ('app-restarted', 'environment-1', 'restarted', 'local', 1, 1)",
            "INSERT INTO applications (id, environment_id, name, server_id, created_at, updated_at) VALUES ('app-new', 'environment-1', 'new', 'local', 1, 1)",
            // The monitor already rewrote every status to `stopped` after the last reboot.
            "INSERT INTO application_deployments (application_id, status, container_name, hostname, updated_at) VALUES ('app-running', 'stopped', 'a', 'a.test', 1)",
            "INSERT INTO application_deployments (application_id, status, container_name, hostname, updated_at) VALUES ('app-user-stopped', 'stopped', 'b', 'b.test', 1)",
            "INSERT INTO application_deployments (application_id, status, container_name, hostname, updated_at) VALUES ('app-restarted', 'running', 'c', 'c.test', 1)",
            "INSERT INTO application_deployments (application_id, status, container_name, hostname, updated_at) VALUES ('app-new', 'not_deployed', 'd', 'd.test', 1)",
            // Ids are uuid v7 text, so lexical order is time order.
            "INSERT INTO audit_logs (id, organization_id, action, target_type, target_id) VALUES ('0001', 'organization-1', 'deployment.started', 'application', 'app-user-stopped')",
            "INSERT INTO audit_logs (id, organization_id, action, target_type, target_id) VALUES ('0002', 'organization-1', 'deployment.stopped', 'application', 'app-user-stopped')",
            "INSERT INTO audit_logs (id, organization_id, action, target_type, target_id) VALUES ('0003', 'organization-1', 'deployment.stopped', 'application', 'app-restarted')",
            "INSERT INTO audit_logs (id, organization_id, action, target_type, target_id) VALUES ('0004', 'organization-1', 'deployment.started', 'application', 'app-restarted')",
            "INSERT INTO notification_providers (id, organization_id, name, kind, config_encrypted, created_at, updated_at) VALUES ('provider-1', 'organization-1', 'Slack', 'slack', 'x', 1, 1)",
            "INSERT INTO notification_subscriptions (provider_id, action) VALUES ('provider-1', 'app_deploy')",
        ] {
            sqlx::query(sql).execute(&pool).await.unwrap();
        }

        for path in &from {
            sqlx::raw_sql(&std::fs::read_to_string(path).unwrap())
                .execute(&pool)
                .await
                .unwrap();
        }

        let desired = |id: &'static str| {
            let pool = pool.clone();
            async move {
                sqlx::query(
                    "SELECT desired_state FROM application_deployments WHERE application_id = ?1",
                )
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap()
                .try_get::<String, _>(0)
                .unwrap()
            }
        };
        assert_eq!(
            desired("app-running").await,
            "running",
            "no audit trail: a deployed app counts as wanted"
        );
        assert_eq!(
            desired("app-user-stopped").await,
            "stopped",
            "the last user action was a stop"
        );
        assert_eq!(
            desired("app-restarted").await,
            "running",
            "the last user action was a start"
        );
        assert_eq!(desired("app-new").await, "stopped", "never deployed");

        // The trigger records intent whenever something reports the app running.
        sqlx::query("UPDATE application_deployments SET status = 'running' WHERE application_id = 'app-user-stopped'")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(desired("app-user-stopped").await, "running");
        // ...and never flips it back on `stopped`: a reboot is not a user stop.
        sqlx::query("UPDATE application_deployments SET status = 'stopped' WHERE application_id = 'app-user-stopped'")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(desired("app-user-stopped").await, "running");

        let policy: String =
            sqlx::query("SELECT boot_policy FROM applications WHERE id = 'app-running'")
                .fetch_one(&pool)
                .await
                .unwrap()
                .try_get(0)
                .unwrap();
        assert_eq!(policy, "restore");
        assert!(
            sqlx::query(
                "UPDATE applications SET boot_policy = 'sometimes' WHERE id = 'app-running'"
            )
            .execute(&pool)
            .await
            .is_err()
        );

        let kept: i64 = sqlx::query("SELECT COUNT(*) FROM notification_subscriptions WHERE provider_id = 'provider-1' AND action = 'app_deploy'")
            .fetch_one(&pool)
            .await
            .unwrap()
            .try_get(0)
            .unwrap();
        assert_eq!(kept, 1, "the rebuild must keep existing subscriptions");
        sqlx::query("INSERT INTO notification_subscriptions (provider_id, action) VALUES ('provider-1', 'server_reboot')")
            .execute(&pool)
            .await
            .unwrap();
        assert!(
            sqlx::query("SELECT last_boot_id FROM servers")
                .fetch_all(&pool)
                .await
                .is_ok()
        );
        assert!(
            sqlx::query("PRAGMA foreign_key_check")
                .fetch_all(&pool)
                .await
                .unwrap()
                .is_empty()
        );

        drop(pool);
        cleanup_temp_db(&path).await;
    }

    #[tokio::test]
    async fn rbac_tables_enforce_shape_and_clean_up_after_their_targets() {
        let (path, pool) = temp_db().await;
        run(&pool).await;

        let member: String = sqlx::query_scalar("SELECT name FROM roles WHERE id = 4")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(member, "Member");

        for sql in [
            "INSERT INTO users (id, email, hashed_password, name) VALUES ('u1', 'u1@x', 'h', 'u1')",
            "INSERT INTO organizations (id, name, slug, created_by) VALUES ('o1', 'o1', 'o1', 'u1')",
            "INSERT INTO organization_members (organization_id, user_id, role_id) VALUES ('o1', 'u1', 4)",
            "INSERT INTO projects (id, organization_id, name, created_at, updated_at) VALUES ('p1', 'o1', 'p1', 0, 0)",
            "INSERT INTO environments (id, project_id, name, created_at, updated_at) VALUES ('e1', 'p1', 'e1', 0, 0)",
            "INSERT INTO rbac_roles (id, organization_id, name) VALUES ('r1', 'o1', 'deployer')",
            "INSERT INTO rbac_role_permissions (role_id, permission) VALUES ('r1', 'app.deploy')",
            "INSERT INTO rbac_assignments (id, organization_id, principal_type, principal_id, system_role_id, scope_type, scope_id) \
             VALUES ('a1', 'o1', 'user', 'u1', 2, 'project', 'p1')",
            "INSERT INTO rbac_assignments (id, organization_id, principal_type, principal_id, custom_role_id, scope_type, scope_id) \
             VALUES ('a2', 'o1', 'user', 'u1', 'r1', 'environment', 'e1')",
        ] {
            sqlx::query(sql)
                .execute(&pool)
                .await
                .unwrap_or_else(|e| panic!("{sql}: {e}"));
        }

        // Exactly one of system_role_id / custom_role_id.
        let both = sqlx::query(
            "INSERT INTO rbac_assignments (id, organization_id, principal_type, principal_id, system_role_id, custom_role_id, scope_type, scope_id) \
             VALUES ('bad', 'o1', 'user', 'u1', 2, 'r1', 'org', 'o1')",
        )
        .execute(&pool)
        .await;
        assert!(both.is_err(), "both role columns set must be rejected");

        // SysAdmin (0) and Member (4) are base roles, never assignable.
        let sysadmin = sqlx::query(
            "INSERT INTO rbac_assignments (id, organization_id, principal_type, principal_id, system_role_id, scope_type, scope_id) \
             VALUES ('bad2', 'o1', 'user', 'u1', 0, 'org', 'o1')",
        )
        .execute(&pool)
        .await;
        assert!(sysadmin.is_err());

        // Deleting the project cascades its environments; both assignments go.
        sqlx::query("DELETE FROM projects WHERE id = 'p1'")
            .execute(&pool)
            .await
            .unwrap();
        let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM rbac_assignments")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            left, 0,
            "assignments scoped to a deleted project/env must be removed"
        );

        // Leaving the org drops the user's assignments there.
        sqlx::query(
            "INSERT INTO rbac_assignments (id, organization_id, principal_type, principal_id, system_role_id, scope_type, scope_id) \
             VALUES ('a3', 'o1', 'user', 'u1', 3, 'org', 'o1')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("DELETE FROM organization_members WHERE user_id = 'u1'")
            .execute(&pool)
            .await
            .unwrap();
        let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM rbac_assignments")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            left, 0,
            "leaving the organization must drop its assignments"
        );

        drop(pool);
        cleanup_temp_db(&path).await;
    }

    /// Every `INSERT ... ON CONFLICT(cols)` literal in `src/` must prepare
    /// against the migrated schema. SQLite only rejects a conflict target that
    /// matches no PRIMARY KEY/UNIQUE constraint when it compiles the statement,
    /// so a migration that reshapes a key would otherwise surface as a runtime
    /// "ON CONFLICT clause does not match" mid-deploy.
    #[tokio::test]
    async fn on_conflict_targets_match_schema_constraints() {
        use syn::visit::Visit;

        struct Literals(Vec<String>);
        impl<'ast> Visit<'ast> for Literals {
            fn visit_lit_str(&mut self, node: &'ast syn::LitStr) {
                let sql = node.value();
                if sql.contains("ON CONFLICT")
                    && sql.trim_start().to_ascii_uppercase().starts_with("INSERT")
                {
                    self.0.push(sql);
                }
            }
        }

        fn collect(directory: &std::path::Path, out: &mut Vec<(String, String)>) {
            for entry in std::fs::read_dir(directory).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    collect(&path, out);
                } else if path.extension().is_some_and(|ext| ext == "rs") {
                    let mut literals = Literals(Vec::new());
                    literals.visit_file(
                        &syn::parse_file(&std::fs::read_to_string(&path).unwrap()).unwrap(),
                    );
                    out.extend(
                        literals
                            .0
                            .into_iter()
                            .map(|sql| (path.display().to_string(), sql)),
                    );
                }
            }
        }

        let mut statements = Vec::new();
        collect(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut statements,
        );
        assert!(!statements.is_empty());

        let (path, pool) = temp_db().await;
        run(&pool).await;
        let mut failures = Vec::new();
        for (file, sql) in &statements {
            if let Err(error) = sqlx::query(&format!("EXPLAIN {sql}"))
                .fetch_all(&pool)
                .await
            {
                failures.push(format!("{file}: {error}\n  {sql}"));
            }
        }
        drop(pool);
        cleanup_temp_db(&path).await;
        assert!(
            failures.is_empty(),
            "ON CONFLICT statements out of sync with schema:\n{}",
            failures.join("\n")
        );
    }
}
