//! Engine knowledge for managed database backups: which `template_kind`
//! supports a dump/restore path, the closed `bash -lc` scripts executed
//! *inside* the running container, and the object extension used on S3.
//!
//! All values in these scripts come from the container's own environment
//! (`POSTGRES_PASSWORD`, `MARIADB_ROOT_PASSWORD`, ...), never from request
//! input, so no user-controlled interpolation ever reaches the argv built in
//! `run.rs`. SQL Server writes `.bak` files with no managed database name and
//! is excluded from v1; the Redis-family (DragonflyDB / KeyDB) has a dump path
//! via `--rdb` but no supported restore yet.

use crate::dtos::response::AppError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    Postgres,
    MariaDb,
    MongoDb,
    DragonflyDb,
    KeyDb,
}

/// Managed databases with a v1 backup path, in the order the UI lists them.
#[allow(dead_code)] // consumed by the UI's "Databases" filter; kept next to the engine list.
pub const SUPPORTED_KINDS: &[&str] = &["postgres", "mariadb", "mongodb", "dragonflydb", "keydb"];

impl Engine {
    pub fn from_kind(kind: Option<&str>) -> Result<Self, AppError> {
        match kind {
            Some("postgres") => Ok(Self::Postgres),
            Some("mariadb") => Ok(Self::MariaDb),
            Some("mongodb") => Ok(Self::MongoDb),
            Some("dragonflydb") => Ok(Self::DragonflyDb),
            Some("keydb") => Ok(Self::KeyDb),
            Some("sqlserver") => Err(AppError::ValidationError(
                "SQL Server backups are not supported yet".into(),
            )),
            _ => Err(AppError::ValidationError(
                "Backups only exist for managed databases".into(),
            )),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Postgres => "postgres",
            Self::MariaDb => "mariadb",
            Self::MongoDb => "mongodb",
            Self::DragonflyDb => "dragonflydb",
            Self::KeyDb => "keydb",
        }
    }

    /// True when the restore path is implemented. The RDB-family restores are
    /// deferred to a later release; SQL Server never reaches this question.
    pub fn restore_supported(self) -> bool {
        matches!(self, Self::Postgres | Self::MariaDb | Self::MongoDb)
    }

    /// Extension used in the S3 object key so the file stays recognizable.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Postgres => "pgdump",
            Self::MariaDb => "sql",
            Self::MongoDb => "mongodump",
            Self::DragonflyDb | Self::KeyDb => "rdb",
        }
    }

    /// The dump tool the image must ship for the preflight check, and the
    /// friendly name shown when it is missing.
    pub fn tool(self) -> &'static str {
        match self {
            Self::Postgres => "pg_dump",
            Self::MariaDb => "mysqldump",
            Self::MongoDb => "mongodump",
            Self::DragonflyDb | Self::KeyDb => "redis-cli",
        }
    }

    /// Closed script that writes the dump to `/tmp/litepod-db-backup` inside
    /// the container. Values come from the container's own env; the target
    /// path is fixed because the same constant lives in `run.rs`.
    pub fn dump_script(self) -> &'static str {
        match self {
            Self::Postgres => {
                "pg_dump -U \"$POSTGRES_USER\" -d \"$POSTGRES_DB\" -Fc -f /tmp/litepod-db-backup"
            }
            Self::MariaDb => {
                "mysqldump -u root -p\"$MARIADB_ROOT_PASSWORD\" --single-transaction --routines --triggers \"$MARIADB_DATABASE\" > /tmp/litepod-db-backup"
            }
            Self::MongoDb => {
                "mongodump --archive=/tmp/litepod-db-backup --authenticationDatabase admin -u \"$MONGO_INITDB_ROOT_USERNAME\" -p \"$MONGO_INITDB_ROOT_PASSWORD\" --db \"$MONGO_INITDB_DATABASE\""
            }
            Self::DragonflyDb => {
                "redis-cli -p 6379 -a \"$DRAGONFLY_PASSWORD\" --rdb /tmp/litepod-db-backup"
            }
            Self::KeyDb => "redis-cli -p 6379 -a \"$KEYDB_PASSWORD\" --rdb /tmp/litepod-db-backup",
        }
    }

    /// Closed script that restores the file at `/tmp/litepod-db-restore`
    /// (already written into the container by `run.rs`). Only reached when
    /// [`Engine::restore_supported`] returned true.
    pub fn restore_script(self) -> &'static str {
        match self {
            Self::Postgres => {
                "pg_restore -U \"$POSTGRES_USER\" -d \"$POSTGRES_DB\" --clean --if-exists --no-owner /tmp/litepod-db-restore"
            }
            Self::MariaDb => {
                "mysql -u root -p\"$MARIADB_ROOT_PASSWORD\" \"$MARIADB_DATABASE\" < /tmp/litepod-db-restore"
            }
            Self::MongoDb => {
                "mongorestore --archive=/tmp/litepod-db-restore --drop --authenticationDatabase admin -u \"$MONGO_INITDB_ROOT_USERNAME\" -p \"$MONGO_INITDB_ROOT_PASSWORD\""
            }
            Self::DragonflyDb | Self::KeyDb => unreachable!("restore_supported() gates these"),
        }
    }
}
