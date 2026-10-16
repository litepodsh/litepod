use std::collections::{BTreeMap, HashSet};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use futures_util::StreamExt;
use podman_api::{Podman, conn::TtyChunk, opts::ExecCreateOpts};
use rand::RngExt;
use uuid::Uuid;

use crate::{
    cqrs::commands::project as project_cmd,
    cqrs::queries::project as project_query,
    deployment::compose::store::PortMapping,
    deployment::{self, compose::store::ComposeFileStore, source},
    dtos::{
        project::CreateApplicationRequest,
        response::AppError,
        template::{
            PostgresResourceDetailDto, PostgresTemplateConfigUpdate, PostgresTemplateCreatedDto,
            PostgresTemplateInput, PostgresTemplatePreviewDto, TemplateVariableDto,
            TemplateVolumeDto,
        },
    },
    state::state::SharedState,
};

const DEFAULT_IMAGE: &str = "docker.io/library/postgres:17.2";
const DEFAULT_DATABASE: &str = "postgres";
const DEFAULT_USER: &str = "postgres";
const SERVICE_NAME: &str = "postgres";
const POSTGRES_DATA_PATH: &str = "/var/lib/postgresql/data";

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
    let mut name = application_name
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
    name = name.trim_matches('-').to_string();
    if name.is_empty() {
        "postgres-data".into()
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

fn compose(image: &str, volume: &str, application_id: &str) -> String {
    format!(
        "services:\n  {SERVICE_NAME}:\n    image: {image}\n    environment:\n      POSTGRES_DB: ${{POSTGRES_DB}}\n      POSTGRES_USER: ${{POSTGRES_USER}}\n      POSTGRES_PASSWORD: ${{POSTGRES_PASSWORD}}\n    volumes:\n      - {volume}:{POSTGRES_DATA_PATH}\n{service_networks}volumes:\n  {volume}: {{}}\n{networks}",
        service_networks = crate::templates::resource_service_networks(application_id),
        networks = crate::templates::resource_network_compose(application_id),
    )
}

fn persisted_compose(content: &str) -> Result<(String, String), AppError> {
    let compose: PersistedCompose = serde_yaml::from_str(content).map_err(|_| {
        AppError::ValidationError("PostgreSQL Compose configuration is invalid".into())
    })?;
    let service = compose.services.get(SERVICE_NAME).ok_or_else(|| {
        AppError::ValidationError("PostgreSQL service is missing from Compose".into())
    })?;
    let volume = service
        .volumes
        .iter()
        .find_map(|mount| mount.strip_suffix(&format!(":{POSTGRES_DATA_PATH}")))
        .filter(|name| !name.is_empty())
        .ok_or_else(|| AppError::ValidationError("PostgreSQL data volume is missing".into()))?;
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
    if application.template_kind.as_deref() != Some("postgres") {
        return Err(AppError::ResourceNotFound);
    }
    let store = ComposeFileStore::new(&state.data_dir, application_id);
    let (docker_image, volume_name) = persisted_compose(
        &store
            .read_content()
            .await
            .map_err(AppError::InternalError)?,
    )?;
    let variables = deployment::decrypted_environment(state, application_id, "runtime").await?;
    let values = variables.into_iter().collect::<BTreeMap<_, _>>();
    let database_name = values
        .get("POSTGRES_DB")
        .cloned()
        .ok_or_else(|| AppError::ValidationError("POSTGRES_DB is missing".into()))?;
    let database_user = values
        .get("POSTGRES_USER")
        .cloned()
        .ok_or_else(|| AppError::ValidationError("POSTGRES_USER is missing".into()))?;
    let password = values
        .get("POSTGRES_PASSWORD")
        .cloned()
        .ok_or_else(|| AppError::ValidationError("POSTGRES_PASSWORD is missing".into()))?;
    Ok((
        application,
        docker_image,
        volume_name,
        database_name,
        database_user,
        password,
    ))
}

pub async fn resource_detail(
    state: &SharedState,
    organization_id: &str,
    project_id: &str,
    environment_id: &str,
    application_id: &str,
    public_host: &str,
) -> Result<PostgresResourceDetailDto, AppError> {
    let (application, docker_image, volume_name, database_name, database_user, password) =
        resource_values(
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
        .find(|mapping| mapping.service == SERVICE_NAME && mapping.container_port == 5432)
        .map(|mapping| mapping.host_port);
    let deployment_status =
        crate::templates::live_deployment_status(state, application_id, deployment_state.status)
            .await;
    let running = deployment_status == "running";
    let private_connection_string = Some(connection_string(
        &database_user,
        &password,
        &private_alias,
        5432,
        &database_name,
    ));
    let public_connection_string = running
        .then(|| {
            public_host_port.map(|port| {
                connection_string(&database_user, &password, public_host, port, &database_name)
            })
        })
        .flatten();
    Ok(PostgresResourceDetailDto {
        application,
        docker_image,
        database_name,
        database_user,
        volume: TemplateVolumeDto {
            name: volume_name,
            mount_path: POSTGRES_DATA_PATH.into(),
            service: SERVICE_NAME.into(),
        },
        private_alias,
        internal_port: 5432,
        public_host_port,
        deployment_status,
        config_stale: deployment_state.config_stale,
        password: Some(password),
        private_connection_string,
        public_connection_string,
        compose: store
            .read_content()
            .await
            .map_err(AppError::InternalError)?,
        variables: vec![
            TemplateVariableDto {
                name: "POSTGRES_DB".into(),
                masked: false,
            },
            TemplateVariableDto {
                name: "POSTGRES_USER".into(),
                masked: false,
            },
            TemplateVariableDto {
                name: "POSTGRES_PASSWORD".into(),
                masked: true,
            },
        ],
        runtime_cpu_cores: source_config.build_cpu_cores,
        runtime_memory_mib: source_config.build_memory_mib,
    })
}

pub async fn update_resource(
    state: &SharedState,
    organization_id: &str,
    project_id: &str,
    environment_id: &str,
    application_id: &str,
    public_host: &str,
    input: PostgresTemplateConfigUpdate,
) -> Result<PostgresResourceDetailDto, AppError> {
    let (
        application,
        _,
        volume_name,
        current_database_name,
        current_database_user,
        current_password,
    ) = resource_values(
        state,
        organization_id,
        project_id,
        environment_id,
        application_id,
    )
    .await?;
    let deployment_state = deployment::deployment(state, application_id).await?;
    let credentials_changed =
        input.database_name.is_some() || input.database_user.is_some() || input.password.is_some();
    if credentials_changed && deployment_state.status != "not_deployed" {
        return Err(AppError::ValidationError(
            "Database credentials can only be edited before the first PostgreSQL deployment".into(),
        ));
    }
    let database_name = input.database_name.unwrap_or(current_database_name);
    let database_user = input.database_user.unwrap_or(current_database_user);
    let password = input.password.unwrap_or(current_password);
    let store = ComposeFileStore::new(&state.data_dir, application_id);
    let current_mapping = store
        .read_port_mappings()
        .await
        .map_err(AppError::InternalError)?
        .into_iter()
        .find(|mapping| mapping.service == SERVICE_NAME && mapping.container_port == 5432)
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
        PostgresTemplateInput {
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
            database_name: Some(database_name),
            database_user: Some(database_user),
            password: Some(password),
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
            &format!(
                "POSTGRES_DB={}\nPOSTGRES_USER={}\nPOSTGRES_PASSWORD={}",
                preview.database_name, preview.database_user, preview.resolved_password
            ),
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

pub async fn rotate_password(
    state: &SharedState,
    organization_id: &str,
    project_id: &str,
    environment_id: &str,
    application_id: &str,
    public_host: &str,
    requested_password: Option<String>,
) -> Result<PostgresResourceDetailDto, AppError> {
    let (_, _, _, database_name, database_user, _) = resource_values(
        state,
        organization_id,
        project_id,
        environment_id,
        application_id,
    )
    .await?;
    if deployment::deployment(state, application_id).await?.status != "running" {
        return Err(AppError::ValidationError(
            "Deploy PostgreSQL before rotating its password".into(),
        ));
    }
    let password = requested_password
        .filter(|value| !value.is_empty())
        .unwrap_or_else(generate_password);
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
            "PostgreSQL service is not running".into(),
        ));
    }
    let sql = format!(
        "ALTER ROLE \"{database_user}\" PASSWORD '{}';",
        password.replace('\'', "''")
    );
    let podman = Podman::new(&state.podman_socket).map_err(|error| {
        AppError::InternalError(format!("Could not connect to Podman: {error}"))
    })?;
    let exec = podman
        .containers()
        .get(&container_name)
        .create_exec(
            &ExecCreateOpts::builder()
                .command([
                    "psql",
                    "-v",
                    "ON_ERROR_STOP=1",
                    "-U",
                    &database_user,
                    "-d",
                    &database_name,
                    "-c",
                    &sql,
                ])
                .attach_stdout(true)
                .attach_stderr(true)
                .build(),
        )
        .await
        .map_err(|error| {
            AppError::InternalError(format!(
                "Could not start PostgreSQL password rotation: {error}"
            ))
        })?;
    let mut output = exec
        .start(&Default::default())
        .await
        .map_err(|error| {
            AppError::InternalError(format!(
                "Could not run PostgreSQL password rotation: {error}"
            ))
        })?
        .ok_or_else(|| {
            AppError::InternalError("PostgreSQL password rotation produced no output".into())
        })?;
    while let Some(chunk) = output.next().await {
        match chunk {
            Ok(TtyChunk::StdErr(stderr)) => {
                return Err(AppError::ValidationError(format!(
                    "PostgreSQL did not accept the password change: {}",
                    String::from_utf8_lossy(&stderr).trim()
                )));
            }
            Ok(_) => {}
            Err(error) => return Err(AppError::InternalError(error.to_string())),
        }
    }
    deployment::save_environment(
        state,
        application_id,
        "runtime",
        &format!(
            "POSTGRES_DB={database_name}\nPOSTGRES_USER={database_user}\nPOSTGRES_PASSWORD={password}"
        ),
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

fn connection_string(user: &str, password: &str, host: &str, port: u16, database: &str) -> String {
    format!(
        "postgresql://{}:{}@{host}:{port}/{}",
        urlencoding::encode(user),
        urlencoding::encode(password),
        urlencoding::encode(database),
    )
}

pub fn preview(
    input: PostgresTemplateInput,
    private_alias: &str,
    public_host: &str,
    reserved_ports: &HashSet<u16>,
    used_ports: &HashSet<u16>,
) -> Result<PostgresTemplatePreviewDto, AppError> {
    let application_name = required(Some(&input.application_name), "", "Application name")?;
    let image = required(input.docker_image.as_deref(), DEFAULT_IMAGE, "Docker image")?;
    validate_image(&image)?;
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
    let password = input
        .password
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(generate_password);
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
            if public_host.trim().is_empty() {
                return Err(AppError::ValidationError(
                    "Configure a public server host before publishing PostgreSQL".into(),
                ));
            }
            if reserved_ports.contains(&port) || used_ports.contains(&port) {
                return Err(AppError::ValidationError(format!(
                    "Host port {port} is already in use or reserved"
                )));
            }
            Some(connection_string(
                &database_user,
                &password,
                public_host.trim(),
                port,
                &database_name,
            ))
        }
        None => None,
    };

    Ok(PostgresTemplatePreviewDto {
        template_instance_id: template_instance_id.clone(),
        resolved_password: password.clone(),
        database_name: database_name.clone(),
        database_user: database_user.clone(),
        docker_image: image.clone(),
        private_alias: private_alias.to_string(),
        private_connection_string: connection_string(
            &database_user,
            &password,
            private_alias,
            5432,
            &database_name,
        ),
        public_connection_string,
        compose: compose(&image, &volume_name, &template_instance_id),
        port_mappings: input
            .public_host_port
            .map(|host_port| {
                vec![PortMapping {
                    service: SERVICE_NAME.into(),
                    host_port,
                    container_port: 5432,
                    protocol: "tcp".into(),
                }]
            })
            .unwrap_or_default(),
        variables: vec![
            TemplateVariableDto {
                name: "POSTGRES_DB".into(),
                masked: false,
            },
            TemplateVariableDto {
                name: "POSTGRES_USER".into(),
                masked: false,
            },
            TemplateVariableDto {
                name: "POSTGRES_PASSWORD".into(),
                masked: true,
            },
        ],
        volumes: vec![TemplateVolumeDto {
            name: volume_name,
            mount_path: POSTGRES_DATA_PATH.into(),
            service: SERVICE_NAME.into(),
        }],
    })
}

pub async fn create(
    state: &SharedState,
    organization_id: &str,
    project_id: &str,
    environment_id: &str,
    project_name: &str,
    environment_name: &str,
    public_host: &str,
    mut input: PostgresTemplateInput,
) -> Result<PostgresTemplateCreatedDto, AppError> {
    let instance_id = input
        .template_instance_id
        .clone()
        .ok_or_else(|| AppError::ValidationError("Preview PostgreSQL before creating it".into()))?;
    let runtime_name = deployment::deployment_identity(
        project_name,
        environment_name,
        &input.application_name,
        &instance_id,
        &state.public_ip,
    )
    .0;
    let private_alias = format!("{runtime_name}-{SERVICE_NAME}");
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
        &private_alias,
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
        Some("postgres"),
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
            &format!(
                "POSTGRES_DB={}\nPOSTGRES_USER={}\nPOSTGRES_PASSWORD={}",
                preview.database_name, preview.database_user, preview.resolved_password
            ),
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
    Ok(PostgresTemplateCreatedDto {
        application,
        preview,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    impl PostgresTemplateInput {
        fn for_test(application_name: &str) -> Self {
            Self {
                application_name: application_name.into(),
                server_id: "local".into(),
                description: None,
                docker_image: None,
                database_name: None,
                database_user: None,
                password: Some("safe-password".into()),
                volume_name: None,
                public_host_port: None,
                runtime_cpu_cores: None,
                runtime_memory_mib: None,
                template_instance_id: None,
            }
        }
    }

    #[test]
    fn preview_uses_postgres_defaults_and_private_alias() {
        let preview = preview(
            PostgresTemplateInput::for_test("postgres-main"),
            "postgres-main-postgres",
            "203.0.113.10",
            &HashSet::new(),
            &HashSet::new(),
        )
        .unwrap();

        assert!(
            preview
                .compose
                .contains("image: docker.io/library/postgres:17.2")
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
        assert!(preview.compose.contains("POSTGRES_DB: ${POSTGRES_DB}"));
        assert_eq!(preview.variables[0].name, "POSTGRES_DB");
        assert_eq!(preview.database_name, "postgres");
        assert_eq!(preview.public_connection_string, None);
        assert!(preview.private_connection_string.contains("postgres-main"));
    }

    #[test]
    fn preview_rejects_taken_public_port_and_bad_image_reference() {
        let mut input = PostgresTemplateInput::for_test("postgres-main");
        input.public_host_port = Some(25432);
        assert!(
            preview(
                input,
                "postgres-main-postgres",
                "203.0.113.10",
                &HashSet::new(),
                &HashSet::from([25432]),
            )
            .is_err()
        );

        let mut input = PostgresTemplateInput::for_test("postgres-main");
        input.docker_image = Some("postgres image".into());
        assert!(
            preview(
                input,
                "postgres-main-postgres",
                "203.0.113.10",
                &HashSet::new(),
                &HashSet::new(),
            )
            .is_err()
        );
    }

    #[test]
    fn preview_preserves_custom_image_and_url_encodes_passwords() {
        let mut input = PostgresTemplateInput::for_test("postgres-main");
        input.docker_image = Some("supabase/postgres:17.2.3".into());
        input.password = Some("pa:ss/@word".into());
        input.public_host_port = Some(25432);

        let preview = preview(
            input,
            "store-prod-postgres-main-a3f91c-postgres",
            "db.example.com",
            &HashSet::new(),
            &HashSet::new(),
        )
        .unwrap();

        assert!(preview.compose.contains("image: supabase/postgres:17.2.3"));
        assert_eq!(
            preview.public_connection_string.as_deref(),
            Some("postgresql://postgres:pa%3Ass%2F%40word@db.example.com:25432/postgres")
        );
        assert!(
            preview
                .private_connection_string
                .contains("store-prod-postgres-main-a3f91c-postgres:5432")
        );
    }
}
