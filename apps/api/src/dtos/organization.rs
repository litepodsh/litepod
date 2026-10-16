use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
pub struct OrganizationDto {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub description: String,
    pub icon: String,
    pub icon_bg: String,
    pub plan: String,
    pub is_root: bool,
    pub audit_logs_enabled: bool,
    pub audit_logs_retention_days: i64,
    /// The assistant is hidden and refused while this is false.
    pub assistant_enabled: bool,
    /// The MCP endpoint answers only while this is true.
    pub mcp_enabled: bool,
    pub created_at: String,
}

#[derive(Deserialize, ToSchema)]
pub struct CreateOrganizationRequest {
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub icon_bg: Option<String>,
    pub plan: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct UpdateOrganizationRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub icon_bg: Option<String>,
    pub plan: Option<String>,
    pub audit_logs_enabled: Option<bool>,
    pub audit_logs_retention_days: Option<i64>,
    pub assistant_enabled: Option<bool>,
    pub mcp_enabled: Option<bool>,
}

#[derive(Serialize, ToSchema)]
pub struct OrganizationMemberDto {
    pub user_id: String,
    pub email: String,
    pub name: String,
    pub role_id: i64,
    pub role_name: String,
    pub joined_at: String,
    pub disabled: bool,
}

#[derive(Deserialize, ToSchema)]
pub struct UpdateMemberRoleRequest {
    pub role_id: i64,
}

#[derive(Deserialize, ToSchema)]
#[allow(dead_code)] // invitation DTOs are staged for the upcoming org-member invite flow.
pub struct CreateInvitationRequest {
    pub email: String,
    pub role_id: i64,
}

#[derive(Serialize, ToSchema)]
#[allow(dead_code)]
pub struct InvitationDto {
    pub id: String,
    pub email: String,
    pub role_id: i64,
    pub status: String,
    pub expires_at: String,
    pub created_at: String,
}

#[derive(Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct AcceptInvitationRequest {
    pub token: String,
}
