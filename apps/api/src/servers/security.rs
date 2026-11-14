//! Hardening checks and guided fixes for a remote server. Every action runs the closed,
//! root-owned `server-security` script; request data only selects one of its subcommands.

use std::{ffi::OsString, time::Duration};

use serde::{Deserialize, Serialize};

use crate::{
    dtos::response::AppError,
    servers::{
        distro::{DistroFamily, LogicalPackage},
        model::PodmanMode,
        repository::ServerRecord,
        setup::{
            self, MaintenanceOutcome, configure_fail2ban, credentials, install_root_file,
            maintenance_command, maintenance_outcome, run_maintenance_command, target,
        },
        ssh::SshExecutor,
    },
};

const SCRIPT_PATH: &str = "/usr/local/lib/litepod/server-security";
const SSH_CANDIDATE_PATH: &str = "/usr/local/lib/litepod/ssh-hardening.conf";

/// `ssh-apply` arms a 120 s timer (a systemd timer, or a detached sleep on OpenRC) that restores the previous sshd settings unless a
/// brand-new login runs `ssh-confirm` first, so a change that locks litepod out undoes itself.
const SCRIPT: &str = r#"#!/bin/sh
set -eu
PATH=/usr/sbin:/usr/bin:/sbin:/bin
self=/usr/local/lib/litepod/server-security
conf=/etc/ssh/sshd_config.d/00-litepod-hardening.conf
candidate=/usr/local/lib/litepod/ssh-hardening.conf
previous=/usr/local/lib/litepod/ssh-hardening.previous
pending=/usr/local/lib/litepod/ssh-hardening.pending
unit=litepod-ssh-revert

reload_ssh() {
    if command -v systemctl >/dev/null 2>&1; then
        systemctl reload ssh 2>/dev/null || systemctl reload sshd
    else
        rc-service sshd reload
    fi
}

# systemd hosts arm the revert with a transient timer. OpenRC hosts (Alpine) run a detached
# sleep instead; it needs no cancelling, because ssh_revert does nothing once confirm has
# removed the pending marker.
revert_pid=/usr/local/lib/litepod/ssh-revert.pid

disarm_revert() {
    if command -v systemctl >/dev/null 2>&1; then
        systemctl stop "$unit.timer" 2>/dev/null || true
    elif [ -f "$revert_pid" ]; then
        kill "$(cat "$revert_pid")" 2>/dev/null || true
        rm -f "$revert_pid"
    fi
}

arm_revert() {
    disarm_revert
    if command -v systemd-run >/dev/null 2>&1; then
        # Stopping an elapsed transient timer unloads it, so the unit name is free again.
        systemctl reset-failed "$unit.timer" "$unit.service" 2>/dev/null || true
        systemd-run --quiet --unit="$unit" --on-active=120 "$self" ssh-revert
    else
        nohup setsid sh -c 'sleep 120; exec "$1" ssh-revert' sh "$self" >/dev/null 2>&1 &
        echo $! > "$revert_pid"
    fi
}

service_active() {
    if command -v systemctl >/dev/null 2>&1; then
        systemctl is-active --quiet "$1"
    else
        rc-service "$1" status >/dev/null 2>&1
    fi
}

ssh_value() {
    sshd -T 2>/dev/null | awk -v key="$1" '$1 == key { print $2; exit }'
}

ssh_restore() {
    if [ -f "$previous" ]; then mv "$previous" "$conf"; else rm -f "$conf"; fi
}

audit() {
    echo "ssh_passwordauthentication=$(ssh_value passwordauthentication)"
    kbd=$(ssh_value kbdinteractiveauthentication)
    if [ -z "$kbd" ]; then kbd=$(ssh_value challengeresponseauthentication); fi
    echo "ssh_kbdinteractiveauthentication=$kbd"
    echo "ssh_permitrootlogin=$(ssh_value permitrootlogin)"
    if [ -f "$conf" ]; then echo "ssh_level=$(sed -n 's/^# level: //p' "$conf")"; fi
    if command -v ufw >/dev/null 2>&1; then
        if ufw status 2>/dev/null | grep -q '^Status: active'; then echo firewall=ufw:active; else echo firewall=ufw:inactive; fi
    elif command -v firewall-cmd >/dev/null 2>&1; then
        if firewall-cmd --state >/dev/null 2>&1; then echo firewall=firewalld:active; else echo firewall=firewalld:inactive; fi
    fi
    if ! command -v fail2ban-client >/dev/null 2>&1; then
        echo fail2ban=missing
    elif service_active fail2ban && fail2ban-client status sshd >/dev/null 2>&1; then
        echo fail2ban=active
        echo "fail2ban_banned=$(fail2ban-client status sshd | awk -F: '/Currently banned/ { gsub(/[ \t]/, "", $2); print $2 }')"
    else
        echo fail2ban=inactive
    fi
    if command -v apt-get >/dev/null 2>&1; then
        if dpkg-query -W -f='${Status}' unattended-upgrades 2>/dev/null | grep -q 'ok installed' \
            && apt-config dump APT::Periodic::Unattended-Upgrade 2>/dev/null | grep -q '"1"'; then
            echo auto_updates=on
        else
            echo auto_updates=off
        fi
        # Against the package index as last refreshed; the audit does not refresh it.
        echo "security_updates=$(apt-get -s upgrade 2>/dev/null | grep -ci '^Inst .*security' || true)"
    fi
    if [ -e /var/run/reboot-required ]; then echo reboot_required=yes; fi
    if command -v ss >/dev/null 2>&1; then
        ss -Htln 2>/dev/null | awk '{ print "listen=" $4 }' || true
    else
        # busybox has no ss
        netstat -tln 2>/dev/null | awk '$1 ~ /^tcp/ { print "listen=" $4 }' || true
    fi
}

ssh_apply() {
    if ! command -v systemd-run >/dev/null 2>&1 && ! command -v setsid >/dev/null 2>&1; then
        echo "systemd-run or setsid is required to apply SSH settings safely." >&2
        exit 1
    fi
    [ -f "$candidate" ] || { echo "No staged SSH settings." >&2; exit 1; }
    if [ -n "$(find "$pending" -mmin -3 2>/dev/null)" ]; then
        echo "A previous SSH change is still being verified; try again in two minutes." >&2
        exit 1
    fi
    rm -f "$pending"
    mkdir -p /etc/ssh/sshd_config.d
    if [ -f "$conf" ]; then cp "$conf" "$previous"; else rm -f "$previous"; fi
    # Until the revert timer is armed, any exit puts the old file back so nothing applies later.
    trap 'rm -f "$pending"; ssh_restore' EXIT
    install -m 0644 "$candidate" "$conf"
    if ! sshd -t; then
        echo "sshd rejected the new settings; nothing changed." >&2
        exit 1
    fi
    # sshd -T reads the files on disk, so this proves the drop-in wins before anything reloads.
    if [ "$(ssh_value passwordauthentication)" != no ]; then
        echo "sshd ignores $conf (sshd_config has no Include for sshd_config.d, or an earlier line sets PasswordAuthentication); nothing changed." >&2
        exit 1
    fi
    touch "$pending"
    arm_revert
    trap - EXIT
    reload_ssh
    echo "SSH settings applied; waiting for litepod to confirm it can still log in."
}

# Exactly one of confirm and revert wins the pending marker, so a late confirm cannot report
# success for settings the timer already rolled back.
ssh_confirm() {
    disarm_revert
    if rm "$pending" 2>/dev/null; then
        rm -f "$previous"
        echo "Confirmed: litepod logged in again with the new SSH settings."
    else
        echo "Too late: the server already restored its previous SSH settings." >&2
        exit 1
    fi
}

ssh_revert() {
    rm "$pending" 2>/dev/null || exit 0
    ssh_restore
    reload_ssh
}

auto_updates() {
    command -v apt-get >/dev/null 2>&1 || { echo "Automatic updates are set up on Debian and Ubuntu only." >&2; exit 1; }
    DEBIAN_FRONTEND=noninteractive apt-get install -y unattended-upgrades
    printf 'APT::Periodic::Update-Package-Lists "1";\nAPT::Periodic::Unattended-Upgrade "1";\n' > /etc/apt/apt.conf.d/20auto-upgrades
    systemctl enable --now unattended-upgrades >/dev/null 2>&1 || true
    echo "Security updates now install automatically every day. The server never reboots on its own."
}

case "${1:-}" in
    audit) audit ;;
    ssh-apply) ssh_apply ;;
    ssh-confirm) ssh_confirm ;;
    ssh-revert) ssh_revert ;;
    auto-updates) auto_updates ;;
    *) echo "usage: server-security audit|ssh-apply|ssh-confirm|ssh-revert|auto-updates" >&2; exit 2 ;;
esac
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SecurityAction {
    SshRecommended,
    SshStrict,
    Fail2ban,
    AutoUpdates,
}

impl SecurityAction {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SshRecommended => "ssh_recommended",
            Self::SshStrict => "ssh_strict",
            Self::Fail2ban => "fail2ban",
            Self::AutoUpdates => "auto_updates",
        }
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ApplySecurityRequest {
    pub action: SecurityAction,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct SshSettingsDto {
    pub recommended: String,
    pub strict: String,
}

#[derive(Debug, Clone, Default, Serialize, utoipa::ToSchema)]
pub struct ServerSecurityDto {
    /// sshd accepts passwords (directly or through keyboard-interactive). `None` when sshd
    /// could not report its effective settings.
    pub password_login: Option<bool>,
    /// sshd's effective `PermitRootLogin`.
    pub root_login: Option<String>,
    /// "recommended" or "strict" while litepod's sshd drop-in is in place.
    pub ssh_level: Option<String>,
    /// "ufw" or "firewalld"; `None` when neither is installed.
    pub firewall: Option<String>,
    pub firewall_active: bool,
    /// "active", "inactive" or "missing".
    pub fail2ban: String,
    pub banned_ips: Option<u32>,
    /// `None` on distros where litepod does not set up automatic updates.
    pub auto_updates: Option<bool>,
    pub security_updates: Option<u32>,
    pub reboot_required: bool,
    /// TCP ports listening on anything other than loopback.
    pub public_ports: Vec<u16>,
    /// What each SSH option writes, for operators who apply it by hand.
    pub ssh_settings: Option<SshSettingsDto>,
}

fn ssh_settings(action: SecurityAction) -> &'static str {
    match action {
        SecurityAction::SshStrict => {
            "# Managed by litepod. Delete this file and reload sshd to undo.\n# level: strict\nPasswordAuthentication no\nKbdInteractiveAuthentication no\nPermitEmptyPasswords no\nPermitRootLogin no\nMaxAuthTries 3\nX11Forwarding no\n"
        }
        // TCP forwarding stays allowed: litepod tunnels Podman and the terminal through it.
        _ => {
            "# Managed by litepod. Delete this file and reload sshd to undo.\n# level: recommended\nPasswordAuthentication no\nKbdInteractiveAuthentication no\nPermitEmptyPasswords no\nPermitRootLogin prohibit-password\n"
        }
    }
}

fn parse_audit(output: &str) -> ServerSecurityDto {
    let mut report = ServerSecurityDto {
        fail2ban: "missing".into(),
        ..Default::default()
    };
    let mut password = None;
    let mut keyboard = None;
    for (key, value) in output.lines().filter_map(|line| line.split_once('=')) {
        let value = value.trim();
        let known = |value: &str| (!value.is_empty()).then(|| value.to_string());
        match key {
            "ssh_passwordauthentication" => password = known(value),
            "ssh_kbdinteractiveauthentication" => keyboard = known(value),
            "ssh_permitrootlogin" => report.root_login = known(value),
            "ssh_level" => report.ssh_level = known(value),
            "firewall" => {
                if let Some((tool, state)) = value.split_once(':') {
                    report.firewall = Some(tool.into());
                    report.firewall_active = state == "active";
                }
            }
            "fail2ban" => report.fail2ban = value.into(),
            "fail2ban_banned" => report.banned_ips = value.parse().ok(),
            "auto_updates" => report.auto_updates = Some(value == "on"),
            "security_updates" => report.security_updates = value.parse().ok(),
            "reboot_required" => report.reboot_required = value == "yes",
            "listen" => {
                if let Some(port) = public_port(value) {
                    report.public_ports.push(port);
                }
            }
            _ => {}
        }
    }
    report.password_login =
        password.map(|password| password == "yes" || keyboard.as_deref() == Some("yes"));
    report.public_ports.sort_unstable();
    report.public_ports.dedup();
    report
}

/// `ss` local addresses: `0.0.0.0:22`, `[::]:80`, `*:443`, `127.0.0.53%lo:53`, `[::1]:631`.
fn public_port(local: &str) -> Option<u16> {
    let (address, port) = local.rsplit_once(':')?;
    let address = address.trim_matches(['[', ']']);
    let address = address.split('%').next().unwrap_or(address);
    if address.starts_with("127.") || address == "::1" {
        return None;
    }
    port.parse().ok()
}

fn sudo(server: &ServerRecord) -> &'static [&'static str] {
    PodmanMode::privilege_prefix(server.ssh_user.as_deref().unwrap_or_default())
}

async fn install_script(server: &ServerRecord) -> Result<(), AppError> {
    install_root_file(server, SCRIPT, "server-security", SCRIPT_PATH, "0755").await
}

pub async fn audit(server: &ServerRecord) -> Result<ServerSecurityDto, AppError> {
    install_script(server).await?;
    let run = run_maintenance_command(
        server,
        maintenance_command(sudo(server), &[SCRIPT_PATH, "audit"]),
    )
    .await?;
    if run.exit_code != 0 {
        return Err(AppError::ValidationError(format!(
            "Security check failed: {}",
            run.stderr.trim()
        )));
    }
    let mut report = parse_audit(&run.stdout);
    report.ssh_settings = Some(SshSettingsDto {
        recommended: ssh_settings(SecurityAction::SshRecommended).into(),
        strict: ssh_settings(SecurityAction::SshStrict).into(),
    });
    Ok(report)
}

pub async fn apply(
    server: &ServerRecord,
    action: SecurityAction,
) -> Result<MaintenanceOutcome, AppError> {
    let started = std::time::Instant::now();
    match action {
        SecurityAction::SshRecommended | SecurityAction::SshStrict => {
            apply_ssh(server, action, started).await
        }
        SecurityAction::Fail2ban => {
            let os_release = setup::command(server, &["cat", "/etc/os-release"]).await?;
            let family = DistroFamily::detect(&os_release).ok_or_else(|| {
                AppError::ValidationError(
                    "Could not determine the server's Linux distribution".into(),
                )
            })?;
            if family == DistroFamily::Alpine {
                return Err(AppError::ValidationError(
                    "Fail2ban is not set up on Alpine Linux servers".into(),
                ));
            }
            let packages = family.resolve_packages(&[LogicalPackage::Fail2ban]);
            let install = sudo(server)
                .iter()
                .copied()
                .chain(family.install_argv(&packages))
                .collect::<Vec<_>>();
            let run = run_maintenance_command(server, maintenance_command(&[], &install)).await?;
            if run.exit_code != 0 {
                return Ok(maintenance_outcome(action.as_str(), started, run));
            }
            configure_fail2ban(server).await?;
            Ok(MaintenanceOutcome {
                action: action.as_str().into(),
                success: true,
                stdout: "fail2ban is running and protects the SSH port.".into(),
                stderr: String::new(),
                duration_ms: started.elapsed().as_millis() as u64,
            })
        }
        SecurityAction::AutoUpdates => {
            install_script(server).await?;
            let run = run_maintenance_command(
                server,
                maintenance_command(sudo(server), &[SCRIPT_PATH, "auto-updates"]),
            )
            .await?;
            Ok(maintenance_outcome(action.as_str(), started, run))
        }
    }
}

async fn apply_ssh(
    server: &ServerRecord,
    action: SecurityAction,
    started: std::time::Instant,
) -> Result<MaintenanceOutcome, AppError> {
    if action == SecurityAction::SshStrict && server.ssh_user.as_deref() == Some("root") {
        return Err(AppError::ValidationError(
            "Strict SSH turns root login off, and litepod connects to this server as root. Reconnect it with a sudo user first.".into(),
        ));
    }
    install_script(server).await?;
    install_root_file(
        server,
        ssh_settings(action),
        "ssh-hardening.conf",
        SSH_CANDIDATE_PATH,
        "0644",
    )
    .await?;
    let applied = run_maintenance_command(
        server,
        maintenance_command(sudo(server), &[SCRIPT_PATH, "ssh-apply"]),
    )
    .await?;
    if applied.exit_code != 0 {
        return Ok(maintenance_outcome(action.as_str(), started, applied));
    }

    let target = target(server)?;
    let (key, fingerprint, known_hosts) = credentials(server)?;
    let confirm = sudo(server)
        .iter()
        .chain(&[SCRIPT_PATH, "ssh-confirm"])
        .map(OsString::from)
        .collect::<Vec<_>>();
    let mut last_error = String::new();
    // A few tries ride out a slow reload; all of them fit inside the 120 s revert timer.
    for attempt in 0..3 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
        match SshExecutor::run_fresh_login(
            &target,
            key,
            fingerprint,
            known_hosts,
            &confirm,
            Duration::from_secs(25),
        )
        .await
        {
            Ok(run) if run.exit_code == 0 => {
                let mut outcome = maintenance_outcome(action.as_str(), started, run);
                outcome.stdout = format!("{}\n{}", applied.stdout.trim(), outcome.stdout.trim());
                return Ok(outcome);
            }
            // The timer already won; retrying cannot change that.
            Ok(run) if run.stderr.contains("Too late") => {
                return Ok(maintenance_outcome(action.as_str(), started, run));
            }
            Ok(run) => last_error = run.stderr,
            Err(error) => last_error = error.to_string(),
        }
    }
    Ok(MaintenanceOutcome {
        action: action.as_str().into(),
        success: false,
        stdout: applied.stdout,
        stderr: format!(
            "litepod could not log in again with the new SSH settings ({}). The server restores its previous settings within two minutes; run the check again afterwards.",
            last_error.trim()
        ),
        duration_ms: started.elapsed().as_millis() as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_parses() {
        let status = std::process::Command::new("sh")
            .args(["-n", "-c", SCRIPT])
            .status()
            .unwrap();
        assert!(status.success());
    }

    #[test]
    fn ssh_settings_never_block_litepod_tunnels_or_keys() {
        for action in [SecurityAction::SshRecommended, SecurityAction::SshStrict] {
            let settings = ssh_settings(action);
            assert!(settings.contains("PasswordAuthentication no"));
            assert!(!settings.contains("AllowTcpForwarding"));
            assert!(!settings.contains("PubkeyAuthentication"));
            assert!(!settings.contains("UsePAM"));
        }
        assert!(ssh_settings(SecurityAction::SshRecommended).contains("# level: recommended"));
        assert!(ssh_settings(SecurityAction::SshStrict).contains("PermitRootLogin no"));
    }

    #[test]
    fn audit_output_becomes_a_report() {
        let report = parse_audit(
            "ssh_passwordauthentication=no\nssh_kbdinteractiveauthentication=yes\nssh_permitrootlogin=without-password\nfirewall=ufw:inactive\nfail2ban=active\nfail2ban_banned=3\nauto_updates=off\nsecurity_updates=12\nreboot_required=yes\nlisten=0.0.0.0:22\nlisten=[::]:22\nlisten=*:443\nlisten=127.0.0.1:2019\nlisten=127.0.0.53%lo:53\nlisten=[::1]:631\nlisten=10.0.0.5:8080\n",
        );
        assert_eq!(report.password_login, Some(true));
        assert_eq!(report.root_login.as_deref(), Some("without-password"));
        assert_eq!(report.ssh_level, None);
        assert_eq!(report.firewall.as_deref(), Some("ufw"));
        assert!(!report.firewall_active);
        assert_eq!(report.fail2ban, "active");
        assert_eq!(report.banned_ips, Some(3));
        assert_eq!(report.auto_updates, Some(false));
        assert_eq!(report.security_updates, Some(12));
        assert!(report.reboot_required);
        assert_eq!(report.public_ports, vec![22, 443, 8080]);
    }

    #[test]
    fn missing_sshd_values_stay_unknown() {
        let report = parse_audit("ssh_passwordauthentication=\nssh_level=strict\n");
        assert_eq!(report.password_login, None);
        assert_eq!(report.ssh_level.as_deref(), Some("strict"));
        assert_eq!(report.fail2ban, "missing");
        assert_eq!(report.auto_updates, None);
    }
}
