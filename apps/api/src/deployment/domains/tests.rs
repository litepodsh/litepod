use crate::{
    dtos::{deployment::*, response::AppError},
    servers::model::IngressMode,
    state::state::SharedState,
};

use super::{caddy_sync::*, crud::*, validate::*};
use crate::{migrations, state::state::AppState};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

use uuid::Uuid;

async fn test_state(dev_mode: bool) -> (SharedState, std::path::PathBuf) {
    test_state_with_reserved(dev_mode, HashSet::new()).await
}

async fn test_state_with_reserved(
    dev_mode: bool,
    reserved_ports: HashSet<i64>,
) -> (SharedState, std::path::PathBuf) {
    let path = std::env::temp_dir().join(format!("litepod-domains-crud-{}.db", Uuid::now_v7()));
    let db = crate::testing::sqlite_pool_at(&path).await;
    migrations::run(&db).await;
    let state: SharedState = Arc::new(AppState {
        started_at: chrono::Local::now(),
        db,
        caddy_url: String::new(),
        caddy_admin_url: String::new(),
        caddyfile_path: std::path::PathBuf::new(),
        caddy_server_upstream: String::new(),
        podman_socket: String::new(),
        pending_execs: Mutex::new(HashMap::new()),
        data_dir: std::path::PathBuf::new(),
        data_volume: None,
        host_data_dir: std::path::PathBuf::new(),
        podman_network: String::new(),
        public_url: String::new(),
        oauth_states: Mutex::new(HashMap::new()),
        public_ip: "203.0.113.7".into(),
        is_cloud: false,
        polar: None,
        dev_mode,
        port_range: (10_000, 20_000),
        reserved_ports,
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
    let conn = &state.db;
    sqlx::query("INSERT INTO users (id, email, hashed_password, name) VALUES (?1, ?2, ?3, ?4)")
        .bind("user-1")
        .bind("user@example.com")
        .bind("hash")
        .bind("User")
        .execute(conn)
        .await
        .unwrap();
    sqlx::query("INSERT INTO organizations (id, name, slug, created_by) VALUES (?1, ?2, ?3, ?4)")
        .bind("organization-1")
        .bind("Organization")
        .bind("organization")
        .bind("user-1")
        .execute(conn)
        .await
        .unwrap();
    sqlx::query("INSERT INTO projects (id, organization_id, name, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)")
        .bind("project-1")
        .bind("organization-1")
        .bind("Project")
        .bind(1_i64)
        .bind(1_i64)
        .execute(conn)
        .await
        .unwrap();
    sqlx::query("INSERT INTO environments (id, project_id, name, is_production, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)")
        .bind("environment-1")
        .bind("project-1")
        .bind("prod")
        .bind(1_i64)
        .bind(1_i64)
        .bind(1_i64)
        .execute(conn)
        .await
        .unwrap();
    sqlx::query("INSERT INTO applications (id, environment_id, name, server_id, created_at, updated_at) VALUES (?1, ?2, ?3, 'local', ?4, ?5)")
        .bind("application-1")
        .bind("environment-1")
        .bind("Application")
        .bind(1_i64)
        .bind(1_i64)
        .execute(conn)
        .await
        .unwrap();
    sqlx::query("INSERT INTO application_deployments (application_id, status, container_name, hostname, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)")
        .bind("application-1")
        .bind("not_deployed")
        .bind("litepod-project-prod-application-appli")
        .bind("")
        .bind(1_i64)
        .execute(conn)
        .await
        .unwrap();
    (state, path)
}

fn cleanup(path: std::path::PathBuf) {
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{}", path.display(), suffix));
    }
}

#[test]
fn private_only_rejects_public_domains_but_keeps_local_hosts_available() {
    assert!(validate_domain_for_ingress(IngressMode::PrivateOnly, "gitea.example.com").is_err());
    assert!(validate_domain_for_ingress(IngressMode::PrivateOnly, "localhost").is_ok());
    assert!(validate_domain_for_ingress(IngressMode::PublicEdge, "gitea.example.com").is_ok());
}

fn dns_candidate() -> DomainDto {
    DomainDto {
        certificate_id: None,
        certificate_name: None,
        certificate_provider: "automatic".into(),
        has_custom_certificate: false,
        id: "domain-1".into(),
        application_id: "application-1".into(),
        host: "app.example.com".into(),
        port: 80,
        https: true,
        is_primary: true,
        url: "https://app.example.com".into(),
        is_local: false,
        required_dns_target: Some("203.0.113.10".into()),
        ingress_mode: "public_edge".into(),
        route_health: "ready".into(),
        route_health_reason: None,
        server_id: None,
        created_at: 1,
        updated_at: 1,
    }
}

#[test]
fn dns_check_accepts_only_public_hostnames() {
    for ok in ["app.example.com", "a-b.example.io", "x.co"] {
        assert!(check_public_hostname(ok).is_ok(), "{ok}");
    }
    for bad in [
        "203.0.113.10",
        "::1",
        "localhost",
        "intranet",
        "db.internal",
        "nas.local",
        "printer.lan",
        "10.in-addr.arpa",
        "-bad.example.com",
        "a..example.com",
        "app.example.123",
        "app_1.example.com",
    ] {
        assert!(check_public_hostname(bad).is_err(), "{bad}");
    }
}

#[test]
fn dns_validation_requires_an_eligible_public_edge_domain() {
    assert_eq!(validation_target(&dns_candidate()).unwrap(), "203.0.113.10");

    let mut local = dns_candidate();
    local.is_local = true;
    assert!(validation_target(&local).is_err());

    let mut sslip = dns_candidate();
    sslip.host = "app.203-0-113-10.sslip.io".into();
    assert!(validation_target(&sslip).is_err());

    let mut missing_target = dns_candidate();
    missing_target.required_dns_target = None;
    assert!(
        validation_target(&missing_target)
            .unwrap_err()
            .to_string()
            .contains("public address")
    );
}

#[tokio::test]
async fn first_domain_becomes_primary_and_later_ones_do_not() {
    let (state, path) = test_state(false).await;
    let first = create(
        &state,
        "application-1",
        CreateDomainRequest {
            server_id: None,
            certificate: Default::default(),
            host: "a.example.com".into(),
            port: 80,
            https: None,
            is_primary: None,
        },
    )
    .await
    .unwrap();
    assert!(first.is_primary, "first domain is primary");
    let second = create(
        &state,
        "application-1",
        CreateDomainRequest {
            server_id: None,
            certificate: Default::default(),
            host: "b.example.com".into(),
            port: 80,
            https: None,
            is_primary: None,
        },
    )
    .await
    .unwrap();
    assert!(
        !second.is_primary,
        "later domains are not primary by default"
    );
    assert_eq!(list(&state, "application-1").await.unwrap().len(), 2);
    cleanup(path);
}

#[tokio::test]
async fn group_targeted_application_requires_a_domain_pin() {
    let (state, path) = test_state(false).await;
    let conn = &state.db;
    sqlx::query(
        "INSERT INTO servers (id, kind, name, organization_id, ingress_mode, state, ssh_host, ssh_port, ssh_user, created_at, updated_at) VALUES ('server-2','ssh','Remote','organization-1','public_edge','ready','r.example.test',22,'litepod',1,1)",
    )
    .execute(conn)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO server_groups (id, organization_id, name, created_at, updated_at) VALUES ('group-1','organization-1','dev',1,1)",
    )
    .execute(conn)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO server_group_members (group_id, server_id, created_at) VALUES ('group-1','server-2',1)",
    )
    .execute(conn)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO applications (id, environment_id, name, server_group_id, created_at, updated_at) VALUES ('application-group','environment-1','GroupApp','group-1',1,1)",
    )
    .execute(conn)
    .await
    .unwrap();

    // No pin for a group app is rejected before any write.
    let missing = create(
        &state,
        "application-group",
        CreateDomainRequest {
            server_id: None,
            certificate: Default::default(),
            host: "a.example.com".into(),
            port: 80,
            https: None,
            is_primary: None,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(missing, AppError::ValidationError(_)));

    // A pin to a server outside the group is rejected too.
    let foreign = create(
        &state,
        "application-group",
        CreateDomainRequest {
            server_id: Some("server-99".into()),
            certificate: Default::default(),
            host: "b.example.com".into(),
            port: 80,
            https: None,
            is_primary: None,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(foreign, AppError::ValidationError(_)));

    cleanup(path);
}

#[tokio::test]
async fn a_group_app_does_not_auto_generate_a_domain() {
    let (state, path) = test_state(true).await;
    let conn = &state.db;
    sqlx::query(
        "INSERT INTO servers (id, kind, name, organization_id, ingress_mode, state, ssh_host, ssh_port, ssh_user, created_at, updated_at) VALUES ('server-2','ssh','Remote','organization-1','public_edge','ready','r.example.test',22,'litepod',1,1)",
    )
    .execute(conn)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO server_groups (id, organization_id, name, created_at, updated_at) VALUES ('group-1','organization-1','dev',1,1)",
    )
    .execute(conn)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO server_group_members (group_id, server_id, created_at) VALUES ('group-1','server-2',1)",
    )
    .execute(conn)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO applications (id, environment_id, name, server_group_id, created_at, updated_at) VALUES ('application-group','environment-1','GroupApp','group-1',1,1)",
    )
    .execute(conn)
    .await
    .unwrap();

    // Even inside a group replica's fan-out scope, a group app gets no default
    // domain: each server needs an explicit pin instead.
    crate::servers::replica::for_replica("server-2", seed_if_missing(&state, "application-group"))
        .await
        .expect("seeding a group replica must be a no-op");

    let domains = list(&state, "application-group").await.unwrap();
    assert!(
        domains.is_empty(),
        "a group app must not auto-generate a domain"
    );

    cleanup(path);
}

#[tokio::test]
async fn concurrent_generation_never_reuses_a_local_port() {
    let (state, path) = test_state(true).await;
    // Without reserving the port under the write lock, both calls see the same
    // free port and the loser trips the `(host, port)` unique index.
    let (first, second) = tokio::join!(
        generate_with(&state, "application-1", true),
        generate_with(&state, "application-1", false),
    );
    let first = first.unwrap();
    let second = second.unwrap();
    assert_ne!(
        first.port, second.port,
        "two concurrent generations must not claim the same local port"
    );
    cleanup(path);
}

#[tokio::test]
async fn promoting_a_domain_demotes_the_previous_primary() {
    let (state, path) = test_state(false).await;
    let first = create(
        &state,
        "application-1",
        CreateDomainRequest {
            server_id: None,
            certificate: Default::default(),
            host: "a.example.com".into(),
            port: 80,
            https: None,
            is_primary: None,
        },
    )
    .await
    .unwrap();
    let second = create(
        &state,
        "application-1",
        CreateDomainRequest {
            server_id: None,
            certificate: Default::default(),
            host: "b.example.com".into(),
            port: 80,
            https: None,
            is_primary: None,
        },
    )
    .await
    .unwrap();

    let promoted = update(
        &state,
        "application-1",
        &second.id,
        UpdateDomainRequest {
            server_id: None,
            certificate: Default::default(),
            host: None,
            port: None,
            https: None,
            is_primary: Some(true),
        },
    )
    .await
    .unwrap();
    assert!(promoted.is_primary);

    let domains = list(&state, "application-1").await.unwrap();
    let previous = domains.iter().find(|d| d.id == first.id).unwrap();
    assert!(
        !previous.is_primary,
        "previous primary is demoted in the same transaction"
    );
    cleanup(path);
}

#[tokio::test]
async fn deleting_the_primary_promotes_the_oldest_remaining() {
    let (state, path) = test_state(false).await;
    let first = create(
        &state,
        "application-1",
        CreateDomainRequest {
            server_id: None,
            certificate: Default::default(),
            host: "a.example.com".into(),
            port: 80,
            https: None,
            is_primary: None,
        },
    )
    .await
    .unwrap();
    let second = create(
        &state,
        "application-1",
        CreateDomainRequest {
            server_id: None,
            certificate: Default::default(),
            host: "b.example.com".into(),
            port: 80,
            https: None,
            is_primary: None,
        },
    )
    .await
    .unwrap();

    delete(&state, "application-1", &first.id).await.unwrap();

    let remaining = list(&state, "application-1").await.unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, second.id);
    assert!(
        remaining[0].is_primary,
        "oldest remaining domain is promoted"
    );

    delete(&state, "application-1", &second.id).await.unwrap();
    assert!(
        list(&state, "application-1").await.unwrap().is_empty(),
        "no domains leaves no primary"
    );
    cleanup(path);
}

#[tokio::test]
async fn duplicate_host_and_port_is_a_conflict() {
    let (state, path) = test_state(false).await;
    create(
        &state,
        "application-1",
        CreateDomainRequest {
            server_id: None,
            certificate: Default::default(),
            host: "a.example.com".into(),
            port: 80,
            https: None,
            is_primary: None,
        },
    )
    .await
    .unwrap();
    let duplicate = create(
        &state,
        "application-1",
        CreateDomainRequest {
            server_id: None,
            certificate: Default::default(),
            host: "a.example.com".into(),
            port: 80,
            https: None,
            is_primary: None,
        },
    )
    .await;
    assert!(
        matches!(duplicate, Err(AppError::AlreadyExists)),
        "expected AlreadyExists, got {duplicate:?}"
    );
    cleanup(path);
}

#[tokio::test]
async fn edge_route_targets_lists_non_local_domains_for_one_server() {
    let (state, path) = test_state(false).await;
    create(
        &state,
        "application-1",
        CreateDomainRequest {
            server_id: None,
            certificate: Default::default(),
            host: "svc.example.com".into(),
            port: 80,
            https: None,
            is_primary: None,
        },
    )
    .await
    .unwrap();
    create(
        &state,
        "application-1",
        CreateDomainRequest {
            server_id: None,
            certificate: Default::default(),
            host: "localhost".into(),
            port: 12345,
            https: None,
            is_primary: None,
        },
    )
    .await
    .unwrap();

    let targets = edge_route_targets(&state, "local").await.unwrap();
    let hosts: Vec<&str> = targets.iter().map(|t| t.host.as_str()).collect();
    assert_eq!(hosts, vec!["svc.example.com"], "local hosts are excluded");
    assert!(!targets[0].routable, "no runtime endpoint yet");
    cleanup(path);
}

#[tokio::test]
async fn rejects_a_second_public_domain_reusing_a_host_on_the_same_server() {
    let (state, path) = test_state(false).await;
    create(
        &state,
        "application-1",
        CreateDomainRequest {
            server_id: None,
            certificate: Default::default(),
            host: "kong.example.com".into(),
            port: 8001,
            https: None,
            is_primary: None,
        },
    )
    .await
    .unwrap();

    // Same host, different port: the (host, port) unique index would allow it.
    let dup = create(
        &state,
        "application-1",
        CreateDomainRequest {
            server_id: None,
            certificate: Default::default(),
            host: "kong.example.com".into(),
            port: 8002,
            https: None,
            is_primary: None,
        },
    )
    .await;
    assert!(
        matches!(dup, Err(AppError::ValidationError(ref m)) if m.contains("its own hostname")),
        "expected a hostname-conflict ValidationError, got {dup:?}"
    );

    // A distinct host is fine.
    create(
        &state,
        "application-1",
        CreateDomainRequest {
            server_id: None,
            certificate: Default::default(),
            host: "kong-manager.example.com".into(),
            port: 8002,
            https: None,
            is_primary: None,
        },
    )
    .await
    .unwrap();

    // A local host is never subject to the guard.
    create(
        &state,
        "application-1",
        CreateDomainRequest {
            server_id: None,
            certificate: Default::default(),
            host: "localhost".into(),
            port: 15000,
            https: None,
            is_primary: None,
        },
    )
    .await
    .unwrap();

    cleanup(path);
}

#[tokio::test]
async fn update_may_keep_its_own_host_but_not_take_a_siblings() {
    let (state, path) = test_state(false).await;
    let a = create(
        &state,
        "application-1",
        CreateDomainRequest {
            server_id: None,
            certificate: Default::default(),
            host: "a.example.com".into(),
            port: 80,
            https: None,
            is_primary: None,
        },
    )
    .await
    .unwrap();
    let b = create(
        &state,
        "application-1",
        CreateDomainRequest {
            server_id: None,
            certificate: Default::default(),
            host: "b.example.com".into(),
            port: 80,
            https: None,
            is_primary: None,
        },
    )
    .await
    .unwrap();

    // No-op host update on `a` is allowed (its own row is excluded).
    update(
        &state,
        "application-1",
        &a.id,
        UpdateDomainRequest {
            server_id: None,
            certificate: Default::default(),
            host: Some("a.example.com".into()),
            port: None,
            https: None,
            is_primary: None,
        },
    )
    .await
    .unwrap();

    // Moving `b` onto `a`'s exact host+port is the unique index's job (409).
    let exact = update(
        &state,
        "application-1",
        &b.id,
        UpdateDomainRequest {
            server_id: None,
            certificate: Default::default(),
            host: Some("a.example.com".into()),
            port: None,
            https: None,
            is_primary: None,
        },
    )
    .await;
    assert!(
        matches!(exact, Err(AppError::AlreadyExists)),
        "got {exact:?}"
    );

    // Moving `b` onto `a`'s host on a different port is the guard's job (422).
    let clash = update(
        &state,
        "application-1",
        &b.id,
        UpdateDomainRequest {
            server_id: None,
            certificate: Default::default(),
            host: Some("a.example.com".into()),
            port: Some(8080),
            https: None,
            is_primary: None,
        },
    )
    .await;
    assert!(
        matches!(clash, Err(AppError::ValidationError(_))),
        "got {clash:?}"
    );

    cleanup(path);
}

#[tokio::test]
async fn generate_uses_sslip_io_in_production_and_localhost_in_dev() {
    let (production, production_path) = test_state(false).await;
    let generated = generate(&production, "application-1").await.unwrap();
    // The label is random rather than the application ID, so assert its
    // shape: `litepod-` + 12 hex characters, on the public IP's sslip.io.
    let label = generated
        .host
        .strip_suffix(".203.0.113.7.sslip.io")
        .unwrap_or_else(|| panic!("unexpected host {}", generated.host));
    let random = label
        .strip_prefix("litepod-")
        .unwrap_or_else(|| panic!("unexpected label {label}"));
    assert_eq!(random.len(), 12, "label was {label}");
    assert!(
        random.chars().all(|c| c.is_ascii_hexdigit()),
        "label was {label}"
    );
    assert!(
        !generated.host.contains("application-1"),
        "the application ID must not leak into a public hostname"
    );
    assert_eq!(generated.port, 80);
    assert!(!generated.is_local);
    assert!(generated.is_primary);
    cleanup(production_path);

    let (dev, dev_path) = test_state(true).await;
    let generated = generate(&dev, "application-1").await.unwrap();
    assert_eq!(generated.host, "localhost");
    assert_eq!(generated.port, 10_000, "first free port in range");
    assert!(generated.is_local);
    assert_eq!(generated.url, "http://localhost:10000");
    cleanup(dev_path);
}

#[tokio::test]
async fn generated_sslip_uses_public_edge_server_address() {
    let (state, path) = test_state(false).await;
    let conn = state.db.clone();
    sqlx::query(
        "INSERT INTO servers (id, kind, name, ssh_host, ssh_port, ssh_user, host_key_fingerprint, host_key_known_hosts, ingress_mode, podman_mode, state, public_address, organization_id, created_at, updated_at) VALUES (?1, 'ssh', ?2, ?3, 22, ?4, ?5, ?6, 'public_edge', 'rootless', 'ready', ?7, ?8, 1, 1)",
    )
    .bind("remote-1")
    .bind("Remote")
    .bind("remote.example.test")
    .bind("litepod")
    .bind("SHA256:test-fingerprint")
    .bind("remote.example.test ssh-ed25519 AAAAtest")
    .bind("216.250.119.183")
    .bind("organization-1")
    .execute(&conn)
    .await
    .unwrap();
    sqlx::query("UPDATE applications SET server_id = 'remote-1' WHERE id = 'application-1'")
        .execute(&conn)
        .await
        .unwrap();

    let host = random_sslip_host(&state, "application-1").await.unwrap();
    assert!(host.ends_with(".216.250.119.183.sslip.io"));

    let generated = generate(&state, "application-1").await.unwrap();
    assert!(generated.host.ends_with(".216.250.119.183.sslip.io"));
    cleanup(path);
}

#[tokio::test]
async fn generated_sslip_rejects_public_edge_without_address() {
    let (state, path) = test_state(false).await;
    let conn = state.db.clone();
    sqlx::query(
        "INSERT INTO servers (id, kind, name, ssh_host, ssh_port, ssh_user, ingress_mode, podman_mode, state, organization_id, created_at, updated_at) VALUES (?1, 'ssh', ?2, ?3, 22, ?4, 'public_edge', 'rootless', 'ready', ?5, 1, 1)",
    )
    .bind("remote-1")
    .bind("Remote")
    .bind("remote.example.test")
    .bind("litepod")
    .bind("organization-1")
    .execute(&conn)
    .await
    .unwrap();
    sqlx::query("UPDATE applications SET server_id = 'remote-1' WHERE id = 'application-1'")
        .execute(&conn)
        .await
        .unwrap();

    let error = random_sslip_host(&state, "application-1")
        .await
        .unwrap_err();
    assert!(error.to_string().contains("public IP or hostname"));
    cleanup(path);
}

#[tokio::test]
async fn generated_sslip_rejects_public_edge_with_hostname() {
    let (state, path) = test_state(false).await;
    let conn = state.db.clone();
    sqlx::query(
        "INSERT INTO servers (id, kind, name, ssh_host, ssh_port, ssh_user, ingress_mode, podman_mode, state, public_address, organization_id, created_at, updated_at) VALUES (?1, 'ssh', ?2, ?3, 22, ?4, 'public_edge', 'rootless', 'ready', ?5, ?6, 1, 1)",
    )
    .bind("remote-1")
    .bind("Remote")
    .bind("remote.example.test")
    .bind("litepod")
    .bind("edge.example.com")
    .bind("organization-1")
    .execute(&conn)
    .await
    .unwrap();
    sqlx::query("UPDATE applications SET server_id = 'remote-1' WHERE id = 'application-1'")
        .execute(&conn)
        .await
        .unwrap();

    let error = random_sslip_host(&state, "application-1")
        .await
        .unwrap_err();
    let message = error.to_string();
    assert!(
        message.contains("IP address") && message.contains("edge.example.com"),
        "unexpected error: {message}"
    );
    cleanup(path);
}

#[tokio::test]
async fn generated_sslip_accepts_ipv6_public_edge_address() {
    let (state, path) = test_state(false).await;
    let conn = state.db.clone();
    sqlx::query(
        "INSERT INTO servers (id, kind, name, ssh_host, ssh_port, ssh_user, ingress_mode, podman_mode, state, public_address, organization_id, created_at, updated_at) VALUES (?1, 'ssh', ?2, ?3, 22, ?4, 'public_edge', 'rootless', 'ready', ?5, ?6, 1, 1)",
    )
    .bind("remote-1")
    .bind("Remote")
    .bind("remote.example.test")
    .bind("litepod")
    .bind("2001:db8::1")
    .bind("organization-1")
    .execute(&conn)
    .await
    .unwrap();
    sqlx::query("UPDATE applications SET server_id = 'remote-1' WHERE id = 'application-1'")
        .execute(&conn)
        .await
        .unwrap();

    let host = random_sslip_host(&state, "application-1").await.unwrap();
    assert!(host.ends_with(".2001:db8::1.sslip.io"), "host was {host}");
    cleanup(path);
}

#[tokio::test]
async fn generate_allocates_the_next_free_local_port() {
    let (state, path) = test_state(true).await;
    create(
        &state,
        "application-1",
        CreateDomainRequest {
            server_id: None,
            certificate: Default::default(),
            host: "localhost".into(),
            port: 10_000,
            https: None,
            is_primary: None,
        },
    )
    .await
    .unwrap();
    let generated = generate(&state, "application-1").await.unwrap();
    assert_eq!(generated.port, 10_001, "10000 is taken");
    cleanup(path);
}

#[tokio::test]
async fn validation_rejects_bad_input_before_writing() {
    let (state, path) = test_state(false).await;
    let scheme = create(
        &state,
        "application-1",
        CreateDomainRequest {
            server_id: None,
            certificate: Default::default(),
            host: "http://a.example.com".into(),
            port: 80,
            https: None,
            is_primary: None,
        },
    )
    .await;
    assert!(matches!(scheme, Err(AppError::ValidationError(_))));
    let bad_local_port = create(
        &state,
        "application-1",
        CreateDomainRequest {
            server_id: None,
            certificate: Default::default(),
            host: "localhost".into(),
            port: 0,
            https: None,
            is_primary: None,
        },
    )
    .await;
    assert!(
        matches!(bad_local_port, Err(AppError::ValidationError(_))),
        "port 0 is outside 1-65535"
    );
    let local_https = create(
        &state,
        "application-1",
        CreateDomainRequest {
            server_id: None,
            certificate: Default::default(),
            host: "localhost".into(),
            port: 10_000,
            https: Some(true),
            is_primary: None,
        },
    )
    .await;
    assert!(matches!(local_https, Err(AppError::ValidationError(_))));
    assert!(
        list(&state, "application-1").await.unwrap().is_empty(),
        "nothing was written"
    );
    cleanup(path);
}

#[tokio::test]
async fn port_mappings_cover_local_domains_only() {
    let (state, path) = test_state(true).await;
    create(
        &state,
        "application-1",
        CreateDomainRequest {
            server_id: None,
            certificate: Default::default(),
            host: "localhost".into(),
            port: 10_000,
            https: None,
            is_primary: None,
        },
    )
    .await
    .unwrap();
    create(
        &state,
        "application-1",
        CreateDomainRequest {
            server_id: None,
            certificate: Default::default(),
            host: "127.0.0.1".into(),
            port: 10_001,
            https: None,
            is_primary: None,
        },
    )
    .await
    .unwrap();
    create(
        &state,
        "application-1",
        CreateDomainRequest {
            server_id: None,
            certificate: Default::default(),
            host: "app.example.com".into(),
            port: 80,
            https: Some(true),
            is_primary: None,
        },
    )
    .await
    .unwrap();

    let mappings = port_mappings(&state, "application-1").await.unwrap();
    let mut ports: Vec<u16> = mappings
        .iter()
        .filter_map(|mapping| mapping.host_port)
        .collect();
    ports.sort();
    assert_eq!(
        ports,
        vec![10_000, 10_001],
        "only local domains are published"
    );
    assert!(
        mappings
            .iter()
            .all(|mapping| mapping.container_port == Some(80))
    );
    cleanup(path);
}

#[test]
fn route_ids_are_per_domain() {
    assert_eq!(route_id("domain-1"), "litepod-route-domain-1");
}

/// Both writes need their own short-lived connection so the test never holds
/// a reader open across a later write.
async fn mark_running(state: &SharedState, container_name: &str) {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE application_deployments SET status = 'running', container_name = ?1 WHERE application_id = 'application-1'",
    )
    .bind(container_name)
    .execute(&conn)
    .await
    .unwrap();
}

async fn set_compose_deploy_type(state: &SharedState) {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("UPDATE applications SET deploy_type = 'compose' WHERE id = 'application-1'")
        .execute(&conn)
        .await
        .unwrap();
}

#[tokio::test]
async fn compose_domains_dial_the_routed_service_not_the_compose_project() {
    let (state, path) = test_state(false).await;
    let domain = create(
        &state,
        "application-1",
        CreateDomainRequest {
            server_id: None,
            certificate: Default::default(),
            host: "app.example.com".into(),
            port: 80,
            https: None,
            is_primary: None,
        },
    )
    .await
    .unwrap();
    set_compose_deploy_type(&state).await;
    // What a Compose deploy stores: the project, which is not a container.
    mark_running(&state, "project-prod-application-tion-1-compose").await;

    assert_eq!(
        caddy_target(&state, "application-1", &domain)
            .await
            .unwrap(),
        None,
        "a domain with no service selected is not routable"
    );

    crate::deployment::compose::proxy_routes::upsert(
        &state,
        "application-1",
        &domain.id,
        "pastefy",
        8080,
    )
    .await
    .unwrap();
    let (runtime_name, _) = super::super::identity(&state, "application-1")
        .await
        .unwrap();
    assert_eq!(
        caddy_target(&state, "application-1", &domain)
            .await
            .unwrap(),
        Some((format!("{runtime_name}-pastefy"), 8080)),
        "Caddy dials the routed service on its own container port"
    );
    cleanup(path);
}

#[tokio::test]
async fn non_compose_domains_keep_dialing_the_deployment_container() {
    let (state, path) = test_state(false).await;
    let domain = create(
        &state,
        "application-1",
        CreateDomainRequest {
            server_id: None,
            certificate: Default::default(),
            host: "app.example.com".into(),
            port: 3000,
            https: None,
            is_primary: None,
        },
    )
    .await
    .unwrap();
    mark_running(&state, "project-prod-application-tion-1").await;

    assert_eq!(
        caddy_target(&state, "application-1", &domain)
            .await
            .unwrap(),
        Some(("project-prod-application-tion-1".into(), 3000))
    );
    cleanup(path);
}

/// Lives here because the Compose proxy-route side effects are all domain
/// state: the Caddy route and the redeploy flag.
#[tokio::test]
async fn routing_a_new_service_asks_for_a_redeploy_but_reusing_one_does_not() {
    let (state, path) = test_state(false).await;
    set_compose_deploy_type(&state).await;
    let first = create(
        &state,
        "application-1",
        CreateDomainRequest {
            server_id: None,
            certificate: Default::default(),
            host: "a.example.com".into(),
            port: 80,
            https: None,
            is_primary: None,
        },
    )
    .await
    .unwrap();
    let second = create(
        &state,
        "application-1",
        CreateDomainRequest {
            server_id: None,
            certificate: Default::default(),
            host: "b.example.com".into(),
            port: 80,
            https: None,
            is_primary: None,
        },
    )
    .await
    .unwrap();

    crate::deployment::compose::proxy_routes::save(
        &state,
        "application-1",
        &first.id,
        "pastefy",
        80,
    )
    .await
    .unwrap();
    assert_eq!(
        read_stale(&state).await,
        1,
        "pastefy still has to join the proxy network"
    );

    mark_stale(&state, "application-1", false).await.unwrap();
    crate::deployment::compose::proxy_routes::save(
        &state,
        "application-1",
        &second.id,
        "pastefy",
        80,
    )
    .await
    .unwrap();
    assert_eq!(
        read_stale(&state).await,
        0,
        "the service is already on the proxy network"
    );
    cleanup(path);
}

#[tokio::test]
async fn a_stopped_deployment_is_never_routable() {
    let (state, path) = test_state(false).await;
    let domain = create(
        &state,
        "application-1",
        CreateDomainRequest {
            server_id: None,
            certificate: Default::default(),
            host: "app.example.com".into(),
            port: 3000,
            https: None,
            is_primary: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        caddy_target(&state, "application-1", &domain)
            .await
            .unwrap(),
        None,
        "routing a stopped container would only 502"
    );
    cleanup(path);
}

/// Reads the flag on its own short-lived connection; holding one open across
/// a `mark_stale` write would deadlock against the writer.
async fn read_stale(state: &SharedState) -> i64 {
    use sqlx::Row;
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("SELECT config_stale FROM application_deployments WHERE application_id = ?1")
        .bind("application-1")
        .fetch_one(&conn)
        .await
        .unwrap()
        .try_get::<i64, _>(0)
        .unwrap()
}

#[tokio::test]
async fn marking_stale_toggles_the_deployment_flag() {
    let (state, path) = test_state(true).await;
    mark_stale(&state, "application-1", true).await.unwrap();
    assert_eq!(read_stale(&state).await, 1);

    mark_stale(&state, "application-1", false).await.unwrap();
    assert_eq!(read_stale(&state).await, 0);
    cleanup(path);
}

#[tokio::test]
async fn local_port_changes_flag_stale_but_proxied_edits_do_not() {
    let (state, path) = test_state(true).await;
    let local = create(
        &state,
        "application-1",
        CreateDomainRequest {
            server_id: None,
            certificate: Default::default(),
            host: "localhost".into(),
            port: 10_000,
            https: None,
            is_primary: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        read_stale(&state).await,
        1,
        "a new published port needs a redeploy"
    );

    mark_stale(&state, "application-1", false).await.unwrap();
    create(
        &state,
        "application-1",
        CreateDomainRequest {
            server_id: None,
            certificate: Default::default(),
            host: "app.example.com".into(),
            port: 80,
            https: None,
            is_primary: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        read_stale(&state).await,
        0,
        "proxied domains reconcile on save"
    );

    update(
        &state,
        "application-1",
        &local.id,
        UpdateDomainRequest {
            server_id: None,
            certificate: Default::default(),
            host: None,
            port: Some(10_005),
            https: None,
            is_primary: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        read_stale(&state).await,
        1,
        "changing a published port needs a redeploy"
    );
    cleanup(path);
}

#[test]
fn classifies_local_hosts() {
    assert!(is_local("localhost"));
    assert!(is_local("127.0.0.1"));
    assert!(is_local("::1"));
    assert!(is_local("LOCALHOST"), "host matching is case-insensitive");
    assert!(!is_local("app.example.com"));
    assert!(!is_local("192.168.1.10"));
}

#[test]
fn builds_public_urls_from_the_correct_port_type() {
    assert_eq!(
        domain_url("localhost", 10_000, false),
        "http://localhost:10000"
    );
    assert_eq!(
        domain_url("app.example.com", 443, true),
        "https://app.example.com"
    );
    assert_eq!(
        domain_url("app.example.com", 80, false),
        "http://app.example.com"
    );
    assert_eq!(
        domain_url("app.example.com", 8080, false),
        "http://app.example.com"
    );
    assert_eq!(
        domain_url("app.example.com", 8999, true),
        "https://app.example.com"
    );
}

#[test]
fn rejects_malformed_hosts() {
    assert!(check_host("").is_err(), "empty host");
    assert!(
        check_host("http://app.example.com").is_err(),
        "scheme in host"
    );
    assert!(check_host("app.example.com/path").is_err(), "path in host");
    assert!(check_host("app example.com").is_err(), "whitespace in host");
    assert!(
        check_host("app_example.com").is_err(),
        "underscore is not a hostname character"
    );
    assert!(check_host("app.example.com").is_ok());
    assert!(check_host("127.0.0.1").is_ok());
    assert!(check_host("::1").is_ok());
}

#[test]
fn rejects_https_on_local_hosts() {
    assert!(check_https("localhost", true).is_err());
    assert!(check_https("localhost", false).is_ok());
    assert!(check_https("app.example.com", true).is_ok());
}

#[tokio::test]
async fn local_port_outside_the_generation_range_is_accepted() {
    let (state, path) = test_state(true).await;
    let created = create(
        &state,
        "application-1",
        CreateDomainRequest {
            server_id: None,
            certificate: Default::default(),
            host: "localhost".into(),
            port: 3000,
            https: None,
            is_primary: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(created.port, 3000);
    assert!(created.is_local);
    cleanup(path);
}

#[tokio::test]
async fn reserved_local_port_is_still_rejected() {
    let (state, path) = test_state_with_reserved(true, HashSet::from([8081_i64])).await;
    let error = validate(&state, "localhost", 8081, false).unwrap_err();
    assert!(
        matches!(error, AppError::ValidationError(ref message) if message.contains("reserved")),
        "expected a reserved-port validation error",
    );
    cleanup(path);
}

#[tokio::test]
async fn port_constraints_lists_taken_local_ports_only() {
    let (state, path) = test_state(true).await;
    create(
        &state,
        "application-1",
        CreateDomainRequest {
            server_id: None,
            certificate: Default::default(),
            host: "localhost".into(),
            port: 3000,
            https: None,
            is_primary: None,
        },
    )
    .await
    .unwrap();
    create(
        &state,
        "application-1",
        CreateDomainRequest {
            server_id: None,
            certificate: Default::default(),
            host: "a.example.com".into(),
            port: 80,
            https: None,
            is_primary: None,
        },
    )
    .await
    .unwrap();

    let constraints = port_constraints(&state).await.unwrap();
    assert_eq!(
        constraints.taken_ports,
        vec![3000],
        "only local domains publish a host port"
    );
    cleanup(path);
}

#[tokio::test]
async fn duplicate_local_port_reports_the_conflicting_port() {
    let (state, path) = test_state(true).await;
    create(
        &state,
        "application-1",
        CreateDomainRequest {
            server_id: None,
            certificate: Default::default(),
            host: "localhost".into(),
            port: 3000,
            https: None,
            is_primary: None,
        },
    )
    .await
    .unwrap();
    let error = create(
        &state,
        "application-1",
        CreateDomainRequest {
            server_id: None,
            certificate: Default::default(),
            host: "localhost".into(),
            port: 3000,
            https: None,
            is_primary: None,
        },
    )
    .await
    .unwrap_err();
    assert!(
        matches!(error, AppError::ValidationError(ref message) if message.contains("3000") && message.contains("already used")),
        "expected a port-conflict message naming 3000",
    );
    cleanup(path);
}

#[tokio::test]
async fn allocate_local_port_stays_inside_the_configured_range() {
    let (state, path) = test_state(true).await;
    let port = allocate_local_port(&state).await.unwrap();
    assert!(
        (10_000..=20_000).contains(&port),
        "expected a port in 10000-20000, got {port}"
    );
    cleanup(path);
}

#[tokio::test]
async fn caddy_recovery_handles_late_start_restart_failure_and_concurrent_writes() {
    use crate::deployment::operations::{edit_caddy_config, recover_missing_caddy_routes};
    use axum::{
        Json, Router,
        extract::State,
        http::StatusCode,
        routing::{get, post},
    };
    use serde_json::{Value, json};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    let base = json!({"apps":{"http":{"servers":{"srv0":{
        "listen":[":80"], "routes":[{"handle":[{"handler":"static_response","body":"fallback"}]}]
    }}}}});
    let live = Arc::new(tokio::sync::Mutex::new(base.clone()));
    let writes = Arc::new(AtomicUsize::new(0));
    let fail = Arc::new(AtomicBool::new(false));
    let router = Router::new()
        .route(
            "/config/",
            get(
                |State((live, _, _)): State<(
                    Arc<tokio::sync::Mutex<Value>>,
                    Arc<AtomicUsize>,
                    Arc<AtomicBool>,
                )>| async move {
                    let snapshot = live.lock().await.clone();
                    // Widen the lost-update window: concurrent GETs must be serialized.
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                    Json(snapshot)
                },
            ),
        )
        .route(
            "/load",
            post(
                |State((live, writes, fail)): State<(
                    Arc<tokio::sync::Mutex<Value>>,
                    Arc<AtomicUsize>,
                    Arc<AtomicBool>,
                )>,
                 Json(config): Json<Value>| async move {
                    if fail.swap(false, Ordering::SeqCst) {
                        return StatusCode::SERVICE_UNAVAILABLE;
                    }
                    *live.lock().await = config;
                    writes.fetch_add(1, Ordering::SeqCst);
                    StatusCode::OK
                },
            ),
        )
        .with_state((live.clone(), writes.clone(), fail.clone()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let (mut state, path) = test_state(false).await;
    Arc::get_mut(&mut state).unwrap().caddy_admin_url = url;
    let domain = create(
        &state,
        "application-1",
        CreateDomainRequest {
            server_id: None,
            certificate: Default::default(),
            host: "pastefy.example.com".into(),
            port: 80,
            https: Some(true),
            is_primary: None,
        },
    )
    .await
    .unwrap();
    set_compose_deploy_type(&state).await;
    crate::deployment::compose::proxy_routes::upsert(
        &state,
        "application-1",
        &domain.id,
        "pastefy",
        8080,
    )
    .await
    .unwrap();
    // An intentionally stopped app is never started or given a new route.
    sqlx::query("UPDATE application_deployments SET status = 'stopped'")
        .execute(&state.db)
        .await
        .unwrap();
    recover_missing_caddy_routes(&state).await.unwrap();
    assert_eq!(writes.load(Ordering::SeqCst), 0);
    let status: String = sqlx::query_scalar("SELECT status FROM application_deployments")
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert_eq!(status, "stopped");

    // The monitor observes the app starting after boot recovery already ran.
    mark_running(&state, "project-prod-application-tion-1-compose").await;
    sqlx::query(
        "INSERT INTO application_basic_auth VALUES ('auth', 'application-1', 1, 'Private', 1, 1)",
    )
    .execute(&state.db)
    .await
    .unwrap();
    sqlx::query("INSERT INTO application_basic_auth_users VALUES ('user', 'auth', 'alice', 'test-hash', 1, 1)").execute(&state.db).await.unwrap();
    sqlx::query("INSERT INTO application_basic_auth_domains VALUES ('auth', ?1)")
        .bind(&domain.id)
        .execute(&state.db)
        .await
        .unwrap();
    fail.store(true, Ordering::SeqCst);
    assert!(recover_missing_caddy_routes(&state).await.is_err());
    assert_eq!(*live.lock().await, base);
    recover_missing_caddy_routes(&state).await.unwrap();
    let healthy = live.lock().await.clone();
    let route = &healthy["apps"]["http"]["servers"]["srv0"]["routes"][0];
    assert_eq!(route["handle"][0]["handler"], "authentication");
    assert_eq!(route["handle"][1]["handler"], "reverse_proxy");
    assert!(
        route["handle"][1]["upstreams"][0]["dial"]
            .as_str()
            .unwrap()
            .ends_with("-pastefy:8080")
    );
    assert_eq!(
        healthy["apps"]["tls"]["automation"]["policies"][0]["subjects"][0],
        "pastefy.example.com"
    );
    recover_missing_caddy_routes(&state).await.unwrap();
    assert_eq!(
        writes.load(Ordering::SeqCst),
        1,
        "healthy routes must not reload Caddy"
    );

    // Caddy restarts independently, forgetting its in-memory configuration.
    *live.lock().await = base.clone();
    let (recovered, edited) = tokio::join!(
        recover_missing_caddy_routes(&state),
        edit_caddy_config(&state, |config| {
            config["unrelated"] = json!(true);
            Ok(())
        }),
    );
    recovered.unwrap();
    edited.unwrap();
    let result = live.lock().await.clone();
    assert_eq!(result["unrelated"], true);
    assert_eq!(result["apps"], healthy["apps"]);
    // A route whose TLS policy was lost must also be repaired.
    live.lock().await["apps"]["tls"]["automation"]["policies"] = json!([]);
    recover_missing_caddy_routes(&state).await.unwrap();
    assert_eq!(live.lock().await["apps"], healthy["apps"]);
    // Control-plane and manual routes also survive an independent proxy restart.
    sqlx::query("INSERT INTO server_domain_settings VALUES (1, 'lp.example.com', 'admin@example.com', 1, 1)")
        .execute(&state.db).await.unwrap();
    sqlx::query("INSERT INTO caddy_manual_routes VALUES ('manual', 'local', 'docs.example.com', 'redirect', 'https://example.com', 1, 308, 1, 1)")
        .execute(&state.db).await.unwrap();
    *live.lock().await = base;
    recover_missing_caddy_routes(&state).await.unwrap();
    let count = writes.load(Ordering::SeqCst);
    let config = live.lock().await.clone();
    let routes = config["apps"]["http"]["servers"]["srv0"]["routes"]
        .as_array()
        .unwrap();
    assert!(
        routes
            .iter()
            .any(|route| route["@id"] == "litepod-server-domain")
    );
    assert!(
        routes
            .iter()
            .any(|route| route["@id"] == "litepod-manual-manual")
    );
    recover_missing_caddy_routes(&state).await.unwrap();
    assert_eq!(writes.load(Ordering::SeqCst), count);
    server.abort();
    cleanup(path);
}

#[tokio::test]
async fn certificate_changes_and_primary_survive_failed_caddy_updates() {
    let (state, path) = test_state(false).await;
    let created = create(
        &state,
        "application-1",
        CreateDomainRequest {
            server_id: None,
            host: "api.example.com".into(),
            port: 8080,
            https: Some(true),
            is_primary: Some(true),
            certificate: crate::deployment::certificates::CertificateRequest {
                certificate_provider: Some("letsencrypt".into()),
                ..Default::default()
            },
        },
    )
    .await
    .unwrap();
    assert_eq!(created.certificate_provider, "letsencrypt");
    sqlx::query(
        "UPDATE application_deployments SET status='running' WHERE application_id='application-1'",
    )
    .execute(&state.db)
    .await
    .unwrap();
    let changed = update(
        &state,
        "application-1",
        &created.id,
        UpdateDomainRequest {
            server_id: None,
            host: None,
            port: Some(8081),
            https: None,
            is_primary: None,
            certificate: crate::deployment::certificates::CertificateRequest {
                certificate_provider: Some("automatic".into()),
                ..Default::default()
            },
        },
    )
    .await;
    assert!(changed.is_err());
    let retained = get(&state, "application-1", &created.id).await.unwrap();
    assert_eq!(retained.port, 8080);
    assert_eq!(retained.certificate_provider, "letsencrypt");
    let failed_create = create(
        &state,
        "application-1",
        CreateDomainRequest {
            server_id: None,
            host: "other.example.com".into(),
            port: 8080,
            https: Some(true),
            is_primary: Some(true),
            certificate: Default::default(),
        },
    )
    .await;
    assert!(failed_create.is_err());
    let domains = list(&state, "application-1").await.unwrap();
    assert_eq!(domains.len(), 1);
    assert!(domains[0].is_primary);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM domain_certificates")
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert_eq!(count, 1);
    cleanup(path);
}

#[tokio::test]
async fn generate_with_secondary_keeps_the_primary() {
    let (state, path) = test_state(false).await;
    let primary = generate(&state, "application-1").await.unwrap();
    let secondary = generate_with(&state, "application-1", false).await.unwrap();
    assert!(!secondary.is_primary);
    assert_ne!(primary.host, secondary.host);
    let domains = list(&state, "application-1").await.unwrap();
    assert_eq!(domains.iter().filter(|d| d.is_primary).count(), 1);
    cleanup(path);
}
