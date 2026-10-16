use std::{
    ffi::{OsStr, OsString},
    fs::{OpenOptions, create_dir_all},
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    time::Duration,
};

use tokio::process::Command;
use tracing::warn;
use uuid::Uuid;

use crate::{
    dtos::response::AppError,
    servers::model::SshTarget,
    utils::security::{decrypt_data, encrypt_data, get_encryption_key},
};

pub struct GeneratedKey {
    pub private_key_encrypted: String,
    pub public_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostFingerprint {
    pub value: String,
    known_hosts_line: String,
}

impl HostFingerprint {
    pub fn known_hosts_line(&self) -> &str {
        &self.known_hosts_line
    }

    pub fn into_known_hosts_line(self) -> String {
        self.known_hosts_line
    }
}

#[derive(Debug, Clone)]
pub struct SshOutput {
    pub stdout: String,
    pub stderr: String,
}

/// Output of a remote command that is allowed to fail, so the exit code is part of the result
/// instead of being folded into an error.
#[derive(Debug, Clone)]
pub struct SshRun {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

/// Raw SSH command output for the small set of callers that transfer a tar
/// archive.  Keeping this separate from `SshOutput` prevents accidental UTF-8
/// loss when a workspace contains binary files.
#[derive(Debug, Clone)]
pub struct SshBytesOutput {
    pub stdout: Vec<u8>,
}

pub struct SecretFile(PathBuf);

/// A supervised local SSH forwarding process. The identity and known-hosts
/// files intentionally remain owned for the entire child lifetime.
pub struct SshRelay {
    child: tokio::process::Child,
    _identity: SecretFile,
    _known_hosts: SecretFile,
}

/// A per-terminal SSH forwarding session from a private local Unix socket to
/// the remote Podman API socket. It owns both SSH credential files and the
/// socket directory for its entire lifetime.
pub struct SshUnixRelay {
    child: tokio::process::Child,
    _identity: SecretFile,
    _known_hosts: SecretFile,
    socket_dir: PathBuf,
    socket_path: PathBuf,
}

/// A long-lived command whose output is consumed incrementally instead of being
/// collected. Like [`SshRelay`] it owns its credential files for the whole child
/// lifetime; dropping it kills the process, which for the SSH case ends the
/// remote command with the session. A locally spawned child carries no
/// credentials, so both placements can share one type.
pub struct SshStream {
    child: tokio::process::Child,
    _identity: Option<SecretFile>,
    _known_hosts: Option<SecretFile>,
}

impl SshStream {
    /// Wraps an already-spawned local child, for callers that reach Podman
    /// directly rather than over SSH.
    pub const fn local(child: tokio::process::Child) -> Self {
        Self {
            child,
            _identity: None,
            _known_hosts: None,
        }
    }

    /// Takes the child's stdout once, for the caller to read line by line.
    pub fn take_stdout(&mut self) -> Option<tokio::process::ChildStdout> {
        self.child.stdout.take()
    }

    /// Takes the child's stderr once. Podman reports a missing container there,
    /// so a caller that finds no stdout needs it to explain the failure.
    pub fn take_stderr(&mut self) -> Option<tokio::process::ChildStderr> {
        self.child.stderr.take()
    }

    /// Waits for the process to exit, for a caller that needs its exit status
    /// rather than an indefinite follow (e.g. streaming a bounded command like
    /// `podman compose up` instead of `podman logs -f`).
    pub async fn wait(&mut self) -> Result<std::process::ExitStatus, AppError> {
        self.child.wait().await.map_err(|error| {
            AppError::InternalError(format!("Could not wait for the remote command: {error}"))
        })
    }

    /// Kills the process, for a caller enforcing its own timeout on a streamed
    /// command. `kill_on_drop` already covers an abandoned stream; this is for
    /// a caller that wants the exit status of the kill itself.
    pub async fn kill(&mut self) {
        let _ = self.child.kill().await;
    }
}

impl SshRelay {
    pub fn is_running(&mut self) -> Result<bool, AppError> {
        Ok(self
            .child
            .try_wait()
            .map_err(|error| {
                AppError::InternalError(format!("Could not inspect SSH relay: {error}"))
            })?
            .is_none())
    }

    pub async fn stop(mut self) {
        let _ = self.child.start_kill();
        let _ = self.child.wait().await;
    }
}

impl SshUnixRelay {
    pub fn socket_url(&self) -> String {
        format!("unix://{}", self.socket_path.display())
    }
}

impl Drop for SshUnixRelay {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
        let _ = std::fs::remove_file(&self.socket_path);
        let _ = std::fs::remove_dir(&self.socket_dir);
    }
}

impl SecretFile {
    pub fn write(contents: &str) -> Result<Self, AppError> {
        let path = std::env::temp_dir().join(format!("litepod-ssh-{}", Uuid::now_v7()));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .map_err(|error| {
                AppError::InternalError(format!(
                    "Could not create temporary SSH credential: {error}"
                ))
            })?;
        file.write_all(contents.as_bytes()).map_err(|error| {
            AppError::InternalError(format!("Could not write temporary SSH credential: {error}"))
        })?;
        Ok(Self(path))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for SecretFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

pub fn validate_imported_private_key(value: &str) -> Result<(), AppError> {
    if !value.starts_with("-----BEGIN OPENSSH PRIVATE KEY-----")
        || !value.contains("-----END OPENSSH PRIVATE KEY-----")
    {
        return Err(AppError::ValidationError(
            "Import an unencrypted OpenSSH private key".into(),
        ));
    }
    if value.to_ascii_uppercase().contains("ENCRYPTED") {
        return Err(AppError::ValidationError(
            "Passphrase-protected private keys are not supported for unattended deployments".into(),
        ));
    }
    Ok(())
}

pub fn redact(value: &str) -> String {
    let mut redacted = String::with_capacity(value.len());
    let mut hiding = false;
    for line in value.lines() {
        if line.contains("BEGIN OPENSSH PRIVATE KEY") {
            hiding = true;
            redacted.push_str("[redacted SSH private key]\n");
            continue;
        }
        if hiding {
            if line.contains("END OPENSSH PRIVATE KEY") {
                hiding = false;
            }
            continue;
        }
        redacted.push_str(line);
        redacted.push('\n');
    }
    redacted
}

/// Keeps the tail of a command's stderr instead of its head. Builders and
/// registries print progress first and the failing step last, so truncating
/// from the front would hide exactly the line that explains the failure.
pub fn clamp_tail(value: &str, limit: usize) -> String {
    let trimmed = value.trim();
    if trimmed.chars().count() <= limit {
        return trimmed.to_string();
    }
    trimmed
        .chars()
        .rev()
        .take(limit)
        .collect::<String>()
        .chars()
        .rev()
        .collect()
}

/// Every remote command must be a closed argv the caller built itself. An empty
/// command would turn the SSH invocation into an interactive login shell, and a
/// NUL byte cannot survive the exec boundary intact.
fn validate_remote_argv(command: &[OsString]) -> Result<(), AppError> {
    if command.is_empty()
        || command
            .iter()
            .any(|part| part.to_string_lossy().contains('\0'))
    {
        return Err(AppError::InternalError(
            "Refusing an invalid SSH command".into(),
        ));
    }
    Ok(())
}

/// Quotes one argv element for the remote shell. OpenSSH joins the trailing
/// command args with spaces and the remote shell re-parses them, so a single
/// element containing spaces or quotes (e.g. a scheduled task's `sh -c <script>`)
/// would otherwise be split or mangled. POSIX single-quote escaping preserves
/// the exact argument boundaries across that re-parse.
fn shell_quote(value: &OsStr) -> OsString {
    let mut quoted = String::from("'");
    for character in value.to_string_lossy().chars() {
        if character == '\'' {
            quoted.push_str("'\\''");
        } else {
            quoted.push(character);
        }
    }
    quoted.push('\'');
    OsString::from(quoted)
}

pub fn build_ssh_argv(
    target: &SshTarget,
    identity: &Path,
    known_hosts: &Path,
    command: &[OsString],
) -> Vec<OsString> {
    let mut args = vec![
        OsString::from("ssh"),
        OsString::from("-o"),
        OsString::from("BatchMode=yes"),
        OsString::from("-o"),
        OsString::from("PasswordAuthentication=no"),
        OsString::from("-o"),
        OsString::from("KbdInteractiveAuthentication=no"),
        OsString::from("-o"),
        OsString::from("IdentitiesOnly=yes"),
        OsString::from("-o"),
        OsString::from("StrictHostKeyChecking=yes"),
        OsString::from("-o"),
        OsString::from(format!("UserKnownHostsFile={}", known_hosts.display())),
        OsString::from("-o"),
        OsString::from("ConnectTimeout=10"),
        OsString::from("-o"),
        OsString::from("ControlMaster=auto"),
        OsString::from("-o"),
        OsString::from(format!("ControlPath={}", ssh_control_path().display())),
        OsString::from("-o"),
        OsString::from("ControlPersist=120"),
        // Detects a dead transport (network drop, sleeping remote host) in
        // ~45s instead of waiting on the OS-level TCP timeout, which can take
        // many minutes and leave a deploy looking hung rather than failed.
        OsString::from("-o"),
        OsString::from("ServerAliveInterval=15"),
        OsString::from("-o"),
        OsString::from("ServerAliveCountMax=3"),
        OsString::from("-i"),
        identity.as_os_str().to_owned(),
        OsString::from("-p"),
        OsString::from(target.port.to_string()),
        OsString::from(format!("{}@{}", target.user, target.host)),
    ];
    args.extend(command.iter().map(|arg| shell_quote(arg.as_os_str())));
    args
}

/// A mode-0700 directory prevents other local users from attaching to a
/// multiplexed SSH control socket. `%C` is expanded by OpenSSH from the
/// connection tuple, keeping the socket stable per destination but short.
fn ssh_control_path() -> PathBuf {
    PathBuf::from("/tmp").join("lp-ssh").join("c-%C")
}

fn ensure_ssh_control_dir() -> Result<(), AppError> {
    let control_path = ssh_control_path();
    let directory = control_path
        .parent()
        .expect("SSH control path always has a parent");
    create_dir_all(directory).map_err(|error| {
        AppError::InternalError(format!("Could not create SSH control directory: {error}"))
    })?;
    std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700)).map_err(
        |error| AppError::InternalError(format!("Could not secure SSH control directory: {error}")),
    )?;
    Ok(())
}

fn build_scp_argv(
    target: &SshTarget,
    identity: &Path,
    known_hosts: &Path,
    local: &Path,
    remote_path: &str,
) -> Vec<OsString> {
    vec![
        OsString::from("scp"),
        OsString::from("-q"),
        OsString::from("-o"),
        OsString::from("BatchMode=yes"),
        OsString::from("-o"),
        OsString::from("PasswordAuthentication=no"),
        OsString::from("-o"),
        OsString::from("KbdInteractiveAuthentication=no"),
        OsString::from("-o"),
        OsString::from("IdentitiesOnly=yes"),
        OsString::from("-o"),
        OsString::from("StrictHostKeyChecking=yes"),
        OsString::from("-o"),
        OsString::from(format!("UserKnownHostsFile={}", known_hosts.display())),
        OsString::from("-o"),
        OsString::from("ConnectTimeout=10"),
        OsString::from("-o"),
        OsString::from("ControlMaster=auto"),
        OsString::from("-o"),
        OsString::from(format!("ControlPath={}", ssh_control_path().display())),
        OsString::from("-o"),
        OsString::from("ControlPersist=120"),
        OsString::from("-o"),
        OsString::from("ServerAliveInterval=15"),
        OsString::from("-o"),
        OsString::from("ServerAliveCountMax=3"),
        OsString::from("-i"),
        identity.as_os_str().to_owned(),
        OsString::from("-P"),
        OsString::from(target.port.to_string()),
        local.as_os_str().to_owned(),
        OsString::from(format!("{}@{}:{}", target.user, target.host, remote_path)),
    ]
}

pub struct SshExecutor;

impl SshExecutor {
    /// Resolves a caller-supplied shell name to a fixed remote command.
    /// Fish and Nushell may live under Homebrew rather than `/bin`, so their
    /// commands discover the executable in the user's login environment. The
    /// scripts are constants selected from this allowlist; request data is
    /// never interpolated into a remote shell command.
    pub fn resolve_shell(name: &str) -> Result<&'static [&'static str], AppError> {
        match name {
            "sh" => Ok(&["/bin/sh", "-l", "-i"]),
            "bash" => Ok(&["/bin/bash", "-l", "-i"]),
            "fish" => Ok(&[
                "/bin/sh",
                "-lc",
                r#"'shell=$(which fish) || { echo "fish is not installed or is not on PATH" >&2; exit 127; }; case "$shell" in /*) ;; *) echo "which fish returned an invalid path" >&2; exit 126;; esac; [ -x "$shell" ] || exit 126; exec "$shell" -l -i'"#,
            ]),
            "nu" => Ok(&[
                "/bin/sh",
                "-lc",
                r#"'shell=$(which nu) || { echo "nu is not installed or is not on PATH" >&2; exit 127; }; case "$shell" in /*) ;; *) echo "which nu returned an invalid path" >&2; exit 126;; esac; [ -x "$shell" ] || exit 126; exec "$shell" -l -i'"#,
            ]),
            "zsh" => Ok(&[
                "/bin/sh",
                "-lc",
                r#"'shell=$(which zsh) || { echo "zsh is not installed or is not on PATH" >&2; exit 127; }; case "$shell" in /*) ;; *) echo "which zsh returned an invalid path" >&2; exit 126;; esac; [ -x "$shell" ] || exit 126; exec "$shell" -l -i'"#,
            ]),
            // PowerShell takes -Login (first) instead of -l, and is interactive on a TTY.
            "pwsh" => Ok(&[
                "/bin/sh",
                "-lc",
                r#"'shell=$(which pwsh) || { echo "pwsh is not installed or is not on PATH" >&2; exit 127; }; case "$shell" in /*) ;; *) echo "which pwsh returned an invalid path" >&2; exit 126;; esac; [ -x "$shell" ] || exit 126; exec "$shell" -Login -NoLogo'"#,
            ]),
            _ => Err(AppError::ValidationError(
                "Shell must be sh, bash, fish, nu, zsh, or pwsh".into(),
            )),
        }
    }

    /// Resolves a shell for the debug-only direct local PTY path. Production
    /// consoles use SSH and [`Self::resolve_shell`], but keeping this resolver
    /// PATH-aware makes local development match the deployed behaviour.
    pub fn resolve_local_shell(name: &str) -> Result<OsString, AppError> {
        Self::resolve_shell(name)?;
        match name {
            "sh" => Ok("/bin/sh".into()),
            "bash" => Ok("/bin/bash".into()),
            "fish" | "nu" | "zsh" | "pwsh" => {
                let output = std::process::Command::new("which")
                    .arg(name)
                    .output()
                    .map_err(|_| {
                        AppError::ValidationError(
                            "which is required to locate fish, nu, zsh, and pwsh".into(),
                        )
                    })?;
                let path = String::from_utf8_lossy(&output.stdout).trim().to_owned();
                if !output.status.success()
                    || !path.starts_with('/')
                    || !std::path::Path::new(&path).is_file()
                {
                    return Err(AppError::ValidationError(format!(
                        "{name} is not installed or is not on PATH"
                    )));
                }
                Ok(path.into())
            }
            _ => unreachable!("resolve_shell already validated the name"),
        }
    }

    /// Builds a pinned-host-key SSH command for a browser-backed interactive PTY.
    /// `shell` is a bare name (see [`Self::resolve_shell`]), not a path.
    /// The returned secret files must outlive the spawned process.
    pub async fn interactive_shell_argv(
        target: &SshTarget,
        encrypted_private_key: &str,
        expected_fingerprint: &str,
        known_hosts_line: Option<&str>,
        shell: &str,
    ) -> Result<(Vec<OsString>, SecretFile, SecretFile), AppError> {
        let shell_command = Self::resolve_shell(shell)?;
        ensure_ssh_control_dir()?;
        let identity = Self::decrypt_key(encrypted_private_key)?;
        let known_hosts =
            Self::trusted_known_hosts(target, expected_fingerprint, known_hosts_line).await?;
        let mut argv = build_ssh_argv(
            target,
            identity.path(),
            known_hosts.path(),
            &shell_command.iter().map(OsString::from).collect::<Vec<_>>(),
        );
        let destination = argv.len() - 3;
        argv.insert(destination, "-tt".into());
        Ok((argv, identity, known_hosts))
    }
    pub async fn generate_key() -> Result<GeneratedKey, AppError> {
        let private = std::env::temp_dir().join(format!("litepod-generated-{}", Uuid::now_v7()));
        let output = Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-f"])
            .arg(&private)
            .output()
            .await
            .map_err(|error| {
                AppError::InternalError(format!("Could not start ssh-keygen: {error}"))
            })?;
        if !output.status.success() {
            return Err(AppError::InternalError(
                "ssh-keygen could not generate a server key".into(),
            ));
        }
        let private_key = std::fs::read_to_string(&private).map_err(|error| {
            AppError::InternalError(format!("Could not read generated SSH key: {error}"))
        })?;
        let public_key =
            std::fs::read_to_string(private.with_extension("pub")).map_err(|error| {
                AppError::InternalError(format!("Could not read generated SSH public key: {error}"))
            })?;
        let _ = std::fs::remove_file(&private);
        let _ = std::fs::remove_file(private.with_extension("pub"));
        Ok(GeneratedKey {
            private_key_encrypted: encrypt_data(&private_key, &get_encryption_key())
                .map_err(AppError::InternalError)?,
            public_key,
        })
    }

    pub fn decrypt_key(encrypted: &str) -> Result<SecretFile, AppError> {
        let private_key =
            decrypt_data(encrypted, &get_encryption_key()).map_err(AppError::InternalError)?;
        SecretFile::write(&private_key)
    }

    /// Recomputes the public half of a stored key so a sysadmin can copy it into
    /// `authorized_keys` again. The private key stays inside a 0600 temporary file that is
    /// deleted when `SecretFile` drops, and is never returned to the caller.
    pub async fn public_key_from_encrypted(encrypted: &str) -> Result<String, AppError> {
        let identity = Self::decrypt_key(encrypted)?;
        let output = Command::new("ssh-keygen")
            .args(["-y", "-f"])
            .arg(identity.path())
            .output()
            .await
            .map_err(|error| {
                AppError::InternalError(format!("Could not start ssh-keygen: {error}"))
            })?;
        if !output.status.success() {
            return Err(AppError::InternalError(format!(
                "Could not derive the SSH public key: {}",
                redact(&String::from_utf8_lossy(&output.stderr))
                    .trim()
                    .chars()
                    .take(200)
                    .collect::<String>()
            )));
        }
        let public_key = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if public_key.is_empty() {
            return Err(AppError::InternalError(
                "ssh-keygen returned an empty SSH public key".into(),
            ));
        }
        Ok(public_key)
    }

    /// Discover an ED25519 host key without opening an SSH session. The caller must show this
    /// fingerprint to a sysadmin and require an explicit confirmation before storing it.
    pub async fn discover_host_key(target: &SshTarget) -> Result<HostFingerprint, AppError> {
        let scan = tokio::time::timeout(
            Duration::from_secs(15),
            Command::new("ssh-keyscan")
                .args(["-T", "10", "-t", "ed25519", "-p"])
                .arg(target.port.to_string())
                .arg(&target.host)
                .output(),
        )
        .await
        .map_err(|_| AppError::ValidationError("SSH host-key scan timed out".into()))?
        .map_err(|error| {
            AppError::InternalError(format!("Could not start ssh-keyscan: {error}"))
        })?;

        let known_hosts_line = String::from_utf8(scan.stdout)
            .map_err(|_| AppError::ValidationError("SSH host-key scan returned invalid text".into()))?
            .lines()
            .find(|line| !line.is_empty() && !line.starts_with('#'))
            .map(str::to_owned)
            .ok_or_else(|| AppError::ValidationError("No ED25519 SSH host key was found; make sure the server is directly reachable and SSH is running".into()))?;
        let key_file = SecretFile::write(&known_hosts_line)?;
        let output = Command::new("ssh-keygen")
            .args(["-lf"])
            .arg(key_file.path())
            .output()
            .await
            .map_err(|error| {
                AppError::InternalError(format!("Could not start ssh-keygen: {error}"))
            })?;
        if !output.status.success() {
            return Err(AppError::ValidationError(
                "The SSH host returned an invalid ED25519 host key".into(),
            ));
        }
        let description = String::from_utf8_lossy(&output.stdout);
        let fingerprint = description
            .split_whitespace()
            .find(|part| part.starts_with("SHA256:"))
            .ok_or_else(|| {
                AppError::ValidationError("Could not calculate the SSH host-key fingerprint".into())
            })?;
        Ok(HostFingerprint {
            value: fingerprint.to_string(),
            known_hosts_line,
        })
    }

    /// Runs a fixed remote command with the host key explicitly confirmed by an
    /// administrator. Fingerprint-only rows are scanned only as an upgrade
    /// fallback before they acquire a persisted known_hosts entry.
    pub async fn run(
        target: &SshTarget,
        encrypted_private_key: &str,
        expected_fingerprint: &str,
        known_hosts_line: Option<&str>,
        command: &[OsString],
    ) -> Result<SshOutput, AppError> {
        Self::run_with_timeout(
            target,
            encrypted_private_key,
            expected_fingerprint,
            known_hosts_line,
            command,
            Duration::from_secs(45),
        )
        .await
    }

    /// Same as [`Self::run`] with an explicit budget, for steps such as `apt-get install` that
    /// routinely outlast a probe-sized timeout on a slow host or a cold package cache.
    pub async fn run_with_timeout(
        target: &SshTarget,
        encrypted_private_key: &str,
        expected_fingerprint: &str,
        known_hosts_line: Option<&str>,
        command: &[OsString],
        budget: Duration,
    ) -> Result<SshOutput, AppError> {
        let run = Self::run_allow_failure(
            target,
            encrypted_private_key,
            expected_fingerprint,
            known_hosts_line,
            command,
            budget,
        )
        .await?;
        if run.exit_code != 0 {
            return Err(AppError::ValidationError(format!(
                "SSH command failed: {}",
                clamp_tail(&run.stderr, 500)
            )));
        }
        Ok(SshOutput {
            stdout: run.stdout,
            stderr: run.stderr,
        })
    }

    /// Same connection guarantees as [`Self::run_with_timeout`], but a remote command that exits
    /// non-zero is an outcome rather than an error. Only callers that show the failure to the
    /// user verbatim — such as the server test deploy — should use this; everything else wants
    /// the failure to short-circuit. Transport problems are still errors.
    pub async fn run_allow_failure(
        target: &SshTarget,
        encrypted_private_key: &str,
        expected_fingerprint: &str,
        known_hosts_line: Option<&str>,
        command: &[OsString],
        budget: Duration,
    ) -> Result<SshRun, AppError> {
        validate_remote_argv(command)?;
        ensure_ssh_control_dir()?;
        let identity = Self::decrypt_key(encrypted_private_key)?;
        let known_hosts =
            Self::trusted_known_hosts(target, expected_fingerprint, known_hosts_line).await?;
        let argv = build_ssh_argv(target, identity.path(), known_hosts.path(), command);
        // Without kill_on_drop, a timeout here abandons the future but leaves the local
        // ssh client running - the channel never closes, so the remote command it's
        // driving (e.g. `podman compose up`) never dies either, and just keeps holding
        // its locks for every retry to pile up against.
        let output = tokio::time::timeout(
            budget,
            Command::new(&argv[0])
                .args(&argv[1..])
                .kill_on_drop(true)
                .output(),
        )
        .await
        .map_err(|_| AppError::ValidationError("SSH command timed out".into()))?
        .map_err(|error| AppError::InternalError(format!("Could not start SSH: {error}")))?;
        Ok(SshRun {
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: redact(&String::from_utf8_lossy(&output.stderr)),
            // A remote command killed by a signal reports no code; -1 keeps that distinct
            // from success without inventing an exit status the remote never produced.
            exit_code: output.status.code().unwrap_or(-1),
        })
    }

    /// Runs a fixed command and preserves stdout bytes.  This is intentionally
    /// limited to the same trusted-command boundary as `run`; it is used for
    /// tar archive transfer only, never to return arbitrary SSH output.
    pub async fn run_bytes(
        target: &SshTarget,
        encrypted_private_key: &str,
        expected_fingerprint: &str,
        known_hosts_line: Option<&str>,
        command: &[OsString],
        budget: Duration,
    ) -> Result<SshBytesOutput, AppError> {
        validate_remote_argv(command)?;
        ensure_ssh_control_dir()?;
        let identity = Self::decrypt_key(encrypted_private_key)?;
        let known_hosts =
            Self::trusted_known_hosts(target, expected_fingerprint, known_hosts_line).await?;
        let argv = build_ssh_argv(target, identity.path(), known_hosts.path(), command);
        let output = tokio::time::timeout(
            budget,
            Command::new(&argv[0])
                .args(&argv[1..])
                .kill_on_drop(true)
                .output(),
        )
        .await
        .map_err(|_| AppError::ValidationError("SSH command timed out".into()))?
        .map_err(|error| AppError::InternalError(format!("Could not start SSH: {error}")))?;
        let stderr = redact(&String::from_utf8_lossy(&output.stderr));
        if !output.status.success() {
            return Err(AppError::ValidationError(format!(
                "SSH command failed: {}",
                clamp_tail(&stderr, 500)
            )));
        }
        Ok(SshBytesOutput {
            stdout: output.stdout,
        })
    }

    /// Starts a remote command whose output is streamed rather than collected, for
    /// follow-mode reads such as `podman logs -f`. Connection guarantees match
    /// [`Self::run_allow_failure`]; the difference is that no timeout bounds the
    /// child, because outliving the request is the point. The caller drops the
    /// returned [`SshStream`] to stop it: `kill_on_drop` ends the local `ssh`, and
    /// the remote command dies with the session.
    pub async fn stream_command(
        target: &SshTarget,
        encrypted_private_key: &str,
        expected_fingerprint: &str,
        known_hosts_line: Option<&str>,
        command: &[OsString],
    ) -> Result<SshStream, AppError> {
        validate_remote_argv(command)?;
        ensure_ssh_control_dir()?;
        let identity = Self::decrypt_key(encrypted_private_key)?;
        let known_hosts =
            Self::trusted_known_hosts(target, expected_fingerprint, known_hosts_line).await?;
        let argv = build_ssh_argv(target, identity.path(), known_hosts.path(), command);
        let child = Command::new(&argv[0])
            .args(&argv[1..])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| {
                AppError::InternalError(format!("Could not start SSH stream: {error}"))
            })?;
        Ok(SshStream {
            child,
            _identity: Some(identity),
            _known_hosts: Some(known_hosts),
        })
    }

    /// Starts a local forwarding-only SSH session. `bind_port` is reachable
    /// only inside litepod's Podman network (Caddy dials the API container), and
    /// the remote destination is always an application loopback socket.
    pub async fn start_local_forward(
        target: &SshTarget,
        encrypted_private_key: &str,
        expected_fingerprint: &str,
        known_hosts_line: Option<&str>,
        bind_port: u16,
        remote_port: u16,
    ) -> Result<SshRelay, AppError> {
        if bind_port == 0 || remote_port == 0 {
            return Err(AppError::ValidationError(
                "SSH relay ports must be between 1 and 65535".into(),
            ));
        }
        ensure_ssh_control_dir()?;
        let identity = Self::decrypt_key(encrypted_private_key)?;
        let known_hosts =
            Self::trusted_known_hosts(target, expected_fingerprint, known_hosts_line).await?;
        let mut argv = build_ssh_argv(target, identity.path(), known_hosts.path(), &[]);
        // ControlPath=none opts this connection out of the shared-master
        // default from build_ssh_argv: this child must own the forward for
        // its whole life (we track it and kill_on_drop it). ControlMaster=no
        // does NOT achieve that — per ssh_config(5), "no" is the value an
        // *additional* session uses to attach to an existing master at the
        // same ControlPath; it does not disable sharing. A slave attaching
        // that way has no pty/session to hold it open, so it hands off the
        // forward and exits 0 immediately - tripping the readiness check
        // right after with a false "tunnel exited" error, even though the
        // forward may be fine on the master (which we then have no handle to
        // tear down). Only ControlPath=none actually disables connection
        // sharing for this invocation.
        //
        // OpenSSH resolves repeated `-o key=value` first-occurrence-wins,
        // so this override must come *before* build_ssh_argv's own
        // ControlPath=... (argv[1..]) or it's silently ignored.
        argv.splice(
            1..1,
            [OsString::from("-o"), OsString::from("ControlPath=none")],
        );
        // The rest of the forwarding options go immediately before the
        // destination; none can be confused with a remote command.
        let destination = argv.len() - 1;
        argv.splice(
            destination..destination,
            [
                OsString::from("-o"),
                OsString::from("ExitOnForwardFailure=yes"),
                OsString::from("-N"),
                OsString::from("-L"),
                OsString::from(format!("0.0.0.0:{bind_port}:127.0.0.1:{remote_port}")),
            ],
        );
        let mut command = Command::new(&argv[0]);
        command.args(&argv[1..]).kill_on_drop(true);
        let mut child = command.spawn().map_err(|error| {
            AppError::InternalError(format!("Could not start SSH relay: {error}"))
        })?;
        // ExitOnForwardFailure makes a bad key, unavailable remote socket, or
        // local bind collision fail promptly. A living child after this short
        // window is the supervision boundary; later health/reconcile calls
        // replace a child that has exited.
        match tokio::time::timeout(Duration::from_secs(2), child.wait()).await {
            Ok(Ok(status)) => {
                warn!(%status, host = %target.host, "SSH relay exited before becoming ready");
                Err(AppError::ValidationError(format!(
                    "SSH relay exited before becoming ready ({status})"
                )))
            }
            Ok(Err(error)) => Err(AppError::InternalError(format!(
                "Could not wait for SSH relay: {error}"
            ))),
            Err(_) => Ok(SshRelay {
                child,
                _identity: identity,
                _known_hosts: known_hosts,
            }),
        }
    }

    /// Opens a StreamLocal forwarding session for Podman's Unix API socket.
    /// The local endpoint lives in a unique mode-0700 directory, so another
    /// process cannot attach to a user's interactive container session.
    pub async fn start_unix_socket_forward(
        target: &SshTarget,
        encrypted_private_key: &str,
        expected_fingerprint: &str,
        known_hosts_line: Option<&str>,
        remote_socket: &Path,
    ) -> Result<SshUnixRelay, AppError> {
        if !remote_socket.is_absolute() {
            return Err(AppError::InternalError(
                "Remote Podman socket path must be absolute".into(),
            ));
        }
        ensure_ssh_control_dir()?;
        // Same reasoning as ssh_control_path(): sockaddr_un.sun_path tops out at 104-108
        // bytes, and std::env::temp_dir() alone can already eat most of that (macOS's
        // TMPDIR is a long per-process /var/folders/... path), so this must anchor to the
        // short, fixed /tmp rather than TMPDIR — otherwise ssh itself refuses the -L spec
        // with "Bad local forwarding specification" before it ever reaches the network.
        let socket_dir = PathBuf::from("/tmp").join(format!("litepod-terminal-{}", Uuid::now_v7()));
        std::fs::create_dir(&socket_dir).map_err(|error| {
            AppError::InternalError(format!(
                "Could not create terminal socket directory: {error}"
            ))
        })?;
        if let Err(error) =
            std::fs::set_permissions(&socket_dir, std::fs::Permissions::from_mode(0o700))
        {
            let _ = std::fs::remove_dir(&socket_dir);
            return Err(AppError::InternalError(format!(
                "Could not secure terminal socket directory: {error}"
            )));
        }
        let socket_path = socket_dir.join("podman.sock");
        let identity = match Self::decrypt_key(encrypted_private_key) {
            Ok(identity) => identity,
            Err(error) => {
                let _ = std::fs::remove_dir(&socket_dir);
                return Err(error);
            }
        };
        let known_hosts =
            match Self::trusted_known_hosts(target, expected_fingerprint, known_hosts_line).await {
                Ok(known_hosts) => known_hosts,
                Err(error) => {
                    let _ = std::fs::remove_dir(&socket_dir);
                    return Err(error);
                }
            };
        let mut argv = build_ssh_argv(target, identity.path(), known_hosts.path(), &[]);
        // ControlPath=none: see start_local_forward for why this tunnel
        // can't share the multiplexed master (ControlMaster=no alone does
        // NOT disable sharing — it's the attach-to-existing-master value).
        // Must be spliced before build_ssh_argv's own ControlPath=...
        // (first-occurrence-wins) to actually take effect.
        argv.splice(
            1..1,
            [OsString::from("-o"), OsString::from("ControlPath=none")],
        );
        let destination = argv.len() - 1;
        argv.splice(
            destination..destination,
            [
                OsString::from("-o"),
                OsString::from("ExitOnForwardFailure=yes"),
                OsString::from("-N"),
                OsString::from("-L"),
                OsString::from(format!(
                    "{}:{}",
                    socket_path.display(),
                    remote_socket.display()
                )),
            ],
        );
        let mut child = match Command::new(&argv[0])
            .args(&argv[1..])
            .kill_on_drop(true)
            .spawn()
        {
            Ok(child) => child,
            Err(error) => {
                let _ = std::fs::remove_dir(&socket_dir);
                return Err(AppError::InternalError(format!(
                    "Could not start remote Podman tunnel: {error}"
                )));
            }
        };
        match tokio::time::timeout(Duration::from_secs(2), child.wait()).await {
            Ok(Ok(status)) => {
                warn!(%status, host = %target.host, "Remote Podman tunnel exited before becoming ready");
                let _ = std::fs::remove_file(&socket_path);
                let _ = std::fs::remove_dir(&socket_dir);
                Err(AppError::ValidationError(format!(
                    "Remote Podman tunnel exited before becoming ready ({status})"
                )))
            }
            Ok(Err(error)) => {
                let _ = std::fs::remove_file(&socket_path);
                let _ = std::fs::remove_dir(&socket_dir);
                Err(AppError::InternalError(format!(
                    "Could not wait for remote Podman tunnel: {error}"
                )))
            }
            Err(_) => Ok(SshUnixRelay {
                child,
                _identity: identity,
                _known_hosts: known_hosts,
                socket_dir,
                socket_path,
            }),
        }
    }

    /// Copies a local generated deployment file to a pre-created relative path below the SSH
    /// user's home directory. Host key verification is identical to command execution.
    pub async fn copy_file(
        target: &SshTarget,
        encrypted_private_key: &str,
        expected_fingerprint: &str,
        known_hosts_line: Option<&str>,
        local: &Path,
        remote_path: &str,
    ) -> Result<(), AppError> {
        if !local.is_file() {
            return Err(AppError::ValidationError(
                "The local deployment file no longer exists".into(),
            ));
        }
        if remote_path.is_empty()
            || remote_path.starts_with('/')
            || remote_path.contains("..")
            || remote_path.contains(|c: char| c.is_whitespace() || matches!(c, ':' | '\\' | '\0'))
        {
            return Err(AppError::InternalError(
                "Refusing an unsafe remote deployment path".into(),
            ));
        }
        ensure_ssh_control_dir()?;
        let identity = Self::decrypt_key(encrypted_private_key)?;
        let known_hosts =
            Self::trusted_known_hosts(target, expected_fingerprint, known_hosts_line).await?;
        let argv = build_scp_argv(
            target,
            identity.path(),
            known_hosts.path(),
            local,
            remote_path,
        );
        let output = tokio::time::timeout(
            Duration::from_secs(120),
            Command::new(&argv[0])
                .args(&argv[1..])
                .kill_on_drop(true)
                .output(),
        )
        .await
        .map_err(|_| AppError::ValidationError("SSH file transfer timed out".into()))?
        .map_err(|error| AppError::InternalError(format!("Could not start scp: {error}")))?;
        if !output.status.success() {
            return Err(AppError::ValidationError(format!(
                "SSH file transfer failed: {}",
                clamp_tail(&redact(&String::from_utf8_lossy(&output.stderr)), 500)
            )));
        }
        Ok(())
    }

    /// Prefer the entry explicitly confirmed by an administrator. The scan
    /// fallback exists only for fingerprint-only rows from older releases; its
    /// result is still compared before it can be used for an SSH connection.
    async fn trusted_known_hosts(
        target: &SshTarget,
        expected_fingerprint: &str,
        known_hosts_line: Option<&str>,
    ) -> Result<SecretFile, AppError> {
        let line = match known_hosts_line {
            Some(line) if !line.trim().is_empty() => {
                let candidate = SecretFile::write(line)?;
                let output = Command::new("ssh-keygen")
                    .args(["-lf"])
                    .arg(candidate.path())
                    .args(["-E", "sha256"])
                    .output()
                    .await
                    .map_err(|error| {
                        AppError::InternalError(format!("Could not verify SSH host key: {error}"))
                    })?;
                let observed = String::from_utf8_lossy(&output.stdout)
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or_default()
                    .to_owned();
                if !output.status.success() || observed != expected_fingerprint {
                    return Err(AppError::ValidationError("SSH host key changed; a sysadmin must verify the new fingerprint before connecting".into()));
                }
                line.to_owned()
            }
            _ => {
                let observed = Self::discover_host_key(target).await?;
                if observed.value != expected_fingerprint {
                    return Err(AppError::ValidationError("SSH host key changed; a sysadmin must verify the new fingerprint before connecting".into()));
                }
                observed.into_known_hosts_line()
            }
        };
        SecretFile::write(&line)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        SshExecutor, build_scp_argv, build_ssh_argv, clamp_tail, redact, shell_quote,
        ssh_control_path, validate_imported_private_key, validate_remote_argv,
    };
    use crate::servers::model::SshTarget;
    use std::{
        ffi::{OsStr, OsString},
        path::Path,
    };

    #[tokio::test]
    async fn generated_public_key_can_be_derived_again() {
        let generated = SshExecutor::generate_key()
            .await
            .expect("ssh-keygen must be installed");
        let derived = SshExecutor::public_key_from_encrypted(&generated.private_key_encrypted)
            .await
            .unwrap();
        assert!(derived.starts_with("ssh-ed25519 "));
        // `ssh-keygen -y` omits the comment that `-f key.pub` keeps, so compare the key material.
        let material = |value: &str| {
            value
                .split_whitespace()
                .take(2)
                .collect::<Vec<_>>()
                .join(" ")
        };
        assert_eq!(material(&derived), material(&generated.public_key));
    }

    #[test]
    fn argv_is_key_only_and_strict() {
        let args = build_ssh_argv(
            &SshTarget::parse("ops.example.com", 22, "litepod").unwrap(),
            Path::new("/tmp/key"),
            Path::new("/tmp/known-hosts"),
            &[OsString::from("podman"), OsString::from("info")],
        );
        assert_eq!(args[0], "ssh");
        assert!(args.iter().any(|arg| arg == "StrictHostKeyChecking=yes"));
        assert!(args.iter().any(|arg| arg == "PasswordAuthentication=no"));
        assert!(args.iter().any(|arg| arg == "ControlMaster=auto"));
        assert!(args.iter().any(|arg| arg == "ControlPersist=120"));
        assert!(args.iter().any(|arg| arg == "ServerAliveInterval=15"));
        assert!(args.iter().any(|arg| arg == "ServerAliveCountMax=3"));
        assert!(
            args.iter()
                .any(|arg| arg.to_string_lossy().contains("ControlPath=")
                    && arg.to_string_lossy().contains("%C"))
        );
        assert!(!args.iter().any(|arg| arg == "--"));
    }

    #[tokio::test]
    async fn interactive_shell_rejects_any_shell_outside_the_allowlist() {
        // A path (rather than a bare name) must not resolve — the client only
        // ever sends `bash`/`sh`/`fish`/`nu` and the API owns the path.
        let result = SshExecutor::interactive_shell_argv(
            &SshTarget::parse("ops.example.com", 22, "litepod").unwrap(),
            "not reached because validation is first",
            "SHA256:placeholder",
            None,
            "/bin/zsh",
        )
        .await;
        match result {
            Err(error) => assert_eq!(
                error.to_string(),
                "Shell must be sh, bash, fish, nu, zsh, or pwsh"
            ),
            Ok(_) => panic!("an unapproved shell must be rejected before SSH starts"),
        }
    }

    #[test]
    fn resolve_shell_maps_names_to_safe_commands_and_rejects_the_rest() {
        assert_eq!(
            SshExecutor::resolve_shell("bash").unwrap(),
            &["/bin/bash", "-l", "-i"]
        );
        let fish = SshExecutor::resolve_shell("fish").unwrap();
        assert_eq!(fish[..2], ["/bin/sh", "-lc"]);
        assert!(fish[2].contains("which fish"));
        let nu = SshExecutor::resolve_shell("nu").unwrap();
        assert_eq!(nu[..2], ["/bin/sh", "-lc"]);
        assert!(nu[2].contains("which nu"));
        let zsh = SshExecutor::resolve_shell("zsh").unwrap();
        assert_eq!(zsh[..2], ["/bin/sh", "-lc"]);
        assert!(zsh[2].contains("which zsh"));
        let pwsh = SshExecutor::resolve_shell("pwsh").unwrap();
        assert_eq!(pwsh[..2], ["/bin/sh", "-lc"]);
        assert!(pwsh[2].contains("exec \"$shell\" -Login -NoLogo"));
        assert_eq!(
            SshExecutor::resolve_local_shell("bash").unwrap(),
            OsString::from("/bin/bash")
        );
        assert!(SshExecutor::resolve_shell("/bin/bash").is_err());
        assert!(SshExecutor::resolve_shell("bash; rm -rf /").is_err());
    }

    #[test]
    fn control_socket_path_stays_within_openssh_limit() {
        let expanded = ssh_control_path()
            .to_string_lossy()
            .replace("%C", &"a".repeat(40));
        assert!(
            expanded.len() < 104,
            "expanded SSH control socket path is too long: {expanded}"
        );
    }

    #[test]
    fn remote_argv_must_be_a_closed_command() {
        assert!(validate_remote_argv(&[OsString::from("podman"), OsString::from("info")]).is_ok());
        // An empty argv would open an interactive login shell instead of running a command.
        assert!(validate_remote_argv(&[]).is_err());
        assert!(
            validate_remote_argv(&[OsString::from("podman"), OsString::from("lo\0gs")]).is_err()
        );
    }

    #[test]
    fn ssh_argv_quotes_each_remote_command_element() {
        let args = build_ssh_argv(
            &SshTarget::parse("ops.example.com", 22, "litepod").unwrap(),
            Path::new("/tmp/key"),
            Path::new("/tmp/known-hosts"),
            &[
                OsString::from("podman"),
                OsString::from("create"),
                OsString::from("sh"),
                OsString::from("-c"),
                OsString::from("echo \"Hello World\""),
            ],
        );
        // ssh's own flags stay unquoted; only the remote command is quoted.
        assert!(args.iter().any(|arg| arg == "StrictHostKeyChecking=yes"));
        let command = args
            .iter()
            .skip_while(|arg| !arg.to_string_lossy().contains('@'))
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            command,
            [
                "litepod@ops.example.com",
                "'podman'",
                "'create'",
                "'sh'",
                "'-c'",
                "'echo \"Hello World\"'",
            ]
        );
    }

    #[test]
    fn shell_quote_preserves_embedded_single_quotes() {
        assert_eq!(shell_quote(OsStr::new("it's")), "'it'\\''s'");
        assert_eq!(shell_quote(OsStr::new("plain")), "'plain'");
        assert_eq!(shell_quote(OsStr::new("a b")), "'a b'");
        assert_eq!(shell_quote(OsStr::new("")), "''");
    }

    #[test]
    fn clamp_tail_keeps_the_error_lines_not_the_progress_head() {
        let mut stderr = String::new();
        for n in 0..100 {
            stderr.push_str(&format!("line {n}\n"));
        }
        stderr.push_str("ERROR: failed to solve");
        let kept = clamp_tail(&stderr, 80);
        assert!(kept.contains("ERROR: failed to solve"));
        assert!(!kept.contains("line 0"));
        assert!(kept.chars().count() <= 80);
    }

    #[test]
    fn clamp_tail_returns_short_output_verbatim() {
        assert_eq!(clamp_tail("all good", 500), "all good");
        assert_eq!(clamp_tail("", 500), "");
    }

    #[test]
    fn imported_key_and_output_are_safe() {
        assert!(
            validate_imported_private_key(
                "-----BEGIN OPENSSH PRIVATE KEY-----\nkey\n-----END OPENSSH PRIVATE KEY-----"
            )
            .is_ok()
        );
        assert!(
            validate_imported_private_key(
                "-----BEGIN OPENSSH PRIVATE KEY-----\nENCRYPTED\n-----END OPENSSH PRIVATE KEY-----"
            )
            .is_err()
        );
        assert!(!redact("before\n-----BEGIN OPENSSH PRIVATE KEY-----\nsecret\n-----END OPENSSH PRIVATE KEY-----\nafter").contains("secret"));
    }

    #[test]
    fn scp_is_key_only_and_strict() {
        let args = build_scp_argv(
            &SshTarget::parse("ops.example.com", 2222, "litepod").unwrap(),
            Path::new("/tmp/key"),
            Path::new("/tmp/known-hosts"),
            Path::new("/tmp/source"),
            ".local/share/litepod/apps/a/compose.yml",
        );
        assert_eq!(args[0], "scp");
        assert!(args.iter().any(|arg| arg == "StrictHostKeyChecking=yes"));
        assert!(args.iter().any(|arg| arg == "ControlMaster=auto"));
        assert!(args.iter().any(|arg| arg == "ServerAliveInterval=15"));
        assert!(args.iter().any(|arg| arg == "ServerAliveCountMax=3"));
        assert!(args.iter().any(|arg| arg == "-P"));
    }
}
