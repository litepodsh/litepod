//! Where a connection is allowed to point.
//!
//! A base URL is caller-supplied, and the server is the one that dials it. Left
//! unchecked that is a server-side request forgery: `http://127.0.0.1:2375`,
//! `http://169.254.169.254/latest/meta-data/`, anything on the host network -
//! and since a non-2xx body comes back to the caller as the error text, it is a
//! readable probe rather than a blind one.
//!
//! Ollama and LM Studio are the reason this is not simply "no private
//! addresses": they serve a loopback port and nothing else. So loopback and the
//! private ranges are allowed only for the provider kinds that want no key at
//! all, which are exactly those two.
//!
//! The check resolves the name and looks at every address it gets back, so a
//! public name that maps to `127.0.0.1` is refused. It runs immediately before
//! each request rather than once at save time, which is what closes the rebind:
//! a name that answered publicly when the connection was saved is checked again
//! on the turn that uses it.
//!
//! ponytail: resolve-then-connect still leaves a millisecond window where the
//! record could change between this lookup and reqwest's own. Closing it means
//! pinning the resolved address into the client (`ClientBuilder::resolve`) and
//! is worth doing only if this ever guards something more valuable than a
//! model endpoint.

use std::net::IpAddr;

use url::Url;

/// Refuses a URL the server should not dial. The error is the caller's to read,
/// so it says which rule was broken and never dumps the resolved addresses.
pub async fn allowed(base_url: &str, allow_local: bool) -> Result<(), String> {
    let parsed =
        Url::parse(base_url).map_err(|error| format!("`{base_url}` is not a URL: {error}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("The base URL must start with http:// or https://".into());
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| format!("`{base_url}` has no host"))?;
    let port = parsed.port_or_known_default().unwrap_or(443);

    let addresses: Vec<IpAddr> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|error| format!("`{host}` does not resolve: {error}"))?
        .map(|socket| socket.ip())
        .collect();
    if addresses.is_empty() {
        return Err(format!("`{host}` does not resolve to any address"));
    }

    // Every address, not the first: a name that answers with one public and one
    // private record must not pass on the strength of the public one.
    if !allow_local && addresses.iter().any(|address| is_internal(*address)) {
        return Err(format!(
            "`{host}` resolves to an address on this machine's own network. \
             Only Ollama and LM Studio may point at a private address."
        ));
    }
    Ok(())
}

/// Loopback, private, link-local, carrier-grade NAT and the unspecified
/// address - everything that is reachable from the server but not from the
/// internet, which is the whole class this guards against.
fn is_internal(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_unspecified()
                || v4.is_documentation()
                // 0.0.0.0/8, "this network", routed to the local host.
                || a == 0
                // 100.64.0.0/10, carrier-grade NAT. `is_shared` is still unstable.
                || (a == 100 && (64..128).contains(&b))
        }
        IpAddr::V6(v6) => {
            // A mapped v4 address is a v4 address wearing a hat.
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_internal(IpAddr::V4(v4));
            }
            let first = v6.segments()[0];
            v6.is_loopback()
                || v6.is_unspecified()
                // fc00::/7 unique local, fe80::/10 link local. Both predicates
                // are unstable, so the masks are spelled out.
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_addresses_a_server_can_reach_but_the_internet_cannot_are_internal() {
        for address in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "0.0.0.0",
            "100.64.0.1",
            "::1",
            "fd00::1",
            "fe80::1",
            "::ffff:127.0.0.1",
        ] {
            assert!(
                is_internal(address.parse().unwrap()),
                "{address} should be internal"
            );
        }
        for address in ["1.1.1.1", "104.18.0.1", "2606:4700::1111", "100.128.0.1"] {
            assert!(
                !is_internal(address.parse().unwrap()),
                "{address} should be reachable"
            );
        }
    }

    #[tokio::test]
    async fn a_loopback_url_passes_only_for_a_local_provider() {
        assert!(allowed("http://127.0.0.1:11434/v1", true).await.is_ok());
        let refused = allowed("http://127.0.0.1:11434/v1", false)
            .await
            .expect_err("loopback must be refused for a hosted provider");
        assert!(refused.contains("private address"), "{refused}");
    }

    #[tokio::test]
    async fn a_url_that_is_not_one_is_refused_before_anything_is_dialled() {
        assert!(allowed("file:///etc/passwd", false).await.is_err());
        assert!(allowed("not a url", false).await.is_err());
    }
}
