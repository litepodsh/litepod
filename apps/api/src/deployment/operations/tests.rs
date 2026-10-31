use sqlx::Row;

use crate::{dtos::deployment::SourceConfigDto, state::state::SharedState};

use super::{artifacts::*, caddy::*, identity::*, lifecycle::*, queue_ops::*, resources::*};

#[test]
fn build_resource_limits_convert_cores_and_mib_to_podman_units() {
    let source = SourceConfigDto {
        deploy_type: "git".into(),
        build_method: "docker".into(),
        git_provider_id: None,
        git_url: None,
        git_branch: None,
        dockerfile_path: None,
        compose_file_path: None,
        compose_public_service: None,
        railpack_project_path: ".".into(),
        railpack_start_command: None,
        container_port: None,
        auto_deploy: false,
        watch_paths: None,
        clean_build: true,
        build_cpu_cores: Some(1.5),
        build_memory_mib: Some(2048),
        build_swap_mib: None,
        has_git_credentials: false,
        source_upload: false,
        source_archive_bytes: None,
        image_url: None,
        has_registry_credentials: false,
        restart_policy: "unless-stopped".into(),
    };

    assert_eq!(
        resource_limits(&source).unwrap(),
        Some(ResourceLimits {
            cpu_quota_us: 150_000,
            memory_bytes: 2 * 1024 * 1024 * 1024,
            memswap_bytes: 2 * 1024 * 1024 * 1024,
        })
    );
}

#[test]
fn resource_limits_add_extra_swap_to_memswap() {
    let mut source = SourceConfigDto {
        deploy_type: "git".into(),
        build_method: "railpack".into(),
        git_provider_id: None,
        git_url: None,
        git_branch: None,
        dockerfile_path: None,
        compose_file_path: None,
        compose_public_service: None,
        railpack_project_path: ".".into(),
        railpack_start_command: None,
        container_port: None,
        auto_deploy: false,
        watch_paths: None,
        clean_build: true,
        build_cpu_cores: Some(1.0),
        build_memory_mib: Some(2048),
        build_swap_mib: Some(1024),
        has_git_credentials: false,
        source_upload: false,
        source_archive_bytes: None,
        image_url: None,
        has_registry_credentials: false,
        restart_policy: "unless-stopped".into(),
    };
    let mib = 1024 * 1024;
    let limits = resource_limits(&source).unwrap().unwrap();
    assert_eq!(limits.memory_bytes, 2048 * mib);
    assert_eq!(limits.memswap_bytes, 3072 * mib);

    // swap = 0 and swap = None both behave as no extra swap.
    source.build_swap_mib = Some(0);
    assert_eq!(
        resource_limits(&source).unwrap().unwrap().memswap_bytes,
        2048 * mib
    );
    source.build_swap_mib = None;
    assert_eq!(
        resource_limits(&source).unwrap().unwrap().memswap_bytes,
        2048 * mib
    );
}

#[test]
fn resource_log_line_describes_limited_and_unlimited_deploys() {
    let mut source = SourceConfigDto {
        deploy_type: "git".into(),
        build_method: "docker".into(),
        git_provider_id: None,
        git_url: None,
        git_branch: None,
        dockerfile_path: None,
        compose_file_path: None,
        compose_public_service: None,
        railpack_project_path: ".".into(),
        railpack_start_command: None,
        container_port: None,
        auto_deploy: false,
        watch_paths: None,
        clean_build: true,
        build_cpu_cores: None,
        build_memory_mib: None,
        build_swap_mib: None,
        has_git_credentials: false,
        source_upload: false,
        source_archive_bytes: None,
        image_url: None,
        has_registry_credentials: false,
        restart_policy: "unless-stopped".into(),
    };
    assert_eq!(
        resource_log_line(&source),
        "Resources: no CPU/RAM limit configured"
    );
    source.build_cpu_cores = Some(1.5);
    source.build_memory_mib = Some(2048);
    assert_eq!(
        resource_log_line(&source),
        "Resources: limited to 1.5 CPU cores and 2048 MiB RAM"
    );
    source.build_swap_mib = Some(512);
    assert_eq!(
        resource_log_line(&source),
        "Resources: limited to 1.5 CPU cores, 2048 MiB RAM + 512 MiB swap"
    );
}

#[test]
fn deployment_identity_uses_six_characters_from_the_final_uuid_section() {
    let (name, hostname) = deployment_identity(
        "Proyecto",
        "prod",
        "Mi App",
        "019f80a5-6b9d-7453-bc84-830a230fe763",
        "127.0.0.1",
    );

    assert_eq!(name, "mi-app-830a23");
    assert_eq!(
        hostname,
        "019f80a5-6b9d-7453-bc84-830a230fe763.127.0.0.1.sslip.io"
    );
}

#[test]
fn deployment_identity_is_stable_when_project_or_environment_is_renamed() {
    let app_id = "019f80a5-6b9d-7453-bc84-830a230fe763";
    let (before, _) = deployment_identity("Tools", "prod", "Forgejo", app_id, "127.0.0.1");
    let (after, _) =
        deployment_identity("Renamed Tools", "staging", "Forgejo", app_id, "127.0.0.1");

    assert_eq!(before, after);
    assert_eq!(before, "forgejo-830a23");
}

#[test]
fn managed_container_builder_uses_unless_stopped_restart_policy_by_default() {
    let options =
        managed_container_builder("litepod-test", "caddy:latest".into(), "unless-stopped").build();
    let payload = serde_json::to_value(options).unwrap();

    assert_eq!(payload["params"]["restart_policy"], "unless-stopped");
}

#[test]
fn managed_container_builder_maps_restart_policies() {
    assert_eq!(payload_restart_policy("no"), "no");
    assert_eq!(payload_restart_policy("always"), "always");
    assert_eq!(payload_restart_policy("on-failure"), "on-failure");
    assert_eq!(payload_restart_policy("unless-stopped"), "unless-stopped");
}

fn payload_restart_policy(policy: &str) -> serde_json::Value {
    let options = managed_container_builder("litepod-test", "caddy:latest".into(), policy).build();
    serde_json::to_value(options).unwrap()["params"]["restart_policy"].clone()
}

#[test]
fn named_volume_uses_its_podman_name_instead_of_its_storage_path() {
    let mount = serde_json::json!({
        "Type": "volume",
        "Name": "compose_redis-data",
        "Source": "/var/lib/containers/storage/volumes/compose_redis-data/_data"
    });

    assert_eq!(mount_source(&mount), "compose_redis-data");
}

#[test]
fn bind_mount_keeps_its_host_source_path() {
    let mount = serde_json::json!({
        "Type": "bind",
        "Source": "/Users/example/html"
    });

    assert_eq!(mount_source(&mount), "/Users/example/html");
}

#[test]
fn application_domain_route_precedes_litepod_catch_all() {
    let mut config = serde_json::json!({
        "apps": {
            "http": {
                "servers": {
                    "srv0": {
                        "listen": [":80"],
                        "routes": [{
                            "@id": "litepod-default",
                            "handle": [{
                                "handler": "reverse_proxy",
                                "upstreams": [{ "dial": "app:6001" }]
                            }]
                        }]
                    }
                }
            }
        }
    });

    apply_app_domain_to_config(
        &mut config,
        "litepod-route-domain-1",
        "litepod-123.216.250.119.183.sslip.io",
        "static-app",
        80,
        false,
        None,
        None,
    )
    .unwrap();

    let routes = config["apps"]["http"]["servers"]["srv0"]["routes"]
        .as_array()
        .unwrap();
    assert_eq!(routes[0]["@id"], "litepod-route-domain-1");
    assert_eq!(
        routes[0]["match"][0]["host"][0],
        "litepod-123.216.250.119.183.sslip.io"
    );
    assert_eq!(routes[1]["@id"], "litepod-default");
}

#[test]
fn maintenance_replaces_the_proxy_and_the_auth_challenge_but_keeps_tls() {
    let mut config = serde_json::json!({
        "apps": {
            "http": {
                "servers": {
                    "srv0": {
                        "listen": [":80"],
                        "routes": [{ "@id": "litepod-default" }],
                        "automatic_https": { "disable": true }
                    }
                }
            }
        }
    });
    let auth = crate::deployment::basic_auth::BasicAuthPayload {
        realm: "Restricted".into(),
        accounts: vec![("admin".into(), "$2a$12$hash".into())],
    };
    let page = crate::deployment::maintenance::MaintenancePayload {
        html: "<p>back soon</p>".into(),
        site: None,
        companion: None,
    };

    apply_app_domain_to_config(
        &mut config,
        "litepod-route-domain-1",
        "test.litepod.sh",
        "static-app",
        80,
        true,
        Some(&auth),
        Some(&page),
    )
    .unwrap();

    let server = &config["apps"]["http"]["servers"]["srv0"];
    let handles = server["routes"][0]["handle"].as_array().unwrap();
    assert_eq!(handles.len(), 1, "only the page: no auth challenge, no proxy");
    assert_eq!(handles[0]["handler"], "static_response");
    assert_eq!(handles[0]["status_code"], 503);
    // The route still terminates TLS for the host, so the page is served over HTTPS.
    assert!(server["listen"].as_array().unwrap().iter().any(|v| v == ":443"));
    assert!(
        server["tls_connection_policies"]
            .as_array()
            .unwrap()
            .iter()
            .any(|policy| policy["match"]["sni"][0] == "test.litepod.sh")
    );

    // Turning it off rebuilds the same route with the proxy (and auth) back.
    apply_app_domain_to_config(
        &mut config,
        "litepod-route-domain-1",
        "test.litepod.sh",
        "static-app",
        80,
        true,
        Some(&auth),
        None,
    )
    .unwrap();
    let handles = config["apps"]["http"]["servers"]["srv0"]["routes"][0]["handle"]
        .as_array()
        .unwrap();
    assert_eq!(handles[0]["handler"], "authentication");
    assert_eq!(handles[1]["handler"], "reverse_proxy");
}

#[test]
fn a_live_companion_is_proxied_with_a_forced_503_and_a_per_host_fallback() {
    let mut config = serde_json::json!({
        "apps": { "http": { "servers": { "srv0": {
            "listen": [":80"],
            "routes": [{ "@id": "litepod-default" }],
            "automatic_https": { "disable": true }
        }}}}
    });
    let page = crate::deployment::maintenance::MaintenancePayload {
        html: "<p>back soon</p>".into(),
        site: None,
        companion: Some("maintenance-page:80".into()),
    };
    apply_app_domain_to_config(
        &mut config, "litepod-route-domain-1", "app.test", "app", 80, false, None, Some(&page),
    )
    .unwrap();

    let server = &config["apps"]["http"]["servers"]["srv0"];
    let handles = server["routes"][0]["handle"].as_array().unwrap();
    // Headers must come BEFORE the proxy: inside `handle_response` Caddy drops them.
    assert_eq!(handles[0]["handler"], "headers");
    assert_eq!(handles[1]["handler"], "reverse_proxy");
    assert_eq!(handles[1]["upstreams"][0]["dial"], "maintenance-page:80");
    let forced = &handles[1]["handle_response"][0];
    assert_eq!(forced["status_code"], "503");
    assert_eq!(forced["match"]["headers"]["Content-Type"][0], "text/html*");
    assert_eq!(forced["routes"][0]["handle"][0]["handler"], "copy_response");

    // If the container dies behind the route, this host (only) answers the inline page.
    let errors = server["errors"]["routes"].as_array().unwrap();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0]["@id"], "litepod-route-domain-1-maint-errors");
    assert_eq!(errors[0]["match"][0]["host"][0], "app.test");
    assert_eq!(errors[0]["handle"][0]["status_code"], 503);

    // Rebuilding the same route is idempotent: still one fallback.
    apply_app_domain_to_config(
        &mut config, "litepod-route-domain-1", "app.test", "app", 80, false, None, Some(&page),
    )
    .unwrap();
    assert_eq!(config["apps"]["http"]["servers"]["srv0"]["errors"]["routes"].as_array().unwrap().len(), 1);

    // Back to the application: the proxy returns and the fallback is gone.
    apply_app_domain_to_config(
        &mut config, "litepod-route-domain-1", "app.test", "app", 80, false, None, None,
    )
    .unwrap();
    let server = &config["apps"]["http"]["servers"]["srv0"];
    assert_eq!(server["routes"][0]["handle"][0]["upstreams"][0]["dial"], "app:80");
    assert!(server["errors"]["routes"].as_array().unwrap().is_empty());
}

#[test]
fn the_fallback_goes_with_its_route_on_delete_and_on_prune() {
    let page = crate::deployment::maintenance::MaintenancePayload {
        html: "x".into(),
        site: None,
        companion: Some("maintenance-page:80".into()),
    };
    let build = || {
        let mut config = serde_json::json!({
            "apps": { "http": { "servers": { "srv0": {
                "listen": [":80"], "routes": [{ "@id": "litepod-default" }],
                "automatic_https": { "disable": true }
            }}}}
        });
        apply_app_domain_to_config(
            &mut config, "litepod-route-domain-1", "app.test", "app", 80, false, None, Some(&page),
        )
        .unwrap();
        config
    };
    let count = |config: &serde_json::Value| {
        config["apps"]["http"]["servers"]["srv0"]["errors"]["routes"].as_array().unwrap().len()
    };

    let mut deleted = build();
    apply_app_domain_removal_to_config(&mut deleted, "litepod-route-domain-1", "app.test").unwrap();
    assert_eq!(count(&deleted), 0, "deleting the domain leaves no fallback behind");

    let mut pruned = build();
    prune_orphans_in_config(&mut pruned, &Default::default(), &Default::default());
    assert_eq!(count(&pruned), 0, "an orphaned fallback is pruned");

    let mut kept = build();
    let keep: std::collections::HashSet<String> = ["litepod-route-domain-1".to_string()].into();
    prune_orphans_in_config(&mut kept, &keep, &Default::default());
    assert_eq!(count(&kept), 1, "a fallback whose route is kept stays");
}

#[test]
fn https_application_domain_enables_tls_before_the_catch_all() {
    let mut config = serde_json::json!({
        "apps": {
            "http": {
                "servers": {
                    "srv0": {
                        "listen": [":80"],
                        "routes": [{ "@id": "litepod-default" }],
                        "automatic_https": { "disable": true }
                    }
                }
            }
        }
    });

    apply_app_domain_to_config(
        &mut config,
        "litepod-route-domain-1",
        "test.litepod.sh",
        "static-app",
        80,
        true,
        None,
        None,
    )
    .unwrap();

    let server = &config["apps"]["http"]["servers"]["srv0"];
    assert_eq!(server["routes"][0]["@id"], "litepod-route-domain-1");
    assert!(
        server["listen"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == ":443")
    );
    assert_eq!(
        server["tls_connection_policies"][0]["match"]["sni"][0],
        "test.litepod.sh"
    );
    assert!(server["automatic_https"].get("disable").is_none());
    assert_eq!(
        config["apps"]["tls"]["automation"]["policies"][0]["subjects"][0],
        "test.litepod.sh"
    );
}

mod enqueue_dispatch {
    use super::*;
    use crate::{migrations, state::state::AppState};
    use std::collections::HashSet;

    use uuid::Uuid;

    async fn empty_state() -> (SharedState, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!("litepod-enqueue-{}.db", Uuid::now_v7()));
        let db = crate::testing::sqlite_pool_at(&path).await;
        migrations::run(&db).await;
        let state: SharedState = std::sync::Arc::new(AppState {
            started_at: chrono::Local::now(),
            db,
            caddy_url: String::new(),
            caddy_admin_url: String::new(),
            caddyfile_path: std::path::PathBuf::new(),
            caddy_server_upstream: String::new(),
            podman_socket: String::new(),
            pending_execs: std::sync::Mutex::new(std::collections::HashMap::new()),
            data_dir: std::path::PathBuf::new(),
            data_volume: None,
            host_data_dir: std::path::PathBuf::new(),
            podman_network: String::new(),
            public_url: String::new(),
            oauth_states: std::sync::Mutex::new(std::collections::HashMap::new()),
            public_ip: "203.0.113.7".into(),
            is_cloud: false,
            polar: None,
            dev_mode: false,
            port_range: (10_000, 20_000),
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
        (state, path)
    }

    async fn seed_hierarchy(state: &SharedState) {
        let conn = state.db.clone();
        sqlx::query(
            "INSERT INTO users (id, email, hashed_password, name) VALUES ('user-1', 'u@e.test', 'h', 'U')",
        )
        .execute(&conn)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO organizations (id, name, slug, created_by) VALUES ('organization-1', 'T', 't', 'user-1')",
        )
        .execute(&conn)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO projects (id, organization_id, name, created_at, updated_at) VALUES ('project-1', 'organization-1', 'P', 1, 1)",
        )
        .execute(&conn)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO environments (id, project_id, name, is_production, created_at, updated_at) VALUES ('env-1', 'project-1', 'prod', 1, 1, 1)",
        )
        .execute(&conn)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO applications (id, environment_id, name, server_id, created_at, updated_at) VALUES ('app-1', 'env-1', 'A', 'local', 1, 1)",
        )
        .execute(&conn)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn static_enqueue_rejects_when_no_artifact_uploaded() {
        let (state, _path) = empty_state().await;
        seed_hierarchy(&state).await;
        let conn = state.db.clone();
        sqlx::query(
            "UPDATE applications SET deploy_type = 'static', build_method = 'docker', container_port = 80, railpack_project_path = '.', updated_at = 1 WHERE id = 'app-1'",
        )
        .execute(&conn)
        .await
        .unwrap();
        drop(conn);
        let result = enqueue_for_application(&state, "app-1", "deploy_hook", None).await;
        assert!(result.is_err(), "missing artifact must be rejected");
    }

    #[tokio::test]
    async fn enqueue_for_application_dispatches_to_git() {
        let (state, _path) = empty_state().await;
        seed_hierarchy(&state).await;
        let conn = state.db.clone();
        sqlx::query(
            "INSERT INTO git_providers (id, organization_id, name, provider_type, host_url, username, password_encrypted, created_at, updated_at) VALUES ('provider-1', 'organization-1', 'GH', 'github', 'https://github.com', 'octo', 'encrypted', 1, 1)",
        )
        .execute(&conn)
        .await
        .unwrap();
        sqlx::query(
            "UPDATE applications SET deploy_type = 'git', build_method = 'docker', git_provider_id = 'provider-1', git_url = 'https://github.com/octo/repo.git', git_branch = 'main', git_repo_full_name = 'octo/repo', container_port = 3000, dockerfile_path = './Dockerfile', railpack_project_path = '.', updated_at = 1 WHERE id = 'app-1'",
        )
        .execute(&conn)
        .await
        .unwrap();
        drop(conn);
        let job = enqueue_for_application(&state, "app-1", "webhook", None)
            .await
            .unwrap();
        assert_eq!(job.source_type, "git");
        assert_eq!(job.build_method, "docker");
        assert_eq!(job.status, "queued");
    }

    #[tokio::test]
    async fn static_rollback_restores_the_old_artifact_and_refuses_an_unfinished_run() {
        let (state, _path) = empty_state().await;
        seed_hierarchy(&state).await;
        let conn = state.db.clone();
        sqlx::query(
            "UPDATE applications SET deploy_type = 'static', build_method = 'docker', container_port = 80, railpack_project_path = '.', updated_at = 1 WHERE id = 'app-1'",
        )
        .execute(&conn)
        .await
        .unwrap();

        save_artifact(&state, "app-1", "a.html".into(), b"<html>A</html>".to_vec())
            .await
            .unwrap();
        let run_a = enqueue_for_application(&state, "app-1", "manual", None)
            .await
            .unwrap();
        sqlx::query("UPDATE application_deployment_runs SET status = 'succeeded' WHERE id = ?1")
            .bind(&run_a.run_id)
            .execute(&conn)
            .await
            .unwrap();
        let artifact_a: String = sqlx::query(
            "SELECT artifact_id FROM application_deployments WHERE application_id = 'app-1'",
        )
        .fetch_one(&conn)
        .await
        .unwrap()
        .try_get(0)
        .unwrap();

        save_artifact(&state, "app-1", "b.html".into(), b"<html>B</html>".to_vec())
            .await
            .unwrap();
        let run_b = enqueue_for_application(&state, "app-1", "manual", None)
            .await
            .unwrap();
        // run_b is deliberately left 'building' - rolling back to an
        // unfinished run must be refused, there is nothing proven-good
        // to redeploy yet.
        let artifact_b: String = sqlx::query(
            "SELECT artifact_id FROM application_deployments WHERE application_id = 'app-1'",
        )
        .fetch_one(&conn)
        .await
        .unwrap()
        .try_get(0)
        .unwrap();
        assert_ne!(artifact_a, artifact_b, "each upload gets its own artifact");

        let refused = enqueue_rollback_for_application(&state, "app-1", &run_b.run_id, None).await;
        assert!(
            refused.is_err(),
            "rolling back to a still-building run must be refused"
        );

        enqueue_rollback_for_application(&state, "app-1", &run_a.run_id, None)
            .await
            .unwrap();
        let restored: String = sqlx::query(
            "SELECT artifact_id FROM application_deployments WHERE application_id = 'app-1'",
        )
        .fetch_one(&conn)
        .await
        .unwrap()
        .try_get(0)
        .unwrap();
        assert_eq!(
            restored, artifact_a,
            "rollback must point the app back at the old run's own artifact"
        );
    }
}
