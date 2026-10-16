use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateScheduledTaskRequest {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub cron_expression: String,
    pub runner: String,
    pub image: Option<String>,
    pub shell: String,
    pub script: String,
    #[serde(default)]
    pub network_access: bool,
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: i64,
    /// Server the task is pinned to. Required when the control plane is in
    /// cloud mode (IS_CLOUD=true). Defaults to the local server in
    /// self-hosted mode.
    #[serde(default)]
    pub server_id: Option<String>,
    /// Required for the `application_exec` runner: the application whose
    /// running container the script executes inside.
    #[serde(default)]
    pub application_id: Option<String>,
    /// Optional for `application_exec`. Names a Compose service when the
    /// application is a Compose app; omitted, the app's own container is
    /// used.
    #[serde(default)]
    pub compose_service: Option<String>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateScheduledTaskRequest {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub cron_expression: String,
    pub runner: String,
    pub image: Option<String>,
    pub shell: String,
    pub script: String,
    #[serde(default)]
    pub network_access: bool,
    pub timeout_seconds: i64,
    pub enabled: bool,
    /// Optional re-pin. When present the runner uses the new server; when
    /// absent the existing pin is preserved.
    #[serde(default)]
    pub server_id: Option<String>,
    #[serde(default)]
    pub application_id: Option<String>,
    #[serde(default)]
    pub compose_service: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ScheduledTaskDto {
    pub id: String,
    pub organization_id: String,
    pub name: String,
    pub description: String,
    pub cron_expression: String,
    pub runner: String,
    pub image: Option<String>,
    pub application_id: Option<String>,
    pub compose_service: Option<String>,
    pub network_access: bool,
    pub shell: String,
    pub timeout_seconds: i64,
    pub enabled: bool,
    pub server_id: String,
    pub next_run_at: Option<i64>,
    pub active_execution_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ScheduledTaskExecutionDto {
    pub id: String,
    pub task_id: String,
    pub trigger: String,
    pub status: String,
    pub runner: String,
    pub image: Option<String>,
    pub network_access: bool,
    pub server_id: String,
    pub queued_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub exit_code: Option<i64>,
    pub error: Option<String>,
    pub logs_truncated: bool,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[allow(dead_code)] // scheduled-task clock drift endpoint is staged.
pub struct ServerClockDto {
    pub server_time: String,
    pub server_utc_offset_seconds: i32,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ScheduledTaskExecutionLogDto {
    pub sequence: i64,
    pub stream: String,
    pub message: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ScheduledTaskExecutionDetailDto {
    #[serde(flatten)]
    pub execution: ScheduledTaskExecutionDto,
    /// Display name of the server the execution ran on (`None` when the
    /// referenced server no longer exists).
    pub server_name: Option<String>,
    /// SSH user the control plane uses to reach the remote server.
    pub server_user: Option<String>,
    pub logs: Vec<ScheduledTaskExecutionLogDto>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ScheduledTaskDetailDto {
    #[serde(flatten)]
    pub task: ScheduledTaskDto,
    pub script: String,
}

pub const fn default_timeout_seconds() -> i64 {
    3600
}
