use super::{overrides::*, process::*, shared::*};

/// Shape of the rustrak catalog template that broke `rustrakk.litepod.sh`:
/// the primary port (3000) belongs to `ui`, but `server` comes first.
const RUSTRAK_COMPOSE: &str = r#"
services:
  server:
    image: rustrak/rustrak-server:latest
    ports:
      - "8080:8080"
  ui:
    image: rustrak/rustrak-ui:latest
    ports:
      - "3000:3000"
"#;

#[test]
fn primary_port_resolves_to_the_service_that_publishes_it_not_the_first_one() {
    assert_eq!(
        first_service_name(RUSTRAK_COMPOSE).as_deref(),
        Some("server")
    );
    assert_eq!(
        service_for_port(RUSTRAK_COMPOSE, 3000).as_deref(),
        Some("ui")
    );
    assert_eq!(
        service_for_port(RUSTRAK_COMPOSE, 8080).as_deref(),
        Some("server")
    );
}

#[test]
fn service_for_port_reads_every_compose_port_shape() {
    let compose = r#"
services:
  bare:
    ports:
      - "9001"
  host_bound:
    ports:
      - "0.0.0.0:8080:9002/tcp"
  long_syntax:
    ports:
      - target: 9003
        published: 18003
        protocol: tcp
  numeric:
    ports:
      - 9004
  exposed:
    expose:
      - "9005"
"#;
    for (port, service) in [
        (9001, "bare"),
        (9002, "host_bound"),
        (9003, "long_syntax"),
        (9004, "numeric"),
        (9005, "exposed"),
    ] {
        assert_eq!(
            service_for_port(compose, port).as_deref(),
            Some(service),
            "port {port}"
        );
    }
}

#[test]
fn service_for_port_reports_no_match_instead_of_guessing() {
    assert_eq!(service_for_port(RUSTRAK_COMPOSE, 5432), None);
    assert_eq!(service_for_port("not: compose", 3000), None);
    assert_eq!(service_for_port("services:\n  web: {}\n", 3000), None);
}

#[test]
fn port_ranges_never_claim_a_primary_port() {
    let compose = "services:\n  web:\n    ports:\n      - \"3000-3005\"\n";
    assert_eq!(service_for_port(compose, 3000), None);
}

#[test]
fn compose_launch_description_is_useful_without_exposing_runtime_secrets() {
    let description = compose_launch_description(
        "project-name",
        &["docker-compose.yml", "litepod.standalone.override.yml"],
        &["up", "--detach", "--build"],
    );

    assert_eq!(
        description,
        "Launching podman compose --project-name project-name --file docker-compose.yml --file litepod.standalone.override.yml up --detach --build"
    );
    assert!(!description.contains("CONTAINER_HOST"));
    assert!(!description.contains("unix://"));
}

#[test]
fn candidate_private_network_does_not_share_service_dns_with_serving_project() {
    let content = "services:\n  web:\n    image: nginx\n";
    let old = normalize_compose_networks(content, "app").unwrap().unwrap();
    let new = normalize_compose_networks(content, "app-release-123456abcdef")
        .unwrap()
        .unwrap();
    let old: serde_yaml::Value = serde_yaml::from_str(&old).unwrap();
    let new: serde_yaml::Value = serde_yaml::from_str(&new).unwrap();
    assert_ne!(
        old["networks"]["private"]["name"],
        new["networks"]["private"]["name"]
    );
}

#[test]
fn candidate_ports_are_ephemeral_and_preserve_multiple_ports_per_service() {
    let services = vec![ComposeService { name: "web".into() }];
    let content = generate_remote_standalone_override(
        "app-release-123456abcdef",
        &services,
        &[("web", 0, 3000), ("web", 0, 8080)],
    )
    .unwrap();
    let document: serde_yaml::Value = serde_yaml::from_str(&content).unwrap();
    assert_eq!(
        document["services"]["web"]["container_name"].as_str(),
        Some("app-release-123456abcdef-web")
    );
    assert_eq!(
        document["services"]["web"]["ports"]
            .as_sequence()
            .unwrap()
            .len(),
        2
    );
    assert!(content.contains("127.0.0.1::3000/tcp"));
    assert!(content.contains("127.0.0.1::8080/tcp"));
}

#[test]
fn runtime_resources_override_caps_every_service() {
    let services = vec![
        ComposeService { name: "web".into() },
        ComposeService {
            name: "worker".into(),
        },
    ];
    let limits = super::super::ResourceLimits {
        cpu_quota_us: 150_000,
        memory_bytes: 2 * 1024 * 1024 * 1024,
        memswap_bytes: 2 * 1024 * 1024 * 1024,
    };

    let override_yaml = generate_runtime_resources_override(&services, &limits);

    let parsed: serde_yaml::Value = serde_yaml::from_str(&override_yaml).unwrap();
    for service in ["web", "worker"] {
        let limits_map = &parsed["services"][service]["deploy"]["resources"]["limits"];
        assert_eq!(limits_map["cpus"].as_str(), Some("1.5"));
        assert_eq!(limits_map["memory"].as_str(), Some("2147483648"));
    }
}

#[test]
fn remote_override_has_one_loopback_binding_and_joins_no_shared_network() {
    let yaml = generate_remote_standalone_override(
        "project-prod-gitea-a3f91c",
        &[service("gitea"), service("database")],
        &[("gitea", 42_123, 3000)],
    )
    .unwrap();
    assert!(yaml.contains("127.0.0.1:42123:3000/tcp"));
    let value: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
    assert!(value["services"]["database"].get("ports").is_none());
    // Nothing ever dials a remote container by network alias (every
    // ingress mode resolves to the loopback `endpoint` binding instead),
    // so no service - routed or not - joins a shared network here. Each
    // stays on Compose's own private per-app network, so a service
    // named e.g. "db" can never collide with another app's
    // identically-named service - the exact collision that put the
    // wrong Postgres behind a customer's Forgejo deploy.
    assert!(value.get("networks").is_none());
    assert!(value["services"]["gitea"]["networks"].is_null());
    assert!(value["services"]["database"]["networks"].is_null());
}

#[test]
fn remote_override_names_every_service() {
    let yaml = generate_remote_standalone_override(
        "project-prod-gitea-a3f91c",
        &[service("gitea"), service("database")],
        &[],
    )
    .unwrap();
    let value: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
    // Every service still gets a globally-unique container name, so two
    // apps never fight over the same container.
    assert_eq!(
        value["services"]["gitea"]["container_name"].as_str(),
        Some("project-prod-gitea-a3f91c-gitea")
    );
    assert_eq!(
        value["services"]["database"]["container_name"].as_str(),
        Some("project-prod-gitea-a3f91c-database")
    );
}

#[test]
fn remote_override_gives_every_routed_service_its_own_binding() {
    // The actual bug this fixes: a remote Compose app with two publicly
    // routed services (e.g. an API and a UI) gets two independent
    // loopback bindings instead of erroring.
    let yaml = generate_remote_standalone_override(
        "project-prod-rustrak-5c6ufh",
        &[service("server"), service("ui")],
        &[("server", 42_010, 8080), ("ui", 42_011, 3000)],
    )
    .unwrap();
    assert!(yaml.contains("127.0.0.1:42010:8080/tcp"));
    assert!(yaml.contains("127.0.0.1:42011:3000/tcp"));
    let value: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
    assert_eq!(
        value["services"]["server"]["ports"][0].as_str(),
        Some("127.0.0.1:42010:8080/tcp")
    );
    assert_eq!(
        value["services"]["ui"]["ports"][0].as_str(),
        Some("127.0.0.1:42011:3000/tcp")
    );
}

#[test]
fn remote_compose_keeps_source_ports_and_strips_central_proxy_network() {
    let yaml = prepare_remote_compose(
        "services:\n  web:\n    image: nginx\n    ports: [\"0.0.0.0:8080:80\"]\n    networks: [litepod_proxy, private]\nnetworks:\n  litepod_proxy: { external: true }\n  private: {}\n",
    )
    .unwrap();
    let value: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
    // A user-declared `ports:` publishes on the remote host exactly like it
    // already does locally - only the shared `litepod_proxy` network (which
    // makes no sense on a different machine) is stripped.
    assert_eq!(
        value["services"]["web"]["ports"].as_sequence().unwrap()[0].as_str(),
        Some("0.0.0.0:8080:80")
    );
    assert!(
        value["services"]["web"]["networks"]
            .as_sequence()
            .unwrap()
            .iter()
            .all(|network| network.as_str() != Some("litepod_proxy"))
    );
    assert!(value["networks"].get("litepod_proxy").is_none());
}

#[test]
fn compose_failure_includes_exit_code_and_bounded_stderr_context() {
    assert_eq!(
        compose_failure(Some(125), &["image pull failed".into()]),
        "Podman Compose failed with exit code 125: image pull failed"
    );
    assert_eq!(
        compose_failure(None, &[]),
        "Podman Compose failed with termination by signal"
    );
}

#[test]
fn remove_warning_explicitly_mentions_managed_volumes() {
    assert_eq!(
        REMOVE_VOLUMES_WARNING,
        "WARNING: Removing Compose runtime and Compose-managed volumes"
    );
}

fn service(name: &str) -> ComposeService {
    ComposeService { name: name.into() }
}

#[test]
fn rootless_named_volumes_only_picks_up_rootless_services() {
    let content = "services:\n  forgejo:\n    image: codeberg.org/forgejo/forgejo:16-rootless\n    volumes:\n      - forgejo_data:/var/lib/gitea\n      - forgejo_config:/etc/gitea\n  db:\n    image: docker.io/library/postgres:17\n    volumes:\n      - forgejo_postgres_data:/var/lib/postgresql/data\nvolumes:\n  forgejo_data: {}\n  forgejo_config: {}\n  forgejo_postgres_data: {}\n";
    assert_eq!(
        rootless_named_volumes(content),
        vec!["forgejo_config".to_string(), "forgejo_data".to_string()]
    );
}

#[test]
fn rootless_named_volumes_ignores_bind_mounts_and_undeclared_names() {
    let content = "services:\n  app:\n    image: example/app:1-rootless\n    volumes:\n      - ./local:/etc/app\n      - /abs/host/path:/data\n      - not_declared:/other\n      - real_volume:/real\nvolumes:\n  real_volume: {}\n";
    assert_eq!(
        rootless_named_volumes(content),
        vec!["real_volume".to_string()]
    );
}

#[test]
fn rootless_named_volumes_returns_empty_for_invalid_or_service_less_compose() {
    assert!(rootless_named_volumes("not: valid: yaml: [").is_empty());
    assert!(rootless_named_volumes("services: {}\n").is_empty());
}

#[test]
fn override_names_every_service_but_only_networks_the_routed_one() {
    let yaml = generate_standalone_override(
        "proyecto-prod-api-a3f91c",
        "podman",
        &[service("api"), service("worker")],
        &["api".into()],
    );
    let value: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
    let services = value["services"].as_mapping().unwrap();
    let api = services[&serde_yaml::Value::String("api".into())]
        .as_mapping()
        .unwrap();
    let worker = services[&serde_yaml::Value::String("worker".into())]
        .as_mapping()
        .unwrap();

    // Every service still gets a globally-unique container name, so two
    // apps never fight over the same container.
    assert_eq!(
        api["container_name"].as_str(),
        Some("proyecto-prod-api-a3f91c-api")
    );
    assert_eq!(
        worker["container_name"].as_str(),
        Some("proyecto-prod-api-a3f91c-worker")
    );
    // Only "api" is a routed service — it joins the shared proxy network.
    assert!(api.contains_key(serde_yaml::Value::String("networks".into())));
    // "worker" isn't routed anywhere: it must NOT join the network every
    // other app on the host shares too, or its bare service name could
    // collide with another app's identically-named service there.
    assert!(!worker.contains_key(serde_yaml::Value::String("networks".into())));
}

#[test]
fn routed_service_without_own_networks_only_gets_the_proxy_network() {
    let yaml = generate_standalone_override(
        "proyecto-prod-api-a3f91c",
        "podman",
        &[service("web"), service("db")],
        &["web".into()],
    );
    let value: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
    let networks = value["services"]["web"]["networks"].as_mapping().unwrap();

    // The override must not re-introduce a `default` reference the
    // top-level block does not declare; podman-compose rejects it.
    assert!(!networks.contains_key(serde_yaml::Value::String("default".into())));
    assert!(networks.contains_key(serde_yaml::Value::String("litepod_proxy".into())));
    assert_eq!(
        networks[&serde_yaml::Value::String("litepod_proxy".into())]["aliases"][0].as_str(),
        Some("proyecto-prod-api-a3f91c-web")
    );
    // "db" isn't a routed service: it keeps Compose's own private
    // per-app network instead of the one every app shares.
    assert!(value["services"]["db"]["networks"].is_null());
}

#[test]
fn routed_service_with_own_networks_only_gains_the_proxy_network() {
    let yaml = generate_standalone_override(
        "proyecto-prod-api-a3f91c",
        "podman",
        &[service("web")],
        &["web".into()],
    );
    let value: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
    let networks = value["services"]["web"]["networks"].as_mapping().unwrap();

    // Compose merges the mappings, so the user's own networks survive and
    // `default` would attach an extra network nobody asked for.
    assert!(!networks.contains_key(serde_yaml::Value::String("default".into())));
    assert!(networks.contains_key(serde_yaml::Value::String("litepod_proxy".into())));
}

#[test]
fn compose_services_lists_every_service() {
    let services = compose_services(
        "services:\n  web:\n    image: nginx\n    networks:\n      internal: {}\n  db:\n    image: mariadb\n",
    )
    .unwrap();
    let names: Vec<_> = services
        .iter()
        .map(|service| service.name.as_str())
        .collect();
    assert_eq!(names, vec!["web", "db"]);
}

#[test]
fn networks_written_as_a_list_become_a_mapping_the_override_can_merge_with() {
    let normalized = normalize_compose_networks(
        "services:\n  sqlserver:\n    image: mssql\n    volumes:\n      - data:/var/opt/mssql\n    networks:\n      - litepod_proxy\nvolumes:\n  data: {}\n",
        "test-app",
    )
    .unwrap()
    .expect("the list form has to be rewritten");
    let value: serde_yaml::Value = serde_yaml::from_str(&normalized).unwrap();
    let networks = value["services"]["sqlserver"]["networks"]
        .as_mapping()
        .unwrap();

    assert!(networks.contains_key(serde_yaml::Value::String("litepod_proxy".into())));
    // The rest of the file has to survive the rewrite untouched.
    assert_eq!(
        value["services"]["sqlserver"]["image"].as_str(),
        Some("mssql")
    );
    assert_eq!(
        value["services"]["sqlserver"]["volumes"][0].as_str(),
        Some("data:/var/opt/mssql")
    );
    assert!(value["volumes"]["data"].is_mapping());
}

#[test]
fn services_without_networks_gain_one_shared_private_network() {
    let normalized = normalize_compose_networks(
        "services:\n  web:\n    image: nginx\n  db:\n    image: postgres\n",
        "infisical-7vsns",
    )
    .unwrap()
    .expect("implicit networking must be made explicit");
    let value: serde_yaml::Value = serde_yaml::from_str(&normalized).unwrap();

    assert!(value["networks"]["private"].is_mapping());
    assert_eq!(
        value["networks"]["private"]["name"].as_str(),
        Some("infisical-7vsns_private")
    );
    assert!(value["services"]["web"]["networks"]["private"].is_mapping());
    assert!(value["services"]["db"]["networks"]["private"].is_mapping());
}

#[test]
fn network_mode_service_is_not_given_a_conflicting_network() {
    let normalized = normalize_compose_networks(
        "services:\n  vpn:\n    image: wireguard\n    network_mode: host\n  app:\n    image: nginx\n",
        "test-app",
    )
    .unwrap()
    .unwrap();
    let value: serde_yaml::Value = serde_yaml::from_str(&normalized).unwrap();

    assert!(value["services"]["vpn"]["networks"].is_null());
    assert!(value["services"]["app"]["networks"]["private"].is_mapping());
}

#[test]
fn existing_private_network_definition_and_membership_are_preserved() {
    let normalized = normalize_compose_networks(
        "services:\n  web:\n    image: nginx\n    networks: [frontend]\n  db:\n    image: postgres\nnetworks:\n  frontend:\n    driver: bridge\n  private:\n    driver: macvlan\n",
        "test-app",
    )
    .unwrap()
    .unwrap();
    let value: serde_yaml::Value = serde_yaml::from_str(&normalized).unwrap();

    assert!(value["services"]["web"]["networks"]["frontend"].is_mapping());
    assert!(value["services"]["web"]["networks"]["private"].is_null());
    assert!(value["services"]["db"]["networks"]["private"].is_mapping());
    assert_eq!(
        value["networks"]["private"]["driver"].as_str(),
        Some("macvlan")
    );
}

#[test]
fn private_network_normalization_is_idempotent() {
    let first = normalize_compose_networks("services:\n  web:\n    image: nginx\n", "test-app")
        .unwrap()
        .unwrap();
    assert!(
        normalize_compose_networks(&first, "test-app")
            .unwrap()
            .is_none()
    );
}

#[test]
fn malformed_service_networks_are_rejected_with_service_name() {
    let error = normalize_compose_networks(
        "services:\n  broken-api:\n    image: nginx\n    networks: definitely-not-a-list-or-map\n",
        "test-app",
    )
    .unwrap_err();
    assert!(error.contains("broken-api"), "{error}");
    assert!(error.contains("networks"), "{error}");
}

#[test]
fn a_compose_file_that_needs_no_rewrite_is_left_alone() {
    assert!(
        normalize_compose_networks(
            "services:\n  web:\n    image: nginx\n    networks:\n      frontend: {}\n  db:\n    image: mariadb\n    networks:\n      backend: {}\nnetworks:\n  frontend: {}\n  backend: {}\n",
            "test-app",
        )
        .unwrap()
        .is_none()
    );
}

#[test]
fn normalized_private_network_and_routed_override_have_mergeable_shapes() {
    let normalized = normalize_compose_networks(
        "services:\n  web:\n    image: nginx\n  db:\n    image: postgres\n",
        "test-app",
    )
    .unwrap()
    .unwrap();
    let base: serde_yaml::Value = serde_yaml::from_str(&normalized).unwrap();
    let override_yaml = generate_standalone_override(
        "project-app-a3f91c",
        "litepod-network",
        &[service("web"), service("db")],
        &["web".into()],
    );
    let generated: serde_yaml::Value = serde_yaml::from_str(&override_yaml).unwrap();

    assert!(base["services"]["web"]["networks"]["private"].is_mapping());
    assert!(base["services"]["db"]["networks"]["private"].is_mapping());
    assert!(generated["services"]["web"]["networks"]["litepod_proxy"].is_mapping());
    assert!(generated["services"]["db"]["networks"].is_null());
}

#[test]
fn normalized_file_and_override_agree_on_the_networks_shape() {
    let normalized = normalize_compose_networks(
        "services:\n  sqlserver:\n    image: mssql\n    networks:\n      - litepod_proxy\n",
        "test-app",
    )
    .unwrap()
    .unwrap();
    let base: serde_yaml::Value = serde_yaml::from_str(&normalized).unwrap();
    let override_yaml = generate_standalone_override(
        "databases-prod-sqldev-2850c0",
        "litepod-network",
        &[service("sqlserver")],
        &["sqlserver".into()],
    );
    let generated: serde_yaml::Value = serde_yaml::from_str(&override_yaml).unwrap();

    // podman-compose refuses to merge a list with a mapping, so both files
    // have to reach the provider with the same shape.
    assert!(base["services"]["sqlserver"]["networks"].is_mapping());
    assert!(generated["services"]["sqlserver"]["networks"].is_mapping());
}

#[test]
fn an_unsupported_networks_entry_is_reported_with_the_service_name() {
    let error = normalize_compose_networks(
        "services:\n  web:\n    image: nginx\n    networks:\n      - [nested]\n",
        "test-app",
    )
    .unwrap_err();
    assert!(error.contains("web"), "{error}");
}

#[test]
fn first_service_name_picks_the_first_compose_service() {
    let content = "services:\n  mailpit:\n    image: foo\n  extra:\n    image: bar\n";
    assert_eq!(first_service_name(content).as_deref(), Some("mailpit"));
}

#[test]
fn first_service_name_returns_none_for_an_empty_compose() {
    assert_eq!(first_service_name("services: {}\n"), None);
    assert_eq!(first_service_name(""), None);
}
