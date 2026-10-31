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
