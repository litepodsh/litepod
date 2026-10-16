use std::{collections::BTreeMap, ffi::OsString};

pub(super) fn generate_standalone_override(
    runtime_name: &str,
    network: &str,
    services: &[ComposeService],
    proxy_services: &[String],
) -> String {
    let mut yaml = serde_yaml::Mapping::new();
    let mut services_map = serde_yaml::Mapping::new();

    let mut seen: BTreeMap<String, ()> = BTreeMap::new();
    for service in services {
        seen.insert(service.name.clone(), ());
    }
    let proxy_services = proxy_services
        .iter()
        .map(String::as_str)
        .collect::<std::collections::BTreeSet<_>>();

    for (service_name, ()) in &seen {
        let mut service_map = serde_yaml::Mapping::new();
        service_map.insert(
            serde_yaml::Value::String("container_name".into()),
            serde_yaml::Value::String(format!("{runtime_name}-{service_name}")),
        );
        // Only a routed (Caddy-facing) service joins the network every app on
        // this host shares. A service kept off it (a database, a cache, ...)
        // falls back to Compose's own private per-app network, so a service
        // named e.g. "db" here can never resolve to a same-named service in
        // another app - that collision is exactly what put the wrong
        // Postgres behind a customer's Forgejo deploy.
        if proxy_services.contains(service_name.as_str()) {
            let mut networks_map = serde_yaml::Mapping::new();
            let mut network_config = serde_yaml::Mapping::new();
            let mut aliases = serde_yaml::Sequence::new();
            // Caddy routes use a per-application hostname so two Compose apps can
            // both have a service named e.g. "web" on the shared proxy network.
            aliases.push(serde_yaml::Value::String(format!(
                "{runtime_name}-{service_name}"
            )));
            network_config.insert(
                serde_yaml::Value::String("aliases".into()),
                serde_yaml::Value::Sequence(aliases),
            );
            networks_map.insert(
                serde_yaml::Value::String("litepod_proxy".into()),
                serde_yaml::Value::Mapping(network_config),
            );
            service_map.insert(
                serde_yaml::Value::String("networks".into()),
                serde_yaml::Value::Mapping(networks_map),
            );
        }
        services_map.insert(
            serde_yaml::Value::String(service_name.clone()),
            serde_yaml::Value::Mapping(service_map),
        );
    }

    yaml.insert(
        serde_yaml::Value::String("services".into()),
        serde_yaml::Value::Mapping(services_map),
    );

    let mut networks_top = serde_yaml::Mapping::new();
    let mut litepod_proxy = serde_yaml::Mapping::new();
    litepod_proxy.insert(
        serde_yaml::Value::String("external".into()),
        serde_yaml::Value::Bool(true),
    );
    litepod_proxy.insert(
        serde_yaml::Value::String("name".into()),
        serde_yaml::Value::String(network.to_string()),
    );
    networks_top.insert(
        serde_yaml::Value::String("litepod_proxy".into()),
        serde_yaml::Value::Mapping(litepod_proxy),
    );
    yaml.insert(
        serde_yaml::Value::String("networks".into()),
        serde_yaml::Value::Mapping(networks_top),
    );

    serde_yaml::to_string(&yaml).unwrap_or_default()
}

/// Sanitizes the file sent to a remote server. The base Compose file's own
/// `litepod_proxy` declaration (name/aliases meant for the local network) is
/// stripped here; `generate_remote_standalone_override` re-adds the network
/// with the remote host's own network name.
///
/// A `ports:` block the user wrote directly is left untouched - same as a
/// local deploy, whatever the user's Compose file publishes is what actually
/// gets published on the remote host (Dokploy/Coolify behave the same way).
/// The only thing layered on top is the single loopback endpoint
/// `generate_remote_standalone_override` adds for the service wired to an
/// HTTP(S) domain; Compose concatenates `ports:` lists across `-f` files, so
/// that loopback binding and the user's own public bindings both apply.
pub(crate) fn prepare_remote_compose(content: &str) -> Result<String, String> {
    let mut document: serde_yaml::Value =
        serde_yaml::from_str(content).map_err(|error| format!("Invalid Compose file: {error}"))?;
    let services = document
        .get_mut("services")
        .and_then(serde_yaml::Value::as_mapping_mut)
        .ok_or_else(|| "Invalid Compose file: expected a services mapping".to_string())?;

    for (service_name, config) in services.iter_mut() {
        let name = service_name.as_str().unwrap_or("<unnamed>");
        let config = config
            .as_mapping_mut()
            .ok_or_else(|| format!("Invalid Compose file: service '{name}' is not a mapping"))?;
        if let Some(networks) = config.get_mut(serde_yaml::Value::String("networks".into())) {
            if let Some(mapping) = networks.as_mapping_mut() {
                mapping.remove(serde_yaml::Value::String("litepod_proxy".into()));
                if mapping.is_empty() {
                    config.remove(serde_yaml::Value::String("networks".into()));
                }
            } else if let Some(items) = networks.as_sequence_mut() {
                items.retain(|item| item.as_str() != Some("litepod_proxy"));
                if items.is_empty() {
                    config.remove(serde_yaml::Value::String("networks".into()));
                }
            }
        }
    }
    if let Some(networks) = document
        .get_mut("networks")
        .and_then(serde_yaml::Value::as_mapping_mut)
    {
        networks.remove(serde_yaml::Value::String("litepod_proxy".into()));
        if networks.is_empty() {
            document
                .as_mapping_mut()
                .expect("Compose root was a mapping")
                .remove(serde_yaml::Value::String("networks".into()));
        }
    }
    serde_yaml::to_string(&document)
        .map_err(|error| format!("Could not render remote Compose file: {error}"))
}

/// A remote app never needs the shared `litepod_proxy` network: no ingress
/// mode ever dials a remote container by network alias. PublicEdge installs
/// a native Caddy on the remote host itself that proxies to
/// `127.0.0.1:{endpoint_port}`; PrivateRelay SSH-forwards that same loopback
/// port to `litepod-api`, which central Caddy then dials - the remote app
/// container's network membership is never touched either way. So every
/// service here gets Compose's own private per-app network and nothing
/// else, same as `git::compose::remote_override_yaml`'s already-proven
/// pattern: a service named e.g. "db" can never collide with another app's
/// identically-named service, because there's no shared network left to
/// collide on. Any number of services can be additionally published here -
/// one loopback binding per distinct persisted proxy-route target, so a
/// remote app can expose as many services as it has distinct container
/// ports, each behind its own public domain. This is on top of, not instead
/// of, whatever
/// the user's own Compose file already publishes (`prepare_remote_compose`
/// leaves that untouched) and whatever `port-overrides.yml` adds
/// (applied alongside this one in `compose_cmd`, e.g. for internal DB
/// templates); Compose concatenates `ports:` across `-f` files, so all three
/// sources coexist on the running container.
pub(super) fn generate_remote_standalone_override(
    runtime_name: &str,
    services: &[ComposeService],
    endpoints: &[(&str, u16, u16)],
) -> Result<String, String> {
    for (service, _, _) in endpoints {
        if !services.iter().any(|candidate| candidate.name == *service) {
            return Err(format!(
                "Configured remote ingress service '{service}' is not in the Compose file"
            ));
        }
    }
    let mut yaml = serde_yaml::Mapping::new();
    let mut services_map = serde_yaml::Mapping::new();
    for service in services {
        let mut service_map = serde_yaml::Mapping::new();
        service_map.insert(
            serde_yaml::Value::String("container_name".into()),
            serde_yaml::Value::String(format!("{runtime_name}-{}", service.name)),
        );
        let ports = endpoints
            .iter()
            .filter(|(selected, _, _)| service.name == *selected)
            .map(|(_, host_port, container_port)| {
                let host = if *host_port == 0 {
                    String::new()
                } else {
                    host_port.to_string()
                };
                serde_yaml::Value::String(format!("127.0.0.1:{host}:{container_port}/tcp"))
            })
            .collect::<serde_yaml::Sequence>();
        if !ports.is_empty() {
            service_map.insert("ports".into(), serde_yaml::Value::Sequence(ports));
        }
        services_map.insert(
            serde_yaml::Value::String(service.name.clone()),
            serde_yaml::Value::Mapping(service_map),
        );
    }
    yaml.insert(
        serde_yaml::Value::String("services".into()),
        serde_yaml::Value::Mapping(services_map),
    );

    serde_yaml::to_string(&yaml)
        .map_err(|error| format!("Could not render remote ingress override: {error}"))
}

/// Compose has no per-project resource budget, so every service in the file
/// carries the same cap - matching how the port and volume overrides work.
pub(super) fn generate_runtime_resources_override(
    services: &[ComposeService],
    limits: &super::super::ResourceLimits,
) -> String {
    let mut yaml = serde_yaml::Mapping::new();
    let mut services_map = serde_yaml::Mapping::new();

    let cpus = (limits.cpu_quota_us as f64 / super::super::BUILD_CPU_PERIOD_US as f64).to_string();
    let memory = limits.memory_bytes.to_string();

    for service in services {
        let mut service_map = serde_yaml::Mapping::new();
        let mut deploy_map = serde_yaml::Mapping::new();
        let mut resources_map = serde_yaml::Mapping::new();
        let mut limits_map = serde_yaml::Mapping::new();

        limits_map.insert(
            serde_yaml::Value::String("cpus".into()),
            serde_yaml::Value::String(cpus.clone()),
        );
        limits_map.insert(
            serde_yaml::Value::String("memory".into()),
            serde_yaml::Value::String(memory.clone()),
        );
        resources_map.insert(
            serde_yaml::Value::String("limits".into()),
            serde_yaml::Value::Mapping(limits_map),
        );
        deploy_map.insert(
            serde_yaml::Value::String("resources".into()),
            serde_yaml::Value::Mapping(resources_map),
        );
        service_map.insert(
            serde_yaml::Value::String("deploy".into()),
            serde_yaml::Value::Mapping(deploy_map),
        );
        services_map.insert(
            serde_yaml::Value::String(service.name.clone()),
            serde_yaml::Value::Mapping(service_map),
        );
    }

    yaml.insert(
        serde_yaml::Value::String("services".into()),
        serde_yaml::Value::Mapping(services_map),
    );

    serde_yaml::to_string(&yaml).unwrap_or_default()
}

/// A service discovered in the user's Compose file.
pub(super) struct ComposeService {
    pub(super) name: String,
}

pub(super) fn compose_services(content: &str) -> Result<Vec<ComposeService>, String> {
    let document: serde_yaml::Value =
        serde_yaml::from_str(content).map_err(|error| format!("Invalid Compose file: {error}"))?;
    let services = document
        .get("services")
        .and_then(serde_yaml::Value::as_mapping)
        .ok_or_else(|| "Invalid Compose file: expected a services mapping".to_string())?;
    Ok(services
        .iter()
        .filter_map(|(name, _config)| {
            let name = name.as_str()?;
            Some(ComposeService {
                name: name.to_owned(),
            })
        })
        .collect())
}

/// Named (top-level-declared) volumes mounted by a service whose image tag
/// contains `-rootless`. Only the short `name:/path` syntax is recognized —
/// every `-rootless` template in the catalog today uses it; the long
/// `source:`/`target:` mapping form can be added if one shows up that needs it.
pub(super) fn rootless_named_volumes(content: &str) -> Vec<String> {
    let Ok(document) = serde_yaml::from_str::<serde_yaml::Value>(content) else {
        return Vec::new();
    };
    let declared: std::collections::BTreeSet<&str> = document
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
    let mut names = std::collections::BTreeSet::new();
    for (_, service) in services {
        let Some(service_map) = service.as_mapping() else {
            continue;
        };
        let is_rootless = service_map
            .get(serde_yaml::Value::String("image".into()))
            .and_then(serde_yaml::Value::as_str)
            .is_some_and(|image| image.contains("-rootless"));
        if !is_rootless {
            continue;
        }
        let Some(volumes) = service_map
            .get(serde_yaml::Value::String("volumes".into()))
            .and_then(serde_yaml::Value::as_sequence)
        else {
            continue;
        };
        for entry in volumes {
            let Some(name) = entry.as_str().and_then(|text| text.split(':').next()) else {
                continue;
            };
            if declared.contains(name) {
                names.insert(name);
            }
        }
    }
    names.into_iter().map(str::to_owned).collect()
}

/// Some `-rootless` images (Forgejo's is a confirmed case — see
/// codeberg.org/forgejo/forgejo/issues/4269) can leave a file in a freshly
/// created named volume owned by the raw host uid instead of the container's
/// namespace-mapped one, so the container's own next boot gets a permission
/// error trying to rewrite it. This is a known, unresolved upstream quirk,
/// not something Compose or litepod caused — the fix the affected projects'
/// own issue trackers converge on is exactly this: `podman unshare chown`
/// the volume back to the uid the container expects.
///
/// Best-effort and idempotent: a volume that's already correctly owned is
/// untouched, and any failure here (inspect, chown, an unreachable runtime)
/// is swallowed rather than failing the deploy — this is a defensive pass
/// for a known quirk, not something that should block an "up" that would
/// otherwise have succeeded.
///
/// ponytail: hardcodes uid:gid 1000:1000, the convention every `-rootless`
/// image in the catalog uses today. Parameterize per-template if one shows
/// up using a different internal uid.
pub(super) async fn chown_rootless_volumes(
    runtime: &crate::servers::runtime::ServerRuntime,
    project: &str,
    content: &str,
) {
    const ROOTLESS_UID_GID: &str = "1000:1000";
    for volume in rootless_named_volumes(content) {
        let podman_volume = format!("{project}_{volume}");
        let Ok(inspected) = runtime
            .podman(&[
                OsString::from("volume"),
                OsString::from("inspect"),
                OsString::from(&podman_volume),
                OsString::from("--format"),
                OsString::from("{{.Mountpoint}}"),
            ])
            .await
        else {
            continue;
        };
        let mountpoint = inspected.stdout.trim();
        if mountpoint.is_empty() {
            continue;
        }
        let _ = runtime
            .podman(&[
                OsString::from("unshare"),
                OsString::from("chown"),
                OsString::from("-R"),
                OsString::from(ROOTLESS_UID_GID),
                OsString::from(mountpoint),
            ])
            .await;
    }
}

/// The standalone override has to name `litepod_proxy` in long syntax to attach
/// its alias, and podman-compose refuses to merge that mapping with a service
/// that spelled its networks as a list. Rewriting the list to the equivalent
/// mapping keeps both shapes accepted in the file the user edits.
/// Returns `None` when nothing had to change, so a Compose file that is already
/// consistent is handed to the provider untouched.
pub(super) fn normalize_compose_networks(
    content: &str,
    application_id: &str,
) -> Result<Option<String>, String> {
    let mut document: serde_yaml::Value =
        serde_yaml::from_str(content).map_err(|error| format!("Invalid Compose file: {error}"))?;
    let mut changed = false;
    let mut private_network_used = false;
    {
        let services = document
            .get_mut("services")
            .and_then(serde_yaml::Value::as_mapping_mut)
            .ok_or_else(|| "Invalid Compose file: expected a services mapping".to_string())?;
        for (name, config) in services.iter_mut() {
            let service_name = name.as_str().unwrap_or("<unnamed>").to_string();
            let config = config.as_mapping_mut().ok_or_else(|| {
                format!("Invalid Compose file: service '{service_name}' is not a mapping")
            })?;
            if config.contains_key(serde_yaml::Value::String("network_mode".into())) {
                continue;
            }

            let networks_key = serde_yaml::Value::String("networks".into());
            match config.get_mut(&networks_key) {
                None | Some(serde_yaml::Value::Null) => {
                    let mut networks = serde_yaml::Mapping::new();
                    networks.insert(
                        serde_yaml::Value::String("private".into()),
                        serde_yaml::Value::Mapping(serde_yaml::Mapping::new()),
                    );
                    config.insert(networks_key, serde_yaml::Value::Mapping(networks));
                    private_network_used = true;
                    changed = true;
                }
                Some(serde_yaml::Value::Sequence(items)) => {
                    let mut networks = serde_yaml::Mapping::new();
                    for entry in items.iter() {
                        let network = entry.as_str().ok_or_else(|| {
                            format!(
                                "Invalid Compose file: service '{service_name}' has an unsupported networks entry"
                            )
                        })?;
                        networks.insert(
                            serde_yaml::Value::String(network.to_owned()),
                            serde_yaml::Value::Mapping(serde_yaml::Mapping::new()),
                        );
                    }
                    config.insert(networks_key, serde_yaml::Value::Mapping(networks));
                    changed = true;
                }
                Some(serde_yaml::Value::Mapping(_)) => {}
                Some(_) => {
                    return Err(format!(
                        "Invalid Compose file: service '{service_name}' networks must be a list or mapping"
                    ));
                }
            }
        }
    }

    if private_network_used {
        let root = document
            .as_mapping_mut()
            .ok_or_else(|| "Invalid Compose file: expected a mapping".to_string())?;
        let networks_key = serde_yaml::Value::String("networks".into());
        if !root.contains_key(&networks_key) {
            root.insert(
                networks_key.clone(),
                serde_yaml::Value::Mapping(serde_yaml::Mapping::new()),
            );
        }
        let networks = root
            .get_mut(&networks_key)
            .and_then(serde_yaml::Value::as_mapping_mut)
            .ok_or_else(|| {
                "Invalid Compose file: top-level networks must be a mapping".to_string()
            })?;
        let private_key = serde_yaml::Value::String("private".into());
        if !networks.contains_key(&private_key) {
            let mut private_def = serde_yaml::Mapping::new();
            private_def.insert(
                serde_yaml::Value::String("name".into()),
                serde_yaml::Value::String(format!("{application_id}_private")),
            );
            networks.insert(private_key, serde_yaml::Value::Mapping(private_def));
        }
    }
    if !changed {
        return Ok(None);
    }
    serde_yaml::to_string(&document)
        .map(Some)
        .map_err(|error| format!("Could not rewrite the Compose file networks: {error}"))
}

/// The Compose service that exposes `port` inside the container, or `None` when
/// no service claims it. Accepts every shape Compose allows for `ports` (short
/// `"3000"`, `"3000:3000"`, `"0.0.0.0:8080:3000"`, `"3000:3000/tcp"`, and the
/// long `{ target: 3000 }` mapping) plus `expose`.
pub(crate) fn service_for_port(compose_content: &str, port: u16) -> Option<String> {
    let document: serde_yaml::Value = serde_yaml::from_str(compose_content).ok()?;
    let services = document.get("services")?.as_mapping()?;
    for (name, config) in services {
        let Some(name) = name.as_str() else {
            continue;
        };
        if service_container_ports(config).contains(&port) {
            return Some(name.to_owned());
        }
    }
    None
}

/// Every Compose service exposing `port`, in file order. Domain markers need
/// the full list to reject an ambiguous port instead of picking the first.
pub(crate) fn services_for_port(compose_content: &str, port: u16) -> Vec<String> {
    let Ok(document) = serde_yaml::from_str::<serde_yaml::Value>(compose_content) else {
        return Vec::new();
    };
    let Some(services) = document
        .get("services")
        .and_then(serde_yaml::Value::as_mapping)
    else {
        return Vec::new();
    };
    services
        .iter()
        .filter(|(_, config)| service_container_ports(config).contains(&port))
        .filter_map(|(name, _)| name.as_str().map(str::to_owned))
        .collect()
}

pub(crate) fn service_exposes_port(compose_content: &str, service: &str, port: u16) -> bool {
    serde_yaml::from_str::<serde_yaml::Value>(compose_content)
        .ok()
        .and_then(|document| document.get("services")?.get(service).cloned())
        .is_some_and(|config| service_container_ports(&config).contains(&port))
}

pub(super) fn service_container_ports(config: &serde_yaml::Value) -> Vec<u16> {
    let mut ports = Vec::new();
    if let Some(entries) = config.get("ports").and_then(serde_yaml::Value::as_sequence) {
        for entry in entries {
            if let Some(mapping) = entry.as_mapping() {
                // Long syntax: the container-side port is `target`.
                if let Some(target) = mapping
                    .get(serde_yaml::Value::String("target".into()))
                    .and_then(parse_port)
                {
                    ports.push(target);
                }
            } else if let Some(port) = short_syntax_container_port(entry) {
                ports.push(port);
            }
        }
    }
    if let Some(entries) = config
        .get("expose")
        .and_then(serde_yaml::Value::as_sequence)
    {
        ports.extend(entries.iter().filter_map(parse_port));
    }
    ports
}

/// The container-side port of a short-syntax entry: the last colon-separated
/// segment, minus any `/tcp` suffix. Ranges (`3000-3005`) are ignored - a
/// primary port is a single port.
pub(super) fn short_syntax_container_port(entry: &serde_yaml::Value) -> Option<u16> {
    let raw = match entry {
        serde_yaml::Value::Number(number) => number.as_u64()?.to_string(),
        serde_yaml::Value::String(text) => text.clone(),
        _ => return None,
    };
    raw.rsplit(':')
        .next()?
        .split('/')
        .next()?
        .trim()
        .parse()
        .ok()
}

pub(super) fn parse_port(value: &serde_yaml::Value) -> Option<u16> {
    match value {
        serde_yaml::Value::Number(number) => u16::try_from(number.as_u64()?).ok(),
        serde_yaml::Value::String(text) => text.split('/').next()?.trim().parse().ok(),
        _ => None,
    }
}

pub(super) fn first_service_name(compose_content: &str) -> Option<String> {
    let value: serde_yaml::Value = serde_yaml::from_str(compose_content).ok()?;
    value
        .get("services")?
        .as_mapping()?
        .keys()
        .next()
        .and_then(serde_yaml::Value::as_str)
        .map(str::to_owned)
}
