use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::deployment::compose::store::PortMapping;
use crate::dtos::project::ApplicationDto;

fn deserialize_present_nullable<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct PostgresTemplateInput {
    pub application_name: String,
    pub description: Option<String>,
    pub server_id: String,
    pub docker_image: Option<String>,
    pub database_name: Option<String>,
    pub database_user: Option<String>,
    pub password: Option<String>,
    pub volume_name: Option<String>,
    pub public_host_port: Option<u16>,
    pub runtime_cpu_cores: Option<f64>,
    pub runtime_memory_mib: Option<i64>,
    pub template_instance_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TemplateVariableDto {
    pub name: String,
    pub masked: bool,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TemplateVolumeDto {
    pub name: String,
    pub mount_path: String,
    pub service: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PostgresTemplatePreviewDto {
    pub template_instance_id: String,
    pub resolved_password: String,
    pub database_name: String,
    pub database_user: String,
    pub docker_image: String,
    pub private_alias: String,
    pub private_connection_string: String,
    pub public_connection_string: Option<String>,
    pub compose: String,
    pub port_mappings: Vec<PortMapping>,
    pub variables: Vec<TemplateVariableDto>,
    pub volumes: Vec<TemplateVolumeDto>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PostgresTemplateCreatedDto {
    pub application: ApplicationDto,
    pub preview: PostgresTemplatePreviewDto,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PostgresResourceDetailDto {
    pub application: ApplicationDto,
    pub docker_image: String,
    pub database_name: String,
    pub database_user: String,
    pub volume: TemplateVolumeDto,
    pub private_alias: String,
    pub internal_port: u16,
    pub public_host_port: Option<u16>,
    pub deployment_status: String,
    /// Saved configuration no longer matches the running containers.
    pub config_stale: bool,
    pub password: Option<String>,
    pub private_connection_string: Option<String>,
    pub public_connection_string: Option<String>,
    pub compose: String,
    pub variables: Vec<TemplateVariableDto>,
    pub runtime_cpu_cores: Option<f64>,
    pub runtime_memory_mib: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct PostgresTemplateConfigUpdate {
    pub docker_image: String,
    pub public_host_port: Option<u16>,
    pub runtime_cpu_cores: Option<f64>,
    pub runtime_memory_mib: Option<i64>,
    pub database_name: Option<String>,
    pub database_user: Option<String>,
    pub password: Option<String>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct PostgresPasswordRotationRequest {
    pub password: Option<String>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct MongoDbTemplateInput {
    pub application_name: String,
    pub description: Option<String>,
    pub server_id: String,
    pub docker_image: Option<String>,
    pub database_name: Option<String>,
    pub database_user: Option<String>,
    pub password: Option<String>,
    pub volume_name: Option<String>,
    pub public_host_port: Option<u16>,
    pub runtime_cpu_cores: Option<f64>,
    pub runtime_memory_mib: Option<i64>,
    pub template_instance_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MongoDbTemplatePreviewDto {
    pub template_instance_id: String,
    pub resolved_password: String,
    pub database_name: String,
    pub database_user: String,
    pub docker_image: String,
    pub private_alias: String,
    pub private_connection_string: String,
    pub public_connection_string: Option<String>,
    pub compose: String,
    pub port_mappings: Vec<PortMapping>,
    pub variables: Vec<TemplateVariableDto>,
    pub volumes: Vec<TemplateVolumeDto>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MongoDbTemplateCreatedDto {
    pub application: ApplicationDto,
    pub preview: MongoDbTemplatePreviewDto,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MongoDbResourceDetailDto {
    pub application: ApplicationDto,
    pub docker_image: String,
    pub database_name: String,
    pub database_user: String,
    pub volume: TemplateVolumeDto,
    pub private_alias: String,
    pub internal_port: u16,
    pub public_host_port: Option<u16>,
    pub deployment_status: String,
    /// Saved configuration no longer matches the running containers.
    pub config_stale: bool,
    pub password: Option<String>,
    pub private_connection_string: Option<String>,
    pub public_connection_string: Option<String>,
    pub compose: String,
    pub variables: Vec<TemplateVariableDto>,
    pub runtime_cpu_cores: Option<f64>,
    pub runtime_memory_mib: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct MongoDbTemplateConfigUpdate {
    pub docker_image: String,
    pub public_host_port: Option<u16>,
    pub runtime_cpu_cores: Option<f64>,
    pub runtime_memory_mib: Option<i64>,
    pub database_name: Option<String>,
    pub database_user: Option<String>,
    pub password: Option<String>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct MongoDbPasswordRotationRequest {
    pub password: Option<String>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct MariaDbTemplateInput {
    pub application_name: String,
    pub description: Option<String>,
    pub server_id: String,
    pub docker_image: Option<String>,
    pub database_name: Option<String>,
    pub database_user: Option<String>,
    pub password: Option<String>,
    pub root_password: Option<String>,
    pub volume_name: Option<String>,
    pub public_host_port: Option<u16>,
    pub runtime_cpu_cores: Option<f64>,
    pub runtime_memory_mib: Option<i64>,
    pub template_instance_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MariaDbTemplatePreviewDto {
    pub template_instance_id: String,
    pub resolved_password: String,
    pub resolved_root_password: String,
    pub database_name: String,
    pub database_user: String,
    pub docker_image: String,
    pub private_alias: String,
    pub private_connection_string: String,
    pub private_root_connection_string: String,
    pub public_connection_string: Option<String>,
    pub public_root_connection_string: Option<String>,
    pub compose: String,
    pub port_mappings: Vec<PortMapping>,
    pub variables: Vec<TemplateVariableDto>,
    pub volumes: Vec<TemplateVolumeDto>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MariaDbTemplateCreatedDto {
    pub application: ApplicationDto,
    pub preview: MariaDbTemplatePreviewDto,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MariaDbResourceDetailDto {
    pub application: ApplicationDto,
    pub docker_image: String,
    pub database_name: String,
    pub database_user: String,
    pub password: Option<String>,
    pub root_password: Option<String>,
    pub volume: TemplateVolumeDto,
    pub private_alias: String,
    pub internal_port: u16,
    pub public_host_port: Option<u16>,
    pub deployment_status: String,
    /// Saved configuration no longer matches the running containers.
    pub config_stale: bool,
    pub private_connection_string: Option<String>,
    pub private_root_connection_string: Option<String>,
    pub public_connection_string: Option<String>,
    pub public_root_connection_string: Option<String>,
    pub compose: String,
    pub variables: Vec<TemplateVariableDto>,
    pub runtime_cpu_cores: Option<f64>,
    pub runtime_memory_mib: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct MariaDbTemplateConfigUpdate {
    pub docker_image: String,
    pub public_host_port: Option<u16>,
    pub runtime_cpu_cores: Option<f64>,
    pub runtime_memory_mib: Option<i64>,
    pub database_name: Option<String>,
    pub database_user: Option<String>,
    pub password: Option<String>,
    pub root_password: Option<String>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct MariaDbPasswordRotationRequest {
    pub credential: String,
    pub password: Option<String>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct DragonflyDbTemplateInput {
    pub application_name: String,
    pub description: Option<String>,
    pub server_id: String,
    pub docker_image: Option<String>,
    pub password: Option<String>,
    pub snapshot_cron: Option<String>,
    pub max_memory_mib: Option<i64>,
    pub cache_mode: Option<bool>,
    pub volume_name: Option<String>,
    pub public_host_port: Option<u16>,
    pub runtime_cpu_cores: Option<f64>,
    pub runtime_memory_mib: Option<i64>,
    pub template_instance_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DragonflyDbTemplatePreviewDto {
    pub template_instance_id: String,
    pub resolved_password: String,
    pub docker_image: String,
    pub snapshot_cron: String,
    pub max_memory_mib: Option<i64>,
    pub cache_mode: bool,
    pub private_alias: String,
    pub private_connection_string: String,
    pub public_connection_string: Option<String>,
    pub compose: String,
    pub port_mappings: Vec<PortMapping>,
    pub variables: Vec<TemplateVariableDto>,
    pub volumes: Vec<TemplateVolumeDto>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DragonflyDbTemplateCreatedDto {
    pub application: ApplicationDto,
    pub preview: DragonflyDbTemplatePreviewDto,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DragonflyDbResourceDetailDto {
    pub application: ApplicationDto,
    pub docker_image: String,
    pub snapshot_cron: String,
    pub max_memory_mib: Option<i64>,
    pub cache_mode: bool,
    pub password: Option<String>,
    pub volume: TemplateVolumeDto,
    pub private_alias: String,
    pub internal_port: u16,
    pub public_host_port: Option<u16>,
    pub deployment_status: String,
    /// Saved configuration no longer matches the running containers.
    pub config_stale: bool,
    pub private_connection_string: Option<String>,
    pub public_connection_string: Option<String>,
    pub compose: String,
    pub variables: Vec<TemplateVariableDto>,
    pub runtime_cpu_cores: Option<f64>,
    pub runtime_memory_mib: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct DragonflyDbTemplateConfigUpdate {
    pub docker_image: String,
    pub public_host_port: Option<u16>,
    pub runtime_cpu_cores: Option<f64>,
    pub runtime_memory_mib: Option<i64>,
    pub snapshot_cron: Option<String>,
    #[serde(default, deserialize_with = "deserialize_present_nullable")]
    pub max_memory_mib: Option<Option<i64>>,
    pub cache_mode: Option<bool>,
    pub password: Option<String>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct DragonflyDbPasswordRotationRequest {
    pub password: Option<String>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct KeyDbTemplateInput {
    pub application_name: String,
    pub description: Option<String>,
    pub server_id: String,
    pub docker_image: Option<String>,
    pub password: Option<String>,
    pub append_only: Option<bool>,
    pub append_fsync: Option<String>,
    pub max_memory_mib: Option<i64>,
    pub eviction_policy: Option<String>,
    pub volume_name: Option<String>,
    pub public_host_port: Option<u16>,
    pub runtime_cpu_cores: Option<f64>,
    pub runtime_memory_mib: Option<i64>,
    pub template_instance_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct KeyDbTemplatePreviewDto {
    pub template_instance_id: String,
    pub resolved_password: String,
    pub docker_image: String,
    pub append_only: bool,
    pub append_fsync: String,
    pub max_memory_mib: Option<i64>,
    pub eviction_policy: String,
    pub private_alias: String,
    pub private_connection_string: String,
    pub public_connection_string: Option<String>,
    pub compose: String,
    pub port_mappings: Vec<PortMapping>,
    pub variables: Vec<TemplateVariableDto>,
    pub volumes: Vec<TemplateVolumeDto>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct KeyDbTemplateCreatedDto {
    pub application: ApplicationDto,
    pub preview: KeyDbTemplatePreviewDto,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct KeyDbResourceDetailDto {
    pub application: ApplicationDto,
    pub docker_image: String,
    pub append_only: bool,
    pub append_fsync: String,
    pub max_memory_mib: Option<i64>,
    pub eviction_policy: String,
    pub password: Option<String>,
    pub volume: TemplateVolumeDto,
    pub private_alias: String,
    pub internal_port: u16,
    pub public_host_port: Option<u16>,
    pub deployment_status: String,
    /// Saved configuration no longer matches the running containers.
    pub config_stale: bool,
    pub private_connection_string: Option<String>,
    pub public_connection_string: Option<String>,
    pub compose: String,
    pub variables: Vec<TemplateVariableDto>,
    pub runtime_cpu_cores: Option<f64>,
    pub runtime_memory_mib: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct KeyDbTemplateConfigUpdate {
    pub docker_image: String,
    pub public_host_port: Option<u16>,
    pub runtime_cpu_cores: Option<f64>,
    pub runtime_memory_mib: Option<i64>,
    pub append_only: Option<bool>,
    pub append_fsync: Option<String>,
    #[serde(default, deserialize_with = "deserialize_present_nullable")]
    pub max_memory_mib: Option<Option<i64>>,
    pub eviction_policy: Option<String>,
    pub password: Option<String>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct KeyDbPasswordRotationRequest {
    pub password: Option<String>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct SqlServerTemplateInput {
    pub application_name: String,
    pub description: Option<String>,
    pub server_id: String,
    pub docker_image: Option<String>,
    pub password: Option<String>,
    pub volume_name: Option<String>,
    pub public_host_port: Option<u16>,
    pub runtime_cpu_cores: Option<f64>,
    pub runtime_memory_mib: Option<i64>,
    pub template_instance_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SqlServerTemplatePreviewDto {
    pub template_instance_id: String,
    pub resolved_password: String,
    pub docker_image: String,
    pub private_alias: String,
    pub private_connection_string: String,
    pub private_ado_connection_string: String,
    pub public_connection_string: Option<String>,
    pub public_ado_connection_string: Option<String>,
    pub compose: String,
    pub port_mappings: Vec<PortMapping>,
    pub variables: Vec<TemplateVariableDto>,
    pub volumes: Vec<TemplateVolumeDto>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SqlServerTemplateCreatedDto {
    pub application: ApplicationDto,
    pub preview: SqlServerTemplatePreviewDto,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SqlServerResourceDetailDto {
    pub application: ApplicationDto,
    pub docker_image: String,
    pub password: Option<String>,
    pub volume: TemplateVolumeDto,
    pub private_alias: String,
    pub internal_port: u16,
    pub public_host_port: Option<u16>,
    pub deployment_status: String,
    /// Saved configuration no longer matches the running containers.
    pub config_stale: bool,
    pub private_connection_string: Option<String>,
    pub private_ado_connection_string: Option<String>,
    pub public_connection_string: Option<String>,
    pub public_ado_connection_string: Option<String>,
    pub compose: String,
    pub variables: Vec<TemplateVariableDto>,
    pub runtime_cpu_cores: Option<f64>,
    pub runtime_memory_mib: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct SqlServerTemplateConfigUpdate {
    pub docker_image: String,
    pub public_host_port: Option<u16>,
    pub runtime_cpu_cores: Option<f64>,
    pub runtime_memory_mib: Option<i64>,
    pub password: Option<String>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct SqlServerPasswordRotationRequest {
    pub password: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::{DragonflyDbTemplateConfigUpdate, KeyDbTemplateConfigUpdate};

    #[test]
    fn dragonfly_max_memory_distinguishes_omission_clear_and_value() {
        let omitted: DragonflyDbTemplateConfigUpdate =
            serde_json::from_str(r#"{"docker_image":"dragonfly"}"#).unwrap();
        let cleared: DragonflyDbTemplateConfigUpdate =
            serde_json::from_str(r#"{"docker_image":"dragonfly","max_memory_mib":null}"#).unwrap();
        let limited: DragonflyDbTemplateConfigUpdate =
            serde_json::from_str(r#"{"docker_image":"dragonfly","max_memory_mib":512}"#).unwrap();

        assert_eq!(omitted.max_memory_mib, None);
        assert_eq!(cleared.max_memory_mib, Some(None));
        assert_eq!(limited.max_memory_mib, Some(Some(512)));
    }

    #[test]
    fn keydb_max_memory_distinguishes_omission_clear_and_value() {
        let omitted: KeyDbTemplateConfigUpdate =
            serde_json::from_str(r#"{"docker_image":"keydb"}"#).unwrap();
        let cleared: KeyDbTemplateConfigUpdate =
            serde_json::from_str(r#"{"docker_image":"keydb","max_memory_mib":null}"#).unwrap();
        let limited: KeyDbTemplateConfigUpdate =
            serde_json::from_str(r#"{"docker_image":"keydb","max_memory_mib":256}"#).unwrap();

        assert_eq!(omitted.max_memory_mib, None);
        assert_eq!(cleared.max_memory_mib, Some(None));
        assert_eq!(limited.max_memory_mib, Some(Some(256)));
    }
}
