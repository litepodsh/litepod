//! Outbound-URL validation for notification providers.
//!
//! A webhook URL is attacker-controllable input that the server then requests,
//! which is exactly the shape of TM-005 in `SECURITYTHREATS.md`. Rather than
//! repeat it, every provider gets a fixed host allowlist: there is no
//! legitimate reason for a "Discord webhook" to point anywhere but Discord.
//! Telegram has no user-supplied URL at all — the endpoint is built from the
//! bot token server-side.
//!
//! Checks run at create, at update, at test, and again immediately before each
//! send (DNS answers can change between the two).

use url::{Host, Url};

use super::provider::ProviderKind;

pub use crate::security::ssrf::resolve_validated;
#[cfg(test)]
use crate::security::ssrf::is_public;

/// Exact hosts, plus suffixes for tenant-specific hostnames.
struct HostRule {
    exact: &'static [&'static str],
    suffix: &'static [&'static str],
}

fn rule(kind: ProviderKind) -> HostRule {
    match kind {
        ProviderKind::Slack => HostRule {
            exact: &["hooks.slack.com"],
            suffix: &[],
        },
        ProviderKind::Discord => HostRule {
            exact: &["discord.com", "discordapp.com"],
            suffix: &[".discord.com", ".discordapp.com"],
        },
        // Teams "Incoming Webhook" and "Workflows" connectors both mint
        // per-tenant hostnames, so these have to be suffix rules.
        ProviderKind::MsTeams => HostRule {
            exact: &[],
            suffix: &[".webhook.office.com", ".logic.azure.com", ".logic.azure.us"],
        },
        // Never reached: Telegram URLs are constructed, not accepted.
        ProviderKind::Telegram => HostRule {
            exact: &["api.telegram.org"],
            suffix: &[],
        },
    }
}

/// Human-readable list of what a provider will accept, used in error messages
/// and mirrored by the UI so the two never disagree.
pub fn allowed_hosts_label(kind: ProviderKind) -> String {
    let rule = rule(kind);
    let mut parts: Vec<String> = rule.exact.iter().map(|h| (*h).to_string()).collect();
    parts.extend(rule.suffix.iter().map(|s| format!("*{s}")));
    parts.join(", ")
}

/// Parses and allowlists a user-supplied webhook URL. Returns the normalized
/// URL on success.
pub fn validate_webhook_url(kind: ProviderKind, raw: &str) -> Result<Url, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("Webhook URL is required".into());
    }
    let url = crate::security::ssrf::parse_https_url(raw)
        .map_err(|error| error.replace("URL", "Webhook URL"))?;

    let host = url
        .host()
        .ok_or_else(|| "Webhook URL must have a hostname".to_string())?;
    let host = match host {
        Host::Domain(domain) => domain.to_ascii_lowercase(),
        // A literal IP can never match an allowlisted domain, and accepting one
        // would skip the DNS step entirely.
        Host::Ipv4(_) | Host::Ipv6(_) => {
            return Err(format!(
                "Webhook URL must point at {}",
                allowed_hosts_label(kind)
            ));
        }
    };

    let rule = rule(kind);
    let allowed = rule.exact.contains(&host.as_str())
        || rule.suffix.iter().any(|suffix| host.ends_with(suffix));
    if !allowed {
        return Err(format!(
            "{} webhooks must point at {} — got {host}",
            kind.label(),
            allowed_hosts_label(kind)
        ));
    }

    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;
    use std::str::FromStr;

    #[test]
    fn each_provider_accepts_its_own_host() {
        assert!(
            validate_webhook_url(
                ProviderKind::Slack,
                "https://hooks.slack.com/services/T0/B0/XXXX"
            )
            .is_ok()
        );
        assert!(
            validate_webhook_url(
                ProviderKind::Discord,
                "https://discord.com/api/webhooks/123/abc"
            )
            .is_ok()
        );
        assert!(
            validate_webhook_url(
                ProviderKind::Discord,
                "https://discordapp.com/api/webhooks/123/abc"
            )
            .is_ok()
        );
        assert!(
            validate_webhook_url(
                ProviderKind::MsTeams,
                "https://contoso.webhook.office.com/webhookb2/abc"
            )
            .is_ok()
        );
        assert!(
            validate_webhook_url(
                ProviderKind::MsTeams,
                "https://prod-12.westus.logic.azure.com/workflows/abc"
            )
            .is_ok()
        );
    }

    #[test]
    fn providers_reject_each_others_hosts() {
        assert!(
            validate_webhook_url(
                ProviderKind::Slack,
                "https://discord.com/api/webhooks/123/abc"
            )
            .is_err()
        );
        assert!(
            validate_webhook_url(
                ProviderKind::Discord,
                "https://hooks.slack.com/services/T0/B0/XXXX"
            )
            .is_err()
        );
        assert!(
            validate_webhook_url(
                ProviderKind::MsTeams,
                "https://discord.com/api/webhooks/1/a"
            )
            .is_err()
        );
    }

    #[test]
    fn scheme_credentials_port_and_literal_ips_are_rejected() {
        for candidate in [
            "http://discord.com/api/webhooks/1/a",
            "https://user:pw@discord.com/api/webhooks/1/a",
            "https://discord.com:8443/api/webhooks/1/a",
            "https://127.0.0.1/api/webhooks/1/a",
            "https://169.254.169.254/latest/meta-data",
            "https://10.0.0.5/api/webhooks/1/a",
            "https://[::1]/api/webhooks/1/a",
            "https://evil.example.com/api/webhooks/1/a",
            "not-a-url",
            "",
        ] {
            assert!(
                validate_webhook_url(ProviderKind::Discord, candidate).is_err(),
                "should have been rejected: {candidate}"
            );
        }
    }

    #[test]
    fn a_suffix_rule_does_not_match_a_lookalike_domain() {
        // "evilwebhook.office.com.attacker.com" and
        // "notwebhook.office.com" must not slip through the suffix check.
        assert!(
            validate_webhook_url(
                ProviderKind::MsTeams,
                "https://contoso.webhook.office.com.attacker.com/x"
            )
            .is_err()
        );
        assert!(
            validate_webhook_url(ProviderKind::MsTeams, "https://webhook.office.com/x").is_err()
        );
    }

    #[test]
    fn private_and_special_addresses_are_not_public() {
        for candidate in [
            "0.0.0.0",
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "198.18.0.1",
            "240.0.0.1",
            "255.255.255.255",
            "::1",
            "::",
            "fc00::1",
            "fe80::1",
            "::ffff:127.0.0.1",
            "2001:db8::1",
        ] {
            assert!(
                !is_public(IpAddr::from_str(candidate).unwrap()),
                "should not be public: {candidate}"
            );
        }

        for candidate in ["1.1.1.1", "162.159.128.233", "2606:4700::1111"] {
            assert!(
                is_public(IpAddr::from_str(candidate).unwrap()),
                "should be public: {candidate}"
            );
        }
    }
}
