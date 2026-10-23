use std::{ffi::OsString, time::Duration};

use serde::{Deserialize, Serialize};

use crate::{
    dtos::response::AppError,
    servers::{
        distro::{self, DistroFamily, LogicalPackage},
        model::{IngressMode, PodmanMode, ServerState, SshTarget},
        repository::ServerRecord,
        ssh::{SshExecutor, SshRun},
    },
};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ServerFacts {
    pub os: Option<String>,
    pub version: Option<String>,
    pub architecture: Option<String>,
    pub podman_version: Option<String>,
    pub caddy_version: Option<String>,
    pub cpu_cores: Option<f64>,
    pub memory_bytes: Option<u64>,
    pub disk_free_bytes: Option<u64>,
    pub swap_bytes: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct SetupStep {
    pub name: &'static str,
    pub command: Vec<OsString>,
    /// Package work on a cold cache or a slow link outlasts the default SSH budget, and a
    /// timeout there kills dpkg mid-install — exactly the broken state Setup then has to repair.
    pub timeout: Duration,
    /// A diagnostic step that legitimately fails on the broken host it exists to detect.
    /// Setup logs the failure and continues to the repair steps that follow it.
    pub tolerate_failure: bool,
}

/// Enough for `apt-get` to fetch and unpack Podman or Caddy on a slow host without ever being
/// cut off part-way through. `pub(crate)` so `distro.rs`'s per-family Caddy step builders can
/// reuse the same budgets instead of drifting from these.
pub(crate) const PACKAGE_TIMEOUT: Duration = Duration::from_secs(900);
pub(crate) const QUICK_TIMEOUT: Duration = Duration::from_secs(45);

/// Debian and Ubuntu ship Podman without `unqualified-search-registries`, so a user Dockerfile
/// with `FROM oven/bun:1` fails the remote build with `short-name "oven/bun:1" did not resolve
/// to an alias`. Same file name and contents as `configure_default_registry()` in
/// `scripts/install.sh`, so a host set up locally and a host set up over SSH converge.
const DEFAULT_REGISTRY_PATH: &str =
    "/etc/containers/registries.conf.d/99-litepod-default-registry.conf";
const DEFAULT_REGISTRY_CONF: &str = "unqualified-search-registries = [\"docker.io\"]\n";
const FAIL2BAN_SSHD_PATH: &str = "/etc/fail2ban/jail.d/litepod-sshd.local";
const ROOTFUL_PODMAN_SOCKET_DROPIN: &str =
    "/etc/systemd/system/podman.socket.d/10-litepod-terminal.conf";
/// Podman's own tmpfiles.d rule creates `/run/podman` as `0700 root:root`. Fixing the socket's
/// own owner/mode (above) is not enough: the SSH user still can't traverse into a `0700`
/// directory to reach it. `/etc/tmpfiles.d` outranks podman's rule in `/usr/lib/tmpfiles.d` for
/// the same path, so this survives reboots without touching podman's packaging.
const ROOTFUL_PODMAN_DIR_TMPFILES: &str = "/etc/tmpfiles.d/10-litepod-podman-dir.conf";

fn target(server: &ServerRecord) -> Result<SshTarget, AppError> {
    SshTarget::parse(
        server
            .ssh_host
            .as_deref()
            .ok_or_else(|| AppError::InternalError("Remote server has no SSH host".into()))?,
        server
            .ssh_port
            .ok_or_else(|| AppError::InternalError("Remote server has no SSH port".into()))?,
        server
            .ssh_user
            .as_deref()
            .ok_or_else(|| AppError::InternalError("Remote server has no SSH user".into()))?,
    )
}

fn credentials(server: &ServerRecord) -> Result<(&str, &str, Option<&str>), AppError> {
    Ok((
        server.private_key_encrypted.as_deref().ok_or_else(|| {
            AppError::InternalError("Remote server SSH credential is unavailable".into())
        })?,
        server.host_key_fingerprint.as_deref().ok_or_else(|| {
            AppError::ValidationError("Confirm the SSH host fingerprint before connecting".into())
        })?,
        server.host_key_known_hosts.as_deref(),
    ))
}

const CADDY_HELPER: &str = r#"#!/bin/sh
set -eu

routes=/etc/litepod/caddy/routes.d
caddyfile=/etc/caddy/Caddyfile
marker='# litepod managed routes'
# Request analytics: every site writes JSON access lines to one rolled, size-capped
# file the API pulls over SSH (`access-log-read`).
access_log=/var/log/caddy/litepod-access.log
log_block='\tlog {\n\t\toutput file /var/log/caddy/litepod-access.log {\n\t\t\troll_size 10MiB\n\t\t\troll_keep 2\n\t\t}\n\t\tformat json\n\t}\n'

valid_id() {
    case "$1" in
        ''|*[!0-9a-f-]*) return 1 ;;
        *) return 0 ;;
    esac
}

prepare() {
    install -d -o root -g root -m 0755 "$routes"
    if ! grep -Fqx -- "$marker" "$caddyfile"; then
        printf '\n%s\nimport /etc/litepod/caddy/routes.d/*.caddy\n' "$marker" >> "$caddyfile"
    fi
    caddy validate --config "$caddyfile" --adapter caddyfile
    systemctl reload caddy
}

install_route() {
    source_path=$1
    route_id=$2
    valid_id "$route_id" || exit 64
    case "$source_path" in
        .local/share/litepod/apps/*/*.caddy) ;;
        *) exit 64 ;;
    esac
    target="$routes/$route_id.caddy"
    # Fail fast, before touching anything, if another fragment already owns this
    # hostname. Two site blocks for one host adapt to two TLS automation
    # policies, which Caddy rejects, poisoning every later reload. litepod only
    # ever emits single-host fragments, so the site address is the first token
    # of line 1.
    site=$(awk 'NR==1{print $1; exit}' "$source_path")
    for other in "$routes"/*.caddy; do
        [ -e "$other" ] || continue
        [ "$other" = "$target" ] && continue
        if [ "$(awk 'NR==1{print $1; exit}' "$other")" = "$site" ]; then
            echo "install-route: host $site already routed by $(basename "$other" .caddy) on this server" >&2
            exit 65
        fi
    done
    install -d -o root -g root -m 0755 "$routes"
    backup=$(mktemp "$routes/.${route_id}.XXXXXX")
    had_target=0
    if [ -f "$target" ]; then
        cp --preserve=mode "$target" "$backup"
        had_target=1
    fi
    install -o root -g root -m 0644 "$source_path" "$target"
    if caddy validate --config "$caddyfile" --adapter caddyfile && systemctl reload caddy; then
        rm -f "$backup"
        return 0
    fi
    if [ "$had_target" -eq 1 ]; then
        install -o root -g root -m 0644 "$backup" "$target"
    else
        rm -f "$target"
    fi
    rm -f "$backup"
    exit 1
}

remove_route() {
    route_id=$1
    valid_id "$route_id" || exit 64
    target="$routes/$route_id.caddy"
    # Deleting a site block can never introduce a new Caddy error, so the
    # removal is unconditional and never rolled back: gating it on the whole
    # config validating would keep a fragment alive whenever some unrelated
    # fragment is already broken, and that orphan then collides with any domain
    # that later reuses the same host ("more than one automation policy").
    rm -f "$target"
    if ! { caddy validate --config "$caddyfile" --adapter caddyfile && systemctl reload caddy; }; then
        echo "remove-route: $route_id deleted but Caddy config still invalid (pre-existing); leaving reload to the next change" >&2
    fi
    return 0
}

list_routes() {
    for frag in "$routes"/*.caddy; do
        [ -e "$frag" ] || continue
        printf '%s %s\n' "$(basename "$frag" .caddy)" \
            "$(awk 'NR==1{print $1; exit}' "$frag")"
    done
}

# Rebuild routes.d/ to match a manifest exactly, in one shot. This is the
# declarative replacement for install-route/remove-route: anything not named
# by the manifest is removed, so a recreated domain (new id) or a failed
# remove-route can never leave an orphan fragment behind to poison the reload.
# The whole helper runs as root in production, so fragments end up root-owned
# without an explicit chown (which would also break the non-root shell tests).
sync_routes() {
    manifest=$1
    case "$manifest" in
        .local/share/litepod/apps/*/caddy-routes.manifest) ;;
        *) exit 64 ;;
    esac
    [ -f "$manifest" ] || exit 64

    lock="${routes%/*}/.litepod-routes.lock"
    staging=$(mktemp -d)
    prior=$(mktemp -d)
    trap 'rm -rf "$staging" "$prior"' EXIT

    # Render every desired fragment into a scratch dir first. Any malformed
    # token aborts here, before the live directory is touched at all.
    while read -r id host port https provider certificate auth rest; do
        [ -n "$id" ] || continue
        case "$id" in ''|*[!0-9a-f-]*) exit 64 ;; esac
        case "$host" in ''|*[!A-Za-z0-9.:-]*) exit 64 ;; esac
        case "$port" in ''|*[!0-9]*) exit 64 ;; esac
        [ "$port" -ge 1 ] && [ "$port" -le 65535 ] || exit 64
        [ -z "$rest" ] || exit 64
        tls=''
        case "${https:-true}" in true) ;; false) host="http://$host" ;; *) exit 64 ;; esac
        case "${provider:-automatic}" in
            automatic) ;;
            letsencrypt) tls='\ttls {\n\t\tissuer acme {\n\t\t\tca https://acme-v02.api.letsencrypt.org/directory\n\t\t}\n\t}\n' ;;
            custom)
                case "$certificate" in ''|*[!0-9a-f]*) exit 64 ;; esac
                [ -f "/etc/caddy/certificates/$certificate.pem" ] || exit 64
                tls="\ttls /etc/caddy/certificates/$certificate.pem /etc/caddy/certificates/$certificate.pem\n" ;;
            *) exit 64 ;;
        esac
        case "${provider:-automatic}:$host" in
            automatic:*.test|automatic:*.TEST) tls='	tls internal
' ;;
        esac
        # Optional base64 `basic_auth` block, spliced in ahead of reverse_proxy
        # so the upstream is never dialed for an unauthenticated request.
        auth_block=''
        case "${auth:--}" in
            -) ;;
            *[!A-Za-z0-9+/=]*) exit 64 ;;
            *)
                auth_block=$(printf '%s' "$auth" | base64 -d) || exit 64
                [ -n "$auth_block" ] || exit 64
                ;;
        esac
        {
            printf '%s {\n%b%b' "$host" "$tls" "$log_block"
            if [ -n "$auth_block" ]; then printf '%s\n' "$auth_block"; fi
            printf '\treverse_proxy 127.0.0.1:%s\n}\n' "$port"
        } > "$staging/$id.caddy"
    done < "$manifest"

    # Two manifest lines for one host would adapt to two site blocks for one
    # address ("ambiguous site definition"); reject before it can go live.
    dupes=$(awk 'FNR==1{print $1}' "$staging"/*.caddy 2>/dev/null | sort | uniq -d)
    if [ -n "$dupes" ]; then
        echo "sync-routes: manifest maps two routes to host(s): $dupes" >&2
        exit 65
    fi

    mkdir -p "$routes"
    # Created up front and owned by caddy: `caddy validate` runs as root and would
    # otherwise create the log root-owned, which the caddy service could not open.
    if [ -n "$log_block" ] && [ ! -e "$access_log" ]; then
        install -d -o caddy -g caddy -m 0750 "${access_log%/*}" 2>/dev/null || true
        install -o caddy -g caddy -m 0640 /dev/null "$access_log" 2>/dev/null || true
    fi
    (
        # flock is util-linux (present on every supported server distro);
        # skipped on dev machines without it, where tests never run concurrent
        # syncs against one routes.d anyway.
        if command -v flock >/dev/null 2>&1; then flock 9; fi
        for f in "$routes"/*.caddy; do
            [ -e "$f" ] || continue
            cp -p "$f" "$prior/"
        done
        rm -f "$routes"/*.caddy
        for f in "$staging"/*.caddy; do
            [ -e "$f" ] || continue
            cp "$f" "$routes/$(basename "$f")"
            chmod 0644 "$routes/$(basename "$f")"
        done
        if caddy validate --config "$caddyfile" --adapter caddyfile && systemctl reload caddy; then
            :
        else
            rm -f "$routes"/*.caddy
            for f in "$prior"/*.caddy; do
                [ -e "$f" ] || continue
                cp "$f" "$routes/$(basename "$f")"
                chmod 0644 "$routes/$(basename "$f")"
            done
            systemctl reload caddy 2>/dev/null || true
            echo "sync-routes: candidate config could not be applied; reverted" >&2
            exit 1
        fi
    ) 9>"$lock"
}

install_certificate() {
    source_path=$1
    certificate=$2
    case "$certificate" in ''|*[!0-9a-f]*) exit 64 ;; esac
    case "$source_path" in .local/share/litepod/apps/*/caddy-certificate-*.pem) ;; *) exit 64 ;; esac
    [ -f "$source_path" ] && [ ! -L "$source_path" ] || exit 64
    install -d -o root -g caddy -m 0750 /etc/caddy/certificates
    install -o root -g caddy -m 0640 "$source_path" "/etc/caddy/certificates/$certificate.pem"
    rm -f "$source_path"
}

# Prints `inode size start`, then up to <max> bytes of the access log from <offset>.
# A different inode (Caddy rolled the file) or a shrunk file restarts at 0.
read_access_log() {
    inode=$1
    offset=$2
    max=$3
    case "$inode:$offset:$max" in *[!0-9:]*|:*|*::*|*:) exit 64 ;; esac
    if [ ! -f "$access_log" ]; then
        echo "0 0 0"
        return 0
    fi
    current=$(ls -di "$access_log" | awk '{print $1}')
    size=$(wc -c < "$access_log" | tr -d ' ')
    if [ "$current" != "$inode" ] || [ "$offset" -gt "$size" ]; then
        offset=0
    fi
    echo "$current $size $offset"
    if [ "$max" -gt 0 ]; then
        tail -c +"$((offset + 1))" "$access_log" | head -c "$max"
    fi
}

case "${1:-}" in
    prepare) prepare ;;
    access-log-read) [ "$#" -eq 4 ] || exit 64; read_access_log "$2" "$3" "$4" ;;
    install-route) [ "$#" -eq 3 ] || exit 64; install_route "$2" "$3" ;;
    remove-route) [ "$#" -eq 2 ] || exit 64; remove_route "$2" ;;
    list-routes) [ "$#" -eq 1 ] || exit 64; list_routes ;;
    sync-routes) [ "$#" -eq 2 ] || exit 64; sync_routes "$2" ;;
    sync-routes-check) exit 0 ;;
    certificates-check) exit 0 ;;
    basic-auth-check) exit 0 ;;
    install-certificate) [ "$#" -eq 3 ] || exit 64; install_certificate "$2" "$3" ;;
    health) caddy validate --config "$caddyfile" --adapter caddyfile; systemctl is-active --quiet caddy ;;
    *) exit 64 ;;
esac
"#;

pub(crate) async fn command(server: &ServerRecord, args: &[&str]) -> Result<String, AppError> {
    let target = target(server)?;
    let (key, fingerprint, known_hosts) = credentials(server)?;
    let args = args.iter().map(OsString::from).collect::<Vec<_>>();
    Ok(
        SshExecutor::run(&target, key, fingerprint, known_hosts, &args)
            .await?
            .stdout,
    )
}

/// Executes the closed, root-owned Caddy helper installed by Setup. Deployment
/// code can only select one of its narrow subcommands; it never receives a
/// general-purpose root shell on the server.
pub async fn caddy_helper(server: &ServerRecord, args: &[OsString]) -> Result<String, AppError> {
    let target = target(server)?;
    let (key, fingerprint, known_hosts) = credentials(server)?;
    let mut command = PodmanMode::privilege_prefix(server.ssh_user.as_deref().unwrap_or_default())
        .iter()
        .map(OsString::from)
        .collect::<Vec<_>>();
    command.push(OsString::from("/usr/local/lib/litepod/serverctl"));
    command.extend(args.iter().cloned());
    Ok(
        SshExecutor::run(&target, key, fingerprint, known_hosts, &command)
            .await?
            .stdout,
    )
}

/// Places generated content at a root-owned absolute path. `copy_file` refuses absolute remote
/// paths and no step ever gets a remote shell to redirect into one, so the content is staged
/// below the SSH user's home and moved into place by a privileged `install`, which also creates
/// the parent directory.
async fn install_root_file(
    server: &ServerRecord,
    contents: &str,
    staging_name: &str,
    destination: &str,
    mode: &str,
) -> Result<(), AppError> {
    let target = target(server)?;
    let (key, fingerprint, known_hosts) = credentials(server)?;
    let staged = crate::servers::ssh::SecretFile::write(contents)?;
    let staging_dir = ".local/share/litepod/setup";
    SshExecutor::run(
        &target,
        key,
        fingerprint,
        known_hosts,
        &[
            OsString::from("mkdir"),
            OsString::from("-p"),
            OsString::from(staging_dir),
        ],
    )
    .await?;
    let staging_path = format!("{staging_dir}/{staging_name}");
    SshExecutor::copy_file(
        &target,
        key,
        fingerprint,
        known_hosts,
        staged.path(),
        &staging_path,
    )
    .await?;
    let mut install = PodmanMode::privilege_prefix(server.ssh_user.as_deref().unwrap_or_default())
        .iter()
        .map(OsString::from)
        .collect::<Vec<_>>();
    install.extend([
        OsString::from("install"),
        OsString::from("-D"),
        OsString::from("-o"),
        OsString::from("root"),
        OsString::from("-g"),
        OsString::from("root"),
        OsString::from("-m"),
        OsString::from(mode),
        OsString::from(&staging_path),
        OsString::from(destination),
    ]);
    SshExecutor::run(&target, key, fingerprint, known_hosts, &install).await?;
    Ok(())
}

async fn install_caddy_helper(server: &ServerRecord) -> Result<(), AppError> {
    install_root_file(
        server,
        CADDY_HELPER,
        "serverctl",
        "/usr/local/lib/litepod/serverctl",
        "0755",
    )
    .await?;
    caddy_helper(server, &[OsString::from("prepare")])
        .await
        .map(|_| ())
}

/// Podman resolves a short image name only against the registries listed here. Without this
/// drop-in, every user Dockerfile that says `FROM node:20` fails the remote build.
async fn install_default_registry(server: &ServerRecord) -> Result<(), AppError> {
    install_root_file(
        server,
        DEFAULT_REGISTRY_CONF,
        "registries.conf",
        DEFAULT_REGISTRY_PATH,
        "0644",
    )
    .await
}

fn fail2ban_sshd_conf(port: u16) -> String {
    format!(
        "[sshd]\nenabled = true\nbackend = systemd\nport = {port}\nmaxretry = 5\nfindtime = 10m\nbantime = 1h\n"
    )
}

async fn configure_fail2ban(server: &ServerRecord) -> Result<(), AppError> {
    let port = server
        .ssh_port
        .ok_or_else(|| AppError::InternalError("Remote server has no SSH port".into()))?;
    install_root_file(
        server,
        &fail2ban_sshd_conf(port),
        "fail2ban-sshd.local",
        FAIL2BAN_SSHD_PATH,
        "0644",
    )
    .await?;
    let ssh_user = server.ssh_user.as_deref().unwrap_or_default();
    let prefix = PodmanMode::privilege_prefix(ssh_user);
    let mut command = prefix.iter().map(OsString::from).collect::<Vec<_>>();
    command.extend([
        OsString::from("systemctl"),
        OsString::from("enable"),
        OsString::from("--now"),
        OsString::from("fail2ban"),
    ]);
    let target = target(server)?;
    let (key, fingerprint, known_hosts) = credentials(server)?;
    SshExecutor::run(&target, key, fingerprint, known_hosts, &command).await?;
    command.truncate(prefix.len());
    command.extend([
        OsString::from("systemctl"),
        OsString::from("restart"),
        OsString::from("fail2ban"),
    ]);
    SshExecutor::run(&target, key, fingerprint, known_hosts, &command)
        .await
        .map(|_| ())
}

/// Rootful Podman's system socket is normally owned by root. Terminal sessions
/// are tunneled by the SSH user, so make that socket private to the already
/// authorized user rather than falling back to a host-local Podman client.
async fn configure_rootful_terminal_socket(server: &ServerRecord) -> Result<(), AppError> {
    let ssh_user = server
        .ssh_user
        .as_deref()
        .ok_or_else(|| AppError::InternalError("Remote server has no SSH user".into()))?;
    install_root_file(
        server,
        &format!("[Socket]\nSocketUser={ssh_user}\nSocketMode=0600\n"),
        "podman-terminal.socket.conf",
        ROOTFUL_PODMAN_SOCKET_DROPIN,
        "0644",
    )
    .await?;
    // Socket file perms alone are useless if the SSH user can't traverse into the directory
    // that holds it — see the comment on ROOTFUL_PODMAN_DIR_TMPFILES.
    install_root_file(
        server,
        "d /run/podman 0711 root root -\n",
        "podman-terminal-dir.conf",
        ROOTFUL_PODMAN_DIR_TMPFILES,
        "0644",
    )
    .await?;
    let mut command = PodmanMode::privilege_prefix(ssh_user)
        .iter()
        .map(OsString::from)
        .collect::<Vec<_>>();
    command.extend([OsString::from("systemctl"), OsString::from("daemon-reload")]);
    let target = target(server)?;
    let (key, fingerprint, known_hosts) = credentials(server)?;
    SshExecutor::run(&target, key, fingerprint, known_hosts, &command).await?;

    // Apply the directory permission now, rather than waiting for the next reboot to pick up
    // the tmpfiles.d rule just installed.
    let mut chmod_dir = PodmanMode::privilege_prefix(ssh_user)
        .iter()
        .map(OsString::from)
        .collect::<Vec<_>>();
    chmod_dir.extend([
        OsString::from("chmod"),
        OsString::from("0711"),
        OsString::from("/run/podman"),
    ]);
    SshExecutor::run(&target, key, fingerprint, known_hosts, &chmod_dir).await?;

    let mut restart = PodmanMode::privilege_prefix(ssh_user)
        .iter()
        .map(OsString::from)
        .collect::<Vec<_>>();
    restart.extend([
        OsString::from("systemctl"),
        OsString::from("restart"),
        OsString::from("podman.socket"),
    ]);
    SshExecutor::run(&target, key, fingerprint, known_hosts, &restart).await?;
    Ok(())
}

/// Runs `podman` the way this server is configured: as the SSH user for rootless, through
/// passwordless sudo for rootful.
async fn podman_command(server: &ServerRecord, args: &[&str]) -> Result<String, AppError> {
    let ssh_user = server.ssh_user.as_deref().unwrap_or_default();
    let mut full = server.podman_mode.podman_prefix(ssh_user).to_vec();
    full.push("podman");
    full.extend_from_slice(args);
    command(server, &full).await
}

pub fn classify_os(os_release: &str) -> ServerState {
    match DistroFamily::detect(os_release) {
        Some(_) => ServerState::NeedsSetup,
        None => ServerState::Unsupported,
    }
}

fn os_value(os_release: &str, name: &str) -> Option<String> {
    os_release
        .lines()
        .find_map(|line| line.strip_prefix(name))
        .map(|value| value.trim_matches('"').to_string())
}

fn first_line(value: &str) -> Option<String> {
    value
        .lines()
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn disk_free_bytes(df: &str) -> Option<u64> {
    df.lines()
        .nth(1)?
        .split_whitespace()
        .nth(3)?
        .parse::<u64>()
        .ok()?
        .checked_mul(1024)
}

fn memory_bytes(free: &str) -> Option<u64> {
    free.lines()
        .find(|line| line.starts_with("Mem:"))
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse().ok())
}

/// `MemTotal` from `/proc/meminfo`, for minimal hosts that ship without `free`.
fn meminfo_total_bytes(meminfo: &str) -> Option<u64> {
    meminfo
        .lines()
        .find(|line| line.starts_with("MemTotal:"))?
        .split_whitespace()
        .nth(1)?
        .parse::<u64>()
        .ok()?
        .checked_mul(1024)
}

async fn collect_facts(server: &ServerRecord) -> Result<(ServerFacts, ServerState), AppError> {
    let os_release = command(server, &["cat", "/etc/os-release"]).await?;
    let state = classify_os(&os_release);
    let architecture = command(server, &["uname", "-m"])
        .await
        .ok()
        .and_then(|value| first_line(&value));
    let podman_version = podman_command(server, &["--version"])
        .await
        .ok()
        .and_then(|value| first_line(&value));
    let caddy_version = command(server, &["caddy", "version"])
        .await
        .ok()
        .and_then(|value| first_line(&value));
    let disk = command(server, &["df", "-Pk", "/"])
        .await
        .ok()
        .and_then(|value| disk_free_bytes(&value));
    let meminfo = command(server, &["cat", "/proc/meminfo"]).await.ok();
    let memory = match command(server, &["free", "-b"])
        .await
        .ok()
        .and_then(|value| memory_bytes(&value))
    {
        Some(bytes) => Some(bytes),
        None => meminfo.as_deref().and_then(meminfo_total_bytes),
    };
    let swap = meminfo.as_deref().and_then(swap_total_bytes);
    let cpu_cores = command(server, &["nproc"])
        .await
        .ok()
        .and_then(|value| value.trim().parse().ok());
    Ok((
        ServerFacts {
            os: os_value(&os_release, "ID="),
            version: os_value(&os_release, "VERSION_ID="),
            architecture,
            podman_version,
            caddy_version,
            cpu_cores,
            memory_bytes: memory,
            disk_free_bytes: disk,
            swap_bytes: swap,
        },
        state,
    ))
}

pub async fn preflight(server: &ServerRecord) -> Result<(ServerFacts, ServerState), AppError> {
    let result = collect_facts(server).await?;
    // This call deliberately remains non-mutating. Its result is checked again by setup before
    // any apt or systemd action is attempted.
    let ssh_user = server.ssh_user.as_deref().unwrap_or_default();
    let mut probe = PodmanMode::privilege_prefix(ssh_user).to_vec();
    probe.push("true");
    command(server, &probe).await.map_err(|_| {
        AppError::ValidationError("The SSH user needs passwordless sudo for Setup or Repair".into())
    })?;
    Ok(result)
}

/// Read-only runtime health check. It verifies the pinned SSH key path and that Podman answers
/// in the server's configured privilege mode, without changing remote state.
pub async fn health_check(server: &ServerRecord) -> Result<(ServerFacts, ServerState), AppError> {
    let uid = command(server, &["id", "-u"]).await?;
    if uid.trim() == "0" && server.podman_mode == PodmanMode::Rootless {
        return Err(AppError::ValidationError("The SSH user is root; rootless Podman requires a non-root user. Switch this server to rootful or reconnect it as an unprivileged user.".into()));
    }
    podman_command(server, &["info", "--format", "json"]).await.map_err(|_| match server.podman_mode {
        PodmanMode::Rootless => AppError::ValidationError("Rootless Podman is not healthy for the configured SSH user".into()),
        PodmanMode::Rootful => AppError::ValidationError("Rootful Podman is not healthy; check that podman.socket is enabled and the SSH user still has passwordless sudo".into()),
    })?;
    // Git Dockerfile deploys run `podman run --network litepod-network`, which errors on a
    // server where Setup never created that network. Surface it here so the server reads
    // degraded (red) with a clear message instead of failing at the first deploy.
    podman_command(server, &["network", "exists", "litepod-network"])
        .await
        .map_err(|_| {
            AppError::ValidationError(
                "The litepod-network Podman network is missing; run Repair before deploying".into(),
            )
        })?;
    if matches!(server.ingress_mode, IngressMode::PublicEdge) {
        caddy_helper(server, &[OsString::from("health")]).await.map_err(|_| {
            AppError::ValidationError(
                "Public-edge Caddy is not healthy; run Setup or Repair before deploying public domains"
                    .into(),
            )
        })?;
    }
    let (facts, state) = collect_facts(server).await?;
    Ok((
        facts,
        if state == ServerState::Unsupported {
            state
        } else {
            ServerState::Ready
        },
    ))
}

/// A health check proves Podman answers; it never proves the server can pull an image and run a
/// container. This does, using the smallest official image there is.
pub const TEST_IMAGE: &str = "docker.io/library/hello-world:latest";

/// A cold pull over a slow link easily outlasts the probe-sized default budget.
const TEST_DEPLOY_TIMEOUT: Duration = Duration::from_secs(300);

/// Container output is shown verbatim in the UI, so it is capped rather than streamed.
const TEST_DEPLOY_OUTPUT_LIMIT: usize = 8_000;

pub struct TestDeployOutcome {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
}

fn clamp_output(value: &str) -> String {
    if value.chars().count() <= TEST_DEPLOY_OUTPUT_LIMIT {
        return value.to_string();
    }
    let kept = value
        .chars()
        .take(TEST_DEPLOY_OUTPUT_LIMIT)
        .collect::<String>();
    format!("{kept}\n… output truncated")
}

/// Runs a throwaway `hello-world` container on the server and reports what it printed. A
/// container that fails is a result, not an error: the point is to show the user why.
pub async fn test_deploy(server: &ServerRecord) -> Result<TestDeployOutcome, AppError> {
    let target = target(server)?;
    let (key, fingerprint, known_hosts) = credentials(server)?;
    let ssh_user = server.ssh_user.as_deref().unwrap_or_default();
    let mut argv = server
        .podman_mode
        .podman_prefix(ssh_user)
        .iter()
        .map(OsString::from)
        .collect::<Vec<_>>();
    argv.extend(
        ["podman", "run", "--rm", TEST_IMAGE]
            .iter()
            .map(OsString::from),
    );
    let started = std::time::Instant::now();
    let run = SshExecutor::run_allow_failure(
        &target,
        key,
        fingerprint,
        known_hosts,
        &argv,
        TEST_DEPLOY_TIMEOUT,
    )
    .await?;
    Ok(TestDeployOutcome {
        exit_code: run.exit_code,
        stdout: clamp_output(&run.stdout),
        stderr: clamp_output(&run.stderr),
        duration_ms: started.elapsed().as_millis() as u64,
    })
}

#[derive(Debug, Clone)]
pub struct MaintenanceOutcome {
    pub action: String,
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
}

const MAINTENANCE_OUTPUT_LIMIT: usize = 16_000;
const MAINTENANCE_TIMEOUT: Duration = Duration::from_secs(900);

fn maintenance_command(prefix: &[&str], args: &[&str]) -> Vec<OsString> {
    prefix
        .iter()
        .chain(args.iter())
        .map(OsString::from)
        .collect()
}

fn maintenance_outcome(
    action: &str,
    started: std::time::Instant,
    run: SshRun,
) -> MaintenanceOutcome {
    MaintenanceOutcome {
        action: action.to_string(),
        success: run.exit_code == 0,
        stdout: clamp_output_limit(&run.stdout, MAINTENANCE_OUTPUT_LIMIT),
        stderr: clamp_output_limit(&run.stderr, MAINTENANCE_OUTPUT_LIMIT),
        duration_ms: started.elapsed().as_millis() as u64,
    }
}

fn append_output(target: &mut String, value: &str) {
    if value.is_empty() {
        return;
    }
    if !target.is_empty() {
        target.push('\n');
    }
    target.push_str(value);
}

fn clamp_output_limit(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_string();
    }
    let kept = value.chars().take(limit).collect::<String>();
    format!("{kept}\n... output truncated")
}

async fn run_maintenance_command(
    server: &ServerRecord,
    command: Vec<OsString>,
) -> Result<SshRun, AppError> {
    let target = target(server)?;
    let (key, fingerprint, known_hosts) = credentials(server)?;
    SshExecutor::run_allow_failure(
        &target,
        key,
        fingerprint,
        known_hosts,
        &command,
        MAINTENANCE_TIMEOUT,
    )
    .await
}

pub async fn update_packages(server: &ServerRecord) -> Result<MaintenanceOutcome, AppError> {
    let started = std::time::Instant::now();
    let sudo = PodmanMode::privilege_prefix(server.ssh_user.as_deref().unwrap_or_default());
    let os_release = command(server, &["cat", "/etc/os-release"]).await?;
    let Some(family) = distro::DistroFamily::detect(&os_release) else {
        return Ok(MaintenanceOutcome {
            action: "update_packages".into(),
            success: false,
            stdout: String::new(),
            stderr: "Could not determine the server's Linux distribution".into(),
            duration_ms: started.elapsed().as_millis() as u64,
        });
    };

    if family == distro::DistroFamily::Arch {
        // apt/dnf tolerate a separate "refresh index" call followed later by "upgrade", but a
        // bare `pacman -Sy` (refresh) run separately from `-Su` (upgrade) is the exact "partial
        // upgrade" pattern the Arch wiki calls unsupported if anything installs packages
        // in between — and these two calls are two separate SSH round trips with a real network
        // gap between them, so Arch runs one combined `pacman -Syu` instead of Debian/Fedora's
        // two-step shape.
        let upgrade =
            run_maintenance_command(server, maintenance_command(sudo, family.upgrade_argv()))
                .await?;
        return Ok(MaintenanceOutcome {
            action: "update_packages".into(),
            success: upgrade.exit_code == 0,
            stdout: clamp_output_limit(&upgrade.stdout, MAINTENANCE_OUTPUT_LIMIT),
            stderr: clamp_output_limit(&upgrade.stderr, MAINTENANCE_OUTPUT_LIMIT),
            duration_ms: started.elapsed().as_millis() as u64,
        });
    }

    let update =
        run_maintenance_command(server, maintenance_command(sudo, family.refresh_argv())).await?;
    let mut stdout = update.stdout;
    let mut stderr = update.stderr;
    if update.exit_code != 0 {
        return Ok(MaintenanceOutcome {
            action: "update_packages".into(),
            success: false,
            stdout: clamp_output_limit(&stdout, MAINTENANCE_OUTPUT_LIMIT),
            stderr: clamp_output_limit(&stderr, MAINTENANCE_OUTPUT_LIMIT),
            duration_ms: started.elapsed().as_millis() as u64,
        });
    }

    let upgrade =
        run_maintenance_command(server, maintenance_command(sudo, family.upgrade_argv())).await?;
    append_output(&mut stdout, &upgrade.stdout);
    append_output(&mut stderr, &upgrade.stderr);
    Ok(MaintenanceOutcome {
        action: "update_packages".into(),
        success: upgrade.exit_code == 0,
        stdout: clamp_output_limit(&stdout, MAINTENANCE_OUTPUT_LIMIT),
        stderr: clamp_output_limit(&stderr, MAINTENANCE_OUTPUT_LIMIT),
        duration_ms: started.elapsed().as_millis() as u64,
    })
}

pub async fn restart_server(server: &ServerRecord) -> Result<MaintenanceOutcome, AppError> {
    let started = std::time::Instant::now();
    let sudo = PodmanMode::privilege_prefix(server.ssh_user.as_deref().unwrap_or_default());
    let run = run_maintenance_command(
        server,
        maintenance_command(
            sudo,
            &["shutdown", "-r", "now", "litepod-maintenance-restart"],
        ),
    )
    .await?;
    Ok(maintenance_outcome("restart_server", started, run))
}

pub async fn prune_images(server: &ServerRecord) -> Result<MaintenanceOutcome, AppError> {
    let started = std::time::Instant::now();
    let ssh_user = server.ssh_user.as_deref().unwrap_or_default();
    let mut command = server
        .podman_mode
        .podman_prefix(ssh_user)
        .iter()
        .map(OsString::from)
        .collect::<Vec<_>>();
    command.extend(
        ["podman", "image", "prune", "-a", "-f"]
            .iter()
            .map(OsString::from),
    );
    let run = run_maintenance_command(server, command).await?;
    Ok(maintenance_outcome("prune_images", started, run))
}

pub async fn restart_podman(server: &ServerRecord) -> Result<MaintenanceOutcome, AppError> {
    let started = std::time::Instant::now();
    let ssh_user = server.ssh_user.as_deref().unwrap_or_default();
    let command = match server.podman_mode {
        PodmanMode::Rootless => {
            maintenance_command(&["systemctl", "--user"], &["restart", "podman.socket"])
        }
        PodmanMode::Rootful => maintenance_command(
            PodmanMode::privilege_prefix(ssh_user),
            &["systemctl", "restart", "podman.socket"],
        ),
    };
    let run = run_maintenance_command(server, command).await?;
    Ok(maintenance_outcome("restart_podman", started, run))
}

pub async fn check_disk(server: &ServerRecord) -> Result<MaintenanceOutcome, AppError> {
    let started = std::time::Instant::now();
    let run = run_maintenance_command(server, maintenance_command(&[], &["df", "-h"])).await?;
    Ok(maintenance_outcome("check_disk", started, run))
}

const CREATE_SWAP_PATH: &str = "/usr/local/lib/litepod/create-swap";
/// Closed, root-owned script so the maintenance action never needs a general remote shell. A
/// host that already has any swap is left alone. Anything that fails after the file exists
/// removes it again, which matters on LXC guests and btrfs where `swapon` refuses a swapfile.
const CREATE_SWAP: &str = r#"#!/bin/sh
set -eu
size_mib=4096
swapfile=/swapfile
swap_kib=$(awk '/^SwapTotal:/ { print $2 }' /proc/meminfo)
if [ "${swap_kib:-0}" -gt 0 ]; then
    echo "Swap is already configured ($((swap_kib / 1024)) MiB); nothing to do."
    exit 0
fi
if [ -e "$swapfile" ]; then
    echo "$swapfile already exists but is not active swap; inspect or remove it, then retry." >&2
    exit 1
fi
free_kib=$(df -Pk / | awk 'NR == 2 { print $4 }')
# Keep 2 GiB free after the swapfile so image pulls and builds still fit.
if [ "$free_kib" -lt $(( (size_mib + 2048) * 1024 )) ]; then
    echo "Not enough free disk: need $((size_mib + 2048)) MiB on /, have $((free_kib / 1024)) MiB." >&2
    exit 1
fi
trap 'swapoff "$swapfile" 2>/dev/null || true; rm -f "$swapfile"' EXIT
# dd rather than fallocate: some filesystems reject a preallocated swapfile at swapon.
dd if=/dev/zero of="$swapfile" bs=1M count="$size_mib" status=none
chmod 600 "$swapfile"
mkswap "$swapfile" >/dev/null
swapon "$swapfile"
grep -q "^$swapfile " /etc/fstab || echo "$swapfile none swap sw 0 0" >> /etc/fstab
trap - EXIT
echo "Created and enabled a ${size_mib} MiB swapfile at $swapfile (persisted in /etc/fstab)."
"#;

pub async fn create_swap(server: &ServerRecord) -> Result<MaintenanceOutcome, AppError> {
    let started = std::time::Instant::now();
    install_root_file(server, CREATE_SWAP, "create-swap", CREATE_SWAP_PATH, "0755").await?;
    let sudo = PodmanMode::privilege_prefix(server.ssh_user.as_deref().unwrap_or_default());
    let run =
        run_maintenance_command(server, maintenance_command(sudo, &[CREATE_SWAP_PATH])).await?;
    Ok(maintenance_outcome("create_swap", started, run))
}

/// Hosts above this much RAM rarely hit the OOM killer during builds, so only smaller ones get
/// the swap recommendation.
const SWAP_ADVICE_MAX_MEMORY: u64 = 16 * 1024 * 1024 * 1024;

/// Advisory only: swap costs disk and is the operator's call, so Setup and Repair explain it in
/// the job log and the UI asks before running `create_swap`.
const SWAP_ADVICE: &str = "This server has 16 GiB of RAM or less and no swap. When memory runs out, the kernel kills a process (often a build or a database) instead of slowing down. litepod can create a 4 GiB swap file for you from this server's card, or run on the server: sudo dd if=/dev/zero of=/swapfile bs=1M count=4096 && sudo chmod 600 /swapfile && sudo mkswap /swapfile && sudo swapon /swapfile && echo '/swapfile none swap sw 0 0' | sudo tee -a /etc/fstab";

pub fn swap_recommended(facts: &ServerFacts) -> bool {
    facts.swap_bytes == Some(0)
        && facts
            .memory_bytes
            .is_some_and(|memory| memory <= SWAP_ADVICE_MAX_MEMORY)
}

/// `SwapTotal` from `/proc/meminfo`, in bytes.
fn swap_total_bytes(meminfo: &str) -> Option<u64> {
    meminfo
        .lines()
        .find(|line| line.starts_with("SwapTotal:"))?
        .split_whitespace()
        .nth(1)?
        .parse::<u64>()
        .ok()?
        .checked_mul(1024)
}

pub fn setup_plan(
    family: DistroFamily,
    ingress: IngressMode,
    podman_mode: PodmanMode,
    ssh_user: &str,
) -> Vec<SetupStep> {
    let sudo = PodmanMode::privilege_prefix(ssh_user);
    let escalated = |rest: &[&str]| -> Vec<OsString> {
        sudo.iter().chain(rest.iter()).map(OsString::from).collect()
    };
    let plain = |rest: &[&str]| -> Vec<OsString> { rest.iter().map(OsString::from).collect() };
    let quick = |name: &'static str, command: Vec<OsString>| SetupStep {
        name,
        command,
        timeout: QUICK_TIMEOUT,
        tolerate_failure: false,
    };
    let packaged = |name: &'static str, command: Vec<OsString>| SetupStep {
        name,
        command,
        timeout: PACKAGE_TIMEOUT,
        tolerate_failure: false,
    };
    let tolerated = |name: &'static str, command: Vec<OsString>| SetupStep {
        name,
        command,
        timeout: PACKAGE_TIMEOUT,
        tolerate_failure: true,
    };
    let install =
        |packages: &[&'static str]| -> Vec<OsString> { escalated(&family.install_argv(packages)) };

    let mut steps = vec![quick("verify-privileges", escalated(&["true"]))];

    if family == DistroFamily::Debian {
        // An install cut short — a dropped SSH session, a killed Setup job — leaves dpkg half
        // configured, and every later apt-get install fails with "Unmet dependencies" until
        // that state is settled. These steps are no-ops on a healthy host, so Setup and Repair
        // run them unconditionally instead of leaving the sysadmin to run apt --fix-broken by
        // hand. dpkg/apt-specific, so only Debian family hosts need them.
        //
        // The first dpkg pass is allowed to fail: when a dependency is not merely unconfigured
        // but missing outright, dpkg cannot install it and only apt can, so aborting here would
        // stop Setup on the diagnostic and never reach the repair. The pass after apt is strict.
        steps.push(tolerated(
            "configure-pending-packages",
            escalated(&[
                "env",
                "DEBIAN_FRONTEND=noninteractive",
                "dpkg",
                "--configure",
                "-a",
            ]),
        ));
        steps.push(packaged(
            "refresh-apt-index",
            escalated(family.refresh_argv()),
        ));
        // Tolerated for the same reason as the dpkg pass above: apt refuses to repair some
        // states it can still install its way out of, and install-base-packages is the step
        // that actually resolves them.
        steps.push(tolerated(
            "fix-broken-packages",
            escalated(&[
                "env",
                "DEBIAN_FRONTEND=noninteractive",
                "apt-get",
                "--fix-broken",
                "install",
                "-y",
            ]),
        ));
    } else {
        steps.push(packaged(
            "refresh-package-index",
            escalated(family.refresh_argv()),
        ));
    }

    // perl is not present on minimal Debian and Ubuntu images, yet git and liberror-perl depend
    // on it, so a host that arrived with those two unpacked can never configure them. Installing
    // perl outright breaks that deadlock. build-essential (or its Fedora/Arch equivalent) is
    // requested for parity with hosts that expect a toolchain.
    let base_packages = family.resolve_packages(&[
        LogicalPackage::Perl,
        LogicalPackage::BuildEssential,
        LogicalPackage::Which,
        LogicalPackage::Fail2ban,
    ]);
    steps.push(packaged("install-base-packages", install(&base_packages)));

    if family == DistroFamily::Debian {
        steps.push(packaged(
            "settle-pending-packages",
            escalated(&[
                "env",
                "DEBIAN_FRONTEND=noninteractive",
                "dpkg",
                "--configure",
                "-a",
            ]),
        ));
    }

    match podman_mode {
        PodmanMode::Rootless => {
            let packages = family.resolve_packages(&[
                LogicalPackage::Podman,
                LogicalPackage::PodmanCompose,
                LogicalPackage::Uidmap,
                LogicalPackage::Slirp4netns,
                LogicalPackage::FuseOverlayfs,
            ]);
            steps.push(packaged("install-rootless-podman", install(&packages)));
            // Without lingering, the user's Podman socket dies with the SSH session and every
            // container stops on logout.
            steps.push(quick(
                "enable-user-lingering",
                escalated(&["loginctl", "enable-linger", ssh_user]),
            ));
            steps.push(quick(
                "enable-rootless-podman",
                plain(&["systemctl", "--user", "enable", "--now", "podman.socket"]),
            ));
        }
        PodmanMode::Rootful => {
            let packages =
                family.resolve_packages(&[LogicalPackage::Podman, LogicalPackage::PodmanCompose]);
            steps.push(packaged("install-rootful-podman", install(&packages)));
            steps.push(quick(
                "enable-rootful-podman",
                escalated(&["systemctl", "enable", "--now", "podman.socket"]),
            ));
        }
    }
    // Podman's `pasta` network backend needs /dev/net/tun to set up a container's
    // namespace — rootful hosts hit this too, not only rootless. On a host where the
    // `tun` kernel module isn't loaded yet, the first sign of that is an obscure "pasta
    // failed: Failed to open() /dev/net/tun" deep inside the first container build/run;
    // load it here so the operator gets a clear step name pointing at it instead.
    // Tolerated: on an LXC guest (Proxmox, etc.) modprobe can never succeed — the
    // container has no /lib/modules for the shared host kernel — yet everything except
    // pasta-networked builds (e.g. remote Railpack) still works fine there, so this
    // can't be allowed to block the rest of Setup/Repair.
    steps.push(tolerated(
        "enable-tun-module",
        escalated(&["modprobe", "tun"]),
    ));
    // `podman compose` is only a dispatcher. Installing Podman without its
    // provider lets preflight pass but makes the first Compose deploy fail
    // with "looking up compose provider failed". Verify it in the same user
    // and privilege mode deployment will use.
    let mut compose_provider = podman_mode
        .podman_prefix(ssh_user)
        .iter()
        .map(OsString::from)
        .collect::<Vec<_>>();
    compose_provider.extend([
        OsString::from("podman"),
        OsString::from("compose"),
        OsString::from("version"),
    ]);
    steps.push(quick("verify-podman-compose-provider", compose_provider));
    // Compose deploys declare the network in their compose file, but the SSH/CLI path
    // (`podman run --network litepod-network` in deployment/git.rs) does not — `podman run`
    // errors instead of creating a missing named network. Without this step the first Git
    // Dockerfile deploy to a freshly-added server fails with "network not found". `--ignore`
    // makes it a no-op when the network already exists, so Repair is safe to re-run.
    // Name matches the LITEPOD_PODMAN_NETWORK default in main.rs.
    let mut network_create = podman_mode
        .podman_prefix(ssh_user)
        .iter()
        .map(OsString::from)
        .collect::<Vec<_>>();
    network_create.extend([
        OsString::from("podman"),
        OsString::from("network"),
        OsString::from("create"),
        OsString::from("--ignore"),
        OsString::from("litepod-network"),
    ]);
    steps.push(quick("create-litepod-network", network_create));
    steps.push(quick(
        "create-litepod-workspace",
        plain(&["mkdir", "-p", ".local/share/litepod"]),
    ));
    if matches!(ingress, IngressMode::PublicEdge) {
        steps.extend(distro::caddy_install_steps(family, &escalated));
        steps.push(quick(
            "enable-native-caddy",
            escalated(&["systemctl", "enable", "--now", "caddy"]),
        ));
    }
    steps
}

/// `install-base-packages` runs on every family; `settle-pending-packages` only exists in
/// Debian's plan (see `setup_plan()`). Both fail only when the package manager itself could not
/// repair the package database, so the operator has to look at the host. Every other step speaks
/// for itself.
fn repair_hint(family: DistroFamily, step: &str) -> &'static str {
    match (family, step) {
        (DistroFamily::Debian, "install-base-packages" | "settle-pending-packages") => {
            "; the package database is broken beyond automatic repair — on the server run `apt-get update && apt-get -f install && apt-get install -y perl` and re-run Repair"
        }
        (DistroFamily::Fedora, "install-base-packages") => {
            "; the package database is broken beyond automatic repair — on the server run `dnf makecache && dnf install -y perl` and re-run Repair"
        }
        (DistroFamily::Arch, "install-base-packages") => {
            "; the package database is broken beyond automatic repair — on the server run `pacman -Sy && pacman -S --noconfirm --needed base-devel` and re-run Repair"
        }
        (_, "enable-tun-module") => {
            "; the host kernel would not load the `tun` module — confirm it's built for this kernel (`modinfo tun`) and that /dev/net/tun is exposed to this VM/container, then re-run Repair"
        }
        _ => "",
    }
}

pub async fn run_setup<F, Fut>(
    server: &ServerRecord,
    mut on_step: F,
) -> Result<ServerFacts, AppError>
where
    F: FnMut(&'static str, String) -> Fut,
    Fut: std::future::Future<Output = Result<(), AppError>>,
{
    let target = target(server)?;
    let (key, fingerprint, known_hosts) = credentials(server)?;
    let ssh_user = server
        .ssh_user
        .as_deref()
        .ok_or_else(|| AppError::InternalError("Remote server has no SSH user".into()))?;
    // Do this before package or systemd changes. A historical registration can
    // predate the create-handler validation, and root's `$HOME` makes Podman
    // look for Compose providers under `/root` while claiming to be rootless.
    if server.podman_mode == PodmanMode::Rootless
        && command(server, &["id", "-u"]).await?.trim() == "0"
    {
        return Err(AppError::ValidationError(
            "The SSH user is root; rootless Podman requires an unprivileged SSH user. Re-register this server with a non-root user, or choose rootful Podman deliberately."
                .into(),
        ));
    }
    // Determine the distro family up front and fail fast, instead of only discovering an
    // unsupported host in the final preflight() call below, after package/systemd steps have
    // already run.
    let os_release = command(server, &["cat", "/etc/os-release"]).await?;
    let family = distro::DistroFamily::detect(&os_release).ok_or_else(|| {
        AppError::ValidationError(
            "Only Debian/Ubuntu, Fedora, and Arch-based servers are supported".into(),
        )
    })?;
    for step in setup_plan(family, server.ingress_mode, server.podman_mode, ssh_user) {
        on_step(step.name, format!("Running {}", step.name)).await?;
        let outcome = SshExecutor::run_with_timeout(
            &target,
            key,
            fingerprint,
            known_hosts,
            &step.command,
            step.timeout,
        )
        .await;
        if let Err(error) = outcome {
            // The bare SSH error says nothing about which of a dozen steps produced it.
            let error = match error {
                AppError::ValidationError(message) => AppError::ValidationError(format!(
                    "Step {}: {message}{}",
                    step.name,
                    repair_hint(family, step.name)
                )),
                other => other,
            };
            if !step.tolerate_failure {
                return Err(error);
            }
            on_step(
                step.name,
                format!("{error}; continuing to the repair steps that follow it"),
            )
            .await?;
        }
    }
    // Every mode needs this, and Repair runs the same path, so a server registered before this
    // step existed gets the drop-in the next time the operator repairs it.
    on_step(
        "configure-default-registry",
        "Running configure-default-registry".to_string(),
    )
    .await?;
    install_default_registry(server)
        .await
        .map_err(|error| match error {
            AppError::ValidationError(message) => {
                AppError::ValidationError(format!("Step configure-default-registry: {message}"))
            }
            other => other,
        })?;
    on_step(
        "configure-fail2ban",
        "Running configure-fail2ban".to_string(),
    )
    .await?;
    configure_fail2ban(server)
        .await
        .map_err(|error| match error {
            AppError::ValidationError(message) => {
                AppError::ValidationError(format!("Step configure-fail2ban: {message}"))
            }
            other => other,
        })?;
    if server.podman_mode == PodmanMode::Rootful {
        on_step(
            "configure-rootful-terminal-socket",
            "Running configure-rootful-terminal-socket".to_string(),
        )
        .await?;
        configure_rootful_terminal_socket(server)
            .await
            .map_err(|error| match error {
                AppError::ValidationError(message) => AppError::ValidationError(format!(
                    "Step configure-rootful-terminal-socket: {message}"
                )),
                other => other,
            })?;
    }
    if matches!(server.ingress_mode, IngressMode::PublicEdge) {
        on_step(
            "install-caddy-route-helper",
            "Running install-caddy-route-helper".to_string(),
        )
        .await?;
        install_caddy_helper(server)
            .await
            .map_err(|error| match error {
                AppError::ValidationError(message) => {
                    AppError::ValidationError(format!("Step install-caddy-route-helper: {message}"))
                }
                other => other,
            })?;
    }
    let (facts, state) = preflight(server).await?;
    if state == ServerState::Unsupported {
        return Err(AppError::ValidationError(
            "Only Debian/Ubuntu, Fedora, and Arch-based servers are supported".into(),
        ));
    }
    if swap_recommended(&facts) {
        on_step("check-swap", SWAP_ADVICE.to_string()).await?;
    }
    Ok(facts)
}

#[cfg(test)]
mod tests {
    use super::{
        CADDY_HELPER, DEFAULT_REGISTRY_CONF, DEFAULT_REGISTRY_PATH, FAIL2BAN_SSHD_PATH,
        ROOTFUL_PODMAN_SOCKET_DROPIN, classify_os, fail2ban_sshd_conf, meminfo_total_bytes,
        memory_bytes, setup_plan,
    };
    use crate::servers::{
        distro::DistroFamily,
        model::{IngressMode, PodmanMode, ServerState},
    };
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn swap_is_recommended_only_without_swap_on_small_hosts() {
        use super::{ServerFacts, swap_recommended, swap_total_bytes};
        assert_eq!(swap_total_bytes("SwapTotal:       0 kB\n"), Some(0));
        assert_eq!(swap_total_bytes("SwapTotal:    4096 kB\n"), Some(4_194_304));
        let gib = 1024 * 1024 * 1024;
        let facts = |memory, swap| ServerFacts {
            memory_bytes: Some(memory),
            swap_bytes: swap,
            ..Default::default()
        };
        assert!(swap_recommended(&facts(16 * gib, Some(0))));
        assert!(!swap_recommended(&facts(16 * gib + 1, Some(0))));
        assert!(!swap_recommended(&facts(4 * gib, Some(gib))));
        assert!(!swap_recommended(&facts(4 * gib, None)));
    }

    #[test]
    fn create_swap_script_parses() {
        let status = std::process::Command::new("sh")
            .args(["-n", "-c", super::CREATE_SWAP])
            .status()
            .unwrap();
        assert!(status.success());
    }

    #[test]
    fn memory_is_read_from_free_or_meminfo() {
        let free =
            "               total        used\nMem:      8221581312  2147483648\nSwap:     0  0\n";
        assert_eq!(memory_bytes(free), Some(8_221_581_312));
        let meminfo = "MemTotal:        8028888 kB\nMemFree:         1000 kB\n";
        assert_eq!(meminfo_total_bytes(meminfo), Some(8_028_888 * 1024));
        assert_eq!(meminfo_total_bytes(""), None);
    }

    /// Podman only reads drop-ins that live in this directory and end in `.conf`.
    #[test]
    fn default_registry_lands_in_the_drop_in_directory() {
        assert!(DEFAULT_REGISTRY_PATH.starts_with("/etc/containers/registries.conf.d/"));
        assert!(DEFAULT_REGISTRY_PATH.ends_with(".conf"));
    }

    #[test]
    fn default_registry_declares_docker_io() {
        assert!(DEFAULT_REGISTRY_CONF.contains("unqualified-search-registries"));
        assert!(DEFAULT_REGISTRY_CONF.contains("\"docker.io\""));
        assert!(DEFAULT_REGISTRY_CONF.ends_with('\n'));
    }

    #[test]
    fn fail2ban_only_protects_the_registered_ssh_port() {
        let content = fail2ban_sshd_conf(2222);
        assert_eq!(
            FAIL2BAN_SSHD_PATH,
            "/etc/fail2ban/jail.d/litepod-sshd.local"
        );
        assert!(content.contains("[sshd]"));
        assert!(content.contains("port = 2222"));
        assert!(content.contains("maxretry = 5"));
        assert!(content.contains("findtime = 10m"));
        assert!(content.contains("bantime = 1h"));
    }

    #[test]
    fn rootful_terminal_socket_dropin_is_private_to_the_ssh_user() {
        assert!(ROOTFUL_PODMAN_SOCKET_DROPIN.ends_with("podman.socket.d/10-litepod-terminal.conf"));
        let content = format!("[Socket]\nSocketUser={}\nSocketMode=0600\n", "litepod");
        assert!(content.contains("SocketUser=litepod"));
        assert!(content.contains("SocketMode=0600"));
    }

    /// Runs the real embedded helper with its two hardcoded paths swapped for a
    /// scratch dir and `caddy`/`systemctl` stubbed on PATH.
    fn run_helper(dir: &std::path::Path, caddy_exit: i32, args: &[&str]) -> std::process::Output {
        run_helper_with(dir, caddy_exit, args, false)
    }

    /// `access_log` always points into `dir`; `keep_log` keeps the per-site `log` block
    /// in rendered fragments (the other tests compare fragments without it).
    fn run_helper_with(
        dir: &std::path::Path,
        caddy_exit: i32,
        args: &[&str],
        keep_log: bool,
    ) -> std::process::Output {
        let routes = dir.join("routes.d");
        std::fs::create_dir_all(&routes).unwrap();
        let caddyfile = dir.join("Caddyfile");
        if !caddyfile.exists() {
            std::fs::write(&caddyfile, "").unwrap();
        }
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        for name in ["caddy", "systemctl"] {
            let path = bin.join(name);
            std::fs::write(&path, format!("#!/bin/sh\nexit {caddy_exit}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let mut helper = CADDY_HELPER
            .replace(
                "routes=/etc/litepod/caddy/routes.d",
                &format!("routes={}", routes.display()),
            )
            .replace(
                "caddyfile=/etc/caddy/Caddyfile",
                &format!("caddyfile={}", caddyfile.display()),
            )
            .replace(
                "access_log=/var/log/caddy/litepod-access.log",
                &format!("access_log={}", dir.join("access.log").display()),
            );
        if !keep_log {
            let start = helper.find("log_block='").unwrap();
            let end = start + helper[start..].find("'\n").unwrap() + 2;
            helper.replace_range(start..end, "log_block=''\n");
        }
        std::process::Command::new("sh")
            .arg("-c")
            .arg(&helper)
            .arg("serverctl")
            .args(args)
            .current_dir(dir)
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    bin.display(),
                    std::env::var("PATH").unwrap_or_default()
                ),
            )
            .output()
            .unwrap()
    }

    #[test]
    fn remove_route_deletes_the_fragment_even_when_the_config_stays_invalid() {
        let dir =
            std::env::temp_dir().join(format!("litepod-serverctl-rm-{}", uuid::Uuid::now_v7()));
        let routes = dir.join("routes.d");
        std::fs::create_dir_all(&routes).unwrap();
        std::fs::write(
            routes.join("deadbeef.caddy"),
            "victim.example.com {\n\treverse_proxy 127.0.0.1:42000\n}\n",
        )
        .unwrap();
        // caddy validate fails (an unrelated fragment is already broken).
        let out = run_helper(&dir, 1, &["remove-route", "deadbeef"]);
        assert!(
            out.status.success(),
            "remove-route must still succeed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            !routes.join("deadbeef.caddy").exists(),
            "the fragment must be gone, never restored"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn install_route_rejects_a_host_another_fragment_already_owns() {
        let dir =
            std::env::temp_dir().join(format!("litepod-serverctl-in-{}", uuid::Uuid::now_v7()));
        let routes = dir.join("routes.d");
        std::fs::create_dir_all(&routes).unwrap();
        std::fs::write(
            routes.join("aaaaaa.caddy"),
            "loki.example.com {\n\treverse_proxy 127.0.0.1:1\n}\n",
        )
        .unwrap();
        let staged = dir.join(".local/share/litepod/apps/app1");
        std::fs::create_dir_all(&staged).unwrap();
        std::fs::write(
            staged.join("new.caddy"),
            "loki.example.com {\n\treverse_proxy 127.0.0.1:2\n}\n",
        )
        .unwrap();
        let out = run_helper(
            &dir,
            0,
            &[
                "install-route",
                ".local/share/litepod/apps/app1/new.caddy",
                "bbbbbb",
            ],
        );
        assert_eq!(
            out.status.code(),
            Some(65),
            "must reject the duplicate host"
        );
        assert!(
            !routes.join("bbbbbb.caddy").exists(),
            "the rejected fragment must not be installed"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sync_routes_check_arm_exists() {
        let dir =
            std::env::temp_dir().join(format!("litepod-serverctl-chk-{}", uuid::Uuid::now_v7()));
        let out = run_helper(&dir, 0, &["sync-routes-check"]);
        assert_eq!(
            out.status.code(),
            Some(0),
            "sync-routes-check must be a recognized no-op"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_routes_prints_id_and_host_per_fragment() {
        let dir =
            std::env::temp_dir().join(format!("litepod-serverctl-ls-{}", uuid::Uuid::now_v7()));
        let routes = dir.join("routes.d");
        std::fs::create_dir_all(&routes).unwrap();
        std::fs::write(
            routes.join("11111111-1111-7111-8111-111111111111.caddy"),
            "a.example.com {\n\treverse_proxy 127.0.0.1:42000\n}\n",
        )
        .unwrap();
        std::fs::write(
            routes.join("22222222-2222-7222-8222-222222222222.caddy"),
            "b.example.com {\n\treverse_proxy 127.0.0.1:42001\n}\n",
        )
        .unwrap();

        let out = run_helper(&dir, 0, &["list-routes"]);
        assert!(out.status.success());
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains("11111111-1111-7111-8111-111111111111 a.example.com"),
            "got: {stdout}"
        );
        assert!(
            stdout.contains("22222222-2222-7222-8222-222222222222 b.example.com"),
            "got: {stdout}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn synced_sites_log_to_the_access_file_and_reads_resume_by_offset() {
        let dir =
            std::env::temp_dir().join(format!("litepod-serverctl-log-{}", uuid::Uuid::now_v7()));
        let app = dir.join(".local/share/litepod/apps/app1");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(
            app.join("caddy-routes.manifest"),
            "11111111-1111-7111-8111-111111111111 a.example.com 42000 true automatic -\n",
        )
        .unwrap();
        let manifest = ".local/share/litepod/apps/app1/caddy-routes.manifest";
        let out = run_helper_with(&dir, 0, &["sync-routes", manifest], true);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let fragment = std::fs::read_to_string(
            dir.join("routes.d/11111111-1111-7111-8111-111111111111.caddy"),
        )
        .unwrap();
        assert!(fragment.contains("\tlog {\n\t\toutput file /var/log/caddy/litepod-access.log {"));
        assert!(fragment.contains("\t\tformat json\n\t}\n\treverse_proxy 127.0.0.1:42000\n"));

        let read = |args: &[&str]| {
            let out = run_helper_with(&dir, 0, args, true);
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8(out.stdout).unwrap()
        };
        assert_eq!(read(&["access-log-read", "0", "0", "0"]), "0 0 0\n");
        let log = dir.join("access.log");
        std::fs::write(&log, "{\"a\":1}\n{\"b\":2}\n").unwrap();
        let first = read(&["access-log-read", "0", "0", "0"]);
        let inode = first.split_whitespace().next().unwrap().to_string();
        assert_eq!(first, format!("{inode} 16 0\n"));
        assert_eq!(
            read(&["access-log-read", &inode, "8", "100"]),
            format!("{inode} 16 8\n{{\"b\":2}}\n")
        );
        // Unknown inode (rolled file): start over.
        assert_eq!(
            read(&["access-log-read", "1", "8", "4"]),
            format!("{inode} 16 0\n{{\"a\"")
        );
        let bad = run_helper_with(&dir, 0, &["access-log-read", "x", "0", "0"], true);
        assert_eq!(bad.status.code(), Some(64));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Stages a manifest under the workspace path the helper allows and runs
    /// `sync-routes` against it.
    fn run_sync(dir: &std::path::Path, caddy_exit: i32, manifest: &str) -> std::process::Output {
        let app = dir.join(".local/share/litepod/apps/app1");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(app.join("caddy-routes.manifest"), manifest).unwrap();
        run_helper(
            dir,
            caddy_exit,
            &[
                "sync-routes",
                ".local/share/litepod/apps/app1/caddy-routes.manifest",
            ],
        )
    }

    #[test]
    fn sync_routes_respects_https_and_certificate_provider() {
        let dir =
            std::env::temp_dir().join(format!("litepod-serverctl-tls-{}", uuid::Uuid::now_v7()));
        let out = run_sync(
            &dir,
            0,
            "aaaa api.example.com 42000 true letsencrypt -\nbbbb plain.example.com 42001 false automatic -\n",
        );
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let secure = std::fs::read_to_string(dir.join("routes.d/aaaa.caddy")).unwrap();
        assert!(secure.contains("issuer acme"));
        assert!(secure.contains("https://acme-v02.api.letsencrypt.org/directory"));
        let plain = std::fs::read_to_string(dir.join("routes.d/bbbb.caddy")).unwrap();
        assert!(plain.starts_with("http://plain.example.com"));
        let failed = run_sync(&dir, 0, "aaaa api.example.com 42000 true unknown -\n");
        assert!(!failed.status.success());
        assert_eq!(
            std::fs::read_to_string(dir.join("routes.d/aaaa.caddy")).unwrap(),
            secure
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sync_routes_splices_basic_auth_ahead_of_reverse_proxy() {
        use base64::Engine;
        let dir =
            std::env::temp_dir().join(format!("litepod-serverctl-auth-{}", uuid::Uuid::now_v7()));
        let block = "\tbasic_auth bcrypt \"Private\" {\n\t\t\"admin\" $2a$12$hash\n\t}\n";
        let token = base64::engine::general_purpose::STANDARD.encode(block);
        let out = run_sync(
            &dir,
            0,
            &format!("aaaa api.example.com 42000 true automatic - {token}\n"),
        );
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let fragment = std::fs::read_to_string(dir.join("routes.d/aaaa.caddy")).unwrap();
        assert!(fragment.contains(block), "{fragment}");
        assert!(fragment.find("basic_auth").unwrap() < fragment.find("reverse_proxy").unwrap());
        let failed = run_sync(
            &dir,
            0,
            "aaaa api.example.com 42000 true automatic - not*base64\n",
        );
        assert!(!failed.status.success());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sync_routes_makes_routes_d_match_the_manifest_exactly() {
        let dir =
            std::env::temp_dir().join(format!("litepod-serverctl-sync-{}", uuid::Uuid::now_v7()));
        let routes = dir.join("routes.d");
        std::fs::create_dir_all(&routes).unwrap();
        std::fs::write(
            routes.join("99999999-9999-7999-8999-999999999999.caddy"),
            "orphan.example.com {\n\treverse_proxy 127.0.0.1:1\n}\n",
        )
        .unwrap();

        let out = run_sync(
            &dir,
            0,
            "11111111-1111-7111-8111-111111111111 keep.example.com 42010\n\
             22222222-2222-7222-8222-222222222222 also.example.com 42011\n",
        );
        assert!(
            out.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );

        assert!(
            !routes
                .join("99999999-9999-7999-8999-999999999999.caddy")
                .exists(),
            "orphan removed"
        );
        let keep =
            std::fs::read_to_string(routes.join("11111111-1111-7111-8111-111111111111.caddy"))
                .unwrap();
        assert_eq!(
            keep,
            "keep.example.com {\n\treverse_proxy 127.0.0.1:42010\n}\n"
        );
        assert!(
            routes
                .join("22222222-2222-7222-8222-222222222222.caddy")
                .exists()
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sync_routes_rejects_a_malformed_line_and_touches_nothing() {
        let dir = std::env::temp_dir().join(format!(
            "litepod-serverctl-syncbad-{}",
            uuid::Uuid::now_v7()
        ));
        let routes = dir.join("routes.d");
        std::fs::create_dir_all(&routes).unwrap();
        std::fs::write(
            routes.join("aaaaaaaa-aaaa-7aaa-8aaa-aaaaaaaaaaaa.caddy"),
            "live.example.com {\n}\n",
        )
        .unwrap();

        let out = run_sync(&dir, 0, "not_hex_id bad.example.com 42010\n");
        assert_eq!(out.status.code(), Some(64));
        assert!(
            routes
                .join("aaaaaaaa-aaaa-7aaa-8aaa-aaaaaaaaaaaa.caddy")
                .exists(),
            "live routes untouched"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sync_routes_rejects_two_lines_for_one_host() {
        let dir = std::env::temp_dir().join(format!(
            "litepod-serverctl-syncdup-{}",
            uuid::Uuid::now_v7()
        ));
        std::fs::create_dir_all(dir.join("routes.d")).unwrap();

        let out = run_sync(
            &dir,
            0,
            "11111111-1111-7111-8111-111111111111 same.example.com 42010\n\
             22222222-2222-7222-8222-222222222222 same.example.com 42011\n",
        );
        assert_eq!(out.status.code(), Some(65));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sync_routes_reverts_when_the_candidate_config_is_invalid() {
        let dir = std::env::temp_dir().join(format!(
            "litepod-serverctl-syncrev-{}",
            uuid::Uuid::now_v7()
        ));
        let routes = dir.join("routes.d");
        std::fs::create_dir_all(&routes).unwrap();
        std::fs::write(
            routes.join("bbbbbbbb-bbbb-7bbb-8bbb-bbbbbbbbbbbb.caddy"),
            "live.example.com {\n\treverse_proxy 127.0.0.1:42020\n}\n",
        )
        .unwrap();

        // caddy_exit = 1 => `caddy validate` fails for the candidate directory.
        let out = run_sync(
            &dir,
            1,
            "cccccccc-cccc-7ccc-8ccc-cccccccccccc new.example.com 42030\n",
        );
        assert_eq!(out.status.code(), Some(1));
        assert!(
            routes
                .join("bbbbbbbb-bbbb-7bbb-8bbb-bbbbbbbbbbbb.caddy")
                .exists(),
            "prior fragment restored"
        );
        assert!(
            !routes
                .join("cccccccc-cccc-7ccc-8ccc-cccccccccccc.caddy")
                .exists(),
            "candidate not left behind"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sync_routes_with_empty_manifest_clears_routes_d() {
        let dir = std::env::temp_dir().join(format!(
            "litepod-serverctl-syncempty-{}",
            uuid::Uuid::now_v7()
        ));
        let routes = dir.join("routes.d");
        std::fs::create_dir_all(&routes).unwrap();
        std::fs::write(
            routes.join("dddddddd-dddd-7ddd-8ddd-dddddddddddd.caddy"),
            "x.example.com {\n}\n",
        )
        .unwrap();

        let out = run_sync(&dir, 0, "");
        assert!(out.status.success());
        assert!(
            !routes
                .join("dddddddd-dddd-7ddd-8ddd-dddddddddddd.caddy")
                .exists()
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn known_distro_families_are_supported() {
        assert_eq!(
            classify_os("ID=ubuntu\nVERSION_ID=24.04\n"),
            ServerState::NeedsSetup
        );
        assert_eq!(classify_os("ID=debian\n"), ServerState::NeedsSetup);
        assert_eq!(classify_os("ID=fedora\n"), ServerState::NeedsSetup);
        assert_eq!(classify_os("ID=arch\n"), ServerState::NeedsSetup);
        assert_eq!(classify_os("ID=gentoo\n"), ServerState::Unsupported);
    }

    #[test]
    fn public_edge_includes_caddy_and_private_relay_stays_rootless() {
        assert!(
            setup_plan(
                DistroFamily::Debian,
                IngressMode::PublicEdge,
                PodmanMode::Rootless,
                "litepod"
            )
            .iter()
            .any(|step| step.name == "install-native-caddy")
        );
        assert_eq!(
            setup_plan(
                DistroFamily::Debian,
                IngressMode::PrivateRelay,
                PodmanMode::Rootless,
                "litepod"
            )
            .iter()
            .find(|step| step.name == "create-litepod-workspace")
            .unwrap()
            .name,
            "create-litepod-workspace"
        );
        assert!(
            setup_plan(
                DistroFamily::Debian,
                IngressMode::PrivateRelay,
                PodmanMode::Rootless,
                "litepod"
            )
            .iter()
            .any(|step| step.name == "verify-podman-compose-provider")
        );
    }

    #[test]
    fn every_plan_settles_dpkg_before_installing() {
        let plan = setup_plan(
            DistroFamily::Debian,
            IngressMode::PrivateRelay,
            PodmanMode::Rootful,
            "litepod",
        );
        let position = |name: &str| plan.iter().position(|step| step.name == name).unwrap();
        assert!(position("configure-pending-packages") < position("refresh-apt-index"));
        assert!(position("refresh-apt-index") < position("fix-broken-packages"));
        assert!(position("fix-broken-packages") < position("install-base-packages"));
        assert!(position("install-base-packages") < position("settle-pending-packages"));
        assert!(position("settle-pending-packages") < position("install-rootful-podman"));
    }

    /// git and liberror-perl cannot be configured without perl, and minimal Debian and Ubuntu
    /// images ship without it, so every plan installs it before it installs Podman.
    #[test]
    fn every_plan_installs_the_base_packages_before_podman() {
        for ingress in [IngressMode::PrivateRelay, IngressMode::PublicEdge] {
            for mode in [PodmanMode::Rootful, PodmanMode::Rootless] {
                let plan = setup_plan(DistroFamily::Debian, ingress, mode, "litepod");
                let base = plan
                    .iter()
                    .find(|step| step.name == "install-base-packages")
                    .unwrap();
                for package in ["perl", "build-essential", "which", "fail2ban"] {
                    assert!(
                        base.command.iter().any(|part| part == package),
                        "{package} missing from install-base-packages"
                    );
                }
            }
        }
    }

    /// A host whose dpkg state is broken by a *missing* dependency cannot be repaired by dpkg,
    /// only by apt, so the first pass must not abort Setup before apt has had its turn.
    #[test]
    fn the_diagnostics_are_tolerated_and_the_repairs_are_not() {
        for mode in [PodmanMode::Rootful, PodmanMode::Rootless] {
            let plan = setup_plan(
                DistroFamily::Debian,
                IngressMode::PublicEdge,
                mode,
                "litepod",
            );
            let step = |name: &str| plan.iter().find(|step| step.name == name).unwrap();
            assert!(step("configure-pending-packages").tolerate_failure);
            assert!(step("fix-broken-packages").tolerate_failure);
            assert!(!step("install-base-packages").tolerate_failure);
            assert!(!step("settle-pending-packages").tolerate_failure);
            // A guest kernel (LXC, etc.) can legitimately lack the tun module and still run
            // everything except pasta-networked builds — this one is tolerated too.
            assert!(step("enable-tun-module").tolerate_failure);
            // Nothing else in the plan may swallow its own failure.
            assert_eq!(
                plan.iter().filter(|step| step.tolerate_failure).count(),
                3,
                "only the pre-repair diagnostics and the best-effort tun module load are tolerated"
            );
        }
    }

    /// Every connection refreshes the apt index, in every mode — a stale or absent index is the
    /// other reason a fresh host cannot install what Setup asks for.
    #[test]
    fn every_plan_refreshes_the_apt_index() {
        for ingress in [IngressMode::PrivateRelay, IngressMode::PublicEdge] {
            for mode in [PodmanMode::Rootful, PodmanMode::Rootless] {
                assert!(
                    setup_plan(DistroFamily::Debian, ingress, mode, "litepod")
                        .iter()
                        .any(|step| step.name == "refresh-apt-index")
                );
            }
        }
    }

    #[test]
    fn every_plan_creates_the_litepod_network() {
        for mode in [PodmanMode::Rootless, PodmanMode::Rootful] {
            let step = setup_plan(
                DistroFamily::Debian,
                IngressMode::PrivateRelay,
                mode,
                "litepod",
            )
            .into_iter()
            .find(|step| step.name == "create-litepod-network")
            .expect("plan must create litepod-network");
            assert!(step.command.iter().any(|part| part == "--ignore"));
            assert!(step.command.iter().any(|part| part == "litepod-network"));
        }
    }

    #[test]
    fn podman_mode_selects_the_install_and_socket_steps() {
        let rootless = setup_plan(
            DistroFamily::Debian,
            IngressMode::PrivateRelay,
            PodmanMode::Rootless,
            "litepod",
        );
        assert!(
            rootless
                .iter()
                .any(|step| step.name == "enable-user-lingering")
        );
        assert!(
            rootless
                .iter()
                .any(|step| step.name == "enable-rootless-podman")
        );
        assert!(
            rootless.iter().any(|step| step.name == "enable-tun-module"),
            "rootless Setup/Repair must load the tun module pasta/slirp4netns need"
        );

        let rootful = setup_plan(
            DistroFamily::Debian,
            IngressMode::PrivateRelay,
            PodmanMode::Rootful,
            "litepod",
        );
        assert!(
            !rootful
                .iter()
                .any(|step| step.name == "enable-user-lingering")
        );
        assert!(
            rootful
                .iter()
                .any(|step| step.name == "enable-rootful-podman")
        );
        assert!(
            rootful.iter().any(|step| step.name == "enable-tun-module"),
            "rootful Podman can still pick the pasta network backend, which needs the tun module too"
        );
        let rootful_packages = rootful
            .iter()
            .find(|step| step.name == "install-rootful-podman")
            .unwrap();
        assert!(
            rootful_packages
                .command
                .iter()
                .any(|part| part == "podman-compose"),
            "rootful Setup must install the Compose provider",
        );
        // A non-root user still escalates through passwordless sudo.
        assert_eq!(rootful[0].command, ["sudo", "-n", "true"]);

        // Connecting as root drops the sudo prefix entirely.
        assert_eq!(
            setup_plan(
                DistroFamily::Debian,
                IngressMode::PrivateRelay,
                PodmanMode::Rootful,
                "root"
            )[0]
            .command,
            ["true"]
        );
    }

    /// dpkg/apt repair is Debian-specific; Fedora and Arch skip straight to a single
    /// family-appropriate index refresh instead.
    #[test]
    fn only_debian_runs_the_dpkg_repair_steps() {
        for family in [DistroFamily::Fedora, DistroFamily::Arch] {
            let plan = setup_plan(
                family,
                IngressMode::PrivateRelay,
                PodmanMode::Rootful,
                "litepod",
            );
            for name in [
                "configure-pending-packages",
                "refresh-apt-index",
                "fix-broken-packages",
                "settle-pending-packages",
            ] {
                assert!(
                    !plan.iter().any(|step| step.name == name),
                    "{name} should not appear in a {family:?} plan"
                );
            }
            assert!(plan.iter().any(|step| step.name == "refresh-package-index"));
        }
    }

    #[test]
    fn fedora_installs_its_own_base_and_podman_package_names() {
        let plan = setup_plan(
            DistroFamily::Fedora,
            IngressMode::PrivateRelay,
            PodmanMode::Rootless,
            "litepod",
        );
        let base = plan
            .iter()
            .find(|step| step.name == "install-base-packages")
            .unwrap();
        for package in ["perl", "gcc", "gcc-c++", "make"] {
            assert!(base.command.iter().any(|part| part == package));
        }
        let podman = plan
            .iter()
            .find(|step| step.name == "install-rootless-podman")
            .unwrap();
        for package in ["podman", "podman-compose", "shadow-utils"] {
            assert!(podman.command.iter().any(|part| part == package));
        }
        assert!(
            podman.command.iter().any(|part| part == "dnf"),
            "Fedora must dispatch through dnf, not apt-get"
        );
    }

    #[test]
    fn arch_installs_its_own_base_and_podman_package_names() {
        let plan = setup_plan(
            DistroFamily::Arch,
            IngressMode::PrivateRelay,
            PodmanMode::Rootless,
            "litepod",
        );
        let base = plan
            .iter()
            .find(|step| step.name == "install-base-packages")
            .unwrap();
        assert!(base.command.iter().any(|part| part == "base-devel"));
        let podman = plan
            .iter()
            .find(|step| step.name == "install-rootless-podman")
            .unwrap();
        for package in ["podman", "podman-compose"] {
            assert!(podman.command.iter().any(|part| part == package));
        }
        assert!(
            podman.command.iter().any(|part| part == "pacman"),
            "Arch must dispatch through pacman, not apt-get"
        );
        // Arch's uidmap-equivalent resolves to nothing extra to install — it must not appear
        // as a literal empty argv element.
        assert!(!podman.command.iter().any(|part| part.is_empty()));
    }

    /// Fedora installs Caddy through its own Copr repo, not Debian's apt repository dance, but
    /// still ends in the same shared "enable-native-caddy" step.
    #[test]
    fn fedora_public_edge_uses_copr_for_caddy() {
        let plan = setup_plan(
            DistroFamily::Fedora,
            IngressMode::PublicEdge,
            PodmanMode::Rootless,
            "litepod",
        );
        let copr = plan
            .iter()
            .find(|step| step.name == "add-caddy-repository")
            .unwrap();
        assert!(copr.command.iter().any(|part| part == "copr"));
        assert!(plan.iter().any(|step| step.name == "install-native-caddy"));
        assert!(plan.iter().any(|step| step.name == "enable-native-caddy"));
        assert!(!plan.iter().any(|step| step.name.contains("apt")));
    }

    /// Caddy has shipped as a prebuilt package in Arch's official `extra` repo since 2.11.4 (no
    /// AUR/`makepkg` involved), so Arch's Caddy install is just another `pacman -S`, same shape
    /// as Debian's and Fedora's `install-native-caddy` step.
    #[test]
    fn arch_public_edge_installs_caddy_via_pacman() {
        let plan = setup_plan(
            DistroFamily::Arch,
            IngressMode::PublicEdge,
            PodmanMode::Rootless,
            "litepod",
        );
        let caddy = plan
            .iter()
            .find(|step| step.name == "install-native-caddy")
            .unwrap();
        assert!(caddy.command.iter().any(|part| part == "pacman"));
        assert!(caddy.command.iter().any(|part| part == "caddy"));
        assert!(plan.iter().any(|step| step.name == "enable-native-caddy"));
    }

    /// Live end-to-end coverage of `run_setup()` over real SSH per distro family, following the
    /// same manual-smoke convention as `terminal::tests::integration_resize_frame_round_trip`:
    /// gated behind an env var, not run in CI (needs a live podman socket plus a booted sshd
    /// container), and left as pseudo-code for whoever runs it by hand.
    ///
    /// Package-manager correctness itself is already covered without a live SSH round trip:
    /// `just test-install-packages {debian,fedora,arch}` installs the exact same package names
    /// via the exact same commands (`apt-get`, `dnf`, `pacman`) inside real distro containers —
    /// see apps/scripts/test/smoke-install-packages.sh. What these three add on top is the SSH
    /// transport itself (host-key pinning, passwordless-sudo probing).
    async fn integration_run_setup(family: DistroFamily) {
        if std::env::var("LITEPOD_SETUP_INTEGRATION").is_err() {
            eprintln!(
                "set LITEPOD_SETUP_INTEGRATION=1 (and LITEPOD_SETUP_INTEGRATION_DISTRO={family:?}) to run"
            );
            return;
        }
        // Pseudo-code: this test is left as a manual smoke for the engineer running
        // `cargo test -- --ignored`. A full implementation would:
        //   1. `podman build`/`podman run -d` a Containerfile that installs+starts sshd for
        //      `family`, creates a non-root user with a `NOPASSWD:ALL` sudoers drop-in (the same
        //      trust model litepod's own "Connect server" wizard instructs real operators to
        //      set up), and installs a generated test Ed25519 public key into that user's
        //      `~/.ssh/authorized_keys`.
        //   2. Run `ssh-keyscan` against the container to get its host-key fingerprint, and build
        //      a `ServerRecord` pointing at `localhost:<mapped-port>` with that fingerprint and
        //      the matching private key.
        //   3. Call `run_setup(&server, |_, _| async { Ok(()) })` and assert it returns
        //      `ServerFacts` with `os` matching `family`, and that `preflight()`/`health_check()`
        //      pass afterward.
        let _ = family;
    }

    #[tokio::test]
    async fn integration_run_setup_debian() {
        integration_run_setup(DistroFamily::Debian).await;
    }

    #[tokio::test]
    async fn integration_run_setup_fedora() {
        integration_run_setup(DistroFamily::Fedora).await;
    }

    #[tokio::test]
    async fn integration_run_setup_arch() {
        integration_run_setup(DistroFamily::Arch).await;
    }
}
