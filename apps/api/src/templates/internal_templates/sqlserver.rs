use std::collections::{BTreeMap, HashSet};

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
            SqlServerPasswordRotationRequest, SqlServerResourceDetailDto,
            SqlServerTemplateConfigUpdate, SqlServerTemplateCreatedDto, SqlServerTemplateInput,
            SqlServerTemplatePreviewDto, TemplateVariableDto, TemplateVolumeDto,
        },
    },
    state::state::SharedState,
};

const DEFAULT_IMAGE: &str = "mcr.microsoft.com/mssql/server:2025-latest";
const SERVICE_NAME: &str = "sqlserver";
const DATA_PATH: &str = "/var/opt/mssql";
const PORT: u16 = 1433;
const SA_USER: &str = "sa";
const PASSWORD_VARIABLE: &str = "MSSQL_SA_PASSWORD";
const MIN_MEMORY_MIB: i64 = 2048;
const GENERATED_PASSWORD_LENGTH: usize = 24;
const UPPERCASE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const LOWERCASE: &[u8] = b"abcdefghijklmnopqrstuvwxyz";
const DIGITS: &[u8] = b"0123456789";
// `;` and `=` are excluded so the password can be inlined in an ADO keyword connection string
// without quoting; the quote/backslash/dollar characters are excluded by `validate_password`.
const SYMBOLS: &[u8] = b"!*+,-.:<>^_~";

#[derive(serde::Deserialize)]
struct PersistedCompose {
    services: BTreeMap<String, PersistedService>,
}

#[derive(serde::Deserialize)]
struct PersistedService {
    image: String,
    #[serde(default)]
    volumes: Vec<String>,
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
        "sqlserver-data".into()
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

fn pick(alphabet: &[u8]) -> char {
    alphabet[rand::rng().random_range(0..alphabet.len())] as char
}

/// SQL Server refuses to start when the `sa` password fails its own policy, so generated
/// passwords always carry at least one character from every class.
fn generate_password() -> String {
    let mut characters = vec![
        pick(UPPERCASE),
        pick(LOWERCASE),
        pick(DIGITS),
        pick(SYMBOLS),
    ];
    let alphabet = [UPPERCASE, LOWERCASE, DIGITS, SYMBOLS].concat();
    while characters.len() < GENERATED_PASSWORD_LENGTH {
        characters.push(pick(&alphabet));
    }
    for index in (1..characters.len()).rev() {
        let target = rand::rng().random_range(0..=index);
        characters.swap(index, target);
    }
    characters.into_iter().collect()
}

fn validate_password(value: &str) -> Result<(), AppError> {
    if value.trim() != value || value.contains('\n') || value.contains('\r') {
        return Err(AppError::ValidationError(
            "SA password must not contain line breaks or surrounding whitespace".into(),
        ));
    }
    if value
        .chars()
        .any(|character| matches!(character, '\'' | '"' | '`' | '\\' | '$'))
    {
        return Err(AppError::ValidationError(
            "SA password must not contain quotes, backslashes, or dollar signs".into(),
        ));
    }
    if value.contains(';') || value.contains('=') {
        return Err(AppError::ValidationError(
            "SA password must not contain semicolons or equals signs so it stays valid inside a connection string"
                .into(),
        ));
    }
    let length = value.chars().count();
    if !(8..=128).contains(&length) {
        return Err(AppError::ValidationError(
            "SA password must be between 8 and 128 characters".into(),
        ));
    }
    let classes = [
        value
            .chars()
            .any(|character| character.is_ascii_uppercase()),
        value
            .chars()
            .any(|character| character.is_ascii_lowercase()),
        value.chars().any(|character| character.is_ascii_digit()),
        value
            .chars()
            .any(|character| !character.is_ascii_alphanumeric()),
    ]
    .into_iter()
    .filter(|present| *present)
    .count();
    if classes < 3 {
        return Err(AppError::ValidationError(
            "SA password must include at least three of: uppercase letters, lowercase letters, digits, symbols"
                .into(),
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

fn compose(image: &str, volume: &str, application_id: &str) -> String {
    format!(
        "services:\n  {SERVICE_NAME}:\n    image: {image}\n    user: \"0:0\"\n    environment:\n      ACCEPT_EULA: \"Y\"\n      MSSQL_PID: Developer\n      {PASSWORD_VARIABLE}: ${{{PASSWORD_VARIABLE}}}\n    volumes:\n      - {volume}:{DATA_PATH}\n{service_networks}volumes:\n  {volume}: {{}}\n{networks}",
        service_networks = crate::templates::resource_service_networks(application_id),
        networks = crate::templates::resource_network_compose(application_id),
    )
}

fn connection_string(password: &str, host: &str, port: u16) -> String {
    format!(
        "sqlserver://{SA_USER}:{}@{host}:{port}",
        urlencoding::encode(password)
    )
}

/// SSMS, Azure Data Studio, and the .NET/ODBC drivers do not accept the `sqlserver://` URI, so the
/// keyword form is rendered alongside it. `;` and `=` are rejected by [`validate_password`], which
/// is what keeps the password safe to inline here without quoting.
fn ado_connection_string(password: &str, host: &str, port: u16) -> String {
    format!(
        "Server={host},{port};User Id={SA_USER};Password={password};TrustServerCertificate=True"
    )
}

fn variables() -> Vec<TemplateVariableDto> {
    vec![TemplateVariableDto {
        name: PASSWORD_VARIABLE.into(),
        masked: true,
    }]
}

fn persisted_compose(content: &str) -> Result<(String, String), AppError> {
    let compose: PersistedCompose = serde_yaml::from_str(content).map_err(|_| {
        AppError::ValidationError("SQL Server Compose configuration is invalid".into())
    })?;
    let service = compose.services.get(SERVICE_NAME).ok_or_else(|| {
        AppError::ValidationError("SQL Server service is missing from Compose".into())
    })?;
    let volume = service
        .volumes
        .iter()
        .find_map(|mount| mount.strip_suffix(&format!(":{DATA_PATH}")))
        .filter(|name| !name.is_empty())
        .ok_or_else(|| AppError::ValidationError("SQL Server data volume is missing".into()))?;
    Ok((service.image.clone(), volume.to_string()))
}

async fn resource_values(
    state: &SharedState,
    organization_id: &str,
    project_id: &str,
    environment_id: &str,
    application_id: &str,
) -> Result<(crate::dtos::project::ApplicationDto, String, String, String), AppError> {
    let application = project_query::get_application(
        state,
        organization_id,
        project_id,
        environment_id,
        application_id,
    )
    .await?;
    if application.template_kind.as_deref() != Some("sqlserver") {
        return Err(AppError::ResourceNotFound);
    }
    let store = ComposeFileStore::new(&state.data_dir, application_id);
    let (docker_image, volume_name) = persisted_compose(
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
    Ok((application, docker_image, volume_name, password))
}

pub fn preview(
    input: SqlServerTemplateInput,
    private_alias: &str,
    public_host: &str,
    reserved_ports: &HashSet<u16>,
    used_ports: &HashSet<u16>,
) -> Result<SqlServerTemplatePreviewDto, AppError> {
    let application_name = required(Some(&input.application_name), "", "Application name")?;
    let docker_image = required(input.docker_image.as_deref(), DEFAULT_IMAGE, "Docker image")?;
    validate_image(&docker_image)?;
    let password = resolve_password(input.password.as_deref())?;
    if input
        .runtime_memory_mib
        .is_some_and(|value| value < MIN_MEMORY_MIB)
    {
        return Err(AppError::ValidationError(format!(
            "SQL Server Developer Edition requires at least {MIN_MEMORY_MIB} MiB of memory"
        )));
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
    let public_endpoint = match input.public_host_port {
        Some(port) => {
            if public_host.trim().is_empty() {
                return Err(AppError::ValidationError(
                    "Configure a public server host before publishing SQL Server Developer Edition"
                        .into(),
                ));
            }
            if reserved_ports.contains(&port) || used_ports.contains(&port) {
                return Err(AppError::ValidationError(format!(
                    "Host port {port} is already in use or reserved"
                )));
            }
            Some((public_host.trim().to_string(), port))
        }
        None => None,
    };
    Ok(SqlServerTemplatePreviewDto {
        template_instance_id: template_instance_id.clone(),
        resolved_password: password.clone(),
        docker_image: docker_image.clone(),
        private_alias: private_alias.to_string(),
        private_connection_string: connection_string(&password, private_alias, PORT),
        private_ado_connection_string: ado_connection_string(&password, private_alias, PORT),
        public_connection_string: public_endpoint
            .as_ref()
            .map(|(host, port)| connection_string(&password, host, *port)),
        public_ado_connection_string: public_endpoint
            .as_ref()
            .map(|(host, port)| ado_connection_string(&password, host, *port)),
        compose: compose(&docker_image, &volume_name, &template_instance_id),
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
) -> Result<SqlServerResourceDetailDto, AppError> {
    let (application, docker_image, volume_name, password) = resource_values(
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
    Ok(SqlServerResourceDetailDto {
        application,
        docker_image,
        password: Some(password.clone()),
        volume: TemplateVolumeDto {
            name: volume_name,
            mount_path: DATA_PATH.into(),
            service: SERVICE_NAME.into(),
        },
        private_alias: private_alias.clone(),
        internal_port: PORT,
        public_host_port,
        deployment_status,
        config_stale: deployment_state.config_stale,
        private_connection_string: Some(connection_string(&password, &private_alias, PORT)),
        private_ado_connection_string: Some(ado_connection_string(&password, &private_alias, PORT)),
        public_connection_string: running
            .then(|| public_host_port.map(|port| connection_string(&password, public_host, port)))
            .flatten(),
        public_ado_connection_string: running
            .then(|| {
                public_host_port.map(|port| ado_connection_string(&password, public_host, port))
            })
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

fn runtime_environment(preview: &SqlServerTemplatePreviewDto) -> String {
    format!("{PASSWORD_VARIABLE}={}", preview.resolved_password)
}

pub async fn create(
    state: &SharedState,
    organization_id: &str,
    project_id: &str,
    environment_id: &str,
    project_name: &str,
    environment_name: &str,
    public_host: &str,
    mut input: SqlServerTemplateInput,
) -> Result<SqlServerTemplateCreatedDto, AppError> {
    if let Some(password) = input.password.as_deref() {
        validate_password(password)?;
    }
    let instance_id = input.template_instance_id.clone().ok_or_else(|| {
        AppError::ValidationError("Preview SQL Server Developer Edition before creating it".into())
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
        Some("sqlserver"),
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
    Ok(SqlServerTemplateCreatedDto {
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
    input: SqlServerTemplateConfigUpdate,
) -> Result<SqlServerResourceDetailDto, AppError> {
    if let Some(password) = input.password.as_deref() {
        validate_password(password)?;
    }
    let (application, _, volume_name, current_password) = resource_values(
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
            "SQL Server Developer Edition credentials can only be edited before the first deployment"
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
    let preview = preview(
        SqlServerTemplateInput {
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
            volume_name: Some(volume_name),
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

/// The SQL Server images no longer ship `mssql-tools`, so the password cannot be changed with an
/// in-container `ALTER LOGIN`. `mssql-conf` is always present, but SQL Server only picks the new
/// value up on the next start, so the container is restarted afterwards.
pub async fn rotate_password(
    state: &SharedState,
    organization_id: &str,
    project_id: &str,
    environment_id: &str,
    application_id: &str,
    public_host: &str,
    request: SqlServerPasswordRotationRequest,
) -> Result<SqlServerResourceDetailDto, AppError> {
    let _ = resource_values(
        state,
        organization_id,
        project_id,
        environment_id,
        application_id,
    )
    .await?;
    if deployment::deployment(state, application_id).await?.status != "running" {
        return Err(AppError::ValidationError(
            "Deploy SQL Server Developer Edition before rotating its password".into(),
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
            "SQL Server Developer Edition service is not running".into(),
        ));
    }
    let podman = Podman::new(&state.podman_socket).map_err(|error| {
        AppError::InternalError(format!("Could not connect to Podman: {error}"))
    })?;
    let container = podman.containers().get(&container_name);
    let exec = container
        .create_exec(
            &ExecCreateOpts::builder()
                .command(["/opt/mssql/bin/mssql-conf", "set-sa-password"])
                .env([(PASSWORD_VARIABLE, password.as_str())])
                .attach_stdout(true)
                .attach_stderr(true)
                .build(),
        )
        .await
        .map_err(|error| {
            AppError::InternalError(format!(
                "Could not start SQL Server password rotation: {error}"
            ))
        })?;
    let mut output = exec
        .start(&Default::default())
        .await
        .map_err(|error| {
            AppError::InternalError(format!(
                "Could not run SQL Server password rotation: {error}"
            ))
        })?
        .ok_or_else(|| {
            AppError::InternalError("SQL Server password rotation produced no output".into())
        })?;
    let mut logs = Vec::new();
    while let Some(chunk) = output.next().await {
        match chunk {
            Ok(TtyChunk::StdOut(bytes)) | Ok(TtyChunk::StdErr(bytes)) => logs.extend(bytes),
            Ok(_) => {}
            Err(error) => return Err(AppError::InternalError(error.to_string())),
        }
    }
    // `mssql-conf` writes progress to both streams, so the exit code decides the outcome.
    let exit_code = exec
        .inspect()
        .await
        .ok()
        .and_then(|value| value.get("ExitCode").and_then(|code| code.as_i64()));
    if exit_code.is_some_and(|code| code != 0) {
        return Err(AppError::ValidationError(format!(
            "SQL Server did not accept the password change: {}",
            String::from_utf8_lossy(&logs).trim()
        )));
    }
    save_rotated_runtime_variable(state, application_id, &password).await?;
    container.restart().await.map_err(|error| {
        AppError::InternalError(format!(
            "SQL Server password was changed but the service could not be restarted: {error}"
        ))
    })?;
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

    fn input(application_name: &str, password: Option<&str>) -> SqlServerTemplateInput {
        SqlServerTemplateInput {
            application_name: application_name.into(),
            server_id: "local".into(),
            description: None,
            docker_image: None,
            password: password.map(|value| value.into()),
            volume_name: None,
            public_host_port: None,
            runtime_cpu_cores: None,
            runtime_memory_mib: None,
            template_instance_id: None,
        }
    }

    #[test]
    fn preview_renders_developer_edition_compose() {
        let preview = preview(
            input("db-main", Some("Passw0rd!secure")),
            "db-main-sqlserver",
            "db.example.com",
            &HashSet::new(),
            &HashSet::new(),
        )
        .unwrap();
        assert!(
            preview
                .compose
                .contains("image: mcr.microsoft.com/mssql/server:2025-latest")
        );
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
        assert!(preview.compose.contains("ACCEPT_EULA: \"Y\""));
        assert!(preview.compose.contains("MSSQL_PID: Developer"));
        assert!(preview.compose.contains("user: \"0:0\""));
        assert!(
            preview
                .compose
                .contains("MSSQL_SA_PASSWORD: ${MSSQL_SA_PASSWORD}")
        );
        assert!(preview.compose.contains("db-main-data:/var/opt/mssql"));
        assert_eq!(
            preview.private_connection_string,
            "sqlserver://sa:Passw0rd%21secure@db-main-sqlserver:1433"
        );
        assert_eq!(
            preview.private_ado_connection_string,
            "Server=db-main-sqlserver,1433;User Id=sa;Password=Passw0rd!secure;TrustServerCertificate=True"
        );
        assert!(preview.public_connection_string.is_none());
        assert!(preview.public_ado_connection_string.is_none());
        assert_eq!(preview.variables.len(), 1);
        assert!(preview.variables[0].masked);
        assert!(preview.port_mappings.is_empty());
    }

    #[test]
    fn preview_maps_the_public_port_and_encodes_credentials() {
        let mut published = input("db-main", Some("pa:ss/@Word1"));
        published.public_host_port = Some(14330);
        let preview = preview(
            published,
            "db-main-sqlserver",
            "203.0.113.10",
            &HashSet::new(),
            &HashSet::new(),
        )
        .unwrap();
        assert_eq!(
            preview.public_connection_string.as_deref(),
            Some("sqlserver://sa:pa%3Ass%2F%40Word1@203.0.113.10:14330")
        );
        assert_eq!(
            preview.public_ado_connection_string.as_deref(),
            Some(
                "Server=203.0.113.10,14330;User Id=sa;Password=pa:ss/@Word1;TrustServerCertificate=True"
            )
        );
        assert_eq!(preview.port_mappings.len(), 1);
        assert_eq!(preview.port_mappings[0].container_port, PORT);
        assert_eq!(preview.port_mappings[0].host_port, 14330);
    }

    #[test]
    fn preview_rejects_passwords_that_fail_the_sa_policy() {
        for password in [
            "short1A",
            "alllowercaseonly",
            "Passw0rd\ninjected",
            " Passw0rd! ",
            "Passw0rd'quote",
            "Passw0rd;semicolon",
            "Passw0rd=equals",
        ] {
            assert!(
                preview(
                    input("db-main", Some(password)),
                    "db-main-sqlserver",
                    "203.0.113.10",
                    &HashSet::new(),
                    &HashSet::new()
                )
                .is_err(),
                "expected {password:?} to be rejected"
            );
        }
    }

    #[test]
    fn preview_rejects_taken_ports_and_undersized_memory() {
        let mut taken = input("db-main", Some("Passw0rd!secure"));
        taken.public_host_port = Some(14330);
        assert!(
            preview(
                taken,
                "db-main-sqlserver",
                "203.0.113.10",
                &HashSet::new(),
                &HashSet::from([14330])
            )
            .is_err()
        );
        let mut undersized = input("db-main", Some("Passw0rd!secure"));
        undersized.runtime_memory_mib = Some(1024);
        assert!(
            preview(
                undersized,
                "db-main-sqlserver",
                "203.0.113.10",
                &HashSet::new(),
                &HashSet::new()
            )
            .is_err()
        );
    }

    #[test]
    fn generated_passwords_satisfy_the_sa_policy() {
        for _ in 0..50 {
            let password = generate_password();
            assert_eq!(password.chars().count(), GENERATED_PASSWORD_LENGTH);
            validate_password(&password).unwrap();
        }
    }

    #[test]
    fn persisted_compose_reads_back_the_rendered_image_and_volume() {
        let rendered = compose(
            "mcr.microsoft.com/mssql/server:2025-latest",
            "db-main-data",
            "db-ab12",
        );
        let (image, volume) = persisted_compose(&rendered).unwrap();
        assert_eq!(image, "mcr.microsoft.com/mssql/server:2025-latest");
        assert_eq!(volume, "db-main-data");
    }
}
