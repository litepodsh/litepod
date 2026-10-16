//! Turning a server's public address into a point on the map.
//!
//! `ip.guide` is a keyless public API, so there is nothing to configure. When it
//! only knows the country (no city, coordinates at the country centroid),
//! `ipwho.is`, also keyless, is asked for something more precise. Every
//! failure maps to `None`: a fleet map that cannot place a marker is a smaller
//! problem than a server list that refuses to load.
//!
//! Nothing here is user-controlled except the address being looked up, and that
//! address is only ever put in the path of a fixed, hardcoded origin.

use std::{net::IpAddr, time::Duration};

use serde::Deserialize;

use crate::security::ssrf::is_public;

const ORIGIN: &str = "https://ip.guide";
const FALLBACK_ORIGIN: &str = "https://ipwho.is";
const TIMEOUT: Duration = Duration::from_secs(5);
const MAX_BYTES: usize = 16 * 1024;

/// A resolved location. Absent coordinates mean the lookup produced nothing
/// mappable, which is common for anycast ranges.
#[derive(Debug, Clone, PartialEq)]
pub struct ServerGeo {
    pub latitude: f64,
    pub longitude: f64,
    pub city: Option<String>,
    pub country: Option<String>,
    /// IANA zone, so the UI can show the server's own wall clock.
    pub timezone: Option<String>,
    pub asn: Option<String>,
}

/// Why a server has no coordinates. The UI turns these into readable text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unlocated {
    /// The server exposes no address to look up.
    NoAddress,
    /// A private, loopback or link-local address, never sent to a third party.
    PrivateAddress,
    /// ip.guide was reached but had no location for it, or could not be reached.
    LookupFailed,
}

impl Unlocated {
    pub fn as_str(self) -> &'static str {
        match self {
            Unlocated::NoAddress => "no_address",
            Unlocated::PrivateAddress => "private_address",
            Unlocated::LookupFailed => "lookup_failed",
        }
    }
}

#[derive(Deserialize)]
struct GuideResponse {
    network: Option<Network>,
    location: Option<Location>,
}

#[derive(Deserialize)]
struct Network {
    autonomous_system: Option<AutonomousSystem>,
}

#[derive(Deserialize)]
struct AutonomousSystem {
    asn: Option<u32>,
    name: Option<String>,
}

#[derive(Deserialize)]
struct Location {
    city: Option<String>,
    country: Option<String>,
    timezone: Option<String>,
    latitude: Option<f64>,
    longitude: Option<f64>,
}

fn parse(body: &[u8]) -> Option<ServerGeo> {
    let response: GuideResponse = serde_json::from_slice(body).ok()?;
    let location = response.location?;
    // Anycast ranges come back with every location field null but a populated
    // network block, so coordinates are what decides whether this is mappable.
    let (latitude, longitude) = (location.latitude?, location.longitude?);
    let asn = response
        .network
        .and_then(|network| network.autonomous_system)
        .map(|system| match (system.asn, system.name) {
            (Some(number), Some(name)) => format!("AS{number} {name}"),
            (Some(number), None) => format!("AS{number}"),
            (None, name) => name.unwrap_or_default(),
        })
        .filter(|value| !value.is_empty());
    Some(ServerGeo {
        latitude,
        longitude,
        city: location.city,
        country: location.country,
        timezone: location.timezone,
        asn,
    })
}

#[derive(Deserialize)]
struct WhoisResponse {
    success: bool,
    city: Option<String>,
    country: Option<String>,
    latitude: Option<f64>,
    longitude: Option<f64>,
    timezone: Option<WhoisTimezone>,
    connection: Option<WhoisConnection>,
}

#[derive(Deserialize)]
struct WhoisTimezone {
    id: Option<String>,
}

#[derive(Deserialize)]
struct WhoisConnection {
    asn: Option<u32>,
    isp: Option<String>,
}

/// `ipwho.is` answer, kept only when it names a city: a country-level answer is
/// no better than what ip.guide already gave.
fn parse_fallback(body: &[u8]) -> Option<ServerGeo> {
    let response: WhoisResponse = serde_json::from_slice(body).ok()?;
    if !response.success {
        return None;
    }
    let city = response.city.filter(|city| !city.is_empty())?;
    let asn = response
        .connection
        .map(|connection| match (connection.asn, connection.isp) {
            (Some(number), Some(isp)) => format!("AS{number} {isp}"),
            (Some(number), None) => format!("AS{number}"),
            (None, isp) => isp.unwrap_or_default(),
        })
        .filter(|value| !value.is_empty());
    Some(ServerGeo {
        latitude: response.latitude?,
        longitude: response.longitude?,
        city: Some(city),
        country: response.country,
        timezone: response.timezone.and_then(|timezone| timezone.id),
        asn,
    })
}

async fn get(url: &str) -> Option<bytes::Bytes> {
    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .ok()?;
    let response = client.get(url).send().await.ok()?;
    if !response.status().is_success() {
        tracing::debug!(status = %response.status(), url, "geolocation lookup rejected");
        return None;
    }
    let bytes = response.bytes().await.ok()?;
    if bytes.len() > MAX_BYTES {
        return None;
    }
    Some(bytes)
}

async fn fetch(url: String) -> Option<ServerGeo> {
    parse(&get(&url).await?)
}

/// First public address `host` resolves to. A hostname is resolved here rather
/// than handed to ip.guide, which only accepts IPs.
async fn public_ip_of(host: &str) -> Result<IpAddr, Unlocated> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return if is_public(ip) {
            Ok(ip)
        } else {
            Err(Unlocated::PrivateAddress)
        };
    }
    let resolved = tokio::net::lookup_host((host, 0u16))
        .await
        .map_err(|_| Unlocated::LookupFailed)?;
    let mut saw_address = false;
    for address in resolved {
        saw_address = true;
        if is_public(address.ip()) {
            return Ok(address.ip());
        }
    }
    Err(if saw_address {
        Unlocated::PrivateAddress
    } else {
        Unlocated::LookupFailed
    })
}

/// Location of whatever `host` (an IP or a hostname) points at.
pub async fn for_host(host: &str) -> Result<ServerGeo, Unlocated> {
    let ip = public_ip_of(host).await?;
    let primary = fetch(format!("{ORIGIN}/{ip}")).await;
    if primary.as_ref().is_some_and(|geo| geo.city.is_some()) {
        return Ok(primary.unwrap());
    }
    let fallback = get(&format!("{FALLBACK_ORIGIN}/{ip}"))
        .await
        .and_then(|body| parse_fallback(&body));
    fallback.or(primary).ok_or(Unlocated::LookupFailed)
}

/// Location of the machine this API runs on. Used for the local server when
/// `LITEPOD_PUBLIC_IP` is a development placeholder: with no path, ip.guide
/// answers about the caller.
pub async fn for_self() -> Result<ServerGeo, Unlocated> {
    fetch(format!("{ORIGIN}/"))
        .await
        .ok_or(Unlocated::LookupFailed)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HETZNER: &[u8] = br#"{"ip":"159.69.120.1","network":{"cidr":"159.69.0.0/16",
      "autonomous_system":{"asn":24940,"name":"HETZNER-AS Hetzner Online GmbH",
      "organization":"Hetzner Online GmbH","country":"DE","rir":"RIPE NCC"}},
      "location":{"city":"Falkenstein","country":"Germany","timezone":"Europe/Berlin",
      "latitude":50.4777,"longitude":12.3649}}"#;

    // Anycast: a real network block, but nothing to put on a map.
    const ANYCAST: &[u8] = br#"{"ip":"1.1.1.1","network":{"cidr":"1.1.1.0/24",
      "autonomous_system":{"asn":13335,"name":"CLOUDFLARENET - Cloudflare, Inc.",
      "organization":"Cloudflare, Inc.","country":"US","rir":"ARIN"}},
      "location":{"city":null,"country":null,"timezone":null,"latitude":null,"longitude":null}}"#;

    #[test]
    fn parses_a_located_address() {
        let geo = parse(HETZNER).expect("Hetzner address is mappable");
        assert_eq!(geo.latitude, 50.4777);
        assert_eq!(geo.longitude, 12.3649);
        assert_eq!(geo.city.as_deref(), Some("Falkenstein"));
        assert_eq!(geo.country.as_deref(), Some("Germany"));
        assert_eq!(geo.timezone.as_deref(), Some("Europe/Berlin"));
        assert_eq!(
            geo.asn.as_deref(),
            Some("AS24940 HETZNER-AS Hetzner Online GmbH")
        );
    }

    #[test]
    fn anycast_has_no_coordinates() {
        assert_eq!(parse(ANYCAST), None);
    }

    // IONOS range ip.guide only places at the US centroid.
    const WHOIS: &[u8] = br#"{"ip":"216.250.119.183","success":true,"country":"United States",
      "city":"Washington","latitude":38.8951121,"longitude":-77.0363706,
      "timezone":{"id":"America/New_York"},"connection":{"asn":8560,"isp":"IONOS SE"}}"#;

    #[test]
    fn fallback_parses_a_city() {
        let geo = parse_fallback(WHOIS).expect("city-level answer");
        assert_eq!(geo.city.as_deref(), Some("Washington"));
        assert_eq!(geo.latitude, 38.8951121);
        assert_eq!(geo.timezone.as_deref(), Some("America/New_York"));
        assert_eq!(geo.asn.as_deref(), Some("AS8560 IONOS SE"));
    }

    #[test]
    fn fallback_without_a_city_is_ignored() {
        assert_eq!(
            parse_fallback(br#"{"success":true,"city":"","latitude":1.0,"longitude":2.0}"#),
            None
        );
        assert_eq!(parse_fallback(br#"{"success":false}"#), None);
    }

    #[test]
    fn garbage_is_not_a_location() {
        assert_eq!(parse(b"not json"), None);
        assert_eq!(parse(b"{}"), None);
    }

    #[tokio::test]
    async fn private_addresses_never_leave_the_instance() {
        for host in ["10.0.0.5", "127.0.0.1", "192.168.1.20", "169.254.1.1"] {
            assert_eq!(
                public_ip_of(host).await,
                Err(Unlocated::PrivateAddress),
                "{host}"
            );
        }
    }

    #[tokio::test]
    async fn public_literals_pass_through_untouched() {
        assert_eq!(
            public_ip_of("159.69.120.1").await,
            Ok("159.69.120.1".parse::<IpAddr>().unwrap())
        );
    }
}
