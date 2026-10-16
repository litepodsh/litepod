//! Operator-only subcommands exposed by the `litepod-api` binary.
//!
//! Dispatched at the very top of `main()` so a process that ran with any
//! argument never binds the HTTP listener, never opens Dragonfly, and never
//! starts the Podman/Caddy reconcilers. This is the only safe shape for an
//! offline tool that mutates the same SQLite database the running server is
//! holding open — the API stays up the whole time, the CLI just talks to
//! the same file via a second connection.
//!
//! Self-hosted installs lose password-reset-by-email when `RESEND_API_KEY`
//! is not configured (`install.sh` makes it optional, `install-cloud.sh`
//! does not). Until that gap is closed upstream, the commands here are the
//! documented recovery path for the sysadmin account.

pub mod change_email;
pub mod commands;
pub mod list_sysadmins;
pub mod reset_2fa;
pub mod reset_password;
pub mod restore;

use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::str::FromStr;

use crate::migrations;

/// `0` success, `1` generic error, `3` user not found. Clap's own parse
/// errors use its own exit codes (typically 2).
pub const EXIT_OK: u8 = 0;
pub const EXIT_GENERIC: u8 = 1;
pub const EXIT_NOT_FOUND: u8 = 3;

/// Public entry point. Returns the process exit code as `i32` so
/// `std::process::exit` can consume it without a cast.
pub fn run(argv: Vec<String>) -> i32 {
    match dispatch(argv) {
        Ok(()) => EXIT_OK as i32,
        Err(code) => code as i32,
    }
}

fn dispatch(argv: Vec<String>) -> Result<(), u8> {
    let cli = commands::parse(&argv).map_err(|code| u8::from(code))?;

    // Same hard fail-fast as `main.rs`: the binary refuses to operate
    // without the secrets its hashes were derived with.
    let encryption_key = std::env::var("ENCRYPTION_KEY").unwrap_or_default();
    if encryption_key.len() != 64 || !encryption_key.bytes().all(|b| b.is_ascii_hexdigit()) {
        eprintln!("ENCRYPTION_KEY must be set to 64 hex characters (see scripts/install.sh)");
        return Err(EXIT_GENERIC);
    }
    if std::env::var("HASH_SECRET")
        .unwrap_or_default()
        .trim()
        .is_empty()
    {
        eprintln!("HASH_SECRET must be set to a non-empty value");
        return Err(EXIT_GENERIC);
    }

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| {
            eprintln!("could not start tokio runtime: {error}");
            EXIT_GENERIC
        })?;
    runtime.block_on(run_async(cli))
}

async fn run_async(cli: commands::Cli) -> Result<(), u8> {
    let pool = open_pool().await.map_err(|error| {
        eprintln!("could not open database: {error}");
        EXIT_GENERIC
    })?;

    migrations::run(&pool).await;

    let result = match cli.command {
        commands::Command::ListSysadmins => list_sysadmins::run(&pool).await,
        commands::Command::ResetPassword { email, generate } => {
            let email = resolve_email(email)?;
            let new_password = resolve_password(generate)?;
            reset_password::run(&pool, &email, &new_password).await
        }
        commands::Command::Reset2fa { email } => {
            let email = resolve_email(email)?;
            reset_2fa::run(&pool, &email).await
        }
        commands::Command::ChangeEmail { email, new_email } => {
            let email = resolve_email(email)?;
            let new_email = resolve_email(new_email)?;
            change_email::run(&pool, &email, &new_email).await
        }
        commands::Command::Restore {
            endpoint,
            bucket,
            region,
            access_key_id,
            key,
            latest,
            yes,
        } => {
            let secret_access_key = match std::env::var("LITEPOD_S3_SECRET_ACCESS_KEY") {
                Ok(value) if !value.trim().is_empty() => value.trim().to_string(),
                _ => {
                    eprint!("S3 secret access key: ");
                    read_line_hidden().map_err(|error| {
                        eprintln!("{error}");
                        EXIT_GENERIC
                    })?
                }
            };
            let config = crate::backup::config::S3Config {
                endpoint_url: endpoint,
                bucket,
                region,
                access_key_id,
                secret_access_key,
                schedule_enabled: false,
                schedule_interval: None,
                schedule_time: None,
            };
            restore::run(
                &pool,
                restore::Args {
                    config,
                    key,
                    latest,
                    yes,
                },
            )
            .await
        }
    };

    result.map_err(|error| match error {
        OpsError::NotFound(message) => {
            eprintln!("{message}");
            EXIT_NOT_FOUND
        }
        OpsError::Invalid(message) => {
            eprintln!("{message}");
            EXIT_GENERIC
        }
        OpsError::Db(error) => {
            eprintln!("database error: {error}");
            EXIT_GENERIC
        }
    })
}

/// Error variants for the subcommand functions. Kept narrow so tests can
/// assert on the shape of a failure without coupling to error string
/// contents.
#[derive(Debug, thiserror::Error)]
pub enum OpsError {
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

async fn open_pool() -> Result<SqlitePool, String> {
    let database_path =
        std::env::var("DATABASE_PATH").unwrap_or_else(|_| "./data/litepod.db".to_string());
    let database_path_ref = std::path::Path::new(&database_path);
    if let Some(parent) = database_path_ref
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|error| format!("create {parent:?}: {error}"))?;
    }
    let connect_options = SqliteConnectOptions::from_str(&database_path)
        .map_err(|error| format!("invalid DATABASE_PATH {database_path}: {error}"))?
        .create_if_missing(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .foreign_keys(true)
        .busy_timeout(std::time::Duration::from_millis(5000));
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect_with(connect_options)
        .await
        .map_err(|error| format!("open: {error}"))?;
    Ok(pool)
}

fn resolve_email(maybe: Option<String>) -> Result<String, u8> {
    if let Some(value) = maybe.filter(|value| !value.trim().is_empty()) {
        return Ok(value.trim().to_string());
    }
    eprint!("email: ");
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).is_err() {
        eprintln!("could not read from stdin");
        return Err(EXIT_GENERIC);
    }
    let trimmed = line.trim();
    if trimmed.is_empty() {
        eprintln!("email is required");
        return Err(EXIT_GENERIC);
    }
    Ok(trimmed.to_string())
}

fn resolve_password(generate: bool) -> Result<String, u8> {
    if generate {
        return Ok(crate::utils::security::generate_token());
    }
    eprint!("new password (min 8 chars): ");
    let first = match read_line_hidden() {
        Ok(value) => value,
        Err(error) => {
            eprintln!("{error}");
            return Err(EXIT_GENERIC);
        }
    };
    eprint!("confirm new password: ");
    let second = match read_line_hidden() {
        Ok(value) => value,
        Err(error) => {
            eprintln!("{error}");
            return Err(EXIT_GENERIC);
        }
    };
    if first != second {
        eprintln!("passwords do not match");
        return Err(EXIT_GENERIC);
    }
    if first.len() < 8 {
        eprintln!("password must be at least 8 characters");
        return Err(EXIT_GENERIC);
    }
    Ok(first)
}

fn read_line_hidden() -> Result<String, String> {
    // Best-effort echo suppression via `stty -echo`. We don't pull in
    // `rpassword`/`dialoguer` because the CLI is invoked from a terminal
    // where `stty` is available; outside one (CI, scripts) the password
    // will be echoed and the operator can opt into `--generate` instead.
    let _ = std::process::Command::new("stty").args(["-echo"]).status();
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).is_err() {
        let _ = std::process::Command::new("stty").args(["echo"]).status();
        return Err("could not read password".to_string());
    }
    let _ = std::process::Command::new("stty").args(["echo"]).status();
    eprintln!();
    Ok(line.trim().to_string())
}

#[cfg(test)]
mod tests {
    //! Each test stands up a fresh migrated SQLite database, seeds a
    //! sysadmin + a regular user, runs the subcommand directly (bypassing
    //! the CLI parser so we exercise only the SQL path), and asserts on
    //! observable side effects in the database.

    use sqlx::Row;
    use uuid::Uuid;

    use super::OpsError;
    use crate::migrations;
    use crate::testing::sqlite_pool_at;

    async fn fresh_pool() -> (sqlx::SqlitePool, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!("litepod-cli-{}.db", Uuid::now_v7()));
        let pool = sqlite_pool_at(&path).await;
        migrations::run(&pool).await;
        (pool, path)
    }

    fn cleanup(path: &std::path::Path) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{}", path.display(), suffix));
        }
    }

    async fn seed_sysadmin(pool: &sqlx::SqlitePool, email: &str) -> String {
        let user_id = format!("user-{}", Uuid::now_v7());
        sqlx::query(
            "INSERT INTO users (id, email, hashed_password, name, totp_secret, totp_enabled) \
             VALUES (?1, ?2, 'old-hash', 'Admin', 'encrypted-secret', 1)",
        )
        .bind(&user_id)
        .bind(email)
        .execute(pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO user_global_roles (user_id, role_id) VALUES (?1, 0)")
            .bind(&user_id)
            .execute(pool)
            .await
            .unwrap();
        // Recovery code row that reset-2fa should wipe.
        sqlx::query(
            "INSERT INTO recovery_codes (id, user_id, code_digest) VALUES (?1, ?2, 'digest')",
        )
        .bind(format!("rc-{}", Uuid::now_v7()))
        .bind(&user_id)
        .execute(pool)
        .await
        .unwrap();
        user_id
    }

    #[tokio::test]
    async fn reset_password_replaces_hash_and_revokes_sessions() {
        let (pool, path) = fresh_pool().await;
        let user_id = seed_sysadmin(&pool, "admin@example.test").await;
        // Plant a session so we can confirm the DELETE.
        sqlx::query("INSERT INTO sessions (id, user_id, token_digest, expires_at) VALUES (?1, ?2, 'd', '2099-01-01 00:00:00')")
            .bind(format!("s-{}", Uuid::now_v7()))
            .bind(&user_id)
            .execute(&pool)
            .await
            .unwrap();

        super::reset_password::run(&pool, "admin@example.test", "new-passphrase-1234")
            .await
            .expect("reset_password should succeed");

        let row = sqlx::query("SELECT hashed_password FROM users WHERE id = ?1")
            .bind(&user_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        let stored: String = row.try_get(0).unwrap();
        assert_ne!(stored, "old-hash", "hash should be replaced");
        assert!(
            crate::utils::security::verify_password(&stored, b"new-passphrase-1234"),
            "new password should verify"
        );
        assert!(
            !crate::utils::security::verify_password(&stored, b"old-passphrase"),
            "old password should no longer verify"
        );

        let session_count: i64 = sqlx::query("SELECT COUNT(*) FROM sessions WHERE user_id = ?1")
            .bind(&user_id)
            .fetch_one(&pool)
            .await
            .unwrap()
            .try_get(0)
            .unwrap();
        assert_eq!(session_count, 0, "sessions should be revoked");

        drop(pool);
        cleanup(&path);
    }

    #[tokio::test]
    async fn reset_password_unknown_email_returns_not_found() {
        let (pool, path) = fresh_pool().await;
        let error = super::reset_password::run(&pool, "ghost@example.test", "whatever-1234")
            .await
            .expect_err("unknown email should error");
        assert!(matches!(error, OpsError::NotFound(_)));
        drop(pool);
        cleanup(&path);
    }

    #[tokio::test]
    async fn reset_password_short_password_is_rejected() {
        let (pool, path) = fresh_pool().await;
        seed_sysadmin(&pool, "admin@example.test").await;
        let error = super::reset_password::run(&pool, "admin@example.test", "short")
            .await
            .expect_err("short password should be rejected");
        assert!(matches!(error, OpsError::Invalid(_)));
        drop(pool);
        cleanup(&path);
    }

    #[tokio::test]
    async fn reset_2fa_clears_secret_and_recovery_codes() {
        let (pool, path) = fresh_pool().await;
        let user_id = seed_sysadmin(&pool, "admin@example.test").await;

        super::reset_2fa::run(&pool, "admin@example.test")
            .await
            .expect("reset_2fa should succeed");

        let row = sqlx::query("SELECT totp_secret, totp_enabled FROM users WHERE id = ?1")
            .bind(&user_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        let totp_secret: Option<String> = row.try_get(0).unwrap();
        let totp_enabled: i64 = row.try_get(1).unwrap();
        assert!(totp_secret.is_none(), "totp_secret must be NULL");
        assert_eq!(totp_enabled, 0, "totp_enabled must be 0");

        let recovery_count: i64 =
            sqlx::query("SELECT COUNT(*) FROM recovery_codes WHERE user_id = ?1")
                .bind(&user_id)
                .fetch_one(&pool)
                .await
                .unwrap()
                .try_get(0)
                .unwrap();
        assert_eq!(recovery_count, 0, "recovery codes must be cleared");

        drop(pool);
        cleanup(&path);
    }

    #[tokio::test]
    async fn change_email_rejects_duplicate() {
        let (pool, path) = fresh_pool().await;
        seed_sysadmin(&pool, "admin@example.test").await;
        // Insert a second user to collide against.
        let other_id = format!("user-{}", Uuid::now_v7());
        sqlx::query(
            "INSERT INTO users (id, email, hashed_password, name) VALUES (?1, ?2, 'h', 'Other')",
        )
        .bind(&other_id)
        .bind("taken@example.test")
        .execute(&pool)
        .await
        .unwrap();

        let error = super::change_email::run(&pool, "admin@example.test", "taken@example.test")
            .await
            .expect_err("duplicate email should be rejected");
        match error {
            OpsError::Invalid(message) => assert!(message.contains("already in use")),
            other => panic!("expected Invalid, got {other:?}"),
        }

        let row = sqlx::query("SELECT email FROM users WHERE id = ?1")
            .bind(&other_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        let email: String = row.try_get(0).unwrap();
        assert_eq!(email, "taken@example.test", "other user untouched");

        drop(pool);
        cleanup(&path);
    }

    #[tokio::test]
    async fn change_email_rejects_malformed_target() {
        let (pool, path) = fresh_pool().await;
        seed_sysadmin(&pool, "admin@example.test").await;
        let error = super::change_email::run(&pool, "admin@example.test", "no-at-symbol")
            .await
            .expect_err("malformed email should be rejected");
        assert!(matches!(error, OpsError::Invalid(_)));
        drop(pool);
        cleanup(&path);
    }

    #[tokio::test]
    async fn change_email_rejects_same_value() {
        let (pool, path) = fresh_pool().await;
        seed_sysadmin(&pool, "admin@example.test").await;
        let error = super::change_email::run(&pool, "admin@example.test", "admin@example.test")
            .await
            .expect_err("same email should be rejected");
        assert!(matches!(error, OpsError::Invalid(_)));
        drop(pool);
        cleanup(&path);
    }

    #[tokio::test]
    async fn change_email_succeeds_and_revokes_sessions() {
        let (pool, path) = fresh_pool().await;
        let user_id = seed_sysadmin(&pool, "admin@example.test").await;
        sqlx::query("INSERT INTO sessions (id, user_id, token_digest, expires_at) VALUES (?1, ?2, 'd', '2099-01-01 00:00:00')")
            .bind(format!("s-{}", Uuid::now_v7()))
            .bind(&user_id)
            .execute(&pool)
            .await
            .unwrap();

        super::change_email::run(&pool, "admin@example.test", "new-admin@example.test")
            .await
            .expect("change_email should succeed");

        let row = sqlx::query("SELECT email FROM users WHERE id = ?1")
            .bind(&user_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        let email: String = row.try_get(0).unwrap();
        assert_eq!(email, "new-admin@example.test");

        let session_count: i64 = sqlx::query("SELECT COUNT(*) FROM sessions WHERE user_id = ?1")
            .bind(&user_id)
            .fetch_one(&pool)
            .await
            .unwrap()
            .try_get(0)
            .unwrap();
        assert_eq!(session_count, 0, "sessions should be revoked");

        drop(pool);
        cleanup(&path);
    }

    #[tokio::test]
    async fn list_sysadmins_returns_seeded_account() {
        let (pool, path) = fresh_pool().await;
        // `list_sysadmins` writes to stdout via `println!`. We only
        // assert that the call returns Ok for an empty / populated
        // database — the rendering is a side effect we don't capture
        // here, and the SQL path is the part that can break.
        super::list_sysadmins::run(&pool).await.unwrap();
        seed_sysadmin(&pool, "admin@example.test").await;
        super::list_sysadmins::run(&pool).await.unwrap();
        drop(pool);
        cleanup(&path);
    }
}
