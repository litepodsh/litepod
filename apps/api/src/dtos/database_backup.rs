use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Per-application backup policy. There is at most one row per managed
/// database application; the row exists (defaulted, disabled) from the first
/// read so the UI always has something to render. `s3_target_id` is the named
/// destination (see `backup::target`); `None` means the global S3 config.
#[derive(Debug, Serialize, ToSchema)]
pub struct DatabaseBackupPolicyDto {
    pub application_id: String,
    pub enabled: bool,
    pub cron_expression: String,
    pub retention_count: i64,
    pub next_run_at: Option<i64>,
    pub updated_at: i64,
    pub updated_by: Option<String>,
    pub s3_target_id: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateDatabaseBackupPolicyRequest {
    pub enabled: bool,
    pub cron_expression: String,
    pub retention_count: i64,
    #[serde(default)]
    pub s3_target_id: Option<String>,
}

/// One run of a database backup, whatever its trigger. `status` is one of
/// `running`, `succeeded`, `failed`. `s3_target_id` is the destination the
/// archive was uploaded to (historical, even if the target is later deleted).
#[derive(Debug, Serialize, ToSchema)]
pub struct DatabaseBackupDto {
    pub id: String,
    pub application_id: String,
    pub status: String,
    pub engine: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub size_bytes: Option<i64>,
    pub s3_key: Option<String>,
    pub error_message: Option<String>,
    pub s3_target_id: Option<String>,
}

/// Start a manual backup. Empty body; the target application rides in the URL.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RunDatabaseBackupRequest {}
