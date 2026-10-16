use std::{net::IpAddr, str::FromStr};

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::dtos::response::AppError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum IngressMode {
    Central,
    PublicEdge,
    PrivateRelay,
    PrivateOnly,
}

impl IngressMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Central => "central",
            Self::PublicEdge => "public_edge",
            Self::PrivateRelay => "private_relay",
            Self::PrivateOnly => "private_only",
        }
    }

    pub const fn allows_public_domains(self) -> bool {
        !matches!(self, Self::PrivateOnly)
    }
}

impl FromStr for IngressMode {
    type Err = AppError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "central" => Ok(Self::Central),
            "public_edge" => Ok(Self::PublicEdge),
            "private_relay" => Ok(Self::PrivateRelay),
            "private_only" => Ok(Self::PrivateOnly),
            _ => Err(AppError::ValidationError(
                "Unknown server ingress mode".into(),
            )),
        }
    }
}

/// How Podman runs on a remote host. Rootless is the default: containers run as the
/// unprivileged SSH user, so a container escape does not become host root. Rootful runs the
/// system Podman instance and is opt-in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PodmanMode {
    #[default]
    Rootless,
    Rootful,
}

impl PodmanMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Rootless => "rootless",
            Self::Rootful => "rootful",
        }
    }

    /// Argv prefix that escalates a command. Root needs none; everyone else uses passwordless
    /// sudo, which Setup verifies before it touches the host.
    pub fn privilege_prefix(ssh_user: &str) -> &'static [&'static str] {
        if ssh_user == "root" {
            &[]
        } else {
            &["sudo", "-n"]
        }
    }

    /// Argv prefix for `podman` itself. Rootless always talks to the SSH user's own Podman.
    pub fn podman_prefix(self, ssh_user: &str) -> &'static [&'static str] {
        match self {
            Self::Rootless => &[],
            Self::Rootful => Self::privilege_prefix(ssh_user),
        }
    }
}

impl FromStr for PodmanMode {
    type Err = AppError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "rootless" => Ok(Self::Rootless),
            "rootful" => Ok(Self::Rootful),
            _ => Err(AppError::ValidationError("Unknown Podman mode".into())),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ServerState {
    NeedsVerification,
    NeedsSetup,
    SettingUp,
    Ready,
    Degraded,
    Offline,
    Unsupported,
}

impl ServerState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NeedsVerification => "needs_verification",
            Self::NeedsSetup => "needs_setup",
            Self::SettingUp => "setting_up",
            Self::Ready => "ready",
            Self::Degraded => "degraded",
            Self::Offline => "offline",
            Self::Unsupported => "unsupported",
        }
    }

    #[allow(dead_code)] // used by the future client-side "allowed transitions" surface.
    pub const fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (
                Self::NeedsVerification,
                Self::NeedsSetup | Self::Unsupported | Self::Offline
            ) | (
                Self::NeedsSetup,
                Self::SettingUp | Self::Offline | Self::Unsupported
            ) | (
                Self::SettingUp,
                Self::Ready | Self::NeedsSetup | Self::Degraded | Self::Unsupported
            ) | (
                Self::Ready,
                Self::SettingUp | Self::Degraded | Self::Offline
            ) | (
                // A health check - manual or from `servers::monitor` - restores a host that
                // started answering again, so recovery never requires a fresh setup run.
                Self::Degraded,
                Self::SettingUp | Self::Ready | Self::Offline | Self::Unsupported
            ) | (
                Self::Offline,
                Self::NeedsSetup | Self::Ready | Self::Degraded | Self::Unsupported
            ) | (Self::Unsupported, Self::NeedsVerification)
        )
    }
}

impl FromStr for ServerState {
    type Err = AppError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "needs_verification" => Ok(Self::NeedsVerification),
            "needs_setup" => Ok(Self::NeedsSetup),
            "setting_up" => Ok(Self::SettingUp),
            "ready" => Ok(Self::Ready),
            "degraded" => Ok(Self::Degraded),
            "offline" => Ok(Self::Offline),
            "unsupported" => Ok(Self::Unsupported),
            _ => Err(AppError::DatabaseError(
                "Unknown server state in database".into(),
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshTarget {
    pub host: String,
    pub port: u16,
    pub user: String,
}

impl SshTarget {
    pub fn parse(host: &str, port: u16, user: &str) -> Result<Self, AppError> {
        let host = host.trim();
        let user = user.trim();
        let hostname_is_valid = host.len() <= 253
            && !host.is_empty()
            && !host.starts_with('.')
            && !host.ends_with('.')
            && host.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && !label.starts_with('-')
                    && !label.ends_with('-')
                    && label
                        .chars()
                        .all(|value| value.is_ascii_alphanumeric() || value == '-')
            });
        if host.parse::<IpAddr>().is_err() && !hostname_is_valid {
            return Err(AppError::ValidationError(
                "SSH host must be a hostname or IP address".into(),
            ));
        }
        if user.is_empty()
            || user.len() > 32
            || !user
                .chars()
                .all(|value| value.is_ascii_alphanumeric() || matches!(value, '_' | '-' | '.'))
        {
            return Err(AppError::ValidationError(
                "SSH user contains unsupported characters".into(),
            ));
        }
        if port == 0 {
            return Err(AppError::ValidationError(
                "SSH port must be between 1 and 65535".into(),
            ));
        }
        Ok(Self {
            host: host.to_string(),
            port,
            user: user.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{IngressMode, PodmanMode, ServerState, SshTarget};

    #[test]
    fn podman_mode_decides_privilege_escalation() {
        assert!(PodmanMode::Rootless.podman_prefix("litepod").is_empty());
        assert!(PodmanMode::Rootless.podman_prefix("root").is_empty());
        assert_eq!(PodmanMode::Rootful.podman_prefix("litepod"), ["sudo", "-n"]);
        assert!(PodmanMode::Rootful.podman_prefix("root").is_empty());
        assert!(PodmanMode::privilege_prefix("root").is_empty());
    }

    #[test]
    fn ssh_target_rejects_shell_and_url_syntax() {
        assert!(SshTarget::parse("ops.example.com", 2222, "litepod").is_ok());
        assert!(SshTarget::parse("ssh://ops.example.com", 22, "litepod").is_err());
        assert!(SshTarget::parse("host;id", 22, "litepod").is_err());
    }

    #[test]
    fn states_and_modes_are_closed_sets() {
        assert!(ServerState::NeedsVerification.can_transition_to(ServerState::NeedsSetup));
        // A passing health check recovers an unreachable host without a new setup run.
        assert!(ServerState::Offline.can_transition_to(ServerState::Ready));
        assert!(ServerState::Degraded.can_transition_to(ServerState::Ready));
        assert!(!ServerState::NeedsVerification.can_transition_to(ServerState::Ready));
        assert!(!IngressMode::PrivateOnly.allows_public_domains());
    }
}
