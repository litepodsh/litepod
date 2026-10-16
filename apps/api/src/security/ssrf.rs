//! Shared guards for server-side outbound URLs.
//!
//! Every user-controlled URL the server requests must go through one of the
//! two async entry points: `validate_public_url` for anything that must stay on
//! the public internet, or `validate_for_host` for URLs that must match a
//! configured provider host exactly. The `parse_*` helpers are only for
//! shape-only checks done while saving configuration, not for I/O.
//!
//! Host reachability is re-checked immediately before each request, because
//! DNS answers can change after configuration is saved.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use url::{Host, Url};

/// Parse a user-controlled outbound URL with safe transport defaults.
///
/// Host reachability is checked separately immediately before the request,
/// because DNS answers can change after configuration is saved.
pub fn parse_https_url(raw: &str) -> Result<Url, String> {
    parse_https_url_inner(raw, false)
}

/// Parse an HTTPS URL that may use a custom port, for explicitly configured
/// self-hosted services.
pub fn parse_https_url_with_port(raw: &str) -> Result<Url, String> {
    parse_https_url_inner(raw, true)
}

fn parse_https_url_inner(raw: &str, allow_port: bool) -> Result<Url, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("URL is required".into());
    }
    let url = Url::parse(raw).map_err(|_| "That is not a valid URL".to_string())?;
    if url.scheme() != "https" {
        return Err("URL must use https".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("URL must not contain credentials".into());
    }
    if !allow_port && url.port().is_some() {
        return Err("URL must not specify a port".into());
    }
    match url.host() {
        Some(Host::Domain(_)) => Ok(url),
        Some(Host::Ipv4(_) | Host::Ipv6(_)) => {
            Err("URL must use a hostname, not a literal IP address".into())
        }
        None => Err("URL must have a hostname".into()),
    }
}

/// Parse an explicitly configured self-hosted endpoint. A hostname is
/// required; localhost and literal IPs are not valid service domains.
pub fn parse_self_hosted_url(raw: &str) -> Result<Url, String> {
    let url = parse_https_url_with_port(raw)?;
    let host = url
        .host_str()
        .ok_or_else(|| "URL must have a hostname".to_string())?;
    let labels = host.trim_end_matches('.').split('.').collect::<Vec<_>>();
    if host.eq_ignore_ascii_case("localhost")
        || labels.len() < 2
        || labels
            .iter()
            .any(|label| label.is_empty() || label.starts_with('-') || label.ends_with('-'))
        || labels
            .iter()
            .flat_map(|label| label.chars())
            .any(|character| !(character.is_ascii_alphanumeric() || character == '-'))
    {
        return Err("Self-hosted URL must use a valid domain name".into());
    }
    Ok(url)
}

/// Parse an outbound URL and require it to use the configured provider host.
pub fn parse_url_for_host(raw: &str, expected_host_url: &str) -> Result<Url, String> {
    let url = parse_https_url_with_port(raw)?;
    let expected = parse_https_url_with_port(expected_host_url)?;
    let actual_host = url
        .host_str()
        .ok_or_else(|| "URL must have a hostname".to_string())?;
    let expected_host = expected
        .host_str()
        .ok_or_else(|| "Provider URL must have a hostname".to_string())?;
    if !actual_host.eq_ignore_ascii_case(expected_host) {
        return Err(format!(
            "URL host must match configured provider host {expected_host}"
        ));
    }
    if url.port_or_known_default() != expected.port_or_known_default() {
        return Err("URL port must match configured provider host".into());
    }
    Ok(url)
}

/// Validate an outbound URL that may only reach the public internet (AI custom
/// providers, anything that must never touch the internal network). HTTPS, no
/// credentials, no literal IPs; DNS is re-resolved and verified public.
pub async fn validate_public_url(raw: &str) -> Result<Url, String> {
    let url = parse_https_url(raw)?;
    assert_host(&url, false).await?;
    Ok(url)
}

/// Validate an outbound URL that must use a configured provider host exactly
/// (git clone URLs, branch listing). Private addresses are allowed only when
/// the provider is explicitly self-hosted.
pub async fn validate_for_host(
    raw: &str,
    provider_host: &str,
    allow_private: bool,
) -> Result<Url, String> {
    let url = parse_url_for_host(raw, provider_host)?;
    assert_host(&url, allow_private).await?;
    Ok(url)
}

/// Resolve URL host and reject every non-public address immediately before I/O.
pub async fn assert_public_host(url: &Url) -> Result<(), String> {
    assert_host(url, false).await
}

/// Resolve URL host immediately before I/O. Private addresses are allowed only
/// for explicitly configured self-hosted providers.
pub async fn assert_host(url: &Url, allow_private: bool) -> Result<(), String> {
    let host = url
        .host_str()
        .ok_or_else(|| "URL has no hostname".to_string())?;
    let port = url.port_or_known_default().unwrap_or(443);
    let addresses = tokio::net::lookup_host((host, port))
        .await
        .map_err(|error| format!("Could not resolve {host}: {error}"))?;

    let mut any = false;
    for address in addresses {
        any = true;
        if !allow_private && !is_public(address.ip()) {
            return Err(format!(
                "{host} resolves to a non-public address ({}) and will not be contacted",
                address.ip()
            ));
        }
    }
    if !any {
        return Err(format!("{host} did not resolve to any address"));
    }
    Ok(())
}

/// `IpAddr::is_global` is still unstable, so the ranges are spelled out.
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => is_public_v4(v4),
            None => is_public_v6(v6),
        },
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, _, _] = ip.octets();
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        || (a == 100 && (64..128).contains(&b))
        || (a == 192 && b == 0 && ip.octets()[2] == 0)
        || (a == 198 && (b == 18 || b == 19))
        || a >= 240)
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    let first = ip.segments()[0];
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_multicast()
        || (first & 0xfe00) == 0xfc00
        || (first & 0xffc0) == 0xfe80
        || (first == 0x2001 && ip.segments()[1] == 0x0db8))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn rejects_unsafe_url_shapes() {
        for candidate in [
            "http://example.com",
            "https://user:pass@example.com",
            "https://example.com:8443",
            "https://127.0.0.1",
            "https://[::1]",
            "not-a-url",
            "",
        ] {
            assert!(parse_https_url(candidate).is_err(), "accepted {candidate}");
        }
    }

    #[test]
    fn requires_repository_host_to_match_provider() {
        assert!(
            parse_url_for_host(
                "https://git.example.com/org/repo.git",
                "https://git.example.com"
            )
            .is_ok()
        );
        assert!(
            parse_url_for_host(
                "https://attacker.example/org/repo.git",
                "https://git.example.com"
            )
            .is_err()
        );
    }

    #[test]
    fn accepts_valid_self_hosted_domains_and_ports() {
        assert!(parse_self_hosted_url("https://git.sebasgc.xyz").is_ok());
        assert!(parse_self_hosted_url("https://git.internal.example:8443").is_ok());
        assert!(parse_self_hosted_url("https://127.0.0.1:8443").is_err());
        assert!(parse_self_hosted_url("https://localhost").is_err());
        assert!(parse_self_hosted_url("http://git.example.com").is_err());
    }

    #[test]
    fn rejects_private_and_mapped_addresses() {
        for candidate in [
            "127.0.0.1",
            "10.0.0.1",
            "169.254.169.254",
            "::1",
            "::ffff:127.0.0.1",
        ] {
            assert!(!is_public(IpAddr::from_str(candidate).unwrap()));
        }
    }

    #[tokio::test]
    async fn public_urls_reject_unsafe_shapes_and_private_hosts() {
        for candidate in [
            "http://example.com",
            "https://user:pass@example.com",
            "https://example.com:8443",
            "https://127.0.0.1",
            "https://[::1]",
            "https://169.254.169.254",
            "https://localhost",
            "not-a-url",
            "",
        ] {
            assert!(
                validate_public_url(candidate).await.is_err(),
                "accepted {candidate}"
            );
        }
    }

    #[tokio::test]
    async fn provider_host_urls_must_match_and_stay_public() {
        // Foreign hosts, wrong scheme and literal IPs are rejected at parse time.
        for candidate in [
            "https://attacker.example/o/r.git",
            "https://localhost/o/r.git",
            "http://github.com/o/r.git",
            "https://169.254.169.254/o/r.git",
        ] {
            assert!(
                validate_for_host(candidate, "https://github.com", false)
                    .await
                    .is_err(),
                "accepted {candidate}"
            );
        }
        // A matching self-hosted host on a loopback address is reachable only
        // when the caller opts into private addresses.
        assert!(
            validate_for_host(
                "https://localhost:8443/o/r.git",
                "https://localhost:8443",
                true
            )
            .await
            .is_ok()
        );
        assert!(
            validate_for_host(
                "https://localhost:8443/o/r.git",
                "https://localhost:8443",
                false
            )
            .await
            .is_err()
        );
    }
}
