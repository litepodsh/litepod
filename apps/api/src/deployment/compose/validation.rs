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
    let Ok(document) = serde_yaml::from_str::<serde_yaml::Value>(content) else {
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
    let Ok(document) = serde_yaml::from_str::<serde_yaml::Value>(content) else {
        return Vec::new();
    };
    let declared: Vec<&str> = document
        .get("volumes")
        .and_then(serde_yaml::Value::as_mapping)
        .map(|volumes| volumes.keys().filter_map(serde_yaml::Value::as_str).collect())
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
}
