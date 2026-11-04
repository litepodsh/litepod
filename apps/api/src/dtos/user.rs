use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
pub struct UserDto {
    pub id: String,
    pub email: String,
    pub name: String,
    pub is_sysadmin: bool,
    pub created_at: String,
}

#[derive(Serialize, ToSchema)]
pub struct UserOrganizationDto {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub is_root: bool,
    pub role_id: i64,
    pub role_name: String,
    pub joined_at: String,
}

#[derive(Serialize, ToSchema)]
pub struct UserWithOrgsDto {
    pub id: String,
    pub email: String,
    pub name: String,
    pub created_at: String,
    /// Locked out by a SysAdmin; only a SysAdmin lifts it.
    pub disabled: bool,
    pub is_sysadmin: bool,
    /// Did something in Litepod within the last few minutes, or has a deployment or scheduled
    /// task running.
    pub online: bool,
    /// What they have running or just did, e.g. `deployment.building`, `deployment.triggered`.
    pub recent_action: Option<String>,
    pub recent_action_target: Option<String>,
    pub last_login_at: Option<String>,
    /// Remote servers and applications in the organizations this user created.
    pub remote_servers: i64,
    pub remote_servers_ready: i64,
    pub applications: i64,
    pub applications_running: i64,
    pub organizations: Vec<UserOrganizationDto>,
}

#[derive(Deserialize, ToSchema)]
pub struct SetUserDisabledRequest {
    pub disabled: bool,
}

#[derive(Deserialize, ToSchema)]
pub struct UpdateUserRequest {
    pub name: Option<String>,
}
