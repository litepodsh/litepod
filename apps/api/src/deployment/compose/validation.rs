/// Compose service names that would shadow a control-plane container on the
/// shared proxy network.
///
/// Compose registers every service name as a DNS alias on each network the
/// service joins, and an override can only *add* aliases - never drop that
/// default one. A deployed app with a service named `libsql` therefore answers
/// to the same name the API dials for its own database, and roughly half the
/// control-plane queries land on the tenant's sqld, which rejects the API's JWT.
///
/// Control-plane traffic now uses the `litepod-*` container names, which no
/// service name can produce; this list stays as the second line of defence.
pub const RESERVED_SERVICE_NAMES: [&str; 6] =
    ["api", "app", "buildkit", "caddy", "dragonfly", "libsql"];

/// The document with `<<: *anchor` merge keys applied, so a `volumes:` or
/// `privileged:` inherited from an anchor is seen like one written inline.
fn parse(content: &str) -> Option<serde_yaml::Value> {
    let mut document = serde_yaml::from_str::<serde_yaml::Value>(content).ok()?;
    document.apply_merge().ok()?;
    Some(document)
}

/// Every check a tenant-authored Compose file must pass before it is stored or
/// deployed: the editor, the assistant, templates and a file read from a Git
/// repository. `allow_privileged` is true only for the instance SysAdmin.
pub fn validate_tenant_compose(content: &str, allow_privileged: bool) -> Result<(), String> {
    let reserved = reserved_services_in_compose(content);
    if !reserved.is_empty() {
        return Err(reserved_service_message(&reserved));
    }
    let mounts = unnamed_mounts_in_compose(content);
    if !mounts.is_empty() {
        return Err(unnamed_mounts_message(&mounts));
    }
    let files = external_files_in_compose(content);
    if !files.is_empty() {
        return Err(format!(
            "Compose `include:` and `extends: file:` are refused, the referenced files are not validated: {}",
            files.join(", ")
        ));
    }
    let networks = control_networks_in_compose(content);
    if !networks.is_empty() {
        return Err(format!(
            "`{CONTROL_NETWORK}` is litepod's internal network and cannot be joined: {}",
            networks.join(", ")
        ));
    }
    let volumes = foreign_volumes_in_compose(content);
    if !volumes.is_empty() {
        return Err(format!(
            "Top-level volumes may not set `driver`, `driver_opts`, `external` or `name`: litepod \
             creates and names them. Declare the volume as `app_data: {{}}`: {}",
            volumes.join(", ")
        ));
    }
    if !allow_privileged {
        let privileged = privileged_services_in_compose(content);
        if !privileged.is_empty() {
            return Err(privileged_services_message(&privileged));
        }
        let host_access = host_access_in_compose(content);
        if !host_access.is_empty() {
            return Err(format!(
                "These service options reach past the container into the server and are refused: {}",
                host_access.join(", ")
            ));
        }
    }
    Ok(())
}

/// The internal network of the control plane (API, Dragonfly, Caddy), kept apart
/// from `litepod-network` so applications cannot reach Dragonfly.
pub const CONTROL_NETWORK: &str = "litepod-control";

/// Top-level networks that resolve to `CONTROL_NETWORK`: by `name:`, by
/// `external: { name: }` (Compose v2 syntax) or by the key of an external network.
pub fn control_networks_in_compose(content: &str) -> Vec<String> {
    let Some(document) = parse(content) else {
        return Vec::new();
    };
    let Some(networks) = document
        .get("networks")
        .and_then(serde_yaml::Value::as_mapping)
    else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for (key, config) in networks {
        let key = key.as_str().unwrap_or_default();
        let name = config.get("name").and_then(serde_yaml::Value::as_str);
        let external = config.get("external");
        let external_name = external
            .and_then(|external| external.get("name"))
            .and_then(serde_yaml::Value::as_str);
        let resolved = name
            .or(external_name)
            .or_else(|| external.is_some().then_some(key));
        if resolved == Some(CONTROL_NETWORK) {
            found.push(key.to_owned());
        }
    }
    found
}

/// Top-level volumes that are not a plain litepod-owned volume. `driver_opts`
/// turns a "named" volume into a bind of any host path, and `external` or
/// `name:` points it at an existing volume such as `litepod_data` (the control
/// plane's database). The volumes litepod manages itself are declared in a
/// generated override, never in this content.
pub fn foreign_volumes_in_compose(content: &str) -> Vec<String> {
    let Some(document) = parse(content) else {
        return Vec::new();
    };
    let Some(volumes) = document
        .get("volumes")
        .and_then(serde_yaml::Value::as_mapping)
    else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for (name, config) in volumes {
        let name = name.as_str().unwrap_or_default();
        for key in ["driver_opts", "external", "name"] {
            if config.get(key).is_some() {
                found.push(format!("{name}: {key}"));
            }
        }
        let driver = config.get("driver").and_then(serde_yaml::Value::as_str);
        if driver.is_some_and(|driver| driver != "local") {
            found.push(format!("{name}: driver"));
        }
    }
    found
}

/// Service options that, like `privileged`, hand a container the host's
/// kernel, devices, namespaces or another container's mounts. Only the
/// instance SysAdmin may use them. `network_mode` is fine as `bridge`, `none`
/// or `service:` of a service in the same file.
pub fn host_access_in_compose(content: &str) -> Vec<String> {
    const REFUSED: [&str; 10] = [
        "cap_add",
        "security_opt",
        "devices",
        "device_cgroup_rules",
        "pid",
        "ipc",
        "userns_mode",
        "cgroup",
        "sysctls",
        "volumes_from",
    ];
    let Some(document) = parse(content) else {
        return Vec::new();
    };
    let Some(services) = document
        .get("services")
        .and_then(serde_yaml::Value::as_mapping)
    else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for (name, service) in services {
        let name = name.as_str().unwrap_or_default();
        for key in REFUSED {
            if service.get(key).is_some() {
                found.push(format!("{name}: {key}"));
            }
        }
        if let Some(mode) = service.get("network_mode") {
            let allowed = match mode.as_str() {
                Some("bridge" | "none") => true,
                Some(mode) => mode
                    .strip_prefix("service:")
                    .is_some_and(|other| services.contains_key(other)),
                None => false,
            };
            if !allowed {
                found.push(format!("{name}: network_mode"));
            }
        }
    }
    found
}

/// Top-level `include:` and every service `extends:` that names a `file:` -
/// they pull in another Compose file (from the repository or the host) that
/// none of these checks ever read.
pub fn external_files_in_compose(content: &str) -> Vec<String> {
    let Some(document) = parse(content) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    if document.get("include").is_some() {
        found.push("include".to_owned());
    }
    if let Some(services) = document
        .get("services")
        .and_then(serde_yaml::Value::as_mapping)
    {
        for (name, service) in services {
            if service
                .get("extends")
                .and_then(|extends| extends.get("file"))
                .is_some()
            {
                found.push(format!("{}: extends", name.as_str().unwrap_or_default()));
            }
        }
    }
    found
}

/// Reserved names among `names`, in the order given, without duplicates.
pub fn reserved_service_conflicts<'a>(names: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut conflicts: Vec<String> = Vec::new();
    for name in names {
        let name = name.trim().to_ascii_lowercase();
        if RESERVED_SERVICE_NAMES.contains(&name.as_str()) && !conflicts.contains(&name) {
            conflicts.push(name);
        }
    }
    conflicts
}

/// Reserved service names declared by a Compose document. An unparseable file
/// yields none - the deploy and validation paths report the syntax error on
/// their own.
pub fn reserved_services_in_compose(content: &str) -> Vec<String> {
    let Some(document) = parse(content) else {
        return Vec::new();
    };
    let Some(services) = document
        .get("services")
        .and_then(serde_yaml::Value::as_mapping)
    else {
        return Vec::new();
    };
    reserved_service_conflicts(services.keys().filter_map(serde_yaml::Value::as_str))
}

/// User-facing explanation for `conflicts`, which must not be empty.
pub fn reserved_service_message(conflicts: &[String]) -> String {
    let names = conflicts
        .iter()
        .map(|name| format!("'{name}'"))
        .collect::<Vec<_>>()
        .join(", ");
    let subject = if conflicts.len() == 1 {
        "service name is"
    } else {
        "service names are"
    };
    format!(
        "Compose {subject} reserved by litepod's own containers on the shared network: {names}. \
         Rename the service, for example 'web' instead of 'app'."
    )
}

/// `service: mount` for every volume mount that is not a named volume declared
/// under top-level `volumes:` - a host path (`./data`, `/srv/x`, `~/x`,
/// `${DIR}`), a long-syntax `type: bind`, an anonymous volume (`- /data`), or
/// a name nothing declares.
///
/// Litepod runs containers on rootless Podman. A named volume is created and
/// owned inside the user namespace, so the container's uid can write to it; a
/// host directory keeps the host's ownership, which maps to `nobody` inside
/// the container and fails with permission errors. Named volumes also survive
/// a redeploy, which an anonymous one may not. `tmpfs` holds no data and is
/// left alone. An unparseable file yields none, like
/// `reserved_services_in_compose`.
pub fn unnamed_mounts_in_compose(content: &str) -> Vec<String> {
    let Some(document) = parse(content) else {
        return Vec::new();
    };
    let declared: Vec<&str> = document
        .get("volumes")
        .and_then(serde_yaml::Value::as_mapping)
        .map(|volumes| {
            volumes
                .keys()
                .filter_map(serde_yaml::Value::as_str)
                .collect()
        })
        .unwrap_or_default();
    let Some(services) = document
        .get("services")
        .and_then(serde_yaml::Value::as_mapping)
    else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for (name, service) in services {
        let Some(name) = name.as_str() else {
            continue;
        };
        let Some(volumes) = service
            .get("volumes")
            .and_then(serde_yaml::Value::as_sequence)
        else {
            continue;
        };
        for entry in volumes {
            let source = match entry {
                serde_yaml::Value::String(short) => match short.split_once(':') {
                    Some((source, _)) => source.to_owned(),
                    None => {
                        found.push(format!("{name}: {short} (anonymous)"));
                        continue;
                    }
                },
                serde_yaml::Value::Mapping(long) => {
                    let field = |key: &str| {
                        long.get(serde_yaml::Value::String(key.into()))
                            .and_then(serde_yaml::Value::as_str)
                            .unwrap_or_default()
                            .to_owned()
                    };
                    match field("type").as_str() {
                        "tmpfs" => continue,
                        "volume" if !field("source").is_empty() => field("source"),
                        "volume" => {
                            found.push(format!("{name}: {} (anonymous)", field("target")));
                            continue;
                        }
                        _ => {
                            found.push(format!("{name}: {}", field("source")));
                            continue;
                        }
                    }
                }
                _ => continue,
            };
            if !declared.contains(&source.as_str()) {
                found.push(format!("{name}: {source}"));
            }
        }
    }
    found
}

/// User-facing explanation for `mounts`, which must not be empty.
pub fn unnamed_mounts_message(mounts: &[String]) -> String {
    format!(
        "Rootless Podman needs named volumes: mount `app_data:/data` and declare `app_data:` \
         under top-level `volumes:`. Host paths, binds and anonymous volumes are refused: {}",
        mounts.join(", ")
    )
}

/// Every service name that sets `privileged: true`. A privileged container on
/// the shared `local` server gains every capability and device inside the
/// runtime, so only the instance SysAdmin may deploy one; an Editor in any
/// organization must be refused before the file is persisted.
pub fn privileged_services_in_compose(content: &str) -> Vec<String> {
    let Some(document) = parse(content) else {
        return Vec::new();
    };
    let Some(services) = document
        .get("services")
        .and_then(serde_yaml::Value::as_mapping)
    else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for (name, service) in services {
        let Some(name) = name.as_str() else {
            continue;
        };
        let privileged = service
            .get("privileged")
            .and_then(serde_yaml::Value::as_bool)
            .unwrap_or(false);
        if privileged {
            found.push(name.to_owned());
        }
    }
    found
}

/// User-facing explanation for `services`, which must not be empty.
pub fn privileged_services_message(services: &[String]) -> String {
    let names = services
        .iter()
        .map(|name| format!("'{name}'"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "Privileged containers are refused on the shared server: {names}. \
         Remove `privileged: true` from the service."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_named_volumes_and_tmpfs_pass() {
        let content = "services:\n  kuma:\n    volumes:\n      - kuma_data:/app/data\n      - type: volume\n        source: other\n        target: /x\n      - type: tmpfs\n        target: /tmp\nvolumes:\n  kuma_data:\n  other:\n";
        assert!(unnamed_mounts_in_compose(content).is_empty());
    }

    #[test]
    fn everything_else_is_refused() {
        let content = "services:\n  web:\n    volumes:\n      - ./data:/data\n      - /srv/x:/x:ro\n      - ${DIR}:/y\n      - /cache\n      - undeclared:/u\n      - type: bind\n        source: /etc/z\n        target: /z\n      - type: volume\n        target: /anon\nvolumes:\n  used:\n";
        assert_eq!(
            unnamed_mounts_in_compose(content),
            vec![
                "web: ./data",
                "web: /srv/x",
                "web: ${DIR}",
                "web: /cache (anonymous)",
                "web: undeclared",
                "web: /etc/z",
                "web: /anon (anonymous)",
            ]
        );
    }

    #[test]
    fn flags_reserved_services_case_insensitively() {
        let content = "services:\n  LibSQL:\n    image: x\n  web:\n    image: y\n";
        assert_eq!(reserved_services_in_compose(content), vec!["libsql"]);
    }

    #[test]
    fn allows_a_file_without_reserved_names() {
        let content = "services:\n  web:\n    image: x\n  postgres:\n    image: y\n";
        assert!(reserved_services_in_compose(content).is_empty());
    }

    #[test]
    fn reports_every_conflict_once() {
        let content = "services:\n  app:\n    image: x\n  caddy:\n    image: y\n";
        let conflicts = reserved_services_in_compose(content);
        assert_eq!(conflicts, vec!["app", "caddy"]);
        let message = reserved_service_message(&conflicts);
        assert!(message.contains("'app'"), "{message}");
        assert!(message.contains("'caddy'"), "{message}");
        assert!(message.contains("service names are"), "{message}");
    }

    #[test]
    fn ignores_an_unparseable_file() {
        assert!(reserved_services_in_compose(": : :").is_empty());
    }

    #[test]
    fn flags_privileged_services() {
        let content =
            "services:\n  web:\n    image: x\n    privileged: true\n  db:\n    image: y\n";
        assert_eq!(privileged_services_in_compose(content), vec!["web"]);
        assert_eq!(
            privileged_services_in_compose("services:\n  web:\n    image: x\n"),
            Vec::<String>::new()
        );
        // A `privileged: false` and a string `"true"` (not a YAML bool) are ignored.
        let content = "services:\n  web:\n    image: x\n    privileged: false\n  db:\n    image: y\n    privileged: \"true\"\n";
        assert_eq!(
            privileged_services_in_compose(content),
            Vec::<String>::new()
        );
    }

    #[test]
    fn tenant_compose_sees_through_anchors_and_external_files() {
        let merged = "x-base: &base\n  privileged: true\n  volumes: [\"/:/host\"]\nservices:\n  web:\n    <<: *base\n    image: alpine\n";
        assert!(
            validate_tenant_compose(merged, true)
                .unwrap_err()
                .contains("/")
        );
        let privileged = "x-base: &base\n  privileged: true\nservices:\n  web:\n    <<: *base\n    image: alpine\n";
        assert!(validate_tenant_compose(privileged, false).is_err());
        assert!(validate_tenant_compose(privileged, true).is_ok());
        assert!(
            validate_tenant_compose(
                "include: [other.yml]\nservices:\n  web:\n    image: x\n",
                false
            )
            .is_err()
        );
        assert!(
            validate_tenant_compose(
                "services:\n  web:\n    extends:\n      file: evil.yml\n      service: x\n",
                false
            )
            .is_err()
        );
        assert!(
            validate_tenant_compose(
                "services:\n  base:\n    image: x\n  web:\n    extends:\n      service: base\n",
                false
            )
            .is_ok()
        );
        assert!(validate_tenant_compose("services:\n  caddy:\n    image: x\n", true).is_err());
    }

    #[test]
    fn the_control_network_is_refused_for_everyone() {
        for networks in [
            "  cp:\n    external: true\n    name: litepod-control\n",
            "  cp:\n    name: litepod-control\n",
            "  cp:\n    external:\n      name: litepod-control\n",
            "  litepod-control:\n    external: true\n",
        ] {
            let compose = format!(
                "services:\n  web:\n    image: x\n    networks: [cp, litepod-control]\nnetworks:\n{networks}"
            );
            assert!(
                validate_tenant_compose(&compose, true)
                    .unwrap_err()
                    .contains("litepod-control"),
                "{compose}"
            );
        }
        let shared = "services:\n  web:\n    image: x\nnetworks:\n  default:\n    external: true\n    name: litepod-network\n  own: {}\n";
        assert!(control_networks_in_compose(shared).is_empty());
    }

    #[test]
    fn foreign_volumes_are_refused_for_everyone() {
        let bind = "services:\n  web:\n    image: alpine\n    volumes: [hostroot:/host]\nvolumes:\n  hostroot:\n    driver_opts: { type: none, o: bind, device: / }\n";
        assert!(
            validate_tenant_compose(bind, true)
                .unwrap_err()
                .contains("hostroot: driver_opts")
        );
        let control_plane = "services:\n  web:\n    image: alpine\n    volumes: [cp:/cp]\nvolumes:\n  cp:\n    external: true\n    name: litepod_data\n";
        assert_eq!(
            foreign_volumes_in_compose(control_plane),
            vec!["cp: external", "cp: name"]
        );
        assert!(validate_tenant_compose(control_plane, true).is_err());
        let other_driver = "services:\n  web:\n    image: x\n    volumes: [d:/d]\nvolumes:\n  d:\n    driver: nfs\n";
        assert_eq!(foreign_volumes_in_compose(other_driver), vec!["d: driver"]);
        let plain = "services:\n  web:\n    image: x\n    volumes: [a:/a, b:/b]\nvolumes:\n  a:\n  b:\n    driver: local\n";
        assert!(validate_tenant_compose(plain, false).is_ok());
    }

    #[test]
    fn host_access_options_need_the_sysadmin() {
        for option in [
            "cap_add: [SYS_ADMIN]",
            "security_opt: [seccomp=unconfined]",
            "devices: [/dev/sda]",
            "device_cgroup_rules: ['c 1:3 mr']",
            "pid: host",
            "ipc: host",
            "userns_mode: host",
            "cgroup: host",
            "sysctls: { kernel.x: 1 }",
            "volumes_from: [other]",
            "network_mode: host",
            "network_mode: container:litepod-api",
            "network_mode: service:missing",
        ] {
            let content = format!("services:\n  web:\n    image: x\n    {option}\n");
            assert!(
                validate_tenant_compose(&content, false).is_err(),
                "{option}"
            );
            assert!(validate_tenant_compose(&content, true).is_ok(), "{option}");
        }
        let sidecar = "services:\n  vpn:\n    image: x\n  web:\n    image: y\n    network_mode: service:vpn\n  db:\n    image: z\n    network_mode: bridge\n";
        assert!(validate_tenant_compose(sidecar, false).is_ok());
    }

    #[test]
    fn privileged_message_names_every_service() {
        let message = privileged_services_message(&["web".into(), "db".into()]);
        assert!(message.contains("'web'"), "{message}");
        assert!(message.contains("'db'"), "{message}");
    }
}
