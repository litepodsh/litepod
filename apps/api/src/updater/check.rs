use std::{cmp::Ordering, collections::HashMap, time::Duration};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::UpdateChannel;

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("manifest fetch failed: {0}")]
    Fetch(String),
    #[error("manifest is not valid JSON: {0}")]
    Parse(String),
    #[error("manifest does not publish the {0} channel")]
    UnknownChannel(String),
    #[error("manifest does not publish core service `{0}`")]
    MissingCoreService(String),
    #[error("manifest publishes invalid {service} tag `{tag}`")]
    InvalidCoreTag { service: String, tag: String },
}

/// A published version and its optional release metadata for one update channel.
#[derive(Debug, Clone, Deserialize)]
pub struct ChannelEntry {
    pub version: String,
    #[serde(default)]
    pub caddy: Option<CoreServiceEntry>,
    #[serde(default)]
    pub dragonfly: Option<CoreServiceEntry>,
    #[serde(default)]
    #[allow(dead_code)] // surfaced in release-note API response; consumers don't read it yet.
    pub released_at: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CoreServiceEntry {
    pub tag: String,
    #[serde(default)]
    #[allow(dead_code)] // populated by release pipeline; not yet displayed in update UI.
    pub released_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CoreServiceTargets {
    pub caddy: String,
    pub dragonfly: String,
}

/// The published release manifest, with root fields retained for stable-channel
/// compatibility with manifests published before channels existed.
///
/// Only the fields litepod reads are modelled and unknown keys are ignored, so
/// the manifest can grow without breaking instances running older code.
#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub channels: HashMap<String, ChannelEntry>,
    #[serde(default)]
    pub released_at: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

impl Manifest {
    /// Resolve a channel to its published entry. Stable falls back to the
    /// root-level version so older manifests remain compatible.
    pub fn channel(&self, channel: UpdateChannel) -> Result<ChannelEntry, ManifestError> {
        if let Some(entry) = self.channels.get(channel.as_str()) {
            return Ok(entry.clone());
        }

        if channel == UpdateChannel::Stable {
            if let Some(version) = &self.version {
                return Ok(ChannelEntry {
                    version: version.clone(),
                    caddy: None,
                    dragonfly: None,
                    released_at: self.released_at.clone(),
                    notes: self.notes.clone(),
                });
            }
        }

        Err(ManifestError::UnknownChannel(channel.as_str().to_string()))
    }
}

impl ChannelEntry {
    pub fn core_targets(&self) -> Result<CoreServiceTargets, ManifestError> {
        let caddy = self
            .caddy
            .as_ref()
            .ok_or_else(|| ManifestError::MissingCoreService("caddy".into()))?
            .tag
            .clone();
        let dragonfly = self
            .dragonfly
            .as_ref()
            .ok_or_else(|| ManifestError::MissingCoreService("dragonfly".into()))?
            .tag
            .clone();
        for (service, tag) in [("caddy", &caddy), ("dragonfly", &dragonfly)] {
            if !is_valid_core_tag(tag) {
                return Err(ManifestError::InvalidCoreTag {
                    service: service.into(),
                    tag: tag.clone(),
                });
            }
        }
        Ok(CoreServiceTargets { caddy, dragonfly })
    }
}

pub fn is_valid_core_tag(tag: &str) -> bool {
    tag != "latest"
        && !tag.is_empty()
        && tag.len() <= 128
        && tag.as_bytes()[0].is_ascii_alphanumeric()
        && tag
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

/// Fetch the release manifest. Anonymous GET — the bucket allows public reads
/// so instances never need registry or S3 credentials just to learn what the
/// current version is.
pub async fn fetch_manifest(url: &str) -> Result<Manifest, ManifestError> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| ManifestError::Fetch(e.to_string()))?;

    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| ManifestError::Fetch(e.to_string()))?;

    if !resp.status().is_success() {
        return Err(ManifestError::Fetch(format!("status {}", resp.status())));
    }

    let body = resp
        .text()
        .await
        .map_err(|e| ManifestError::Fetch(e.to_string()))?;

    serde_json::from_str(&body).map_err(|e| ManifestError::Parse(e.to_string()))
}

/// A release tag with an optional alpha counter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReleaseVersion {
    major: u64,
    minor: u64,
    patch: u64,
    // None represents a stable version, which sorts after alpha for a base.
    prerelease: Option<u64>,
}

impl Ord for ReleaseVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            .then_with(|| match (self.prerelease, other.prerelease) {
                (Some(left), Some(right)) => left.cmp(&right),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            })
    }
}

impl PartialOrd for ReleaseVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Parse `v0.1.19`, `0.1.19`, or `v0.1.19-alpha.3` into a comparable value.
///
/// Comparing tags as strings is wrong here — lexicographically `"0.1.19"`
/// sorts *before* `"0.1.9"` — and the tag format is produced by our own
/// release pipeline, so pulling in the `semver` crate for it is overkill.
pub fn parse_version(s: &str) -> Option<ReleaseVersion> {
    let trimmed = s.trim();
    let stripped = trimmed.strip_prefix('v').unwrap_or(trimmed);
    let (base, prerelease) = match stripped.split_once("-alpha.") {
        Some((base, counter)) => (base, Some(counter.parse().ok()?)),
        None => (stripped, None),
    };
    let mut parts = base.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some(ReleaseVersion {
        major,
        minor,
        patch,
        prerelease,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::updater::UpdateChannel;

    #[test]
    fn update_channel_parses_to_a_closed_set() {
        assert_eq!(UpdateChannel::parse("alpha"), UpdateChannel::Alpha);
        assert_eq!(UpdateChannel::parse(" ALPHA "), UpdateChannel::Alpha);
        assert_eq!(UpdateChannel::parse("stable"), UpdateChannel::Stable);
        assert_eq!(UpdateChannel::parse("unexpected"), UpdateChannel::Stable);
        assert_eq!(UpdateChannel::Alpha.as_str(), "alpha");
    }

    #[test]
    fn parse_version_accepts_optional_v_prefix() {
        assert!(parse_version("v0.1.19").is_some());
        assert!(parse_version("0.1.19").is_some());
        assert!(parse_version("  v1.2.3  ").is_some());
        assert!(parse_version("v1.2.3-alpha.4").is_some());
    }

    #[test]
    fn parse_version_rejects_malformed_input() {
        assert_eq!(parse_version(""), None);
        assert_eq!(parse_version("v1.2"), None);
        assert_eq!(parse_version("v1.2.3.4"), None);
        assert_eq!(parse_version("latest"), None);
        assert_eq!(parse_version("v1.2.x"), None);
        assert_eq!(parse_version("v1.2.3-rc.1"), None);
        assert_eq!(parse_version("v1.2.3-alpha"), None);
    }

    #[test]
    fn parse_version_orders_numerically_not_lexicographically() {
        // The whole reason this function exists: as strings, "0.1.19" sorts
        // *before* "0.1.9", which would make the updater refuse to move from
        // v0.1.9 to v0.1.19.
        let nineteen = parse_version("v0.1.19").unwrap();
        let nine = parse_version("v0.1.9").unwrap();
        assert!(nineteen > nine);
        assert!(parse_version("v0.2.0").unwrap() > parse_version("v0.1.99").unwrap());
        assert!(parse_version("v1.0.0").unwrap() > parse_version("v0.99.99").unwrap());
    }

    #[test]
    fn manifest_resolves_each_published_channel() {
        let manifest: Manifest = serde_json::from_str(
            r#"{
            "version":"v0.1.22",
            "channels": {
                "stable":{"version":"v0.1.22"},
                "alpha":{"version":"v0.1.23-alpha.2"}
            }
        }"#,
        )
        .unwrap();

        assert_eq!(
            manifest.channel(UpdateChannel::Stable).unwrap().version,
            "v0.1.22"
        );
        assert_eq!(
            manifest.channel(UpdateChannel::Alpha).unwrap().version,
            "v0.1.23-alpha.2"
        );
    }

    #[test]
    fn old_root_only_manifest_remains_a_stable_fallback() {
        let manifest: Manifest = serde_json::from_str(r#"{"version":"v0.1.22"}"#).unwrap();
        assert_eq!(
            manifest.channel(UpdateChannel::Stable).unwrap().version,
            "v0.1.22"
        );
        assert!(matches!(
            manifest.channel(UpdateChannel::Alpha),
            Err(ManifestError::UnknownChannel(_))
        ));
    }

    #[test]
    fn stable_sorting_wins_over_same_base_alpha_and_alpha_counter_increases() {
        assert!(parse_version("v1.2.3").unwrap() > parse_version("v1.2.3-alpha.9").unwrap());
        assert!(
            parse_version("v1.2.3-alpha.10").unwrap() > parse_version("v1.2.3-alpha.9").unwrap()
        );
    }

    #[test]
    fn released_at_and_notes_are_optional() {
        let manifest: Manifest =
            serde_json::from_str(r#"{"channels":{"stable":{"version":"v0.1.22"}}}"#).unwrap();
        let stable = manifest.channel(UpdateChannel::Stable).unwrap();
        assert!(stable.released_at.is_none());
        assert!(stable.notes.is_none());
    }

    #[test]
    fn manifest_rejects_corrupt_json() {
        assert!(serde_json::from_str::<Manifest>("{not json").is_err());
    }

    #[test]
    fn channel_resolves_its_own_core_targets() {
        let manifest: Manifest = serde_json::from_str(
            r#"{"channels":{"stable":{"version":"v0.1.22","caddy":{"tag":"2.11.4"},"dragonfly":{"tag":"v1.39.0"}},"alpha":{"version":"v0.1.23-alpha.2","caddy":{"tag":"2.12.0"},"dragonfly":{"tag":"v1.39.0"}}}}"#,
        )
        .unwrap();
        let stable = manifest
            .channel(UpdateChannel::Stable)
            .unwrap()
            .core_targets()
            .unwrap();
        let alpha = manifest
            .channel(UpdateChannel::Alpha)
            .unwrap()
            .core_targets()
            .unwrap();
        assert_eq!(stable.caddy, "2.11.4");
        assert_eq!(stable.dragonfly, "v1.39.0");
        assert_eq!(alpha.caddy, "2.12.0");
        assert_eq!(alpha.dragonfly, "v1.39.0");
    }

    #[test]
    fn core_targets_require_both_services_and_safe_tags() {
        let missing: Manifest =
            serde_json::from_str(r#"{"channels":{"stable":{"version":"v0.1.22"}}}"#).unwrap();
        assert!(matches!(
            missing.channel(UpdateChannel::Stable).unwrap().core_targets(),
            Err(ManifestError::MissingCoreService(ref service)) if service == "caddy"
        ));
        assert!(!is_valid_core_tag("latest"));
        assert!(!is_valid_core_tag("bad/tag"));
        assert!(is_valid_core_tag("2.11.4-alpine"));
    }
}
