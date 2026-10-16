use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::dtos::deployment::{
    ApplicationNetworkAttachmentDto, ApplicationNetworkContainerDto, ApplicationNetworkDto,
    ApplicationNetworkFindingDto,
};

#[derive(Debug, Clone)]
pub struct ObservedContainer {
    pub name: String,
    pub service: Option<String>,
    pub networks: Vec<ApplicationNetworkAttachmentDto>,
}

#[derive(Debug, Clone, Default)]
pub struct ObservedNetwork {
    pub driver: Option<String>,
    pub member_names: BTreeSet<String>,
}

pub fn container_networks(inspect: &Value) -> Vec<ApplicationNetworkAttachmentDto> {
    let networks = inspect
        .get("NetworkSettings")
        .or_else(|| inspect.get("network_settings"))
        .and_then(|value| value.get("Networks").or_else(|| value.get("networks")))
        .and_then(Value::as_object);
    let mut result = networks
        .into_iter()
        .flat_map(|networks| networks.iter())
        .map(|(name, value)| {
            let mut aliases = value
                .get("Aliases")
                .or_else(|| value.get("aliases"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect::<Vec<_>>();
            aliases.sort();
            aliases.dedup();
            ApplicationNetworkAttachmentDto {
                name: name.clone(),
                ip_address: value
                    .get("IPAddress")
                    .or_else(|| value.get("ip_address"))
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned),
                aliases,
            }
        })
        .collect::<Vec<_>>();
    result.sort_by(|left, right| left.name.cmp(&right.name));
    result
}

pub fn network_members(inspect: &Value) -> ObservedNetwork {
    let driver = inspect
        .get("Driver")
        .or_else(|| inspect.get("driver"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let member_names = inspect
        .get("Containers")
        .or_else(|| inspect.get("containers"))
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|members| members.values())
        .filter_map(|member| {
            member
                .get("Name")
                .or_else(|| member.get("name"))
                .and_then(Value::as_str)
        })
        .map(str::to_owned)
        .collect();
    ObservedNetwork {
        driver,
        member_names,
    }
}

pub fn summarize(
    mut containers: Vec<ObservedContainer>,
    inspected_networks: BTreeMap<String, ObservedNetwork>,
    proxy_services: &BTreeSet<String>,
    proxy_network: &str,
    remote: bool,
    is_compose: bool,
) -> (
    Vec<ApplicationNetworkContainerDto>,
    Vec<ApplicationNetworkDto>,
    Vec<ApplicationNetworkFindingDto>,
) {
    for container in &mut containers {
        container
            .networks
            .sort_by(|left, right| left.name.cmp(&right.name));
    }
    containers.sort_by(|left, right| left.name.cmp(&right.name));
    let owned_names = containers
        .iter()
        .map(|container| container.name.clone())
        .collect::<BTreeSet<_>>();
    let mut networks = BTreeMap::<String, ApplicationNetworkDto>::new();
    for container in &containers {
        for attachment in &container.networks {
            let inspected = inspected_networks.get(&attachment.name);
            let members =
                inspected.map_or_else(BTreeSet::new, |network| network.member_names.clone());
            let application_member_count = members.intersection(&owned_names).count();
            let external_member_count = members.difference(&owned_names).count();
            networks
                .entry(attachment.name.clone())
                .or_insert_with(|| ApplicationNetworkDto {
                    name: attachment.name.clone(),
                    driver: inspected.and_then(|network| network.driver.clone()),
                    scope: if attachment.name == proxy_network {
                        "shared"
                    } else {
                        "private"
                    }
                    .into(),
                    application_member_count,
                    external_member_count,
                });
        }
    }

    let mut findings = Vec::new();
    let proxy_attached = containers
        .iter()
        .filter(|container| {
            container
                .networks
                .iter()
                .any(|network| network.name == proxy_network)
        })
        .collect::<Vec<_>>();
    if remote {
        for container in &proxy_attached {
            findings.push(finding_on(
                "remote_shared_network",
                "warning",
                &format!(
                    "Container \"{}\" is attached to Litepod's shared proxy network. Remote ingress should use its loopback endpoint instead.",
                    container.name
                ),
                &container.name,
                proxy_network,
            ));
        }
    } else {
        for container in &proxy_attached {
            if container
                .service
                .as_ref()
                .is_none_or(|service| !proxy_services.contains(service))
            {
                findings.push(finding_on(
                    "unexpected_proxy_membership",
                    "warning",
                    &format!(
                        "Container \"{}\" is attached to Litepod's shared proxy network without a configured proxy route.",
                        container.name
                    ),
                    &container.name,
                    proxy_network,
                ));
            }
        }
    }
    if is_compose
        && containers.len() > 1
        && !networks
            .values()
            .any(|network| network.scope == "private" && network.application_member_count >= 2)
    {
        findings.push(finding("missing_private_compose_network", "warning", "Compose services do not share a private network, so service-to-service routing may fail."));
    }
    if let Some(network) = networks
        .get(proxy_network)
        .filter(|network| network.external_member_count > 0)
    {
        findings.push(finding(
            "shared_network_external_members",
            "info",
            &format!(
                "Litepod's shared proxy network has {} container(s) outside this application.",
                network.external_member_count
            ),
        ));
    }
    findings.sort_by(|left, right| left.code.cmp(&right.code));
    (
        containers
            .into_iter()
            .map(|container| ApplicationNetworkContainerDto {
                name: container.name,
                service: container.service,
                networks: container.networks,
            })
            .collect(),
        networks.into_values().collect(),
        findings,
    )
}

/// Parse a `podman stats` size string ("1.234kB", "12B", "5.6MiB") to bytes.
/// Podman prints SI units in `stats`; a few binary suffixes are tolerated too.
pub fn parse_size(text: &str) -> u64 {
    let text = text.trim();
    let suffixes: [(&str, f64); 8] = [
        ("GiB", 1_073_741_824.0),
        ("MiB", 1_048_576.0),
        ("KiB", 1_024.0),
        ("GB", 1_000_000_000.0),
        ("MB", 1_000_000.0),
        ("kB", 1_000.0),
        ("KB", 1_000.0),
        ("B", 1.0),
    ];
    for (suffix, mult) in suffixes {
        if let Some(number) = text.strip_suffix(suffix) {
            return number
                .trim()
                .parse::<f64>()
                .map(|value| (value * mult) as u64)
                .unwrap_or(0);
        }
    }
    text.parse::<f64>().map(|value| value as u64).unwrap_or(0)
}

/// Split a `podman stats` NetIO field ("1.2kB / 3.4kB") into (rx, tx) bytes.
pub fn parse_net_io(text: &str) -> (u64, u64) {
    let mut parts = text.split('/');
    let rx = parts.next().map(parse_size).unwrap_or(0);
    let tx = parts.next().map(parse_size).unwrap_or(0);
    (rx, tx)
}

/// The probe script (see `handler::application_network_probe`) always exits 0
/// and prints one token. Map that token to a stable reason string.
pub fn classify_probe(stdout: &str) -> &'static str {
    let line = stdout.trim();
    if line.starts_with("PORTOK") {
        "reachable"
    } else if line.starts_with("PORTERR") {
        if line.to_ascii_lowercase().contains("refused") {
            "connection_refused"
        } else {
            "unreachable"
        }
    } else if line == "DNSOK" {
        "resolved"
    } else if line == "DNSERR" {
        "name_not_resolved"
    } else if line == "NOTOOLS" {
        "no_tools"
    } else {
        "unknown"
    }
}

fn finding(code: &str, severity: &str, message: &str) -> ApplicationNetworkFindingDto {
    ApplicationNetworkFindingDto {
        code: code.into(),
        severity: severity.into(),
        message: message.into(),
        container: None,
        network: None,
    }
}

fn finding_on(
    code: &str,
    severity: &str,
    message: &str,
    container: &str,
    network: &str,
) -> ApplicationNetworkFindingDto {
    ApplicationNetworkFindingDto {
        code: code.into(),
        severity: severity.into(),
        message: message.into(),
        container: Some(container.into()),
        network: Some(network.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn container(name: &str, service: &str, networks: &[&str]) -> ObservedContainer {
        ObservedContainer {
            name: name.into(),
            service: Some(service.into()),
            networks: networks
                .iter()
                .map(|name| ApplicationNetworkAttachmentDto {
                    name: (*name).into(),
                    ip_address: None,
                    aliases: vec![],
                })
                .collect(),
        }
    }

    fn members(names: &[&str]) -> ObservedNetwork {
        ObservedNetwork {
            driver: Some("bridge".into()),
            member_names: names.iter().map(|name| (*name).into()).collect(),
        }
    }

    #[test]
    fn normal_compose_network_has_no_findings() {
        let (containers, networks, findings) = summarize(
            vec![
                container("web", "web", &["app_default"]),
                container("db", "db", &["app_default"]),
            ],
            BTreeMap::from([("app_default".into(), members(&["web", "db"]))]),
            &BTreeSet::new(),
            "litepod_proxy",
            false,
            true,
        );
        assert_eq!(containers.len(), 2);
        assert_eq!(networks[0].application_member_count, 2);
        assert!(findings.is_empty());
    }

    #[test]
    fn proxy_findings_cover_local_and_remote() {
        let routed = BTreeSet::from(["web".into()]);
        let (_, _, local_ok) = summarize(
            vec![container("web", "web", &["litepod_proxy"])],
            BTreeMap::from([("litepod_proxy".into(), members(&["web"]))]),
            &routed,
            "litepod_proxy",
            false,
            true,
        );
        assert!(local_ok.is_empty());
        let (_, _, local_bad) = summarize(
            vec![container("db", "db", &["litepod_proxy"])],
            BTreeMap::new(),
            &routed,
            "litepod_proxy",
            false,
            true,
        );
        assert_eq!(local_bad[0].code, "unexpected_proxy_membership");
        assert_eq!(local_bad[0].container.as_deref(), Some("db"));
        assert_eq!(local_bad[0].network.as_deref(), Some("litepod_proxy"));
        let (_, _, remote) = summarize(
            vec![container("web", "web", &["litepod_proxy"])],
            BTreeMap::new(),
            &routed,
            "litepod_proxy",
            true,
            true,
        );
        assert_eq!(remote[0].code, "remote_shared_network");
    }

    #[test]
    fn missing_shared_private_network_is_reported() {
        let (_, _, findings) = summarize(
            vec![
                container("web", "web", &["web_net"]),
                container("db", "db", &["db_net"]),
            ],
            BTreeMap::new(),
            &BTreeSet::new(),
            "litepod_proxy",
            false,
            true,
        );
        assert_eq!(findings[0].code, "missing_private_compose_network");
    }

    #[test]
    fn foreign_member_is_counted_but_never_returned() {
        let (_, networks, findings) = summarize(
            vec![container("web", "web", &["litepod_proxy"])],
            BTreeMap::from([("litepod_proxy".into(), members(&["web", "other-app-db"]))]),
            &BTreeSet::from(["web".into()]),
            "litepod_proxy",
            false,
            false,
        );
        assert_eq!(networks[0].external_member_count, 1);
        assert_eq!(findings[0].code, "shared_network_external_members");
        assert!(!format!("{networks:?}{findings:?}").contains("other-app-db"));
    }

    #[test]
    fn parse_size_and_net_io() {
        assert_eq!(parse_size("1.234kB"), 1234);
        assert_eq!(parse_size("12B"), 12);
        assert_eq!(parse_size("5MB"), 5_000_000);
        assert_eq!(parse_size("1MiB"), 1_048_576);
        assert_eq!(parse_size("--"), 0);
        assert_eq!(parse_net_io("1.2kB / 3.4kB"), (1200, 3400));
        assert_eq!(parse_net_io("0B / 0B"), (0, 0));
    }

    #[test]
    fn classify_probe_maps_every_token() {
        assert_eq!(classify_probe("PORTOK\n"), "reachable");
        assert_eq!(
            classify_probe("PORTERR:nc: connect to db port 5432 (tcp) failed: Connection refused"),
            "connection_refused"
        );
        assert_eq!(classify_probe("PORTERR:timed out"), "unreachable");
        assert_eq!(classify_probe("DNSOK"), "resolved");
        assert_eq!(classify_probe("DNSERR"), "name_not_resolved");
        assert_eq!(classify_probe("NOTOOLS"), "no_tools");
        assert_eq!(classify_probe(""), "unknown");
    }

    #[test]
    fn parser_handles_podman_and_docker_casing() {
        let networks = container_networks(
            &json!({"NetworkSettings":{"Networks":{"private":{"IPAddress":"10.0.0.2","Aliases":["web", "web"]}}}}),
        );
        assert_eq!(networks[0].aliases, ["web"]);
        let members =
            network_members(&json!({"driver":"bridge","containers":{"id":{"name":"web"}}}));
        assert!(members.member_names.contains("web"));
    }
}
