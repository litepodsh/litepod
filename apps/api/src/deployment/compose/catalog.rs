// Port-allocation helpers for the standalone Compose catalog path. The wiring
// is staged but not yet wired into the request handler.
#![allow(dead_code)]

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::dtos::response::AppError;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PortSpan {
    pub start: u16,
    pub end: u16,
}

impl PortSpan {
    fn parse(value: &str) -> Result<Self, String> {
        let (start, end) = value
            .split_once('-')
            .map_or((value, value), |(start, end)| (start, end));
        let start = start
            .parse::<u16>()
            .map_err(|_| format!("Invalid port '{start}'"))?;
        let end = end
            .parse::<u16>()
            .map_err(|_| format!("Invalid port '{end}'"))?;
        if start == 0 || end == 0 || start > end {
            return Err(format!("Invalid port range '{value}'"));
        }
        Ok(Self { start, end })
    }
    pub fn display(&self) -> String {
        if self.start == self.end {
            self.start.to_string()
        } else {
            format!("{}-{}", self.start, self.end)
        }
    }
    pub fn overlaps(&self, other: &Self) -> bool {
        self.start <= other.end && other.start <= self.end
    }
    pub fn len(&self) -> u32 {
        self.end as u32 - self.start as u32
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DirectExposure {
    pub service: String,
    pub host_ip: Option<String>,
    pub published: PortSpan,
    pub target: PortSpan,
    pub protocol: String,
    pub legacy: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ComposeServiceCatalog {
    pub name: String,
    pub suggested_internal_ports: Vec<u16>,
    pub direct_exposures: Vec<DirectExposure>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ComposeCatalog {
    pub services: Vec<ComposeServiceCatalog>,
}

#[derive(Debug, Clone)]
pub struct OccupiedPort {
    pub owner: String,
    pub host_ip: Option<String>,
    pub published: PortSpan,
    pub protocol: String,
}

fn protocol(value: Option<&str>) -> Result<String, String> {
    let value = value.unwrap_or("tcp").to_ascii_lowercase();
    if matches!(value.as_str(), "tcp" | "udp") {
        Ok(value)
    } else {
        Err(format!("Unsupported port protocol '{value}'"))
    }
}

fn parse_short_port(service: &str, value: &str, legacy: bool) -> Result<DirectExposure, String> {
    let (value, protocol_value) = value
        .rsplit_once('/')
        .map_or((value, None), |(value, protocol)| (value, Some(protocol)));
    let protocol = protocol(protocol_value)?;
    let parts: Vec<_> = value.split(':').collect();
    let (host_ip, published, target) = match parts.as_slice() {
        [target] => (None, *target, *target),
        [published, target] => (None, *published, *target),
        [host_ip, published, target] => (Some((*host_ip).to_string()), *published, *target),
        _ => return Err(format!("Invalid Compose port '{value}'")),
    };
    let published = PortSpan::parse(published)?;
    let target = PortSpan::parse(target)?;
    if published.len() != target.len() {
        return Err(format!(
            "Published and target ranges must have equal sizes in '{value}'"
        ));
    }
    Ok(DirectExposure {
        service: service.into(),
        host_ip,
        published,
        target,
        protocol,
        legacy,
    })
}

fn parse_port(
    service: &str,
    value: &serde_yaml::Value,
    legacy: bool,
) -> Result<DirectExposure, String> {
    if let Some(value) = value.as_str() {
        return parse_short_port(service, value, legacy);
    }
    let map = value
        .as_mapping()
        .ok_or_else(|| "Compose port must be a string or mapping".to_string())?;
    let get = |key: &str| {
        map.get(serde_yaml::Value::String(key.into()))
            .and_then(|value| value.as_str())
            .map(str::to_owned)
            .or_else(|| {
                map.get(serde_yaml::Value::String(key.into()))
                    .and_then(|value| value.as_i64())
                    .map(|value| value.to_string())
            })
    };
    let target = PortSpan::parse(
        &get("target").ok_or_else(|| "Compose port target is required".to_string())?,
    )?;
    let published = PortSpan::parse(&get("published").unwrap_or_else(|| target.display()))?;
    if published.len() != target.len() {
        return Err("Published and target ranges must have equal sizes".into());
    }
    Ok(DirectExposure {
        service: service.into(),
        host_ip: get("host_ip"),
        published,
        target,
        protocol: protocol(get("protocol").as_deref())?,
        legacy,
    })
}

/// Whether a `ports:` entry interpolates a variable, e.g. `"${PORT}:3000"`.
///
/// Podman resolves these at deploy time, so the published port is unknowable
/// here and the entry has to be tolerated rather than rejected.
fn port_entry_has_variable(value: &serde_yaml::Value) -> bool {
    match value {
        serde_yaml::Value::String(value) => value.contains('$'),
        serde_yaml::Value::Mapping(map) => map.values().any(port_entry_has_variable),
        _ => false,
    }
}

pub fn catalog_from_yaml(content: &str, legacy: bool) -> Result<ComposeCatalog, AppError> {
    let value: serde_yaml::Value = serde_yaml::from_str(content).map_err(|error| {
        AppError::ValidationError(format!("Invalid resolved Compose YAML: {error}"))
    })?;
    let services = value
        .get("services")
        .and_then(serde_yaml::Value::as_mapping)
        .ok_or_else(|| {
            AppError::ValidationError("Compose file must contain a services mapping".into())
        })?;
    let mut out = Vec::new();
    for (name, config) in services {
        let name = name.as_str().ok_or_else(|| {
            AppError::ValidationError("Compose service name must be a string".into())
        })?;
        let config = config.as_mapping().ok_or_else(|| {
            AppError::ValidationError(format!("Service '{name}' must be a mapping"))
        })?;
        let mut suggested = HashSet::new();
        if let Some(expose) = config
            .get(serde_yaml::Value::String("expose".into()))
            .and_then(serde_yaml::Value::as_sequence)
        {
            for entry in expose {
                if let Some(value) = entry.as_str() {
                    if let Ok(span) = PortSpan::parse(value.split('/').next().unwrap_or(value)) {
                        for port in span.start..=span.end {
                            suggested.insert(port);
                        }
                    }
                }
            }
        }
        let mut direct = Vec::new();
        if let Some(ports) = config
            .get(serde_yaml::Value::String("ports".into()))
            .and_then(serde_yaml::Value::as_sequence)
        {
            for entry in ports {
                let exposure = match parse_port(name, entry, legacy) {
                    Ok(exposure) => exposure,
                    // A variable-bearing port is skipped rather than rejected: it
                    // publishes nothing we can reason about until deploy time, so
                    // it also stays invisible to host-port conflict detection.
                    Err(_) if port_entry_has_variable(entry) => continue,
                    Err(error) => return Err(AppError::ValidationError(error)),
                };
                for port in exposure.target.start..=exposure.target.end {
                    suggested.insert(port);
                }
                direct.push(exposure);
            }
        }
        let mut suggested_internal_ports: Vec<_> = suggested.into_iter().collect();
        suggested_internal_ports.sort_unstable();
        direct.sort_by(|a, b| {
            (&a.protocol, &a.published.start, &a.service).cmp(&(
                &b.protocol,
                &b.published.start,
                &b.service,
            ))
        });
        out.push(ComposeServiceCatalog {
            name: name.into(),
            suggested_internal_ports,
            direct_exposures: direct,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(ComposeCatalog { services: out })
}

fn ips_overlap(left: Option<&str>, right: Option<&str>) -> bool {
    match (left, right) {
        (None, _) | (_, None) => true,
        (Some(left), Some(right)) if left == right => true,
        (Some("0.0.0.0"), Some(right)) | (Some(right), Some("0.0.0.0")) => !right.contains(':'),
        (Some("::"), Some(right)) | (Some(right), Some("::")) => {
            right.contains(':') || right == "::"
        }
        _ => false,
    }
}

pub fn conflicts(
    exposures: &[DirectExposure],
    reserved: &HashSet<i64>,
    occupied: &[OccupiedPort],
) -> Vec<String> {
    let mut errors = Vec::new();
    for exposure in exposures {
        if (exposure.published.start..=exposure.published.end)
            .any(|port| reserved.contains(&(port as i64)))
        {
            errors.push(format!(
                "{} {} uses a litepod-reserved port",
                exposure.protocol.to_uppercase(),
                exposure.published.display()
            ));
        }
        for existing in occupied {
            if exposure.protocol == existing.protocol
                && exposure.published.overlaps(&existing.published)
                && ips_overlap(exposure.host_ip.as_deref(), existing.host_ip.as_deref())
            {
                errors.push(format!(
                    "{} {} is already used by {}",
                    exposure.protocol.to_uppercase(),
                    exposure.published.display(),
                    existing.owner
                ));
            }
        }
    }
    errors.sort();
    errors.dedup();
    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalogs_short_long_tcp_udp_and_ranges() {
        let catalog = catalog_from_yaml("services:\n  web:\n    expose: [\"3000\"]\n    ports:\n      - \"127.0.0.1:8080:3000/tcp\"\n      - target: \"3001-3003\"\n        published: \"9000-9002\"\n        protocol: udp\n", false).unwrap();
        assert_eq!(
            catalog.services[0].suggested_internal_ports,
            vec![3000, 3001, 3002, 3003]
        );
        assert_eq!(catalog.services[0].direct_exposures[1].protocol, "udp");
    }
    #[test]
    fn skips_variable_bearing_ports_instead_of_failing() {
        let catalog = catalog_from_yaml(
            "services:\n  web:\n    ports:\n      - \"${PORT}:3000\"\n      - \"8080:80\"\n",
            false,
        )
        .unwrap();
        let published: Vec<_> = catalog.services[0]
            .direct_exposures
            .iter()
            .map(|exposure| exposure.published.display())
            .collect();
        assert_eq!(published, vec!["8080"]);
        // The target of the skipped entry is unknown, so it suggests nothing.
        assert_eq!(catalog.services[0].suggested_internal_ports, vec![80]);
    }
    #[test]
    fn skips_variable_bearing_long_form_ports() {
        let catalog = catalog_from_yaml("services:\n  web:\n    ports:\n      - target: \"3000\"\n        published: \"${PORT}\"\n", false).unwrap();
        assert!(catalog.services[0].direct_exposures.is_empty());
    }
    #[test]
    fn still_rejects_ports_that_are_merely_invalid() {
        assert!(
            catalog_from_yaml(
                "services:\n  web:\n    ports:\n      - \"notaport:3000\"\n",
                false
            )
            .is_err()
        );
    }
    #[test]
    fn detects_protocol_aware_conflicts() {
        let exposure = parse_short_port("web", "33010:33010/tcp", false).unwrap();
        let mut reserved = HashSet::new();
        reserved.insert(80);
        assert!(
            conflicts(
                &[exposure.clone()],
                &reserved,
                &[OccupiedPort {
                    owner: "other".into(),
                    host_ip: None,
                    published: PortSpan {
                        start: 33010,
                        end: 33010
                    },
                    protocol: "tcp".into()
                }]
            )
            .len()
                == 1
        );
        assert!(
            conflicts(
                &[exposure],
                &reserved,
                &[OccupiedPort {
                    owner: "other".into(),
                    host_ip: None,
                    published: PortSpan {
                        start: 33010,
                        end: 33010
                    },
                    protocol: "udp".into()
                }]
            )
            .is_empty()
        );
    }
}
