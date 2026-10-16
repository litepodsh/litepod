use std::{ffi::OsString, time::Duration};

use crate::servers::setup::{PACKAGE_TIMEOUT, QUICK_TIMEOUT, SetupStep};

/// Linux distro family, driving package-manager dispatch across `setup_plan()`,
/// `run_setup()`'s Caddy phase, and `update_packages()`. Mirrors
/// `apps/scripts/lib/distro.sh`'s `distro_family()` — keep both in sync when
/// adding a family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DistroFamily {
    Debian,
    Fedora,
    Arch,
}

/// Logical packages Setup needs, resolved per family by `DistroFamily::packages`.
/// Add a new variant + one match arm per family when a new package is needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicalPackage {
    Perl,
    BuildEssential,
    Which,
    Fail2ban,
    Podman,
    PodmanCompose,
    Uidmap,
    Slirp4netns,
    FuseOverlayfs,
}

impl DistroFamily {
    /// Matches `ID=` first, then falls back to `ID_LIKE=` tokens for derivatives
    /// (e.g. Pop!_OS -> Debian, Nobara -> Fedora, Manjaro/EndeavourOS -> Arch).
    /// `None` means the host is unrecognized (`ServerState::Unsupported`).
    pub fn detect(os_release: &str) -> Option<Self> {
        let id = os_release
            .lines()
            .find_map(|line| line.strip_prefix("ID="))
            .map(|value| value.trim_matches('"'));
        if let Some(family) = id.and_then(Self::from_id) {
            return Some(family);
        }
        os_release
            .lines()
            .find_map(|line| line.strip_prefix("ID_LIKE="))
            .map(|value| value.trim_matches('"'))
            .and_then(|id_like| id_like.split_whitespace().find_map(Self::from_id))
    }

    fn from_id(id: &str) -> Option<Self> {
        match id {
            "debian" | "ubuntu" => Some(Self::Debian),
            "fedora" | "rhel" => Some(Self::Fedora),
            "arch" => Some(Self::Arch),
            _ => None,
        }
    }

    /// Argv (no privilege prefix) that refreshes this family's package index.
    pub const fn refresh_argv(self) -> &'static [&'static str] {
        match self {
            Self::Debian => &["apt-get", "update"],
            Self::Fedora => &["dnf", "makecache"],
            Self::Arch => &["pacman", "-Sy"],
        }
    }

    /// Argv (no privilege prefix) that upgrades every installed package. Not used
    /// for Arch — see `update_packages()` in `setup.rs` for why a bare refresh
    /// followed by a separate upgrade call is unsafe there.
    pub const fn upgrade_argv(self) -> &'static [&'static str] {
        match self {
            Self::Debian => &[
                "env",
                "DEBIAN_FRONTEND=noninteractive",
                "apt-get",
                "upgrade",
                "-y",
            ],
            Self::Fedora => &["dnf", "upgrade", "-y"],
            Self::Arch => &["pacman", "-Syu", "--noconfirm"],
        }
    }

    /// Argv (no privilege prefix) that installs `packages` non-interactively.
    pub fn install_argv(self, packages: &[&'static str]) -> Vec<&'static str> {
        let mut argv: Vec<&'static str> = match self {
            Self::Debian => vec![
                "env",
                "DEBIAN_FRONTEND=noninteractive",
                "apt-get",
                "install",
                "-y",
            ],
            Self::Fedora => vec!["dnf", "install", "-y"],
            Self::Arch => vec!["pacman", "-S", "--noconfirm", "--needed"],
        };
        argv.extend_from_slice(packages);
        argv
    }

    /// One logical package -> zero or more real package names on this family.
    /// Empty means "already present in the base system" (e.g. Arch's `perl`,
    /// which is a dependency of `pacman` itself — VERIFY against a fresh image
    /// before relying on this in production; confirmed only for `podman`,
    /// `podman-compose` and `curl`/`openssl`/`git` via container smoke tests so
    /// far).
    pub const fn packages(self, logical: LogicalPackage) -> &'static [&'static str] {
        use LogicalPackage::*;
        match (self, logical) {
            (Self::Debian, Perl) => &["perl"],
            (Self::Debian, BuildEssential) => &["build-essential"],
            (Self::Debian, Which) => &["which"],
            (Self::Debian, Fail2ban) => &["fail2ban"],
            (Self::Debian, Podman) => &["podman"],
            (Self::Debian, PodmanCompose) => &["podman-compose"],
            (Self::Debian, Uidmap) => &["uidmap"],
            (Self::Debian, Slirp4netns) => &["slirp4netns"],
            (Self::Debian, FuseOverlayfs) => &["fuse-overlayfs"],

            (Self::Fedora, Perl) => &["perl"],
            // Fedora has no single "build-essential" meta-package; this is its
            // closest functional equivalent (gcc/g++/make ~= what
            // build-essential pulls via dpkg-dev on Debian).
            (Self::Fedora, BuildEssential) => &["gcc", "gcc-c++", "make"],
            (Self::Fedora, Which) => &["which"],
            (Self::Fedora, Fail2ban) => &["fail2ban"],
            (Self::Fedora, Podman) => &["podman"],
            (Self::Fedora, PodmanCompose) => &["podman-compose"],
            (Self::Fedora, Uidmap) => &["shadow-utils"],
            (Self::Fedora, Slirp4netns) => &["slirp4netns"],
            (Self::Fedora, FuseOverlayfs) => &["fuse-overlayfs"],

            // perl ships in Arch's `base` group already.
            (Self::Arch, Perl) => &[],
            (Self::Arch, BuildEssential) => &["base-devel"],
            (Self::Arch, Which) => &["which"],
            (Self::Arch, Fail2ban) => &["fail2ban"],
            (Self::Arch, Podman) => &["podman"],
            (Self::Arch, PodmanCompose) => &["podman-compose"],
            // newuidmap/newgidmap ship in Arch's base `shadow` package already.
            (Self::Arch, Uidmap) => &[],
            (Self::Arch, Slirp4netns) => &["slirp4netns"],
            (Self::Arch, FuseOverlayfs) => &["fuse-overlayfs"],
        }
    }

    /// Flattens several logical packages into one resolved, de-duplicated
    /// install list for this family, dropping packages that need no separate
    /// install.
    pub fn resolve_packages(self, logicals: &[LogicalPackage]) -> Vec<&'static str> {
        let mut resolved: Vec<&'static str> = Vec::new();
        for logical in logicals {
            for package in self.packages(*logical) {
                if !resolved.contains(package) {
                    resolved.push(package);
                }
            }
        }
        resolved
    }
}

fn step(
    name: &'static str,
    command: Vec<OsString>,
    timeout: Duration,
    tolerate_failure: bool,
) -> SetupStep {
    SetupStep {
        name,
        command,
        timeout,
        tolerate_failure,
    }
}

/// Caddy is not in Debian's or Fedora's default repositories, so those two need
/// their own repo-registration path before the shared "enable-native-caddy"
/// step (owned by `setup.rs`, appended after this) can start it. Arch is the
/// simple case: `caddy` has been in Arch's official `extra` repo (prebuilt,
/// not AUR) since 2.11.4 — confirmed live via `pacman -Si caddy` — so it's
/// just another `pacman -S` install, same shape as Podman itself.
pub fn caddy_install_steps(
    family: DistroFamily,
    escalated: &impl Fn(&[&str]) -> Vec<OsString>,
) -> Vec<SetupStep> {
    match family {
        DistroFamily::Debian => debian_caddy_steps(escalated),
        DistroFamily::Fedora => fedora_caddy_steps(escalated),
        DistroFamily::Arch => arch_caddy_steps(escalated),
    }
}

/// Caddy's own signing key + apt repository, mirroring the vendor's documented
/// install with one difference: the key and the source list are written
/// straight to disk with `curl -o` rather than piped through `gpg`/`tee`, so no
/// step needs a remote shell to interpret it.
const CADDY_KEY_URL: &str = "https://dl.cloudsmith.io/public/caddy/stable/gpg.key";
const CADDY_LIST_URL: &str = "https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt";
const CADDY_KEYRING_ASC: &str = "/usr/share/keyrings/caddy-stable-archive-keyring.asc";
const CADDY_KEYRING: &str = "/usr/share/keyrings/caddy-stable-archive-keyring.gpg";
const CADDY_LIST: &str = "/etc/apt/sources.list.d/caddy-stable.list";

fn debian_caddy_steps(escalated: &impl Fn(&[&str]) -> Vec<OsString>) -> Vec<SetupStep> {
    vec![
        step(
            "install-caddy-repository-tools",
            escalated(&[
                "env",
                "DEBIAN_FRONTEND=noninteractive",
                "apt-get",
                "install",
                "-y",
                "debian-keyring",
                "debian-archive-keyring",
                "apt-transport-https",
                "curl",
                "gpg",
            ]),
            PACKAGE_TIMEOUT,
            false,
        ),
        step(
            "fetch-caddy-signing-key",
            escalated(&["curl", "-1sLf", "-o", CADDY_KEYRING_ASC, CADDY_KEY_URL]),
            QUICK_TIMEOUT,
            false,
        ),
        step(
            "install-caddy-signing-key",
            // --yes so a re-run over an existing keyring is not an interactive overwrite prompt.
            escalated(&[
                "gpg",
                "--batch",
                "--yes",
                "--dearmor",
                "-o",
                CADDY_KEYRING,
                CADDY_KEYRING_ASC,
            ]),
            QUICK_TIMEOUT,
            false,
        ),
        step(
            "add-caddy-repository",
            escalated(&["curl", "-1sLf", "-o", CADDY_LIST, CADDY_LIST_URL]),
            QUICK_TIMEOUT,
            false,
        ),
        step(
            // apt ignores a keyring or a source list it cannot read as an unprivileged user.
            "permit-caddy-repository",
            escalated(&["chmod", "0644", CADDY_KEYRING, CADDY_LIST]),
            QUICK_TIMEOUT,
            false,
        ),
        step(
            "refresh-caddy-apt-index",
            escalated(&["apt-get", "update"]),
            PACKAGE_TIMEOUT,
            false,
        ),
        step(
            "install-native-caddy",
            escalated(&[
                "env",
                "DEBIAN_FRONTEND=noninteractive",
                "apt-get",
                "install",
                "-y",
                "caddy",
            ]),
            PACKAGE_TIMEOUT,
            false,
        ),
    ]
}

/// Fedora has no official Caddy package; the Caddy team maintains a Copr repo
/// (community-built, not part of Fedora's audited main repos, but it ships a
/// prebuilt binary rather than an arbitrary build script).
fn fedora_caddy_steps(escalated: &impl Fn(&[&str]) -> Vec<OsString>) -> Vec<SetupStep> {
    vec![
        step(
            "install-caddy-repository-tools",
            escalated(&["dnf", "install", "-y", "dnf-plugins-core"]),
            PACKAGE_TIMEOUT,
            false,
        ),
        step(
            "add-caddy-repository",
            escalated(&["dnf", "copr", "enable", "-y", "@caddy/caddy"]),
            QUICK_TIMEOUT,
            false,
        ),
        step(
            "install-native-caddy",
            escalated(&["dnf", "install", "-y", "caddy"]),
            PACKAGE_TIMEOUT,
            false,
        ),
    ]
}

fn arch_caddy_steps(escalated: &impl Fn(&[&str]) -> Vec<OsString>) -> Vec<SetupStep> {
    vec![step(
        "install-native-caddy",
        escalated(&["pacman", "-S", "--noconfirm", "--needed", "caddy"]),
        PACKAGE_TIMEOUT,
        false,
    )]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_matches_exact_ids() {
        assert_eq!(
            DistroFamily::detect("ID=ubuntu\n"),
            Some(DistroFamily::Debian)
        );
        assert_eq!(
            DistroFamily::detect("ID=debian\n"),
            Some(DistroFamily::Debian)
        );
        assert_eq!(
            DistroFamily::detect("ID=fedora\n"),
            Some(DistroFamily::Fedora)
        );
        assert_eq!(DistroFamily::detect("ID=arch\n"), Some(DistroFamily::Arch));
    }

    #[test]
    fn detect_falls_back_to_id_like_for_derivatives() {
        assert_eq!(
            DistroFamily::detect("ID=pop\nID_LIKE=ubuntu debian\n"),
            Some(DistroFamily::Debian)
        );
        assert_eq!(
            DistroFamily::detect("ID=nobara\nID_LIKE=fedora\n"),
            Some(DistroFamily::Fedora)
        );
        assert_eq!(
            DistroFamily::detect("ID=manjaro\nID_LIKE=arch\n"),
            Some(DistroFamily::Arch)
        );
    }

    #[test]
    fn detect_rejects_unknown_distros() {
        assert_eq!(DistroFamily::detect("ID=gentoo\n"), None);
        assert_eq!(DistroFamily::detect(""), None);
    }

    #[test]
    fn resolve_packages_dedupes_and_drops_empties() {
        let resolved = DistroFamily::Arch.resolve_packages(&[
            LogicalPackage::Podman,
            LogicalPackage::Uidmap,
            LogicalPackage::PodmanCompose,
        ]);
        assert_eq!(resolved, vec!["podman", "podman-compose"]);
    }

    #[test]
    fn install_argv_matches_each_family_native_invocation() {
        assert_eq!(
            DistroFamily::Debian.install_argv(&["curl"]),
            vec![
                "env",
                "DEBIAN_FRONTEND=noninteractive",
                "apt-get",
                "install",
                "-y",
                "curl"
            ]
        );
        assert_eq!(
            DistroFamily::Fedora.install_argv(&["curl"]),
            vec!["dnf", "install", "-y", "curl"]
        );
        assert_eq!(
            DistroFamily::Arch.install_argv(&["curl"]),
            vec!["pacman", "-S", "--noconfirm", "--needed", "curl"]
        );
    }
}
