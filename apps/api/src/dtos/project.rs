use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::dtos::server::ServerSummaryDto;
use crate::dtos::server_group::ServerGroupSummaryDto;

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ProjectDto {
    pub id: String,
    pub organization_id: String,
    pub name: String,
    pub description: String,
    pub icon_url: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EnvironmentDto {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub description: String,
    pub is_production: bool,
    pub created_at: i64,
    pub updated_at: i64,
    pub applications: Vec<ApplicationDto>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ApplicationDto {
    pub id: String,
    pub environment_id: String,
    pub name: String,
    pub description: String,
    pub template_kind: Option<String>,
    pub catalog_template_id: Option<String>,
    pub catalog_icon_url: Option<String>,
    /// User-uploaded or pasted override; falls back to `catalog_icon_url` when
    /// absent. Never touches the template-derived field.
    pub icon_url: Option<String>,
    pub server: Option<ServerSummaryDto>,
    pub server_group: Option<ServerGroupSummaryDto>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ResourceBaselineDto {
    pub cpu_cores: f64,
    pub memory_bytes: u64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ProjectDetailDto {
    pub project: ProjectDto,
    pub environments: Vec<EnvironmentDto>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateProjectRequest {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateProjectRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    #[serde(default, deserialize_with = "deserialize_present_nullable")]
    pub icon_url: Option<Option<String>>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateEnvironmentRequest {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateEnvironmentRequest {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateApplicationRequest {
    pub name: String,
    pub description: Option<String>,
    pub server_id: Option<String>,
    pub server_group_id: Option<String>,
}

/// `None` (field omitted) leaves the icon override unchanged; `Some(None)`
/// (`"icon_url": null`) clears it back to the catalog default; `Some(Some(url))`
/// sets it. Same tri-state shape as `max_memory_mib` in
/// `dtos::template::DragonflyDbTemplateConfigUpdate`.
fn deserialize_present_nullable<'de, D>(deserializer: D) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer).map(Some)
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateApplicationRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    #[serde(default, deserialize_with = "deserialize_present_nullable")]
    pub icon_url: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_present_nullable")]
    pub server_id: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_present_nullable")]
    pub server_group_id: Option<Option<String>>,
}
