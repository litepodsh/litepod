use std::collections::{BTreeMap, HashSet};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use futures_util::StreamExt;
use podman_api::{Podman, conn::TtyChunk, opts::ExecCreateOpts};
use rand::RngExt;
use uuid::Uuid;

use crate::{
    cqrs::commands::project as project_cmd,
    cqrs::queries::project as project_query,
    deployment::{
        self,
        compose::store::{ComposeFileStore, PortMapping},
        source,
    },
    dtos::{
        project::CreateApplicationRequest,
        response::AppError,
        template::{
            DragonflyDbPasswordRotationRequest, DragonflyDbResourceDetailDto,
            DragonflyDbTemplateConfigUpdate, DragonflyDbTemplateCreatedDto,
            DragonflyDbTemplateInput, DragonflyDbTemplatePreviewDto, TemplateVariableDto,
            TemplateVolumeDto,
        },
    },
    state::state::SharedState,
};

const DEFAULT_IMAGE: &str = "docker.dragonflydb.io/dragonflydb/dragonfly";
const DEFAULT_SNAPSHOT_CRON: &str = "*/5 * * * *";
const SERVICE_NAME: &str = "dragonflydb";
const DATA_PATH: &str = "/data";
const PORT: u16 = 6379;
const PASSWORD_VARIABLE: &str = "DRAGONFLY_PASSWORD";

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
    command: Vec<String>,
}

struct PersistedConfig {
    image: String,
    volume: String,
    snapshot_cron: String,
    max_memory_mib: Option<i64>,
    cache_mode: bool,
}

fn required(value: Option<&str>, default: &str, label: &str) -> Result<String, AppError> {
    let value = value.unwrap_or(default).trim();
    if value.is_empty() {
        return Err(AppError::ValidationError(format!("{label} is required")));
    }
    Ok(value.to_string())
}

fn validate_image(value: &str) -> Result<(), AppError> {
    let valid = !value.is_empty()
        && !value.contains(char::is_whitespace)
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric()
                || matches!(character, '.' | '_' | '-' | '/' | ':' | '@' | '+')
        });
    if !valid {
        return Err(AppError::ValidationError(
            "Docker image must be a complete image reference without whitespace".into(),
        ));
    }
    Ok(())
}

fn derive_volume_name(application_name: &str) -> String {
    let name = application_name
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    let name = name.trim_matches('-');
    if name.is_empty() {
        "dragonflydb-data".into()
    } else {
        format!("{name}-data")
    }
}

fn validate_volume_name(value: &str) -> Result<(), AppError> {
    let valid = !value.is_empty()
        && value.len() <= 100
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        });
    if !valid {
        return Err(AppError::ValidationError(
            "Volume name must contain only letters, digits, hyphens, underscores, or periods"
                .into(),
        ));
    }
    Ok(())
}

fn validate_cron_number(value: &str, min: u32, max: u32) -> bool {
    value
        .parse::<u32>()
        .is_ok_and(|number| (min..=max).contains(&number))
}

fn validate_cron_base(value: &str, min: u32, max: u32) -> bool {
    if value == "*" || validate_cron_number(value, min, max) {
        return true;
    }
    value.split_once('-').is_some_and(|(start, end)| {
        match (start.parse::<u32>(), end.parse::<u32>()) {
            (Ok(start), Ok(end)) => {
                (min..=max).contains(&start) && (min..=max).contains(&end) && start <= end
            }
            _ => false,
        }
    })
}

fn validate_cron_field(value: &str, min: u32, max: u32) -> bool {
    !value.is_empty()
        && value.split(',').all(|part| match part.split_once('/') {
            Some((base, step)) => {
                validate_cron_base(base, min, max)
                    && step
                        .parse::<u32>()
                        .is_ok_and(|number| number > 0 && number <= max - min + 1)
            }
            None => validate_cron_base(part, min, max),
        })
}

fn validate_snapshot_cron(value: &str) -> Result<(), AppError> {
    let fields = value.split_whitespace().collect::<Vec<_>>();
    let valid = fields.len() == 5
        && validate_cron_field(fields[0], 0, 59)
        && validate_cron_field(fields[1], 0, 23)
        && validate_cron_field(fields[2], 1, 31)
        && validate_cron_field(fields[3], 1, 12)
        && validate_cron_field(fields[4], 0, 7);
    if !valid {
        return Err(AppError::ValidationError(
            "Snapshot cron must be a valid five-field cron expression".into(),
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
            "DragonflyDB password must not contain line breaks or surrounding whitespace".into(),
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

/// Redis-compatible clients are handed the password verbatim, so the URL shown
/// in the dashboard is the exact string they expect - no percent-encoding.
fn connection_string(password: &str, host: &str, port: u16) -> String {
    format!("redis://:{password}@{host}:{port}")
}

fn compose(
    image: &str,
    volume: &str,
    snapshot_cron: &str,
    max_memory_mib: Option<i64>,
    cache_mode: bool,
    application_id: &str,
) -> String {
    let mut command = format!(
        "      - --requirepass=${{{PASSWORD_VARIABLE}}}\n      - --dir={DATA_PATH}\n      - --dbfilename=dump\n      - --snapshot_cron={snapshot_cron}\n      - --primary_port_http_enabled=false\n"
    );
    if let Some(max_memory_mib) = max_memory_mib {
        command.push_str(&format!("      - --maxmemory={max_memory_mib}MiB\n"));
    }
    if cache_mode {
        command.push_str("      - --cache_mode=true\n");
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
    let invalid =
        || AppError::ValidationError("DragonflyDB Compose configuration is invalid".into());
    let compose: PersistedCompose = serde_yaml::from_str(content).map_err(|_| invalid())?;
    let service = compose.services.get(SERVICE_NAME).ok_or_else(&invalid)?;
    let volume = service
        .volumes
        .iter()
        .find_map(|mount| mount.strip_suffix(&format!(":{DATA_PATH}")))
        .filter(|name| !name.is_empty())
        .ok_or_else(&invalid)?;
    if !compose.volumes.contains_key(volume) {
        return Err(invalid());
    }
    let mut requirepass = false;
    let mut data_dir = false;
    let mut dbfilename = false;
    let mut http_console_disabled = false;
    let mut snapshot_cron = None;
    let mut max_memory_mib = None;
    let mut cache_mode = false;
    for option in &service.command {
        match option.as_str() {
            "--requirepass=${DRAGONFLY_PASSWORD}" if !requirepass => requirepass = true,
            "--dir=/data" if !data_dir => data_dir = true,
            "--dbfilename=dump" if !dbfilename => dbfilename = true,
            "--primary_port_http_enabled=false" if !http_console_disabled => {
                http_console_disabled = true
            }
            "--cache_mode=true" if !cache_mode => cache_mode = true,
            _ if option.starts_with("--snapshot_cron=") && snapshot_cron.is_none() => {
                let value = option
                    .strip_prefix("--snapshot_cron=")
                    .ok_or_else(&invalid)?;
                if validate_snapshot_cron(value).is_err() {
                    return Err(invalid());
                }
                snapshot_cron = Some(value.to_string());
            }
            _ if option.starts_with("--maxmemory=") && max_memory_mib.is_none() => {
                let value = option
                    .strip_prefix("--maxmemory=")
                    .and_then(|value| value.strip_suffix("MiB"))
                    .and_then(|value| value.parse::<i64>().ok())
                    .filter(|value| *value > 0)
                    .ok_or_else(&invalid)?;
                max_memory_mib = Some(value);
            }
            _ => return Err(invalid()),
        }
    }
    if !requirepass || !data_dir || !dbfilename || !http_console_disabled {
        return Err(invalid());
    }
    let snapshot_cron = snapshot_cron.ok_or_else(&invalid)?;
    if cache_mode && max_memory_mib.is_none() {
        return Err(invalid());
    }
    Ok(PersistedConfig {
        image: service.image.clone(),
        volume: volume.to_string(),
        snapshot_cron,
        max_memory_mib,
        cache_mode,
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
    if application.template_kind.as_deref() != Some("dragonflydb") {
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
    input: DragonflyDbTemplateInput,
    private_alias: &str,
    public_host: &str,
    reserved_ports: &HashSet<u16>,
    used_ports: &HashSet<u16>,
) -> Result<DragonflyDbTemplatePreviewDto, AppError> {
    let application_name = required(Some(&input.application_name), "", "Application name")?;
    let docker_image = required(input.docker_image.as_deref(), DEFAULT_IMAGE, "Docker image")?;
    validate_image(&docker_image)?;
    let password = resolve_password(input.password.as_deref())?;
    let snapshot_cron = required(
        input.snapshot_cron.as_deref(),
        DEFAULT_SNAPSHOT_CRON,
        "Snapshot cron",
    )?;
    validate_snapshot_cron(&snapshot_cron)?;
    let cache_mode = input.cache_mode.unwrap_or(false);
    if cache_mode && input.max_memory_mib.is_none_or(|value| value <= 0) {
        return Err(AppError::ValidationError(
            "Cache mode requires a positive max memory MiB value".into(),
        ));
    }
    if input.max_memory_mib.is_some_and(|value| value <= 0) {
        return Err(AppError::ValidationError(
            "Max memory MiB must be positive".into(),
        ));
    }
    let volume_name = input
        .volume_name
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(|value| value.trim().to_string())
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
                    "Configure a public server host before publishing DragonflyDB".into(),
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
    Ok(DragonflyDbTemplatePreviewDto {
        template_instance_id: template_instance_id.clone(),
        resolved_password: password.clone(),
        docker_image: docker_image.clone(),
        snapshot_cron: snapshot_cron.clone(),
        max_memory_mib: input.max_memory_mib,
        cache_mode,
        private_alias: private_alias.to_string(),
        private_connection_string: connection_string(&password, private_alias, PORT),
        public_connection_string,
        compose: compose(
            &docker_image,
            &volume_name,
            &snapshot_cron,
            input.max_memory_mib,
            cache_mode,
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
) -> Result<DragonflyDbResourceDetailDto, AppError> {
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
        .find(|mapping| mapping.service == SERVICE_NAME && mapping.container_port == PORT)
        .map(|mapping| mapping.host_port);
    let deployment_status =
        crate::templates::live_deployment_status(state, application_id, deployment_state.status)
            .await;
    let running = deployment_status == "running";
    Ok(DragonflyDbResourceDetailDto {
        application,
        docker_image: config.image,
        snapshot_cron: config.snapshot_cron,
        max_memory_mib: config.max_memory_mib,
        cache_mode: config.cache_mode,
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

fn runtime_environment(preview: &DragonflyDbTemplatePreviewDto) -> String {
    format!("{PASSWORD_VARIABLE}={}", preview.resolved_password)
}

fn password_rotation_command<'a>(current_password: &'a str, new_password: &'a str) -> [&'a str; 8] {
    [
        "redis-cli",
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
            AppError::InternalError(format!(
                "Could not start DragonflyDB password rotation: {error}"
            ))
        })?;
    let mut output = exec
        .start(&Default::default())
        .await
        .map_err(|error| {
            AppError::InternalError(format!(
                "Could not run DragonflyDB password rotation: {error}"
            ))
        })?
        .ok_or_else(|| {
            AppError::InternalError("DragonflyDB password rotation produced no output".into())
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
            "DragonflyDB did not accept the password change: {}",
            details.trim()
        )));
    }
    Ok(())
}

pub async fn create(
    state: &SharedState,
    organization_id: &str,
    project_id: &str,
    environment_id: &str,
    project_name: &str,
    environment_name: &str,
    public_host: &str,
    mut input: DragonflyDbTemplateInput,
) -> Result<DragonflyDbTemplateCreatedDto, AppError> {
    if let Some(password) = input.password.as_deref() {
        validate_password(password)?;
    }
    let instance_id = input.template_instance_id.clone().ok_or_else(|| {
        AppError::ValidationError("Preview DragonflyDB before creating it".into())
    })?;
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
        Some("dragonflydb"),
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
    Ok(DragonflyDbTemplateCreatedDto {
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
    input: DragonflyDbTemplateConfigUpdate,
) -> Result<DragonflyDbResourceDetailDto, AppError> {
    source::validate_runtime_limits(input.runtime_cpu_cores, input.runtime_memory_mib)?;
    if let Some(password) = input.password.as_deref() {
        validate_password(password)?;
    }
    let (application, config, current_password) = resource_values(
        state,
        organization_id,
        project_id,
        environment_id,
        application_id,
    )
    .await?;
    let deployment_state = deployment::deployment(state, application_id).await?;
    let password_changed = input.password.is_some();
    if password_changed && deployment_state.status != "not_deployed" {
        return Err(AppError::ValidationError(
            "DragonflyDB password can only be edited before the first DragonflyDB deployment"
                .into(),
        ));
    }
    let store = ComposeFileStore::new(&state.data_dir, application_id);
    let current_mapping = store
        .read_port_mappings()
        .await
        .map_err(AppError::InternalError)?
        .into_iter()
        .find(|mapping| mapping.service == SERVICE_NAME && mapping.container_port == PORT)
        .map(|mapping| mapping.host_port);
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
        DragonflyDbTemplateInput {
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
            password: Some(input.password.unwrap_or(current_password)),
            snapshot_cron: input.snapshot_cron.or(Some(config.snapshot_cron)),
            max_memory_mib,
            cache_mode: input.cache_mode.or(Some(config.cache_mode)),
            volume_name: Some(config.volume),
            public_host_port: input.public_host_port,
            runtime_cpu_cores: input.runtime_cpu_cores,
            runtime_memory_mib: input.runtime_memory_mib,
            template_instance_id: Some(application_id.to_string()),
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
        .map_err(|error| AppError::InternalError(error.to_string()))?
        .as_millis() as i64;
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("INSERT INTO application_environment_variables (application_id, scope, name, encrypted_value, updated_at) VALUES (?1, 'runtime', ?2, ?3, ?4) ON CONFLICT(application_id, scope, name) DO UPDATE SET encrypted_value = excluded.encrypted_value, updated_at = excluded.updated_at").bind(application_id).bind(PASSWORD_VARIABLE).bind(encrypted).bind(now).execute(&conn).await.map_err(|error| AppError::DatabaseError(error.to_string()))?;
    Ok(())
}

pub async fn rotate_password(
    state: &SharedState,
    organization_id: &str,
    project_id: &str,
    environment_id: &str,
    application_id: &str,
    public_host: &str,
    request: DragonflyDbPasswordRotationRequest,
) -> Result<DragonflyDbResourceDetailDto, AppError> {
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
            "Deploy DragonflyDB before rotating its password".into(),
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
            "DragonflyDB service is not running".into(),
        ));
    }
    let podman = Podman::new(&state.podman_socket).map_err(|error| {
        AppError::InternalError(format!("Could not connect to Podman: {error}"))
    })?;
    execute_password_change(&podman, &container_name, &current_password, &password).await?;
    if let Err(persistence_error) =
        save_rotated_runtime_variable(state, application_id, &password).await
    {
        return match execute_password_change(&podman, &container_name, &password, &current_password)
            .await
        {
            Ok(()) => Err(AppError::InternalError(format!(
                "Could not persist the rotated DragonflyDB password; the live password change was rolled back: {persistence_error}"
            ))),
            Err(rollback_error) => Err(AppError::InternalError(format!(
                "Could not persist the rotated DragonflyDB password and could not roll back the live password change: {persistence_error}; rollback error: {rollback_error}"
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

    fn input(application_name: &str, password: &str) -> DragonflyDbTemplateInput {
        DragonflyDbTemplateInput {
            application_name: application_name.into(),
            server_id: "local".into(),
            description: None,
            docker_image: None,
            password: Some(password.into()),
            snapshot_cron: None,
            max_memory_mib: None,
            cache_mode: None,
            volume_name: None,
            public_host_port: None,
            runtime_cpu_cores: None,
            runtime_memory_mib: None,
            template_instance_id: None,
        }
    }

    #[test]
    fn preview_renders_secure_persistent_dragonfly() {
        let preview = preview(
            input("cache-main", "pa:ss/@word"),
            "cache-main-dragonflydb",
            "cache.example.com",
            &HashSet::new(),
            &HashSet::new(),
        )
        .unwrap();
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
                .contains("--requirepass=${DRAGONFLY_PASSWORD}")
        );
        assert!(preview.compose.contains("--snapshot_cron=*/5 * * * *"));
        assert!(
            preview
                .compose
                .contains("--primary_port_http_enabled=false")
        );
        assert!(preview.compose.contains("/data"));
        assert_eq!(
            preview.private_connection_string,
            "redis://:pa:ss/@word@cache-main-dragonflydb:6379"
        );
    }

    #[test]
    fn preview_rejects_bad_cron_and_cache_mode_without_limit() {
        let mut invalid = input("cache-main", "safe");
        invalid.snapshot_cron = Some("every five".into());
        assert!(
            preview(
                invalid,
                "cache",
                "203.0.113.10",
                &HashSet::new(),
                &HashSet::new()
            )
            .is_err()
        );
        let mut invalid = input("cache-main", "safe");
        invalid.cache_mode = Some(true);
        assert!(
            preview(
                invalid,
                "cache",
                "203.0.113.10",
                &HashSet::new(),
                &HashSet::new()
            )
            .is_err()
        );
    }

    #[test]
    fn preview_rejects_passwords_that_cannot_be_serialized_as_one_runtime_value() {
        for password in [" leading", "trailing ", "safe\nDRAGONFLY_PASSWORD=injected"] {
            assert!(
                preview(
                    input("cache-main", password),
                    "cache",
                    "203.0.113.10",
                    &HashSet::new(),
                    &HashSet::new()
                )
                .is_err()
            );
        }
    }

    #[test]
    fn preview_adds_optional_memory_cache_mode_and_rejects_taken_public_port() {
        let mut configured = input("cache-main", "safe");
        configured.snapshot_cron = Some("0 */2 * * 1-5".into());
        configured.max_memory_mib = Some(512);
        configured.cache_mode = Some(true);
        configured.public_host_port = Some(16379);
        let configured_preview = preview(
            configured,
            "cache",
            "cache.example.com",
            &HashSet::new(),
            &HashSet::new(),
        )
        .unwrap();
        assert!(configured_preview.compose.contains("--maxmemory=512MiB"));
        assert!(configured_preview.compose.contains("--cache_mode=true"));
        assert_eq!(
            configured_preview.public_connection_string.as_deref(),
            Some("redis://:safe@cache.example.com:16379")
        );
        let mut conflict = input("cache-main", "safe");
        conflict.public_host_port = Some(16379);
        assert!(
            super::preview(
                conflict,
                "cache",
                "203.0.113.10",
                &HashSet::new(),
                &HashSet::from([16379])
            )
            .is_err()
        );

        let mut zero = input("cache-main", "safe");
        zero.public_host_port = Some(0);
        assert!(
            super::preview(
                zero,
                "cache",
                "203.0.113.10",
                &HashSet::new(),
                &HashSet::new()
            )
            .is_err()
        );
    }

    #[test]
    fn persisted_compose_requires_exact_managed_commands_and_declared_data_volume() {
        let rendered = compose(
            DEFAULT_IMAGE,
            "cache-main-data",
            DEFAULT_SNAPSHOT_CRON,
            None,
            false,
            "cache-ab12",
        );
        let persisted = persisted_compose(&rendered).unwrap();
        assert_eq!(persisted.volume, "cache-main-data");
        assert_eq!(persisted.snapshot_cron, DEFAULT_SNAPSHOT_CRON);
        assert!(rendered.contains("--requirepass=${DRAGONFLY_PASSWORD}"));
        assert!(persisted_compose(&rendered.replace("--dir=/data", "--dir=/broken")).is_err());
        assert!(
            persisted_compose(&rendered.replace(
                "      - --dir=/data\n",
                "      - --dir=/data\n      - --dir=/data\n"
            ))
            .is_err()
        );
        assert!(
            persisted_compose(&rendered.replace(
                "      - --snapshot_cron=*/5 * * * *\n",
                "      - --snapshot_cron=*/5 * * * *\n      - --snapshot_cron=0 * * * *\n"
            ))
            .is_err()
        );
        assert!(
            persisted_compose(&rendered.replace(
                "--primary_port_http_enabled=false",
                "--primary_port_http_enabled=true"
            ))
            .is_err()
        );
        assert!(
            persisted_compose(&rendered.replace(
                "      - --dir=/data\n",
                "      - --dir=/data\n      - --unknown_flag=true\n"
            ))
            .is_err()
        );
        assert!(
            persisted_compose(&rendered.replace("volumes:\n  cache-main-data: {}", "volumes: {}"))
                .is_err()
        );
    }

    #[test]
    fn rotation_command_and_runtime_environment_preserve_password_boundaries() {
        assert_eq!(
            password_rotation_command("old-password", "new-password"),
            [
                "redis-cli",
                "--no-auth-warning",
                "-a",
                "old-password",
                "CONFIG",
                "SET",
                "requirepass",
                "new-password",
            ]
        );
        let preview = preview(
            input("cache-main", "safe-password"),
            "cache",
            "203.0.113.10",
            &HashSet::new(),
            &HashSet::new(),
        )
        .unwrap();
        assert_eq!(
            runtime_environment(&preview),
            "DRAGONFLY_PASSWORD=safe-password"
        );
        assert!(resolve_password(Some("safe\npassword")).is_err());
    }
}
