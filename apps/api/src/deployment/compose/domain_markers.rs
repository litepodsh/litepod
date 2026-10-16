//! Domain markers in a Compose app's runtime env: `{{url}}`, `{{domain}}`,
//! `{{url:8080}}`, `{{url:web:3000}}`. Unlike the secret markers in
//! `templates::secrets`, which are replaced once at creation, these are stored
//! as written and rewritten on every deploy with the address of the route they
//! name, so a variable follows its domain when the user changes it.
//!
//! Pure: no DB access. `marker_routes` loads the inputs.

use std::collections::{BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

use crate::{
    deployment::compose_standalone::{service_exposes_port, services_for_port},
    dtos::deployment::{ComposeProxyRouteDto, DomainDto},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerKind {
    Url,
    Domain,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarkerTarget {
    Primary,
    Port(u16),
    ServicePort(String, u16),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomainMarker {
    pub kind: MarkerKind,
    pub target: MarkerTarget,
    pub raw: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RouteTarget {
    pub service: String,
    pub port: u16,
}

/// One `[[ports]]` entry of the catalog template an app was created from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogPort {
    pub container: u16,
    #[serde(default)]
    pub service: Option<String>,
    #[serde(default)]
    pub primary: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteAddress {
    pub domain_id: String,
    pub url: String,
    pub host: String,
}

fn shown(inner: &str) -> String {
    format!("{{{{{inner}}}}}")
}

fn parse_port(text: &str) -> Option<u16> {
    text.parse::<u16>().ok().filter(|port| *port > 0)
}

fn valid_service(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// `None` when `inner` is not a domain marker at all (a secret marker,
/// `{{urlx}}`, ...); `Some(Err)` when it is one but malformed.
fn parse_inner(inner: &str) -> Option<Result<(MarkerKind, MarkerTarget), String>> {
    let trimmed = inner.trim();
    let (kind, rest) = if let Some(rest) = trimmed.strip_prefix("url") {
        (MarkerKind::Url, rest)
    } else if let Some(rest) = trimmed.strip_prefix("domain") {
        (MarkerKind::Domain, rest)
    } else {
        return None;
    };
    if rest.is_empty() {
        return Some(Ok((kind, MarkerTarget::Primary)));
    }
    let args = rest.strip_prefix(':')?;
    let parts: Vec<&str> = args.split(':').collect();
    let target = match parts.as_slice() {
        [port] => parse_port(port).map(MarkerTarget::Port),
        [service, port] if valid_service(service) => {
            parse_port(port).map(|port| MarkerTarget::ServicePort((*service).to_owned(), port))
        }
        _ => None,
    };
    Some(
        target
            .map(|target| (kind, target))
            .ok_or_else(|| format!("malformed domain marker {}", shown(inner))),
    )
}

/// Walks every `{{...}}` in `value`, replacing domain markers with whatever
/// `replace` returns and copying everything else verbatim.
fn rewrite(
    value: &str,
    mut replace: impl FnMut(&DomainMarker) -> Result<String, String>,
) -> Result<String, String> {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else { break };
        let inner = &after[..end];
        out.push_str(&rest[..start]);
        match parse_inner(inner) {
            Some(parsed) => {
                let (kind, target) = parsed?;
                out.push_str(&replace(&DomainMarker {
                    kind,
                    target,
                    raw: shown(inner),
                })?);
            }
            None => out.push_str(&shown(inner)),
        }
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    Ok(out)
}

pub fn markers_in(value: &str) -> Result<Vec<DomainMarker>, String> {
    let mut found = Vec::new();
    rewrite(value, |marker| {
        found.push(marker.clone());
        Ok(marker.raw.clone())
    })?;
    Ok(found)
}

/// Whether any value holds a domain marker, malformed ones included, so callers
/// that skip work for marker-free envs still surface a typo.
pub fn has_markers(env: &[(String, String)]) -> bool {
    env.iter()
        .any(|(_, value)| markers_in(value).map_or(true, |found| !found.is_empty()))
}

fn catalog_route(port: &CatalogPort, compose: &str) -> Result<RouteTarget, String> {
    let service = match &port.service {
        Some(service) if service_exposes_port(compose, service, port.container) => service.clone(),
        Some(service) => {
            return Err(format!(
                "service '{service}' does not expose port {}",
                port.container
            ));
        }
        None => {
            let owners = services_for_port(compose, port.container);
            match owners.as_slice() {
                [only] => only.clone(),
                [] => return Err(format!("no service exposes port {}", port.container)),
                _ => {
                    return Err(format!(
                        "port {} is exposed by several services ({})",
                        port.container,
                        owners.join(", ")
                    ));
                }
            }
        }
    };
    Ok(RouteTarget {
        service,
        port: port.container,
    })
}

pub fn route_for(
    target: &MarkerTarget,
    ports: &[CatalogPort],
    compose: &str,
) -> Result<RouteTarget, String> {
    match target {
        MarkerTarget::Primary => {
            let port = ports
                .iter()
                .find(|port| port.primary)
                .or(ports.first())
                .ok_or_else(|| {
                    "{{url}} and {{domain}} need a catalog template's primary port; use {{url:SERVICE:PORT}}"
                        .to_string()
                })?;
            catalog_route(port, compose)
        }
        MarkerTarget::Port(number) => {
            let port = ports
                .iter()
                .find(|port| port.container == *number)
                .ok_or_else(|| {
                    format!(
                        "port {number} is not declared by the template; use {{{{url:SERVICE:{number}}}}}"
                    )
                })?;
            catalog_route(port, compose)
        }
        MarkerTarget::ServicePort(service, port) => {
            if service_exposes_port(compose, service, *port) {
                Ok(RouteTarget {
                    service: service.clone(),
                    port: *port,
                })
            } else {
                Err(format!("service '{service}' does not expose port {port}"))
            }
        }
    }
}

pub fn required_routes(
    env: &[(String, String)],
    ports: &[CatalogPort],
    compose: &str,
) -> Result<BTreeSet<RouteTarget>, String> {
    let mut routes = BTreeSet::new();
    for (name, value) in env {
        for marker in markers_in(value).map_err(|error| format!("{name}: {error}"))? {
            routes.insert(
                route_for(&marker.target, ports, compose)
                    .map_err(|error| format!("{name}: {error}"))?,
            );
        }
    }
    Ok(routes)
}

/// The address each routed `(service, port)` answers on: its primary domain,
/// else the oldest one.
pub fn addresses(
    routes: &[ComposeProxyRouteDto],
    domains: &[DomainDto],
) -> HashMap<RouteTarget, RouteAddress> {
    let mut best: HashMap<RouteTarget, &DomainDto> = HashMap::new();
    for route in routes {
        let Some(domain) = domains.iter().find(|domain| domain.id == route.domain_id) else {
            continue;
        };
        let target = RouteTarget {
            service: route.service.clone(),
            port: route.container_port,
        };
        best.entry(target)
            .and_modify(|current| {
                if (domain.is_primary, std::cmp::Reverse(domain.created_at))
                    > (current.is_primary, std::cmp::Reverse(current.created_at))
                {
                    *current = domain;
                }
            })
            .or_insert(domain);
    }
    best.into_iter()
        .map(|(target, domain)| {
            (
                target,
                RouteAddress {
                    domain_id: domain.id.clone(),
                    url: domain.url.trim_end_matches('/').to_owned(),
                    host: domain.host.clone(),
                },
            )
        })
        .collect()
}

pub fn resolve_env(
    env: Vec<(String, String)>,
    ports: &[CatalogPort],
    compose: &str,
    addresses: &HashMap<RouteTarget, RouteAddress>,
) -> Result<Vec<(String, String)>, String> {
    env.into_iter()
        .map(|(name, value)| {
            let resolved = rewrite(&value, |marker| {
                let route = route_for(&marker.target, ports, compose)?;
                let address = addresses.get(&route).ok_or_else(|| {
                    format!(
                        "references {}:{}, which has no domain",
                        route.service, route.port
                    )
                })?;
                Ok(match marker.kind {
                    MarkerKind::Url => address.url.clone(),
                    MarkerKind::Domain => address.host.clone(),
                })
            })
            .map_err(|error| format!("{name} {error}"))?;
            Ok((name, resolved))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENTE: &str = "services:\n  ente-museum:\n    expose: [\"8080\"]\n  ente-web:\n    expose: [\"3000\", \"3002\"]\n  ente-db:\n    image: postgres\n";

    fn ente_ports() -> Vec<CatalogPort> {
        vec![
            CatalogPort {
                container: 3000,
                service: None,
                primary: true,
            },
            CatalogPort {
                container: 3002,
                service: None,
                primary: false,
            },
            CatalogPort {
                container: 8080,
                service: None,
                primary: false,
            },
        ]
    }

    fn domain(id: &str, host: &str, https: bool, is_primary: bool, created_at: i64) -> DomainDto {
        DomainDto {
            server_id: None,
            certificate_id: None,
            certificate_name: None,
            certificate_provider: "automatic".into(),
            has_custom_certificate: false,
            id: id.into(),
            application_id: "app".into(),
            host: host.into(),
            port: 80,
            https,
            is_primary,
            url: format!("{}://{host}", if https { "https" } else { "http" }),
            is_local: false,
            required_dns_target: None,
            ingress_mode: "central".into(),
            route_health: "pending".into(),
            route_health_reason: None,
            created_at,
            updated_at: created_at,
        }
    }

    fn route(domain_id: &str, service: &str, port: u16) -> ComposeProxyRouteDto {
        ComposeProxyRouteDto {
            domain_id: domain_id.into(),
            application_id: "app".into(),
            service: service.into(),
            container_port: port,
        }
    }

    fn env(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).into(), (*v).into()))
            .collect()
    }

    #[test]
    fn parses_every_form_and_ignores_secret_markers() {
        let found =
            markers_in("{{url}}/ {{ domain:8080 }} {{url:web:3000}} {{password_32}} {{urlx}}")
                .unwrap();
        let targets: Vec<_> = found.iter().map(|m| (m.kind, m.target.clone())).collect();
        assert_eq!(
            targets,
            vec![
                (MarkerKind::Url, MarkerTarget::Primary),
                (MarkerKind::Domain, MarkerTarget::Port(8080)),
                (
                    MarkerKind::Url,
                    MarkerTarget::ServicePort("web".into(), 3000)
                ),
            ]
        );
    }

    #[test]
    fn rejects_malformed_markers() {
        for value in [
            "{{url:}}",
            "{{url:abc}}",
            "{{domain:0}}",
            "{{url:a:b:c}}",
            "{{url:70000}}",
        ] {
            assert!(markers_in(value).is_err(), "{value} should be rejected");
        }
    }

    #[test]
    fn maps_ente_ports_to_services() {
        let routes = required_routes(
            &env(&[
                ("API", "{{url:8080}}"),
                ("ALBUMS", "{{url:3002}}"),
                ("WEB", "{{url}}"),
            ]),
            &ente_ports(),
            ENTE,
        )
        .unwrap();
        let routes: Vec<_> = routes.into_iter().map(|r| (r.service, r.port)).collect();
        assert_eq!(
            routes,
            vec![
                ("ente-museum".to_string(), 8080),
                ("ente-web".to_string(), 3000),
                ("ente-web".to_string(), 3002),
            ]
        );
    }

    #[test]
    fn rejects_undeclared_and_ambiguous_ports() {
        let err = required_routes(&env(&[("X", "{{url:9999}}")]), &ente_ports(), ENTE).unwrap_err();
        assert!(err.starts_with("X: port 9999 is not declared"), "{err}");

        let compose = "services:\n  a:\n    expose: [\"80\"]\n  b:\n    expose: [\"80\"]\n";
        let ports = vec![CatalogPort {
            container: 80,
            service: None,
            primary: true,
        }];
        assert!(required_routes(&env(&[("X", "{{url}}")]), &ports, compose).is_err());
        let ports = vec![CatalogPort {
            container: 80,
            service: Some("b".into()),
            primary: true,
        }];
        assert_eq!(
            required_routes(&env(&[("X", "{{url}}")]), &ports, compose)
                .unwrap()
                .into_iter()
                .next()
                .unwrap()
                .service,
            "b"
        );
    }

    #[test]
    fn bare_markers_need_catalog_ports() {
        assert!(required_routes(&env(&[("X", "{{url}}")]), &[], ENTE).is_err());
        assert!(required_routes(&env(&[("X", "{{url:ente-web:3000}}")]), &[], ENTE).is_ok());
    }

    #[test]
    fn resolves_ente_with_primary_and_https_domains() {
        let domains = vec![
            domain("d1", "litepod-a.1.2.3.4.sslip.io", false, true, 1),
            domain("d2", "litepod-b.1.2.3.4.sslip.io", false, false, 2),
            domain("d3", "api.fotos.example.com", true, false, 3),
            domain("d4", "litepod-c.1.2.3.4.sslip.io", false, false, 4),
        ];
        // d2 and d3 both route to the API; neither is primary, so the older d2 wins.
        let routes = vec![
            route("d1", "ente-web", 3000),
            route("d2", "ente-museum", 8080),
            route("d3", "ente-museum", 8080),
            route("d4", "ente-web", 3002),
        ];
        let resolved = resolve_env(
            env(&[
                ("API", "{{url:8080}}"),
                ("ALBUMS", "{{url:3002}}"),
                ("HOST", "{{domain}}:443"),
                ("SECRET", "{{password_32}}"),
            ]),
            &ente_ports(),
            ENTE,
            &addresses(&routes, &domains),
        )
        .unwrap();
        assert_eq!(
            resolved,
            env(&[
                ("API", "http://litepod-b.1.2.3.4.sslip.io"),
                ("ALBUMS", "http://litepod-c.1.2.3.4.sslip.io"),
                ("HOST", "litepod-a.1.2.3.4.sslip.io:443"),
                ("SECRET", "{{password_32}}"),
            ])
        );
    }

    #[test]
    fn primary_domain_wins_within_a_route() {
        let domains = vec![
            domain("old", "litepod-b.sslip.io", false, false, 1),
            domain("new", "api.example.com", true, true, 2),
        ];
        let routes = vec![
            route("old", "ente-museum", 8080),
            route("new", "ente-museum", 8080),
        ];
        let addrs = addresses(&routes, &domains);
        let target = RouteTarget {
            service: "ente-museum".into(),
            port: 8080,
        };
        assert_eq!(addrs[&target].url, "https://api.example.com");
    }

    #[test]
    fn missing_route_names_the_variable() {
        let err = resolve_env(
            env(&[("API", "{{url:8080}}")]),
            &ente_ports(),
            ENTE,
            &HashMap::new(),
        )
        .unwrap_err();
        assert_eq!(err, "API references ente-museum:8080, which has no domain");
    }
}
