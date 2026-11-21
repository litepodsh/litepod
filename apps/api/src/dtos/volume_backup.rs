use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Backup policy of one named volume of an application. Created (disabled) on
/// first read, like `DatabaseBackupPolicyDto`. `s3_target_id: None` means the
/// global S3 config.
#[derive(Debug, Serialize, ToSchema)]
pub struct VolumeBackupPolicyDto {
    pub volume_name: String,
    pub enabled: bool,
    pub cron_expression: String,
    pub retention_count: i64,
    pub next_run_at: Option<i64>,
    pub updated_at: i64,
    pub updated_by: Option<String>,
    pub s3_target_id: Option<String>,
    /// Whether "no target" means a working destination: the install-wide
    /// default S3 storage exists (never on cloud). Readable by any member,
    /// unlike the default's settings.
    pub default_available: bool,
    /// Stop the application while the volume is exported (databases).
    pub stop_app: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateVolumeBackupPolicyRequest {
    pub enabled: bool,
    pub cron_expression: String,
    pub retention_count: i64,
    #[serde(default)]
    pub s3_target_id: Option<String>,
    #[serde(default)]
    pub stop_app: bool,
}

/// Start a manual backup to the destination picked in the form; `None` (or
/// empty) is the default storage.
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct RunVolumeBackupRequest {
    #[serde(default)]
    pub s3_target_id: Option<String>,
    /// As picked in the form, saved or not.
    #[serde(default)]
    pub stop_app: bool,
    /// Podman names of the volumes to copy; empty copies every named volume.
    #[serde(default)]
    pub volumes: Vec<String>,
}

/// One backup run of a volume. `status` is `running`, `succeeded` or `failed`;
/// `triggered_by` is `scheduled`, `pre-restore` or the user who started it.
#[derive(Debug, Serialize, ToSchema)]
pub struct VolumeBackupDto {
    pub id: String,
    pub volume_name: String,
    pub status: String,
    pub triggered_by: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub size_bytes: Option<i64>,
    pub s3_key: Option<String>,
    pub error_message: Option<String>,
    pub s3_target_id: Option<String>,
    /// Shared by every volume copied in one whole-application run.
    pub group_id: Option<String>,
}

/// Whole-application runs, one row per volume (group them by `group_id`).
#[derive(Debug, Serialize, ToSchema)]
pub struct AppVolumeBackupsDto {
    /// See `VolumeBackupPolicyDto::default_available`.
    pub default_available: bool,
    pub runs: Vec<VolumeBackupDto>,
}

/// Where a restore lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RestoreTarget {
    /// A new detached managed volume, to check the files first.
    NewVolume,
    /// The original volume: stops the app, saves the current contents as a
    /// `pre-restore` backup, replaces them, starts the app again.
    InPlace,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RestoreVolumeBackupRequest {
    pub target: RestoreTarget,
    /// Name of the new volume (`NewVolume` only); defaults to `<volume>-restore-<time>`.
    #[serde(default)]
    pub name: Option<String>,
    /// Where the new volume mounts (`NewVolume` only, used by "Add volume →
    /// from a backup"); defaults to where the backup was taken from.
    #[serde(default)]
    pub mount_path: Option<String>,
    #[serde(default)]
    pub compose_service: Option<String>,
}

/// `volume` is the new managed volume for `NewVolume`, `None` for `InPlace`.
#[derive(Debug, Serialize, ToSchema)]
pub struct VolumeRestoreResultDto {
    pub volume: Option<crate::dtos::deployment::ManagedVolumeDto>,
}

/// Name and placement of a volume created from an outside `.tar`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NewVolumeFromTarRequest {
    pub name: String,
    pub mount_path: String,
    #[serde(default)]
    pub compose_service: Option<String>,
}

/// A `.tar` object in an S3 destination.
#[derive(Debug, Serialize, ToSchema)]
pub struct S3TarDto {
    pub key: String,
    pub size_bytes: i64,
    pub last_modified: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct S3TarsQuery {
    #[serde(default)]
    pub s3_target_id: Option<String>,
}

/// Create a volume from `key` in `s3_target_id` (`None`: the default storage).
#[derive(Debug, Deserialize, ToSchema)]
pub struct RestoreFromS3Request {
    #[serde(default)]
    pub s3_target_id: Option<String>,
    pub key: String,
    #[serde(flatten)]
    pub volume: NewVolumeFromTarRequest,
}
