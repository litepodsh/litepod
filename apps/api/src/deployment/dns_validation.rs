use std::{
    net::IpAddr,
    str::FromStr,
    sync::{OnceLock, RwLock},
    time::Duration,
};

use hickory_resolver::TokioResolver;
use hickory_resolver::proto::rr::RData;
use ipnet::IpNet;
use serde::Deserialize;

use crate::dtos::deployment::DomainDnsValidationDto;

const DNS_TIMEOUT: Duration = Duration::from_secs(3);
const CDN_FEED_TIMEOUT: Duration = Duration::from_secs(10);
const CDN_REFRESH_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const CDN_FEED_USER_AGENT: &str = "Litepod CDN range refresher";

const CLOUDFLARE_IPV4_URL: &str = "https://www.cloudflare.com/ips-v4";
const CLOUDFLARE_IPV6_URL: &str = "https://www.cloudflare.com/ips-v6";
const FASTLY_PUBLIC_IP_LIST_URL: &str = "https://api.fastly.com/public-ip-list";

// Published address blocks used before the first successful refresh, and if every later feed
// request fails. Validation itself always reads memory and never fetches provider feeds.
const CLOUDFLARE_RANGES: &[&str] = &[
    "173.245.48.0/20",
    "103.21.244.0/22",
    "103.22.200.0/22",
    "103.31.4.0/22",
    "141.101.64.0/18",
    "108.162.192.0/18",
    "190.93.240.0/20",
    "188.114.96.0/20",
    "197.234.240.0/22",
    "198.41.128.0/17",
    "162.158.0.0/15",
    "104.16.0.0/13",
    "104.24.0.0/14",
    "172.64.0.0/13",
    "131.0.72.0/22",
    "2400:cb00::/32",
    "2606:4700::/32",
    "2803:f800::/32",
    "2405:b500::/32",
    "2405:8100::/32",
    "2a06:98c0::/29",
    "2c0f:f248::/32",
];

const FASTLY_RANGES: &[&str] = &[
    "23.235.32.0/20",
    "43.249.72.0/22",
    "103.244.50.0/24",
    "103.245.222.0/23",
    "103.245.224.0/24",
    "104.156.80.0/20",
    "140.248.64.0/18",
    "140.248.128.0/17",
    "146.75.0.0/16",
    "151.101.0.0/16",
    "157.52.64.0/18",
    "167.82.0.0/17",
    "167.82.128.0/20",
    "167.82.160.0/20",
    "167.82.224.0/20",
    "172.111.64.0/18",
    "185.31.16.0/22",
    "199.27.72.0/21",
    "199.232.0.0/16",
    "2a04:4e42::/32",
];

struct ProviderRanges {
    name: &'static str,
    values: RwLock<Vec<IpNet>>,
}

impl ProviderRanges {
    fn with_fallback(name: &'static str, fallback: &[&str]) -> Self {
        Self {
            name,
            values: RwLock::new(
                parse_cidrs(fallback.iter().copied()).expect("CDN CIDR constants must be valid"),
            ),
        }
    }

    fn contains(&self, address: &IpAddr) -> bool {
        self.values
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .any(|range| range.contains(address))
    }

    fn replace(&self, next: Vec<IpNet>) {
        debug_assert!(!next.is_empty());
        *self
            .values
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = next;
    }
}

struct CdnRangeCache {
    cloudflare: ProviderRanges,
    fastly: ProviderRanges,
}

impl CdnRangeCache {
    fn with_fallbacks() -> Self {
        Self {
            cloudflare: ProviderRanges::with_fallback("Cloudflare", CLOUDFLARE_RANGES),
            fastly: ProviderRanges::with_fallback("Fastly", FASTLY_RANGES),
        }
    }
}

fn cdn_ranges() -> &'static CdnRangeCache {
    static CACHE: OnceLock<CdnRangeCache> = OnceLock::new();
    CACHE.get_or_init(CdnRangeCache::with_fallbacks)
}

fn detect_cdn(address: &IpAddr) -> Option<&'static str> {
    let cache = cdn_ranges();
    [&cache.cloudflare, &cache.fastly]
        .into_iter()
        .find(|provider| provider.contains(address))
        .map(|provider| provider.name)
}

fn parse_cidrs<'a>(values: impl IntoIterator<Item = &'a str>) -> Result<Vec<IpNet>, String> {
    let ranges = values
        .into_iter()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            value
                .parse::<IpNet>()
                .map_err(|error| format!("invalid CIDR {value:?}: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if ranges.is_empty() {
        Err("feed did not contain any CIDR ranges".to_string())
    } else {
        Ok(ranges)
    }
}

fn parse_plain_cidrs(body: &str) -> Result<Vec<IpNet>, String> {
    parse_cidrs(body.lines())
}

#[derive(Deserialize)]
struct FastlyPublicIpList {
    addresses: Vec<String>,
    ipv6_addresses: Vec<String>,
}

fn parse_fastly_cidrs(body: &str) -> Result<Vec<IpNet>, String> {
    let response: FastlyPublicIpList =
        serde_json::from_str(body).map_err(|error| format!("invalid Fastly response: {error}"))?;
    parse_cidrs(
        response
            .addresses
            .iter()
            .chain(response.ipv6_addresses.iter())
            .map(String::as_str),
    )
}

async fn fetch_text(request: reqwest::RequestBuilder) -> Result<String, String> {
    let response = request
        .send()
        .await
        .map_err(|error| format!("request failed: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("request returned HTTP {status}"));
    }
    response
        .text()
        .await
        .map_err(|error| format!("could not read response: {error}"))
}

async fn refresh_cloudflare(client: &reqwest::Client) -> Result<(), String> {
    let (ipv4, ipv6) = tokio::try_join!(
        fetch_text(client.get(CLOUDFLARE_IPV4_URL)),
        fetch_text(client.get(CLOUDFLARE_IPV6_URL)),
    )?;
    let mut ranges = parse_plain_cidrs(&ipv4)?;
    ranges.extend(parse_plain_cidrs(&ipv6)?);
    cdn_ranges().cloudflare.replace(ranges);
    Ok(())
}

async fn refresh_fastly(client: &reqwest::Client) -> Result<bool, String> {
    let Ok(token) = std::env::var("FASTLY_API_TOKEN") else {
        return Ok(false);
    };
    if token.trim().is_empty() {
        return Ok(false);
    }
    let body = fetch_text(
        client
            .get(FASTLY_PUBLIC_IP_LIST_URL)
            .header("Fastly-Key", token),
    )
    .await?;
    cdn_ranges().fastly.replace(parse_fastly_cidrs(&body)?);
    Ok(true)
}

async fn refresh_cdn_ranges(client: &reqwest::Client) {
    match refresh_cloudflare(client).await {
        Ok(()) => {}
        Err(error) => {
            tracing::warn!(provider = "Cloudflare", error = %error, "could not refresh CDN IP ranges; keeping the current ranges");
        }
    }
    match refresh_fastly(client).await {
        Ok(true) => {}
        Ok(false) => tracing::debug!(
            "skipping Fastly IP range refresh because FASTLY_API_TOKEN is not configured"
        ),
        Err(error) => {
            tracing::warn!(provider = "Fastly", error = %error, "could not refresh CDN IP ranges; keeping the current ranges")
        }
    }
}

/// Runs an immediate best-effort refresh, then refreshes provider ranges daily. DNS validation
/// remains available throughout because the cache starts with embedded fallback CIDRs.
pub fn spawn_cdn_range_refresh() {
    tokio::spawn(async move {
        let client = match reqwest::Client::builder()
            .timeout(CDN_FEED_TIMEOUT)
            .user_agent(CDN_FEED_USER_AGENT)
            .build()
        {
            Ok(client) => client,
            Err(error) => {
                tracing::warn!(error = %error, "could not create CDN range refresh client");
                return;
            }
        };
        refresh_cdn_ranges(&client).await;

        let mut ticker = tokio::time::interval(CDN_REFRESH_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        ticker.tick().await;
        loop {
            ticker.tick().await;
            refresh_cdn_ranges(&client).await;
        }
    });
}

fn addresses(values: &[IpAddr]) -> Vec<String> {
    values.iter().map(ToString::to_string).collect()
}

fn failed(status: &str, message: String, cname: Option<String>) -> DomainDnsValidationDto {
    DomainDnsValidationDto {
        status: status.into(),
        resolved_addresses: Vec::new(),
        expected_addresses: Vec::new(),
        cname,
        cdn_provider: None,
        message: Some(message),
    }
}

pub(crate) fn evaluate(
    resolved: Vec<IpAddr>,
    expected: Vec<IpAddr>,
    cname: Option<String>,
) -> DomainDnsValidationDto {
    let mut result = DomainDnsValidationDto {
        status: "valid".into(),
        resolved_addresses: addresses(&resolved),
        expected_addresses: addresses(&expected),
        cname,
        cdn_provider: None,
        message: None,
    };
    if resolved.is_empty() {
        result.status = "unresolved".into();
        result.message = Some("DNS did not return an A or AAAA address".into());
    } else if let Some(provider) = resolved.iter().find_map(detect_cdn) {
        result.status = "cdn".into();
        result.cdn_provider = Some(provider.into());
        result.message = Some(format!(
            "Domain resolves through {provider}; direct IP matching is not expected."
        ));
    } else if !resolved.iter().any(|address| expected.contains(address)) {
        result.status = "invalid".into();
        result.message = Some(format!(
            "Domain resolves to {} but should point to {}.",
            result.resolved_addresses.join(", "),
            result.expected_addresses.join(", "),
        ));
    }
    result
}

/// A, AAAA and the first CNAME hop for `host`. "No such record" is an empty
/// answer, not an error: only timeouts and resolver failures are `Err`.
async fn resolve(host: &str) -> Result<(Vec<IpAddr>, Option<String>), String> {
    if let Ok(address) = IpAddr::from_str(host) {
        return Ok((vec![address], None));
    }
    let mut builder = TokioResolver::builder_tokio().map_err(|error| error.to_string())?;
    builder.options_mut().timeout = DNS_TIMEOUT;
    builder.options_mut().attempts = 1;
    let resolver = builder.build().map_err(|error| error.to_string())?;
    let lookup = match tokio::time::timeout(DNS_TIMEOUT, resolver.lookup_ip(host)).await {
        Err(_) => return Err("lookup timed out".into()),
        Ok(Err(error)) if error.is_no_records_found() => return Ok((Vec::new(), None)),
        Ok(Err(error)) => return Err(error.to_string()),
        Ok(Ok(lookup)) => lookup,
    };
    let cname = lookup
        .as_lookup()
        .answers()
        .iter()
        .find_map(|record| match &record.data {
            RData::CNAME(target) => Some(target.0.to_string().trim_end_matches('.').to_string()),
            _ => None,
        });
    let mut addresses = lookup.iter().collect::<Vec<_>>();
    addresses.sort_unstable();
    addresses.dedup();
    Ok((addresses, cname))
}

/// Resolves `host` and compares it with every address the targets resolve to
/// (one server, or each member of a server group).
pub async fn validate(host: &str, expected_targets: &[String]) -> DomainDnsValidationDto {
    let (resolved, cname) = match resolve(host).await {
        Ok(found) => found,
        Err(error) => {
            return failed(
                "error",
                format!("Could not resolve DNS for {host}: {error}"),
                None,
            );
        }
    };
    let mut expected = Vec::new();
    for target in expected_targets {
        match resolve(target).await {
            Ok((addresses, _)) if !addresses.is_empty() => expected.extend(addresses),
            Ok(_) => {
                return failed(
                    "error",
                    format!(
                        "DNS did not return an A or AAAA address for the server target {target}"
                    ),
                    cname,
                );
            }
            Err(error) => {
                return failed(
                    "error",
                    format!("Could not resolve the server target {target}: {error}"),
                    cname,
                );
            }
        }
    }
    expected.sort_unstable();
    expected.dedup();
    evaluate(resolved, expected, cname)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(value: &str) -> IpAddr {
        value.parse().unwrap()
    }

    #[test]
    fn a_matching_address_is_valid() {
        let result = evaluate(vec![ip("203.0.113.10")], vec![ip("203.0.113.10")], None);
        assert_eq!(result.status, "valid");
        assert_eq!(result.cdn_provider, None);
    }

    #[test]
    fn an_unrecognized_mismatch_is_invalid() {
        let result = evaluate(vec![ip("203.0.113.11")], vec![ip("203.0.113.10")], None);
        assert_eq!(result.status, "invalid");
        assert!(result.message.unwrap().contains("203.0.113.10"));
    }

    #[test]
    fn a_known_cdn_is_valid_with_a_provider_warning() {
        let result = evaluate(vec![ip("173.245.48.5")], vec![ip("203.0.113.10")], None);
        assert_eq!(result.status, "cdn");
        assert_eq!(result.cdn_provider.as_deref(), Some("Cloudflare"));
    }

    #[test]
    fn ipv6_match_is_valid() {
        let result = evaluate(vec![ip("2001:db8::10")], vec![ip("2001:db8::10")], None);
        assert_eq!(result.status, "valid");
    }

    #[test]
    fn no_address_is_unresolved_and_keeps_the_cname() {
        let result = evaluate(
            vec![],
            vec![ip("203.0.113.10")],
            Some("edge.example".into()),
        );
        assert_eq!(result.status, "unresolved");
        assert_eq!(result.cname.as_deref(), Some("edge.example"));
    }

    #[test]
    fn any_server_of_a_group_is_valid() {
        let result = evaluate(
            vec![ip("198.51.100.7")],
            vec![ip("203.0.113.10"), ip("198.51.100.7")],
            None,
        );
        assert_eq!(result.status, "valid");
    }

    #[test]
    fn fastly_address_is_recognized() {
        assert_eq!(detect_cdn(&ip("151.101.1.1")), Some("Fastly"));
    }

    #[test]
    fn parses_plain_text_cidrs_and_rejects_empty_or_invalid_feeds() {
        assert_eq!(
            parse_plain_cidrs("203.0.113.0/24\n2001:db8::/32\n")
                .unwrap()
                .len(),
            2
        );
        assert!(parse_plain_cidrs("not-a-cidr").is_err());
        assert!(parse_plain_cidrs("\n").is_err());
    }

    #[test]
    fn parses_fastly_public_ip_list() {
        let ranges = parse_fastly_cidrs(
            r#"{"addresses":["203.0.113.0/24"],"ipv6_addresses":["2001:db8::/32"]}"#,
        )
        .unwrap();
        assert_eq!(ranges.len(), 2);
    }

    #[test]
    fn invalid_refresh_payload_leaves_existing_provider_ranges_unchanged() {
        let provider = ProviderRanges::with_fallback("Test CDN", &["203.0.113.0/24"]);
        let address = ip("203.0.113.5");
        assert!(provider.contains(&address));
        assert!(parse_plain_cidrs("invalid").is_err());
        assert!(provider.contains(&address));

        provider.replace(parse_plain_cidrs("198.51.100.0/24").unwrap());
        assert!(!provider.contains(&address));
        assert!(provider.contains(&ip("198.51.100.5")));
    }
}
