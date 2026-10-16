use std::{
    collections::{BTreeMap, HashSet},
    fmt::Write as _,
};

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
            MariaDbPasswordRotationRequest, MariaDbResourceDetailDto, MariaDbTemplateConfigUpdate,
            MariaDbTemplateCreatedDto, MariaDbTemplateInput, MariaDbTemplatePreviewDto,
            TemplateVariableDto, TemplateVolumeDto,
        },
    },
    state::state::SharedState,
};

const DEFAULT_IMAGE: &str = "docker.io/library/mariadb:11.8";
const DEFAULT_DATABASE: &str = "mariadb";
const DEFAULT_USER: &str = "mariadb";
const SERVICE_NAME: &str = "mariadb";
const DATA_PATH: &str = "/var/lib/mysql";
const PORT: u16 = 3306;

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

fn validate_identifier(value: &str, label: &str) -> Result<(), AppError> {
    let valid = value.len() <= 63
        && value
            .chars()
            .enumerate()
            .all(|(index, character)| match character {
                'a'..='z' | 'A'..='Z' | '_' => true,
                '0'..='9' => index > 0,
                _ => false,
            });
    if !valid {
        return Err(AppError::ValidationError(format!(
            "{label} must start with a letter or underscore and contain only letters, digits, or underscores"
        )));
    }
    Ok(())
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
        "mariadb-data".into()
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

fn generate_password() -> String {
    URL_SAFE_NO_PAD.encode(rand::rng().random::<[u8; 24]>())
}

fn validate_password(value: &str) -> Result<(), AppError> {
    if value.trim() != value || value.contains('\n') || value.contains('\r') {
        return Err(AppError::ValidationError(
            "MariaDB passwords must not contain line breaks or surrounding whitespace".into(),
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
        "services:\n  {SERVICE_NAME}:\n    image: {image}\n    environment:\n      MARIADB_DATABASE: ${{MARIADB_DATABASE}}\n      MARIADB_USER: ${{MARIADB_USER}}\n      MARIADB_PASSWORD: ${{MARIADB_PASSWORD}}\n      MARIADB_ROOT_PASSWORD: ${{MARIADB_ROOT_PASSWORD}}\n      MARIADB_ROOT_HOST: \"%\"\n    volumes:\n      - {volume}:{DATA_PATH}\n{service_networks}volumes:\n  {volume}: {{}}\n{networks}",
        service_networks = crate::templates::resource_service_networks(application_id),
        networks = crate::templates::resource_network_compose(application_id),
    )
}

fn connection_string(user: &str, password: &str, host: &str, port: u16, database: &str) -> String {
    format!(
        "mariadb://{}:{}@{host}:{port}/{}",
        urlencoding::encode(user),
        urlencoding::encode(password),
        urlencoding::encode(database),
    )
}

fn persisted_compose(content: &str) -> Result<(String, String), AppError> {
    let compose: PersistedCompose = serde_yaml::from_str(content).map_err(|_| {
        AppError::ValidationError("MariaDB Compose configuration is invalid".into())
    })?;
    let service = compose.services.get(SERVICE_NAME).ok_or_else(|| {
        AppError::ValidationError("MariaDB service is missing from Compose".into())
    })?;
    let volume = service
        .volumes
        .iter()
        .find_map(|mount| mount.strip_suffix(&format!(":{DATA_PATH}")))
        .filter(|name| !name.is_empty())
        .ok_or_else(|| AppError::ValidationError("MariaDB data volume is missing".into()))?;
    Ok((service.image.clone(), volume.to_string()))
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
        String,
        String,
        String,
        String,
        String,
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
    if application.template_kind.as_deref() != Some("mariadb") {
        return Err(AppError::ResourceNotFound);
    }
    let store = ComposeFileStore::new(&state.data_dir, application_id);
    let (docker_image, volume_name) = persisted_compose(
        &store
            .read_content()
            .await
            .map_err(AppError::InternalError)?,
    )?;
    let values = deployment::decrypted_environment(state, application_id, "runtime")
        .await?
        .into_iter()
        .collect::<BTreeMap<_, _>>();
    let value = |name: &str| {
        values
            .get(name)
            .cloned()
            .ok_or_else(|| AppError::ValidationError(format!("{name} is missing")))
    };
    Ok((
        application,
        docker_image,
        volume_name,
        value("MARIADB_DATABASE")?,
        value("MARIADB_USER")?,
        value("MARIADB_PASSWORD")?,
        value("MARIADB_ROOT_PASSWORD")?,
    ))
}

fn variables() -> Vec<TemplateVariableDto> {
    vec![
        TemplateVariableDto {
            name: "MARIADB_DATABASE".into(),
            masked: false,
        },
        TemplateVariableDto {
            name: "MARIADB_USER".into(),
            masked: false,
        },
        TemplateVariableDto {
            name: "MARIADB_PASSWORD".into(),
            masked: true,
        },
        TemplateVariableDto {
            name: "MARIADB_ROOT_PASSWORD".into(),
            masked: true,
        },
    ]
}

pub fn preview(
    input: MariaDbTemplateInput,
    private_alias: &str,
    public_host: &str,
    reserved_ports: &HashSet<u16>,
    used_ports: &HashSet<u16>,
) -> Result<MariaDbTemplatePreviewDto, AppError> {
    let application_name = required(Some(&input.application_name), "", "Application name")?;
    let docker_image = required(input.docker_image.as_deref(), DEFAULT_IMAGE, "Docker image")?;
    validate_image(&docker_image)?;
    let database_name = required(
        input.database_name.as_deref(),
        DEFAULT_DATABASE,
        "Database name",
    )?;
    validate_identifier(&database_name, "Database name")?;
    let database_user = required(
        input.database_user.as_deref(),
        DEFAULT_USER,
        "Database user",
    )?;
    validate_identifier(&database_user, "Database user")?;
    let password = resolve_password(input.password.as_deref())?;
    let root_password = resolve_password(input.root_password.as_deref())?;
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
    let public_strings = match input.public_host_port {
        Some(port) => {
            if port == 0 {
                return Err(AppError::ValidationError(
                    "Public host port must be between 1 and 65535".into(),
                ));
            }
            if public_host.trim().is_empty() {
                return Err(AppError::ValidationError(
                    "Configure a public server host before publishing MariaDB".into(),
                ));
            }
            if reserved_ports.contains(&port) || used_ports.contains(&port) {
                return Err(AppError::ValidationError(format!(
                    "Host port {port} is already in use or reserved"
                )));
            }
            Some((
                connection_string(
                    &database_user,
                    &password,
                    public_host.trim(),
                    port,
                    &database_name,
                ),
                connection_string(
                    "root",
                    &root_password,
                    public_host.trim(),
                    port,
                    &database_name,
                ),
            ))
        }
        None => None,
    };
    Ok(MariaDbTemplatePreviewDto {
        template_instance_id: template_instance_id.clone(),
        resolved_password: password.clone(),
        resolved_root_password: root_password.clone(),
        database_name: database_name.clone(),
        database_user: database_user.clone(),
        docker_image: docker_image.clone(),
        private_alias: private_alias.to_string(),
        private_connection_string: connection_string(
            &database_user,
            &password,
            private_alias,
            PORT,
            &database_name,
        ),
        private_root_connection_string: connection_string(
            "root",
            &root_password,
            private_alias,
            PORT,
            &database_name,
        ),
        public_connection_string: public_strings.as_ref().map(|strings| strings.0.clone()),
        public_root_connection_string: public_strings.map(|strings| strings.1),
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
) -> Result<MariaDbResourceDetailDto, AppError> {
    let (
        application,
        docker_image,
        volume_name,
        database_name,
        database_user,
        password,
        root_password,
    ) = resource_values(
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
    let private_connection_string = Some(connection_string(
        &database_user,
        &password,
        &private_alias,
        PORT,
        &database_name,
    ));
    let private_root_connection_string = Some(connection_string(
        "root",
        &root_password,
        &private_alias,
        PORT,
        &database_name,
    ));
    let public_connection_string = running
        .then(|| {
            public_host_port.map(|port| {
                connection_string(&database_user, &password, public_host, port, &database_name)
            })
        })
        .flatten();
    let public_root_connection_string = running
        .then(|| {
            public_host_port.map(|port| {
                connection_string("root", &root_password, public_host, port, &database_name)
            })
        })
        .flatten();
    Ok(MariaDbResourceDetailDto {
        application,
        docker_image,
        database_name,
        database_user,
        password: Some(password),
        root_password: Some(root_password),
        volume: TemplateVolumeDto {
            name: volume_name,
            mount_path: DATA_PATH.into(),
            service: SERVICE_NAME.into(),
        },
        private_alias,
        internal_port: PORT,
        public_host_port,
        deployment_status,
        config_stale: deployment_state.config_stale,
        private_connection_string,
        private_root_connection_string,
        public_connection_string,
        public_root_connection_string,
        compose: store
            .read_content()
            .await
            .map_err(AppError::InternalError)?,
        variables: variables(),
        runtime_cpu_cores: source_config.build_cpu_cores,
        runtime_memory_mib: source_config.build_memory_mib,
    })
}

fn runtime_environment(preview: &MariaDbTemplatePreviewDto) -> String {
    format!(
        "MARIADB_DATABASE={}\nMARIADB_USER={}\nMARIADB_PASSWORD={}\nMARIADB_ROOT_PASSWORD={}",
        preview.database_name,
        preview.database_user,
        preview.resolved_password,
        preview.resolved_root_password
    )
}

pub async fn create(
    state: &SharedState,
    organization_id: &str,
    project_id: &str,
    environment_id: &str,
    project_name: &str,
    environment_name: &str,
    public_host: &str,
    mut input: MariaDbTemplateInput,
) -> Result<MariaDbTemplateCreatedDto, AppError> {
    let instance_id = input
        .template_instance_id
        .clone()
        .ok_or_else(|| AppError::ValidationError("Preview MariaDB before creating it".into()))?;
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
        Some("mariadb"),
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
    Ok(MariaDbTemplateCreatedDto {
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
    input: MariaDbTemplateConfigUpdate,
) -> Result<MariaDbResourceDetailDto, AppError> {
    source::validate_runtime_limits(input.runtime_cpu_cores, input.runtime_memory_mib)?;
    let (
        application,
        _,
        volume_name,
        current_database_name,
        current_database_user,
        current_password,
        current_root_password,
    ) = resource_values(
        state,
        organization_id,
        project_id,
        environment_id,
        application_id,
    )
    .await?;
    let deployment_state = deployment::deployment(state, application_id).await?;
    let credentials_changed = input.database_name.is_some()
        || input.database_user.is_some()
        || input.password.is_some()
        || input.root_password.is_some();
    if credentials_changed && deployment_state.status != "not_deployed" {
        return Err(AppError::ValidationError(
            "MariaDB credentials can only be edited before the first MariaDB deployment".into(),
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
        MariaDbTemplateInput {
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
            database_name: Some(input.database_name.unwrap_or(current_database_name)),
            database_user: Some(input.database_user.unwrap_or(current_database_user)),
            password: Some(input.password.unwrap_or(current_password)),
            root_password: Some(input.root_password.unwrap_or(current_root_password)),
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
    if credentials_changed {
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
    name: &str,
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
    sqlx::query("INSERT INTO application_environment_variables (application_id, scope, name, encrypted_value, updated_at) VALUES (?1, 'runtime', ?2, ?3, ?4) ON CONFLICT(application_id, scope, name) DO UPDATE SET encrypted_value = excluded.encrypted_value, updated_at = excluded.updated_at").bind(application_id).bind(name).bind(encrypted).bind(now).execute(&conn).await.map_err(|error| AppError::DatabaseError(error.to_string()))?;
    Ok(())
}

fn sql_hex_literal(value: &str) -> String {
    let mut literal = String::with_capacity(value.len() * 2 + 3);
    literal.push_str("X'");
    for byte in value.as_bytes() {
        let _ = write!(literal, "{byte:02x}");
    }
    literal.push('\'');
    literal
}

fn password_rotation_sql(user: &str, password: &str, rotate_localhost: bool) -> String {
    let password = sql_hex_literal(password);
    if rotate_localhost {
        format!(
            "ALTER USER 'root'@'localhost' IDENTIFIED BY {password}; ALTER USER IF EXISTS 'root'@'%' IDENTIFIED BY {password};"
        )
    } else {
        format!("ALTER USER '{user}'@'%' IDENTIFIED BY {password};")
    }
}

async fn execute_password_change(
    podman: &Podman,
    container_name: &str,
    current_root_password: &str,
    user: &str,
    password: &str,
    rotate_localhost: bool,
) -> Result<(), AppError> {
    let sql = password_rotation_sql(user, password, rotate_localhost);
    let root_password_argument = format!("-p{current_root_password}");
    let exec = podman
        .containers()
        .get(container_name)
        .create_exec(
            &ExecCreateOpts::builder()
                .command(["mariadb", "-uroot", &root_password_argument, "-e", &sql])
                .attach_stdout(true)
                .attach_stderr(true)
                .build(),
        )
        .await
        .map_err(|error| {
            AppError::InternalError(format!(
                "Could not start MariaDB password rotation: {error}"
            ))
        })?;
    let mut output = exec
        .start(&Default::default())
        .await
        .map_err(|error| {
            AppError::InternalError(format!("Could not run MariaDB password rotation: {error}"))
        })?
        .ok_or_else(|| {
            AppError::InternalError("MariaDB password rotation produced no output".into())
        })?;
    let mut logs = Vec::new();
    while let Some(chunk) = output.next().await {
        match chunk {
            Ok(TtyChunk::StdOut(bytes)) | Ok(TtyChunk::StdErr(bytes)) => logs.extend(bytes),
            Ok(_) => {}
            Err(error) => return Err(AppError::InternalError(error.to_string())),
        }
    }
    let exit_code = exec
        .inspect()
        .await
        .ok()
        .and_then(|value| value.get("ExitCode").and_then(|code| code.as_i64()));
    if exit_code != Some(0) {
        return Err(AppError::ValidationError(format!(
            "MariaDB did not accept the password change: {}",
            String::from_utf8_lossy(&logs).trim()
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
    request: MariaDbPasswordRotationRequest,
) -> Result<MariaDbResourceDetailDto, AppError> {
    let (_, _, _, _, database_user, current_password, current_root_password) = resource_values(
        state,
        organization_id,
        project_id,
        environment_id,
        application_id,
    )
    .await?;
    if deployment::deployment(state, application_id).await?.status != "running" {
        return Err(AppError::ValidationError(
            "Deploy MariaDB before rotating its password".into(),
        ));
    }
    let (variable, user, previous_password, password, rotate_localhost) =
        match request.credential.as_str() {
            "database" => (
                "MARIADB_PASSWORD",
                database_user.as_str(),
                current_password.as_str(),
                resolve_password(request.password.as_deref())?,
                false,
            ),
            "root" => (
                "MARIADB_ROOT_PASSWORD",
                "root",
                current_root_password.as_str(),
                resolve_password(request.password.as_deref())?,
                true,
            ),
            _ => {
                return Err(AppError::ValidationError(
                    "MariaDB credential must be database or root".into(),
                ));
            }
        };
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
            "MariaDB service is not running".into(),
        ));
    }
    let podman = Podman::new(&state.podman_socket).map_err(|error| {
        AppError::InternalError(format!("Could not connect to Podman: {error}"))
    })?;
    execute_password_change(
        &podman,
        &container_name,
        &current_root_password,
        user,
        &password,
        rotate_localhost,
    )
    .await?;
    if let Err(persistence_error) =
        save_rotated_runtime_variable(state, application_id, variable, &password).await
    {
        let rollback_authentication = if rotate_localhost {
            password.as_str()
        } else {
            current_root_password.as_str()
        };
        return match execute_password_change(
            &podman,
            &container_name,
            rollback_authentication,
            user,
            previous_password,
            rotate_localhost,
        )
        .await
        {
            Ok(()) => Err(AppError::InternalError(format!(
                "Could not persist the rotated MariaDB password; the live password change was rolled back: {persistence_error}"
            ))),
            Err(rollback_error) => Err(AppError::InternalError(format!(
                "Could not persist the rotated MariaDB password and could not roll back the live password change: {persistence_error}; rollback error: {rollback_error}"
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
    use std::collections::HashSet;

    fn input(application_name: &str, password: &str, root_password: &str) -> MariaDbTemplateInput {
        MariaDbTemplateInput {
            application_name: application_name.into(),
            server_id: "local".into(),
            description: None,
            docker_image: None,
            database_name: None,
            database_user: None,
            password: Some(password.into()),
            root_password: Some(root_password.into()),
            volume_name: None,
            public_host_port: None,
            runtime_cpu_cores: None,
            runtime_memory_mib: None,
            template_instance_id: None,
        }
    }

    #[test]
    fn preview_renders_mariadb_credentials_volume_and_urls() {
        let preview = preview(
            input("maria-main", "pa:ss/@word", "root-secret"),
            "store-prod-maria-main-a3f91c-mariadb",
            "db.example.com",
            &HashSet::new(),
            &HashSet::new(),
        )
        .unwrap();
        assert!(
            preview
                .compose
                .contains("image: docker.io/library/mariadb:11.8")
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
        assert!(
            preview
                .compose
                .contains("MARIADB_ROOT_PASSWORD: ${MARIADB_ROOT_PASSWORD}")
        );
        assert!(preview.compose.contains("MARIADB_ROOT_HOST: \"%\""));
        assert!(preview.compose.contains("/var/lib/mysql"));
        assert_eq!(
            preview.private_connection_string,
            "mariadb://mariadb:pa%3Ass%2F%40word@store-prod-maria-main-a3f91c-mariadb:3306/mariadb"
        );
        assert!(
            preview
                .private_root_connection_string
                .contains("root-secret")
        );
    }

    #[test]
    fn preview_rejects_invalid_identifier_and_taken_port() {
        let mut bad = input("maria-main", "safe", "root");
        bad.database_user = Some("bad-user".into());
        assert!(
            preview(
                bad,
                "maria",
                "203.0.113.10",
                &HashSet::new(),
                &HashSet::new()
            )
            .is_err()
        );
        let mut public = input("maria-main", "safe", "root");
        public.public_host_port = Some(23306);
        assert!(
            preview(
                public,
                "maria",
                "203.0.113.10",
                &HashSet::new(),
                &HashSet::from([23306])
            )
            .is_err()
        );

        let mut zero = input("maria-main", "safe", "root");
        zero.public_host_port = Some(0);
        assert!(
            preview(
                zero,
                "maria",
                "203.0.113.10",
                &HashSet::new(),
                &HashSet::new()
            )
            .is_err()
        );
    }

    #[test]
    fn preview_rejects_passwords_that_cannot_be_one_runtime_value() {
        for password in [" leading", "trailing ", "safe\nMARIADB_PASSWORD=injected"] {
            assert!(
                preview(
                    input("maria-main", password, "root"),
                    "maria",
                    "203.0.113.10",
                    &HashSet::new(),
                    &HashSet::new()
                )
                .is_err()
            );
        }
        assert!(
            preview(
                input("maria-main", "safe", "root\rpassword"),
                "maria",
                "203.0.113.10",
                &HashSet::new(),
                &HashSet::new()
            )
            .is_err()
        );
    }

    #[test]
    fn rotation_sql_uses_hex_literals_and_updates_both_root_accounts() {
        let password = "pa'ss\\word-å";
        let literal = sql_hex_literal(password);
        assert_eq!(literal, "X'70612773735c776f72642dc3a5'");
        assert_eq!(
            password_rotation_sql("mariadb", password, false),
            format!("ALTER USER 'mariadb'@'%' IDENTIFIED BY {literal};")
        );
        assert_eq!(
            password_rotation_sql("root", password, false),
            format!("ALTER USER 'root'@'%' IDENTIFIED BY {literal};")
        );
        let root_sql = password_rotation_sql("root", password, true);
        assert!(root_sql.contains(&format!(
            "ALTER USER 'root'@'localhost' IDENTIFIED BY {literal};"
        )));
        assert!(root_sql.contains(&format!(
            "ALTER USER IF EXISTS 'root'@'%' IDENTIFIED BY {literal};"
        )));
    }
}
