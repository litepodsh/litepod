use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::dtos::server::ServerSummaryDto;
use crate::servers::groups::ServerGroupRecord;

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ServerGroupDto {
    pub id: String,
    pub name: String,
    pub description: String,
    pub member_count: u64,
    pub member_servers: Vec<ServerSummaryDto>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// The compact shape an application card or picker carries: enough to name the
/// group and how many replicas it fans out to, without the member list.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ServerGroupSummaryDto {
    pub id: String,
    pub name: String,
    pub description: String,
    pub member_count: u64,
}

pub fn server_group_summary(group: &ServerGroupRecord, member_count: u64) -> ServerGroupSummaryDto {
    ServerGroupSummaryDto {
        id: group.id.clone(),
        name: group.name.clone(),
        description: group.description.clone(),
        member_count,
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateServerGroupRequest {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateServerGroupRequest {
    pub name: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ReplaceServerGroupMembersRequest {
    pub server_ids: Vec<String>,
}
