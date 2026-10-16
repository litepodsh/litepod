use clap::{Parser, Subcommand};

use crate::cli::{EXIT_GENERIC, EXIT_OK};

#[derive(Parser, Debug)]
#[command(
    name = "litepod-api ops",
    about = "Operator subcommands for a self-hosted litepod instance. \
             Run inside the `litepod-api` container (or against the same \
             SQLite file with matching HASH_SECRET / ENCRYPTION_KEY).",
    version
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// List every account that holds the global SysAdmin role.
    #[command(name = "list-sysadmins")]
    ListSysadmins,
    /// Replace a user's password and revoke all of their active sessions.
    /// Without `--email` / `--generate`, reads both from stdin.
    #[command(name = "reset-password")]
    ResetPassword {
        /// Email of the account to reset. If omitted, prompts.
        #[arg(long)]
        email: Option<String>,
        /// Generate a strong random password and print it instead of
        /// prompting. Useful in scripts and over `podman exec` with no TTY.
        #[arg(long)]
        generate: bool,
    },
    /// Clear a user's TOTP secret and recovery codes. The next login will
    /// skip the 2FA challenge and the user can re-enroll from the
    /// account settings page.
    #[command(name = "reset-2fa")]
    Reset2fa {
        /// Email of the account to reset. If omitted, prompts.
        #[arg(long)]
        email: Option<String>,
    },
    /// Change a user's login email. Useful when the original inbox is
    /// unreachable (provider shutdown, lost credentials, etc.).
    #[command(name = "change-email")]
    ChangeEmail {
        /// Current email of the account. If omitted, prompts.
        #[arg(long)]
        email: Option<String>,
        /// New email. If omitted, prompts.
        #[arg(long)]
        new_email: Option<String>,
    },
    /// Restore a backup straight from S3 (disaster recovery on a fresh
    /// install). Without `--latest` / `--key`, lists the bucket's backups.
    /// The secret key is read from `LITEPOD_S3_SECRET_ACCESS_KEY`, else
    /// prompted, so it never lands in shell history or `ps`.
    #[command(name = "restore")]
    Restore {
        /// S3 endpoint URL, e.g. https://s3.eu-central-1.amazonaws.com
        #[arg(long)]
        endpoint: String,
        #[arg(long)]
        bucket: String,
        #[arg(long, default_value = "auto")]
        region: String,
        #[arg(long)]
        access_key_id: String,
        /// Object key to restore, as printed by the listing.
        #[arg(long, conflicts_with = "latest")]
        key: Option<String>,
        /// Restore the newest backup.
        #[arg(long)]
        latest: bool,
        /// Skip the confirmation prompt.
        #[arg(long)]
        yes: bool,
    },
}

/// Parse argv into a `Cli`. On clap parse error, render the message
/// ourselves and return a process exit code (clap's default behavior is to
/// write to stderr and `std::process::exit` from the parser, which we
/// would then have to unwind through `set_exit_code` on a `Child`).
pub fn parse(argv: &[String]) -> Result<Cli, u8> {
    match Cli::try_parse_from(argv) {
        Ok(cli) => Ok(cli),
        Err(error) => {
            // `--help` and `--version` are reported by clap as `Error`
            // but are not failures; clap's `use_stderr()` distinguishes
            // "real bad usage" from "I just printed help".
            let _ = error.print();
            if error.use_stderr() {
                Err(EXIT_GENERIC)
            } else {
                Err(EXIT_OK)
            }
        }
    }
}
