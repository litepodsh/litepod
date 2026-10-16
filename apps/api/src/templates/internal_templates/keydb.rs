use std::collections::{BTreeMap, HashSet};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use futures_util::StreamExt;
use podman_api::{Podman, conn::TtyChunk, opts::ExecCreateOpts};
use rand::RngExt;
use uuid::Uuid;

use crate::{
    cqrs::{commands::project as project_cmd, queries::project as project_query},
    deployment::{
        self,
        compose::store::{ComposeFileStore, PortMapping},
        source,
    },
    dtos::{
        project::CreateApplicationRequest,
        response::AppError,
        template::{
            KeyDbPasswordRotationRequest, KeyDbResourceDetailDto, KeyDbTemplateConfigUpdate,
            KeyDbTemplateCreatedDto, KeyDbTemplateInput, KeyDbTemplatePreviewDto,
            TemplateVariableDto, TemplateVolumeDto,
        },
    },
    state::state::SharedState,
};

const DEFAULT_IMAGE: &str = "docker.io/eqalpha/keydb";
const SERVICE_NAME: &str = "keydb";
const DATA_PATH: &str = "/data";
const PORT: u16 = 6379;
const PASSWORD_VARIABLE: &str = "KEYDB_PASSWORD";
const DEFAULT_FSYNC: &str = "everysec";
const DEFAULT_EVICTION_POLICY: &str = "noeviction";

#[derive(serde::Deserialize)]
struct PersistedCompose {
    services: BTreeMap<String, PersistedService>,
    #[serde(default)]
    volumes: BTreeMap<String, serde_yaml::Value>,
}

#[derive(serde::Deserialize)]
struct PersistedService {
    image: String,
    #[serde(default)]
    volumes: Vec<String>,
    #[serde(default)]
    command: serde_yaml::Value,
}

struct PersistedConfig {
    image: String,
    volume: String,
    append_only: bool,
    append_fsync: String,
    max_memory_mib: Option<i64>,
    eviction_policy: String,
}

fn required(value: Option<&str>, default: &str, label: &str) -> Result<String, AppError> {
    let value = value.unwrap_or(default).trim();
    if value.is_empty() {
        return Err(AppError::ValidationError(format!("{label} is required")));
    }
    Ok(value.to_string())
}

fn validate_image(value: &str) -> Result<(), AppError> {
    if value.is_empty()
        || value.contains(char::is_whitespace)
        || !value.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/' | ':' | '@' | '+')
        })
    {
        return Err(AppError::ValidationError(
            "Docker image must be a complete image reference without whitespace".into(),
        ));
    }
    Ok(())
}

fn derive_volume_name(application_name: &str) -> String {
    let normalized = application_name
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>();
    let normalized = normalized.trim_matches('-');
    if normalized.is_empty() {
        "keydb-data".into()
    } else {
        format!("{normalized}-data")
    }
}

fn validate_volume_name(value: &str) -> Result<(), AppError> {
    if value.is_empty()
        || value.len() > 100
        || !value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(AppError::ValidationError(
            "Volume name must contain only letters, digits, hyphens, underscores, or periods"
                .into(),
        ));
    }
    Ok(())
}

fn generate_password() -> String {
    URL_SAFE_NO_PAD.encode(rand::rng().random::<[u8; 24]>())
}

fn validate_password(value: &str) -> Result<(), AppError> {
    if value.trim() != value || value.contains('\n') || value.contains('\r') {
        return Err(AppError::ValidationError(
            "KeyDB password must not contain line breaks or surrounding whitespace".into(),
        ));
    }
    Ok(())
}

fn resolve_password(value: Option<&str>) -> Result<String, AppError> {
    match value.filter(|value| !value.is_empty()) {
        Some(value) => {
            validate_password(value)?;
            Ok(value.to_string())
        }
        None => Ok(generate_password()),
    }
}

fn validate_fsync(value: &str) -> Result<(), AppError> {
    if !matches!(value, "always" | "everysec" | "no") {
        return Err(AppError::ValidationError(
            "Append fsync must be always, everysec, or no".into(),
        ));
    }
    Ok(())
}

fn validate_eviction_policy(value: &str) -> Result<(), AppError> {
    if !matches!(
        value,
        "noeviction"
            | "allkeys-lru"
            | "allkeys-lfu"
            | "volatile-lru"
            | "volatile-lfu"
            | "allkeys-random"
            | "volatile-random"
    ) {
        return Err(AppError::ValidationError(
            "Eviction policy is invalid".into(),
        ));
    }
    Ok(())
}

/// Redis-compatible clients are handed the password verbatim, so the URL shown
/// in the dashboard is the exact string they expect - no percent-encoding.
fn connection_string(password: &str, host: &str, port: u16) -> String {
    format!("redis://:{password}@{host}:{port}")
}

fn compose(
    image: &str,
    volume: &str,
    append_only: bool,
    append_fsync: &str,
    max_memory_mib: Option<i64>,
    eviction_policy: &str,
    application_id: &str,
) -> String {
    let mut command = format!(
        "      - keydb-server\n      - /etc/keydb/keydb.conf\n      - --requirepass\n      - ${{{PASSWORD_VARIABLE}}}\n      - --appendonly\n      - \"{}\"\n      - --appendfsync\n      - {append_fsync}\n      - --maxmemory-policy\n      - {eviction_policy}\n",
        if append_only { "yes" } else { "no" },
    );
    if let Some(max_memory_mib) = max_memory_mib {
        command.push_str(&format!(
            "      - --maxmemory\n      - \"{max_memory_mib}mb\"\n"
        ));
    }
    format!(
        "services:\n  {SERVICE_NAME}:\n    image: {image}\n    command:\n{command}    volumes:\n      - {volume}:{DATA_PATH}\n{service_networks}volumes:\n  {volume}: {{}}\n{networks}",
        service_networks = crate::templates::resource_service_networks(application_id),
        networks = crate::templates::resource_network_compose(application_id),
    )
}

fn variables() -> Vec<TemplateVariableDto> {
    vec![TemplateVariableDto {
        name: PASSWORD_VARIABLE.into(),
        masked: true,
    }]
}

fn persisted_compose(content: &str) -> Result<PersistedConfig, AppError> {
    let invalid = || AppError::ValidationError("KeyDB Compose configuration is invalid".into());
    let compose: PersistedCompose = serde_yaml::from_str(content).map_err(|_| invalid())?;
    if compose.services.len() != 1 {
        return Err(invalid());
    }
    let service = compose.services.get(SERVICE_NAME).ok_or_else(&invalid)?;
    let data_mounts = service
        .volumes
        .iter()
        .filter_map(|mount| mount.strip_suffix(&format!(":{DATA_PATH}")))
        .collect::<Vec<_>>();
    if data_mounts.len() != 1 || data_mounts[0].is_empty() {
        return Err(invalid());
    }
    let volume = data_mounts[0];
    if !compose.volumes.contains_key(volume) {
        return Err(invalid());
    }
    let command: Vec<String> = match &service.command {
        serde_yaml::Value::Sequence(values) => values
            .iter()
            .map(serde_yaml::Value::as_str)
            .collect::<Option<Vec<_>>>()
            .ok_or_else(&invalid)?
            .into_iter()
            .map(str::to_string)
            .collect(),
        _ => return Err(invalid()),
    };
    let expected = [
        "keydb-server",
        "/etc/keydb/keydb.conf",
        "--requirepass",
        "${KEYDB_PASSWORD}",
        "--appendonly",
    ];
    if command.len() < expected.len()
        || !command
            .iter()
            .take(expected.len())
            .zip(expected)
            .all(|(actual, expected)| actual == expected)
    {
        return Err(invalid());
    }
    let append_only = match command.get(5).map(String::as_str) {
        Some("yes") => true,
        Some("no") => false,
        _ => return Err(invalid()),
    };
    if command.get(6).map(String::as_str) != Some("--appendfsync") {
        return Err(invalid());
    }
    let append_fsync = command.get(7).ok_or_else(&invalid)?.clone();
    if validate_fsync(&append_fsync).is_err()
        || command.get(8).map(String::as_str) != Some("--maxmemory-policy")
    {
        return Err(invalid());
    }
    let eviction_policy = command.get(9).ok_or_else(&invalid)?.clone();
    if validate_eviction_policy(&eviction_policy).is_err() {
        return Err(invalid());
    }
    let max_memory_mib = match command.len() {
        10 => None,
        12 if command.get(10).map(String::as_str) == Some("--maxmemory") => command
            .get(11)
            .and_then(|v| v.strip_suffix("mb"))
            .and_then(|v| v.parse().ok())
            .filter(|v: &i64| *v > 0)
            .ok_or_else(&invalid)
            .map(Some)?,
        _ => return Err(invalid()),
    };
    Ok(PersistedConfig {
        image: service.image.clone(),
        volume: volume.into(),
        append_only,
        append_fsync,
        max_memory_mib,
        eviction_policy,
    })
}

async fn resource_values(
    state: &SharedState,
    organization_id: &str,
    project_id: &str,
    environment_id: &str,
    application_id: &str,
) -> Result<
    (
        crate::dtos::project::ApplicationDto,
        PersistedConfig,
        String,
    ),
    AppError,
> {
    let application = project_query::get_application(
        state,
        organization_id,
        project_id,
        environment_id,
        application_id,
    )
    .await?;
    if application.template_kind.as_deref() != Some("keydb") {
        return Err(AppError::ResourceNotFound);
    }
    let store = ComposeFileStore::new(&state.data_dir, application_id);
    let config = persisted_compose(
        &store
            .read_content()
            .await
            .map_err(AppError::InternalError)?,
    )?;
    let password = deployment::decrypted_environment(state, application_id, "runtime")
        .await?
        .into_iter()
        .collect::<BTreeMap<_, _>>()
        .get(PASSWORD_VARIABLE)
        .cloned()
        .ok_or_else(|| AppError::ValidationError(format!("{PASSWORD_VARIABLE} is missing")))?;
    Ok((application, config, password))
}

pub fn preview(
    input: KeyDbTemplateInput,
    private_alias: &str,
    public_host: &str,
    reserved_ports: &HashSet<u16>,
    used_ports: &HashSet<u16>,
) -> Result<KeyDbTemplatePreviewDto, AppError> {
    let application_name = required(Some(&input.application_name), "", "Application name")?;
    let docker_image = required(input.docker_image.as_deref(), DEFAULT_IMAGE, "Docker image")?;
    validate_image(&docker_image)?;
    let password = resolve_password(input.password.as_deref())?;
    let append_only = input.append_only.unwrap_or(true);
    let append_fsync = required(input.append_fsync.as_deref(), DEFAULT_FSYNC, "Append fsync")?;
    validate_fsync(&append_fsync)?;
    let eviction_policy = required(
        input.eviction_policy.as_deref(),
        DEFAULT_EVICTION_POLICY,
        "Eviction policy",
    )?;
    validate_eviction_policy(&eviction_policy)?;
    if input.max_memory_mib.is_some_and(|value| value <= 0) {
        return Err(AppError::ValidationError(
            "Max memory MiB must be positive".into(),
        ));
    }
    let volume_name = input
        .volume_name
        .as_deref()
        .filter(|v| !v.trim().is_empty())
        .map(|v| v.trim().to_string())
        .unwrap_or_else(|| derive_volume_name(&application_name));
    validate_volume_name(&volume_name)?;
    if private_alias.trim().is_empty() {
        return Err(AppError::ValidationError(
            "Private alias is required".into(),
        ));
    }
    let template_instance_id = input
        .template_instance_id
        .unwrap_or_else(|| Uuid::now_v7().to_string());
    Uuid::parse_str(&template_instance_id)
        .map_err(|_| AppError::ValidationError("Template instance ID is invalid".into()))?;
    let public_connection_string = match input.public_host_port {
        Some(port) => {
            if port == 0 {
                return Err(AppError::ValidationError(
                    "Public host port must be between 1 and 65535".into(),
                ));
            }
            if public_host.trim().is_empty() {
                return Err(AppError::ValidationError(
                    "Configure a public server host before publishing KeyDB".into(),
                ));
            }
            if reserved_ports.contains(&port) || used_ports.contains(&port) {
                return Err(AppError::ValidationError(format!(
                    "Host port {port} is already in use or reserved"
                )));
            }
            Some(connection_string(&password, public_host.trim(), port))
        }
        None => None,
    };
    Ok(KeyDbTemplatePreviewDto {
        template_instance_id: template_instance_id.clone(),
        resolved_password: password.clone(),
        docker_image: docker_image.clone(),
        append_only,
        append_fsync: append_fsync.clone(),
        max_memory_mib: input.max_memory_mib,
        eviction_policy: eviction_policy.clone(),
        private_alias: private_alias.into(),
        private_connection_string: connection_string(&password, private_alias, PORT),
        public_connection_string,
        compose: compose(
            &docker_image,
            &volume_name,
            append_only,
            &append_fsync,
            input.max_memory_mib,
            &eviction_policy,
            &template_instance_id,
        ),
        port_mappings: input
            .public_host_port
            .map(|host_port| {
                vec![PortMapping {
                    service: SERVICE_NAME.into(),
                    host_port,
                    container_port: PORT,
                    protocol: "tcp".into(),
                }]
            })
            .unwrap_or_default(),
        variables: variables(),
        volumes: vec![TemplateVolumeDto {
            name: volume_name,
            mount_path: DATA_PATH.into(),
            service: SERVICE_NAME.into(),
        }],
    })
}

pub async fn resource_detail(
    state: &SharedState,
    organization_id: &str,
    project_id: &str,
    environment_id: &str,
    application_id: &str,
    public_host: &str,
) -> Result<KeyDbResourceDetailDto, AppError> {
    let (application, config, password) = resource_values(
        state,
        organization_id,
        project_id,
        environment_id,
        application_id,
    )
    .await?;
    let deployment_state = deployment::deployment(state, application_id).await?;
    let source_config = source::get(state, application_id).await?;
    let (runtime_name, _) = deployment::identity(state, application_id).await?;
    let private_alias = format!("{runtime_name}-{SERVICE_NAME}");
    let store = ComposeFileStore::new(&state.data_dir, application_id);
    let public_host_port = store
        .read_port_mappings()
        .await
        .map_err(AppError::InternalError)?
        .into_iter()
        .find(|m| m.service == SERVICE_NAME && m.container_port == PORT)
        .map(|m| m.host_port);
    let deployment_status =
        crate::templates::live_deployment_status(state, application_id, deployment_state.status)
            .await;
    let running = deployment_status == "running";
    Ok(KeyDbResourceDetailDto {
        application,
        docker_image: config.image,
        append_only: config.append_only,
        append_fsync: config.append_fsync,
        max_memory_mib: config.max_memory_mib,
        eviction_policy: config.eviction_policy,
        password: Some(password.clone()),
        volume: TemplateVolumeDto {
            name: config.volume,
            mount_path: DATA_PATH.into(),
            service: SERVICE_NAME.into(),
        },
        private_alias: private_alias.clone(),
        internal_port: PORT,
        public_host_port,
        deployment_status,
        config_stale: deployment_state.config_stale,
        private_connection_string: Some(connection_string(&password, &private_alias, PORT)),
        public_connection_string: running
            .then(|| public_host_port.map(|port| connection_string(&password, public_host, port)))
            .flatten(),
        compose: store
            .read_content()
            .await
            .map_err(AppError::InternalError)?,
        variables: variables(),
        runtime_cpu_cores: source_config.build_cpu_cores,
        runtime_memory_mib: source_config.build_memory_mib,
    })
}

fn runtime_environment(preview: &KeyDbTemplatePreviewDto) -> String {
    format!("{PASSWORD_VARIABLE}={}", preview.resolved_password)
}

fn updated_password(
    current_password: String,
    requested_password: Option<String>,
    deployment_status: &str,
) -> Result<(String, bool), AppError> {
    let Some(password) = requested_password else {
        return Ok((current_password, false));
    };
    validate_password(&password)?;
    if deployment_status != "not_deployed" {
        return Err(AppError::ValidationError(
            "KeyDB password can only be edited before the first KeyDB deployment; rotate it after deployment".into(),
        ));
    }
    Ok((password, true))
}

pub async fn create(
    state: &SharedState,
    organization_id: &str,
    project_id: &str,
    environment_id: &str,
    project_name: &str,
    environment_name: &str,
    public_host: &str,
    mut input: KeyDbTemplateInput,
) -> Result<KeyDbTemplateCreatedDto, AppError> {
    if let Some(password) = input.password.as_deref() {
        validate_password(password)?;
    }
    let instance_id = input
        .template_instance_id
        .clone()
        .ok_or_else(|| AppError::ValidationError("Preview KeyDB before creating it".into()))?;
    let runtime_name = deployment::deployment_identity(
        project_name,
        environment_name,
        &input.application_name,
        &instance_id,
        &state.public_ip,
    )
    .0;
    let used_ports = ComposeFileStore::list_host_ports(&state)
        .await
        .map_err(AppError::InternalError)?;
    let reserved_ports = state
        .reserved_ports
        .iter()
        .filter_map(|port| u16::try_from(*port).ok())
        .collect();
    input.template_instance_id = Some(instance_id.clone());
    let preview = preview(
        input.clone(),
        &format!("{runtime_name}-{SERVICE_NAME}"),
        public_host,
        &reserved_ports,
        &used_ports,
    )?;
    let application = project_cmd::create_application_with_id(
        state,
        organization_id,
        project_id,
        environment_id,
        &instance_id,
        Some("keydb"),
        &CreateApplicationRequest {
            name: input.application_name.clone(),
            description: input.description.clone(),
            server_id: Some(input.server_id.clone()),
            server_group_id: None,
        },
    )
    .await?;
    let result = async {
        source::save(
            state,
            &application.id,
            crate::dtos::deployment::UpdateSourceRequest {
                deploy_type: "compose".into(),
                build_method: "compose".into(),
                git_provider_id: None,
                git_url: None,
                git_branch: None,
                auto_deploy: Some(false),
                watch_paths: None,
                dockerfile_path: None,
                compose_file_path: None,
                compose_public_service: None,
                container_port: None,
                railpack_project_path: Some(".".into()),
                railpack_start_command: None,
                build_cpu_cores: input.runtime_cpu_cores,
                build_memory_mib: input.runtime_memory_mib,
                build_swap_mib: None,
                image_url: None,
                registry_username: None,
                registry_password: None,
                restart_policy: None,
            },
        )
        .await?;
        let store = ComposeFileStore::new(&state.data_dir, &application.id);
        store
            .write_content(&preview.compose)
            .await
            .map_err(AppError::InternalError)?;
        store
            .write_port_mappings(&preview.port_mappings)
            .await
            .map_err(AppError::InternalError)?;
        deployment::save_environment(
            state,
            &application.id,
            "runtime",
            &runtime_environment(&preview),
        )
        .await?;
        Ok::<(), AppError>(())
    }
    .await;
    if let Err(error) = result {
        let _ = project_cmd::delete_application(
            state,
            organization_id,
            project_id,
            environment_id,
            &application.id,
        )
        .await;
        return Err(error);
    }
    Ok(KeyDbTemplateCreatedDto {
        application,
        preview,
    })
}

pub async fn update_resource(
    state: &SharedState,
    organization_id: &str,
    project_id: &str,
    environment_id: &str,
    application_id: &str,
    public_host: &str,
    input: KeyDbTemplateConfigUpdate,
) -> Result<KeyDbResourceDetailDto, AppError> {
    source::validate_runtime_limits(input.runtime_cpu_cores, input.runtime_memory_mib)?;
    let (application, config, current_password) = resource_values(
        state,
        organization_id,
        project_id,
        environment_id,
        application_id,
    )
    .await?;
    let deployment_state = deployment::deployment(state, application_id).await?;
    let (password, password_changed) = updated_password(
        current_password,
        input.password.clone(),
        &deployment_state.status,
    )?;
    let store = ComposeFileStore::new(&state.data_dir, application_id);
    let current_mapping = store
        .read_port_mappings()
        .await
        .map_err(AppError::InternalError)?
        .into_iter()
        .find(|m| m.service == SERVICE_NAME && m.container_port == PORT)
        .map(|m| m.host_port);
    let mut used_ports = ComposeFileStore::list_host_ports(&state)
        .await
        .map_err(AppError::InternalError)?;
    if let Some(port) = current_mapping {
        used_ports.remove(&port);
    }
    let reserved_ports = state
        .reserved_ports
        .iter()
        .filter_map(|port| u16::try_from(*port).ok())
        .collect();
    let (runtime_name, _) = deployment::identity(state, application_id).await?;
    let max_memory_mib = input.max_memory_mib.unwrap_or(config.max_memory_mib);
    let preview = preview(
        KeyDbTemplateInput {
            application_name: application.name,
            description: None,
            server_id: application
                .server
                .as_ref()
                .ok_or_else(|| {
                    AppError::ValidationError(
                        "Template applications require a single server".into(),
                    )
                })?
                .id
                .clone(),
            docker_image: Some(input.docker_image),
            password: Some(password),
            append_only: input.append_only.or(Some(config.append_only)),
            append_fsync: input.append_fsync.or(Some(config.append_fsync)),
            max_memory_mib,
            eviction_policy: input.eviction_policy.or(Some(config.eviction_policy)),
            volume_name: Some(config.volume),
            public_host_port: input.public_host_port,
            runtime_cpu_cores: input.runtime_cpu_cores,
            runtime_memory_mib: input.runtime_memory_mib,
            template_instance_id: Some(application_id.into()),
        },
        &format!("{runtime_name}-{SERVICE_NAME}"),
        public_host,
        &reserved_ports,
        &used_ports,
    )?;
    store
        .write_content(&preview.compose)
        .await
        .map_err(AppError::InternalError)?;
    store
        .write_port_mappings(&preview.port_mappings)
        .await
        .map_err(AppError::InternalError)?;
    if password_changed {
        deployment::save_environment(
            state,
            application_id,
            "runtime",
            &runtime_environment(&preview),
        )
        .await?;
    }
    let current_source = source::get(state, application_id).await?;
    source::save(
        state,
        application_id,
        crate::dtos::deployment::UpdateSourceRequest {
            deploy_type: current_source.deploy_type,
            build_method: current_source.build_method,
            git_provider_id: current_source.git_provider_id,
            git_url: current_source.git_url,
            git_branch: current_source.git_branch,
            auto_deploy: Some(current_source.auto_deploy),
            watch_paths: None,
            dockerfile_path: current_source.dockerfile_path,
            compose_file_path: current_source.compose_file_path,
            compose_public_service: current_source.compose_public_service,
            container_port: current_source.container_port,
            railpack_project_path: Some(current_source.railpack_project_path),
            railpack_start_command: current_source.railpack_start_command,
            build_cpu_cores: input.runtime_cpu_cores,
            build_memory_mib: input.runtime_memory_mib,
            build_swap_mib: current_source.build_swap_mib,
            image_url: current_source.image_url,
            registry_username: None,
            registry_password: None,
            restart_policy: None,
        },
    )
    .await?;
    resource_detail(
        state,
        organization_id,
        project_id,
        environment_id,
        application_id,
        public_host,
    )
    .await
}

async fn save_rotated_runtime_variable(
    state: &SharedState,
    application_id: &str,
    value: &str,
) -> Result<(), AppError> {
    let encrypted =
        crate::utils::security::encrypt_data(value, &crate::utils::security::get_encryption_key())
            .map_err(AppError::InternalError)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| AppError::InternalError(e.to_string()))?
        .as_millis() as i64;
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("INSERT INTO application_environment_variables (application_id, scope, name, encrypted_value, updated_at) VALUES (?1, 'runtime', ?2, ?3, ?4) ON CONFLICT(application_id, scope, name) DO UPDATE SET encrypted_value = excluded.encrypted_value, updated_at = excluded.updated_at").bind(application_id).bind(PASSWORD_VARIABLE).bind(encrypted).bind(now).execute(&conn).await.map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok(())
}

fn password_rotation_command<'a>(current_password: &'a str, new_password: &'a str) -> [&'a str; 8] {
    [
        "keydb-cli",
        "--no-auth-warning",
        "-a",
        current_password,
        "CONFIG",
        "SET",
        "requirepass",
        new_password,
    ]
}

async fn execute_password_change(
    podman: &Podman,
    container_name: &str,
    current_password: &str,
    new_password: &str,
) -> Result<(), AppError> {
    let exec = podman
        .containers()
        .get(container_name)
        .create_exec(
            &ExecCreateOpts::builder()
                .command(password_rotation_command(current_password, new_password))
                .attach_stdout(true)
                .attach_stderr(true)
                .build(),
        )
        .await
        .map_err(|error| {
            AppError::InternalError(format!("Could not start KeyDB password rotation: {error}"))
        })?;
    let mut output = exec
        .start(&Default::default())
        .await
        .map_err(|error| {
            AppError::InternalError(format!("Could not run KeyDB password rotation: {error}"))
        })?
        .ok_or_else(|| {
            AppError::InternalError("KeyDB password rotation produced no output".into())
        })?;
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    while let Some(chunk) = output.next().await {
        match chunk {
            Ok(TtyChunk::StdOut(bytes)) => stdout.extend(bytes),
            Ok(TtyChunk::StdErr(bytes)) => stderr.extend(bytes),
            Ok(_) => {}
            Err(error) => return Err(AppError::InternalError(error.to_string())),
        }
    }
    let exit_code = exec
        .inspect()
        .await
        .ok()
        .and_then(|value| value.get("ExitCode").and_then(|code| code.as_i64()));
    if exit_code != Some(0) || String::from_utf8_lossy(&stdout).trim() != "OK" {
        let details = if stderr.is_empty() {
            String::from_utf8_lossy(&stdout)
        } else {
            String::from_utf8_lossy(&stderr)
        };
        return Err(AppError::ValidationError(format!(
            "KeyDB did not accept the password change: {}",
            details.trim()
        )));
    }
    Ok(())
}

pub async fn rotate_password(
    state: &SharedState,
    organization_id: &str,
    project_id: &str,
    environment_id: &str,
    application_id: &str,
    public_host: &str,
    request: KeyDbPasswordRotationRequest,
) -> Result<KeyDbResourceDetailDto, AppError> {
    let (_, _, current_password) = resource_values(
        state,
        organization_id,
        project_id,
        environment_id,
        application_id,
    )
    .await?;
    if deployment::deployment(state, application_id).await?.status != "running" {
        return Err(AppError::ValidationError(
            "Deploy KeyDB before rotating its password".into(),
        ));
    }
    let password = resolve_password(request.password.as_deref())?;
    let (runtime_name, _) = deployment::identity(state, application_id).await?;
    let container_name = format!("{runtime_name}-{SERVICE_NAME}");
    let containers = crate::handler::deployments::valid_container_names(
        state,
        organization_id,
        project_id,
        environment_id,
        application_id,
    )
    .await?;
    if !containers.contains(&container_name) {
        return Err(AppError::ValidationError(
            "KeyDB service is not running".into(),
        ));
    }
    let podman = Podman::new(&state.podman_socket)
        .map_err(|e| AppError::InternalError(format!("Could not connect to Podman: {e}")))?;
    execute_password_change(&podman, &container_name, &current_password, &password).await?;
    if let Err(persistence_error) =
        save_rotated_runtime_variable(state, application_id, &password).await
    {
        return match execute_password_change(&podman, &container_name, &password, &current_password)
            .await
        {
            Ok(()) => Err(AppError::InternalError(format!(
                "Could not persist the rotated KeyDB password; the live password change was rolled back: {persistence_error}"
            ))),
            Err(rollback_error) => Err(AppError::InternalError(format!(
                "Could not persist the rotated KeyDB password and could not roll back the live password change: {persistence_error}; rollback error: {rollback_error}"
            ))),
        };
    }
    resource_detail(
        state,
        organization_id,
        project_id,
        environment_id,
        application_id,
        public_host,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(application_name: &str, password: &str) -> KeyDbTemplateInput {
        KeyDbTemplateInput {
            application_name: application_name.into(),
            server_id: "local".into(),
            description: None,
            docker_image: None,
            password: Some(password.into()),
            append_only: None,
            append_fsync: None,
            max_memory_mib: None,
            eviction_policy: None,
            volume_name: None,
            public_host_port: None,
            runtime_cpu_cores: None,
            runtime_memory_mib: None,
            template_instance_id: None,
        }
    }

    #[test]
    fn preview_renders_persistent_keydb_with_a_password() {
        let preview = preview(
            input("keydb-main", "pa:ss/@word"),
            "keydb-main-keydb",
            "cache.example.com",
            &HashSet::new(),
            &HashSet::new(),
        )
        .unwrap();
        assert!(preview.compose.contains("image: docker.io/eqalpha/keydb"));
        // The resource sits on a network of its own and never on the network
        // every application on the host shares.
        let network_key =
            crate::deployment::resource_networks::network_key(&preview.template_instance_id);
        let network_name =
            crate::deployment::resource_networks::network_name(&preview.template_instance_id);
        assert!(
            preview
                .compose
                .contains(&format!("    networks:\n      {network_key}: {{}}"))
        );
        assert!(preview.compose.contains(&format!(
            "networks:\n  {network_key}:\n    external: true\n    name: {network_name}\n"
        )));
        assert!(!preview.compose.contains("litepod_proxy"));
        assert!(!preview.compose.contains("litepod-network"));
        assert!(
            preview
                .compose
                .contains("    command:\n      - keydb-server\n      - /etc/keydb/keydb.conf")
        );
        assert!(preview.compose.contains("      - ${KEYDB_PASSWORD}"));
        assert!(
            preview
                .compose
                .contains("      - --appendonly\n      - \"yes\"")
        );
        assert!(
            preview
                .compose
                .contains("      - --appendfsync\n      - everysec")
        );
        assert!(preview.compose.contains("/data"));
        assert_eq!(
            preview.private_connection_string,
            "redis://:pa:ss/@word@keydb-main-keydb:6379"
        );
    }

    #[test]
    fn preview_requires_positive_memory_and_known_enums() {
        let mut invalid = input("keydb-main", "safe");
        invalid.max_memory_mib = Some(0);
        assert!(
            preview(
                invalid,
                "keydb",
                "203.0.113.10",
                &HashSet::new(),
                &HashSet::new()
            )
            .is_err()
        );
        let mut invalid = input("keydb-main", "safe");
        invalid.eviction_policy = Some("random".into());
        assert!(
            preview(
                invalid,
                "keydb",
                "203.0.113.10",
                &HashSet::new(),
                &HashSet::new()
            )
            .is_err()
        );
    }

    #[test]
    fn preview_supports_custom_settings_and_rejects_ports() {
        let mut custom = input("keydb-main", "safe");
        custom.append_only = Some(false);
        custom.append_fsync = Some("always".into());
        custom.max_memory_mib = Some(256);
        custom.eviction_policy = Some("allkeys-lru".into());
        custom.public_host_port = Some(26379);
        let custom_preview = preview(
            custom,
            "keydb",
            "cache.example.com",
            &HashSet::new(),
            &HashSet::new(),
        )
        .unwrap();
        assert!(
            custom_preview
                .compose
                .contains("      - --appendonly\n      - \"no\"")
        );
        assert!(
            custom_preview
                .compose
                .contains("      - --maxmemory\n      - \"256mb\"")
        );
        let mut taken = input("keydb-main", "safe");
        taken.public_host_port = Some(26379);
        assert!(
            preview(
                taken,
                "keydb",
                "cache.example.com",
                &HashSet::new(),
                &HashSet::from([26379])
            )
            .is_err()
        );

        let mut zero = input("keydb-main", "safe");
        zero.public_host_port = Some(0);
        assert!(
            preview(
                zero,
                "keydb",
                "203.0.113.10",
                &HashSet::new(),
                &HashSet::new()
            )
            .is_err()
        );
    }

    #[test]
    fn persisted_compose_rejects_uncontrolled_command() {
        let invalid = "services:\n  keydb:\n    image: eqalpha/keydb\n    command: [keydb-server, /etc/keydb/keydb.conf, --requirepass, ${KEYDB_PASSWORD}, --appendonly, yes, --appendfsync, everysec, --maxmemory-policy, noeviction, --save, nope]\n    volumes: [data:/data]\nvolumes:\n  data: {}\n";
        assert!(persisted_compose(invalid).is_err());
    }

    #[test]
    fn persisted_compose_rejects_scalar_commands() {
        let scalar = "services:\n  keydb:\n    image: eqalpha/keydb\n    command: keydb-server /etc/keydb/keydb.conf --requirepass ${KEYDB_PASSWORD} --appendonly yes --appendfsync everysec --maxmemory-policy noeviction\n    volumes: [data:/data]\nvolumes:\n  data: {}\n";
        assert!(persisted_compose(scalar).is_err());
        assert!(
            persisted_compose(&compose(
                DEFAULT_IMAGE,
                "data",
                true,
                DEFAULT_FSYNC,
                None,
                DEFAULT_EVICTION_POLICY,
                "cache-ab12",
            ))
            .is_ok()
        );
    }

    #[test]
    fn persisted_compose_requires_one_keydb_service_and_one_data_mount() {
        let extra_service = "services:\n  keydb:\n    image: eqalpha/keydb\n    command: keydb-server /etc/keydb/keydb.conf --requirepass ${KEYDB_PASSWORD} --appendonly yes --appendfsync everysec --maxmemory-policy noeviction\n    volumes: [data:/data]\n  sidecar:\n    image: busybox\nvolumes:\n  data: {}\n";
        assert!(persisted_compose(extra_service).is_err());
        let duplicate_mount = "services:\n  keydb:\n    image: eqalpha/keydb\n    command: keydb-server /etc/keydb/keydb.conf --requirepass ${KEYDB_PASSWORD} --appendonly yes --appendfsync everysec --maxmemory-policy noeviction\n    volumes: [data:/data, other:/data]\nvolumes:\n  data: {}\n  other: {}\n";
        assert!(persisted_compose(duplicate_mount).is_err());
    }

    #[test]
    fn update_password_allows_only_pre_deployment_changes() {
        assert_eq!(
            updated_password("old".into(), Some("new".into()), "not_deployed").unwrap(),
            ("new".into(), true)
        );
        assert!(updated_password("old".into(), Some("new".into()), "running").is_err());
        assert!(updated_password("old".into(), Some(" bad".into()), "not_deployed").is_err());
    }
}
