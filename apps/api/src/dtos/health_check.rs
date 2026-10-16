use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateHealthCheckRequest {
    pub enabled: bool,
    pub protocol: String,
    #[serde(default)]
    pub path: String,
    pub port: Option<i64>,
    pub interval_seconds: i64,
    pub timeout_seconds: i64,
    pub unhealthy_threshold: i64,
    #[serde(default)]
    pub restart_on_unhealthy: bool,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct HealthCheckDto {
    pub application_id: String,
    pub enabled: bool,
    pub protocol: String,
    pub path: String,
    pub port: Option<i64>,
    pub interval_seconds: i64,
    pub timeout_seconds: i64,
    pub unhealthy_threshold: i64,
    pub restart_on_unhealthy: bool,
    pub consecutive_failures: i64,
    pub last_status: String,
    pub last_checked_at: Option<i64>,
    pub last_error: Option<String>,
    pub last_restarted_at: Option<i64>,
}
