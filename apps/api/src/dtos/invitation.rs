use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Deserialize, ToSchema)]
pub struct InvitationOrganizationRequest {
    pub organization_id: String,
    pub role_id: i64,
}

#[derive(Deserialize, ToSchema)]
pub struct CreateInvitationRequest {
    pub username: String,
    pub email: String,
    pub organizations: Vec<InvitationOrganizationRequest>,
}

#[derive(Deserialize, ToSchema)]
pub struct AcceptInvitationRequest {
    pub token: String,
    pub password: String,
    pub confirm_password: String,
    /// Same as `RegisterRequest::accept_terms`: required on cloud.
    #[serde(default)]
    pub accept_terms: bool,
}

#[derive(Serialize, ToSchema, Clone)]
pub struct InvitationOrganizationDto {
    pub organization_id: String,
    pub organization_name: String,
    pub role_id: i64,
    pub role_name: String,
}

#[derive(Serialize, ToSchema, Clone)]
pub struct InvitationDto {
    pub id: String,
    pub user_id: String,
    pub username: String,
    pub email: String,
    pub created_by: String,
    pub created_by_name: String,
    pub status: String,
    pub expires_at: String,
    pub created_at: String,
    pub organizations: Vec<InvitationOrganizationDto>,
}

#[derive(Serialize, ToSchema)]
pub struct CreatedInvitationDto {
    pub invitation: InvitationDto,
    pub token: String,
}

#[derive(Serialize, ToSchema)]
pub struct InvitationDetailsDto {
    pub username: String,
    pub email: String,
    pub status: String,
}
