//! `litepod-api restore`: disaster recovery from S3 on a fresh install.
//!
//! The UI restore only knows backups listed in this database's
//! `backup_history`, and S3 settings stored in it — both gone when the
//! database itself is what was lost. This reads the bucket directly.

use sqlx::SqlitePool;
use std::path::PathBuf;

use crate::backup::{config::S3Config, restore, s3};
use crate::cli::OpsError;

pub struct Args {
    pub config: S3Config,
    pub key: Option<String>,
    pub latest: bool,
    pub yes: bool,
}

pub async fn run(pool: &SqlitePool, args: Args) -> Result<(), OpsError> {
    let backups = s3::list_backups(&args.config)
        .await
        .map_err(|error| OpsError::Invalid(error.to_string()))?;

    let key = match (args.key, args.latest) {
        (Some(key), _) => key,
        (None, true) => backups
            .first()
            .map(|(key, _)| key.clone())
            .ok_or_else(|| OpsError::NotFound("no backups found under backups/".into()))?,
        (None, false) => {
            if backups.is_empty() {
                println!("no backups found under backups/");
            }
            for (key, size) in &backups {
                println!("{key}  {:.1} MB", *size as f64 / 1_000_000.0);
            }
            eprintln!("\nrerun with --latest or --key <KEY> to restore one");
            return Ok(());
        }
    };

    if !args.yes && !confirm(&key)? {
        return Err(OpsError::Invalid("restore cancelled".into()));
    }

    let data_dir =
        PathBuf::from(std::env::var("LITEPOD_DATA_DIR").unwrap_or_else(|_| "./data".to_string()));
    let stack_env_file = std::env::var("LITEPOD_STACK_ENV_FILE")
        .ok()
        .map(PathBuf::from)
        .filter(|path| path.is_file());

    eprintln!("restoring {key} ...");
    restore::run_pipeline(
        pool,
        &data_dir,
        stack_env_file.as_deref(),
        &args.config,
        &key,
    )
    .await
    .map_err(|error| OpsError::Invalid(format!("restore failed: {error}")))?;

    println!("restored {key}");
    println!(
        "\nNow recreate the stack so every service picks up the restored secrets (run on the host, in the litepod directory):\n\n    podman-compose -f podman-compose-prod.yml up -d --force-recreate\n"
    );
    Ok(())
}

fn confirm(key: &str) -> Result<bool, OpsError> {
    eprintln!("This replaces the whole litepod database and its compose/app files with {key}.");
    eprint!("Type `restore` to continue: ");
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .map_err(|_| OpsError::Invalid("could not read from stdin".into()))?;
    Ok(line.trim() == "restore")
}
