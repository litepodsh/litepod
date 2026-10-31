//! Version of the root-owned `serverctl` script that Setup installs on a remote server.
//!
//! The script lives on the server, so it ages: a litepod update can need a subcommand an
//! older copy does not have. The script reports its version, health checks store it, and the
//! UI says what an outdated server is missing. Bump `HELPER_VERSION` and add a `CHANGELOG`
//! entry whenever the script changes; the tests fail until both match the script.

use std::ffi::OsString;

use crate::servers::{repository::ServerRecord, setup};

pub const HELPER_PATH: &str = "/usr/local/lib/litepod/serverctl";

/// The version the script in this build reports. Keep in step with `helper_version=` in it.
pub const HELPER_VERSION: u32 = 1;

pub struct HelperRelease {
    pub version: u32,
    pub summary: &'static str,
    pub changes: &'static [&'static str],
    /// SHA-256 of the script as released, so editing it without a new entry fails a test.
    pub script_sha256: &'static str,
}

pub const CHANGELOG: &[HelperRelease] = &[HelperRelease {
    version: 1,
    summary: "Versioned helper with basic authentication",
    changes: &[
        "Reports its own version (`serverctl version`)",
        "Applies basic authentication to domains (`basic-auth-check`)",
    ],
    script_sha256: "80482650597e907796ceb2528462baaf74991437eab2f986924431894a843f1b",
}];

/// Releases a server at `installed` is missing, oldest first.
pub fn releases_after(installed: u32) -> impl Iterator<Item = &'static HelperRelease> {
    CHANGELOG
        .iter()
        .filter(move |release| release.version > installed)
}

pub fn is_outdated(installed: u32) -> bool {
    installed < HELPER_VERSION
}

/// What the script on `server` reports: `Some(0)` for a copy too old to report anything,
/// `None` when the server could not be asked or has no script.
pub async fn installed_version(server: &ServerRecord) -> Option<u32> {
    match setup::caddy_helper(server, &[OsString::from("version")]).await {
        Ok(output) => output.trim().parse().ok(),
        // The script answered `version` with an error: it exists and predates versions.
        // An SSH failure fails this probe too, and says nothing about the script.
        Err(_) => setup::command(server, &["test", "-x", HELPER_PATH])
            .await
            .ok()
            .map(|_| 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn the_changelog_ends_at_the_current_version_without_gaps() {
        let versions: Vec<u32> = CHANGELOG.iter().map(|release| release.version).collect();
        assert_eq!(versions, (1..=HELPER_VERSION).collect::<Vec<_>>());
        assert!(
            CHANGELOG
                .iter()
                .all(|release| !release.summary.is_empty() && !release.changes.is_empty())
        );
    }

    #[test]
    fn the_script_reports_the_version_of_this_build() {
        assert!(
            setup::CADDY_HELPER.contains(&format!("helper_version={HELPER_VERSION}\n")),
            "set `helper_version={HELPER_VERSION}` in the script"
        );
    }

    #[test]
    fn editing_the_script_needs_a_new_release() {
        let current = hex::encode(Sha256::digest(setup::CADDY_HELPER.as_bytes()));
        let latest = CHANGELOG.last().unwrap();
        assert_eq!(
            latest.script_sha256, current,
            "the script changed: bump HELPER_VERSION, add a CHANGELOG entry whose script_sha256 is {current}"
        );
    }

    #[test]
    fn an_old_server_is_missing_only_the_newer_releases() {
        assert!(is_outdated(0));
        assert!(!is_outdated(HELPER_VERSION));
        assert_eq!(releases_after(0).count(), CHANGELOG.len());
        assert_eq!(releases_after(HELPER_VERSION).count(), 0);
    }
}
