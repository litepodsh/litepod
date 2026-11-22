//! Dedicated build servers. A Git application can build its image on a build-only
//! server, push it to its registry repository tagged with the commit, and have the
//! server that runs it pull that exact digest. A failed build or push returns
//! before the rollout, so the running deployment keeps serving. Registry logins
//! come from the organization's saved credentials (`servers::registries`).

use std::{
    collections::HashMap,
    ffi::OsString,
    path::Path,
    sync::{Arc, LazyLock, Mutex},
    time::Duration,
};

use base64::Engine;
use chrono::Utc;
use sqlx::Row;

use super::{image_tag, remote_docker_build, remote_railpack_build, stream_podman_step};
use crate::{
    dtos::{
        deployment::{BuildServerDto, SourceConfigDto, UpdateBuildServerRequest},
        response::AppError,
    },
    servers::{
        model::ServerState, registries, repository::ServerRecord, runtime::ServerRuntime,
        setup::ServerFacts,
    },
    state::state::SharedState,
};

pub(crate) struct BuildTarget {
    builder: ServerRecord,
    /// `Some` delivers through this registry repository; `None` copies the image across.
    repository: Option<String>,
    credentials: Option<(String, String)>,
}

/// What a deploy needs to know about a server to tell whether an image built on
/// one runs on another, and whether a build fits on it.
#[derive(Debug)]
struct Machine {
    name: String,
    arch: Option<String>,
    cpu_cores: Option<f64>,
    memory_bytes: Option<u64>,
}

impl Machine {
    fn of(server: &ServerRecord) -> Self {
        let facts = serde_json::from_str::<ServerFacts>(&server.facts_json).unwrap_or_default();
        Self {
            name: server.name.clone(),
            // The local row is never probed over SSH; it is the host litepod runs on.
            arch: if server.kind == "local" {
                Some(std::env::consts::ARCH.to_string())
            } else {
                facts.architecture
            },
            cpu_cores: facts.cpu_cores,
            memory_bytes: facts.memory_bytes,
        }
    }
}

/// `uname -m` names one CPU two ways: FreeBSD says `amd64`/`arm64`, Linux `x86_64`/`aarch64`.
fn normalized_arch(arch: &str) -> &str {
    match arch.trim() {
        "amd64" | "x86_64" => "x86_64",
        "arm64" | "aarch64" => "aarch64",
        other => other,
    }
}

/// Refuses a build that cannot work, before anything is cloned or built. An image
/// built for another CPU only fails once it starts (`exec format error`), and a build
/// limit above the builder's size is never granted. Unknown facts are not guessed at.
fn check_compatible(
    builder: &Machine,
    deploy: &Machine,
    build_cpu_cores: Option<f64>,
    build_memory_mib: Option<i64>,
) -> Result<(), String> {
    if let (Some(built), Some(runs)) = (&builder.arch, &deploy.arch)
        && normalized_arch(built) != normalized_arch(runs)
    {
        return Err(format!(
            "Build server {} is {} but {} is {}: an image built there would not start here. Use a build server with the same architecture",
            builder.name,
            normalized_arch(built),
            deploy.name,
            normalized_arch(runs)
        ));
    }
    if let (Some(wanted), Some(has)) = (build_cpu_cores, builder.cpu_cores)
        && wanted > has
    {
        return Err(format!(
            "The build limit asks for {wanted} CPU cores but build server {} has {has}",
            builder.name
        ));
    }
    if let (Some(wanted), Some(has)) = (build_memory_mib, builder.memory_bytes)
        && u64::try_from(wanted)
            .unwrap_or(u64::MAX)
            .saturating_mul(1024 * 1024)
            > has
    {
        return Err(format!(
            "The build limit asks for {wanted} MiB of RAM but build server {} has {} MiB",
            builder.name,
            has / (1024 * 1024)
        ));
    }
    Ok(())
}

/// The servers an image from this application's build must run on: the replica being
/// deployed, else its server, else every member of its group.
async fn deploy_servers(
    state: &SharedState,
    application_id: &str,
) -> Result<Vec<ServerRecord>, AppError> {
    let ids = if let Some(server_id) = crate::servers::replica::current() {
        vec![server_id]
    } else {
        let conn = crate::git::repositories::db::get_conn(state).await;
        let row = sqlx::query("SELECT server_id, server_group_id FROM applications WHERE id = ?1")
            .bind(application_id)
            .fetch_optional(&conn)
            .await
            .map_err(db_error)?
            .ok_or(AppError::ApplicationNotFound)?;
        match row.try_get::<Option<String>, _>(0).map_err(db_error)? {
            Some(server_id) => vec![server_id],
            None => match row.try_get::<Option<String>, _>(1).map_err(db_error)? {
                Some(group_id) => {
                    return crate::servers::groups::list_members(state, &group_id).await;
                }
                None => Vec::new(),
            },
        }
    };
    let mut servers = Vec::new();
    for id in ids {
        if let Some(server) = crate::servers::repository::get(state, &id).await? {
            servers.push(server);
        }
    }
    Ok(servers)
}

async fn check_against_deploy_servers(
    state: &SharedState,
    application_id: &str,
    builder: &ServerRecord,
    source: &SourceConfigDto,
) -> Result<(), AppError> {
    let builder = Machine::of(builder);
    for server in deploy_servers(state, application_id).await? {
        check_compatible(
            &builder,
            &Machine::of(&server),
            source.build_cpu_cores,
            source.build_memory_mib,
        )
        .map_err(AppError::ValidationError)?;
    }
    Ok(())
}

fn db_error(error: sqlx::Error) -> AppError {
    AppError::DatabaseError(error.to_string())
}

fn registry_host(repository: &str) -> &str {
    repository.split('/').next().unwrap_or_default()
}

/// `registry.example.com/team/app`: a registry host (with a dot, a port or `localhost`)
/// and a lowercase repository path, no tag or digest — the deploy adds those.
fn validate_repository(repository: &str) -> Result<(), String> {
    let valid = repository.split_once('/').is_some_and(|(host, path)| {
        registries::valid_registry_host(host)
            && path.split('/').all(|segment| {
                !segment.is_empty()
                    && segment.chars().all(|c| {
                        c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-')
                    })
            })
    });
    if valid {
        Ok(())
    } else {
        Err("Image repository must look like registry.example.com/team/app: lowercase, with the registry host and no tag".into())
    }
}

/// A Podman auth file for one registry, so the password never appears in an argv.
fn auth_file_contents(repository: &str, username: &str, password: &str) -> String {
    let auth = base64::engine::general_purpose::STANDARD.encode(format!("{username}:{password}"));
    serde_json::json!({ "auths": { registry_host(repository): { "auth": auth } } }).to_string()
}

pub async fn get(state: &SharedState, application_id: &str) -> Result<BuildServerDto, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(
        "SELECT build_server_id, build_image_repository, build_delivery FROM applications WHERE id = ?1",
    )
    .bind(application_id)
    .fetch_optional(&conn)
    .await
    .map_err(db_error)?
    .ok_or(AppError::ApplicationNotFound)?;
    Ok(BuildServerDto {
        build_server_id: row.try_get(0).map_err(db_error)?,
        image_repository: row.try_get(1).map_err(db_error)?,
        delivery: row.try_get(2).map_err(db_error)?,
    })
}

/// Saves where the application builds. No `build_server_id` clears it, so the
/// application builds on the server that runs it again.
pub async fn save(
    state: &SharedState,
    organization_id: &str,
    application_id: &str,
    request: UpdateBuildServerRequest,
) -> Result<BuildServerDto, AppError> {
    let trimmed = |value: Option<String>| {
        value
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    };
    let server_id = trimmed(request.build_server_id);
    let delivery = request.delivery.unwrap_or_else(|| "transfer".into());
    if !matches!(delivery.as_str(), "transfer" | "registry") {
        return Err(AppError::ValidationError(
            "Delivery must be 'transfer' or 'registry'".into(),
        ));
    }
    // Only a registry delivery has a repository to push to.
    let repository =
        trimmed(request.image_repository).filter(|_| server_id.is_some() && delivery == "registry");
    if let Some(server_id) = &server_id {
        let source = super::super::source::get(state, application_id).await?;
        if source.deploy_type != "git" {
            return Err(AppError::ValidationError(
                "Build servers build Git applications; this one only pulls ready-made images"
                    .into(),
            ));
        }
        let server =
            crate::servers::repository::get_for_organization(state, server_id, organization_id)
                .await?
                .ok_or_else(|| {
                    AppError::ValidationError(format!("Server '{server_id}' was not found"))
                })?;
        if server.role != "build" {
            return Err(AppError::ValidationError(format!(
                "{} is not a build server. Connect a server with the build role first",
                server.name
            )));
        }
        if delivery == "registry" {
            let repository = repository.as_deref().ok_or_else(|| {
                AppError::ValidationError(
                    "Set the image repository the build server pushes to".into(),
                )
            })?;
            validate_repository(repository).map_err(AppError::ValidationError)?;
        }
        check_against_deploy_servers(state, application_id, &server, &source).await?;
    }
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE applications SET build_server_id = ?1, build_image_repository = ?2, build_delivery = ?3, updated_at = ?4 WHERE id = ?5",
    )
    .bind(server_id)
    .bind(repository)
    .bind(delivery)
    .bind(Utc::now().timestamp_millis())
    .bind(application_id)
    .execute(&conn)
    .await
    .map_err(db_error)?;
    get(state, application_id).await
}

/// The build server set for this application, `None` when it builds where it runs.
pub(crate) async fn target(
    state: &SharedState,
    application_id: &str,
) -> Result<Option<BuildTarget>, AppError> {
    let settings = get(state, application_id).await?;
    let Some(server_id) = settings.build_server_id else {
        return Ok(None);
    };
    let repository = match settings.delivery.as_str() {
        "registry" => Some(settings.image_repository.ok_or_else(|| {
            AppError::ValidationError("Set the image repository the build server pushes to".into())
        })?),
        _ => None,
    };
    let server = crate::servers::repository::get(state, &server_id)
        .await?
        .ok_or_else(|| AppError::ValidationError("The build server no longer exists".into()))?;
    if server.role != "build" {
        return Err(AppError::ValidationError(format!(
            "{} is not a build server",
            server.name
        )));
    }
    // Build servers always belong to the application's organization (checked on save).
    let organization_id = server
        .organization_id
        .clone()
        .ok_or_else(|| AppError::ValidationError("The build server has no organization".into()))?;
    let credentials = match &repository {
        Some(repository) => {
            registries::for_registry(state, &organization_id, registry_host(repository)).await?
        }
        None => None,
    };
    Ok(Some(BuildTarget {
        builder: server,
        repository,
        credentials,
    }))
}

/// The build server's runtime, for cleaning up after a build that ran there.
pub(crate) async fn builder_runtime(
    state: &SharedState,
    application_id: &str,
) -> Option<ServerRuntime> {
    let server_id = get(state, application_id).await.ok()?.build_server_id?;
    ServerRuntime::for_server(state, &server_id).await.ok()
}

/// Everything that can be known before cloning: the builder is ready, the image it
/// builds can run where the application runs, and the build fits on it. Failing here
/// costs nothing and leaves the running deployment untouched.
pub(crate) async fn prepare(
    state: &SharedState,
    application_id: &str,
    source: &SourceConfigDto,
) -> Result<Option<(BuildTarget, ServerRuntime)>, String> {
    let Some(target) = target(state, application_id)
        .await
        .map_err(|error| error.to_string())?
    else {
        return Ok(None);
    };
    if target.builder.state != ServerState::Ready {
        return Err(format!(
            "Build server {} is {}, so nothing was built and the running deployment is untouched. Repair it in Servers, then deploy again",
            target.builder.name,
            target.builder.state.as_str().replace('_', " ")
        ));
    }
    check_against_deploy_servers(state, application_id, &target.builder, source)
        .await
        .map_err(|error| error.to_string())?;
    let builder = ServerRuntime::for_server(state, &target.builder.id)
        .await
        .map_err(|error| error.to_string())?;
    Ok(Some((target, builder)))
}

/// One image the build server built: `local` is its tag on the builder, `delivered`
/// what the deploy server runs (`<repository>@<digest>`, or `local` once copied
/// across). `service` names the Compose service it is for.
#[derive(Clone, Debug)]
struct Built {
    service: Option<String>,
    local: String,
    delivered: String,
}

/// One build per deploy run. A group deploy runs the pipeline once per server in
/// parallel; building each time would put several builds of one application on the
/// builder at once, sharing its source folder and image tag, so one could overwrite
/// another's source or ship or delete another's image. The first replica builds, the
/// rest wait for it; the entry is dropped by `finish_run`.
type SharedBuild = Arc<tokio::sync::OnceCell<Result<Vec<Built>, String>>>;

static BUILDS: LazyLock<Mutex<HashMap<String, SharedBuild>>> = LazyLock::new(Default::default);

fn shared_build(run_id: &str) -> SharedBuild {
    BUILDS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .entry(run_id.to_string())
        .or_default()
        .clone()
}

/// Called once the whole deploy run is over, on every server: drops the shared build
/// and removes its images from the builder, keeping the layer cache (`--no-prune`).
pub(crate) async fn finish_run(state: &SharedState, application_id: &str, run_id: &str) {
    let shared = BUILDS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(run_id);
    let Some(Ok(built)) = shared.as_ref().and_then(|cell| cell.get()) else {
        return;
    };
    if let Some(builder) = builder_runtime(state, application_id).await {
        for image in built {
            let _ = builder
                .podman(&[
                    OsString::from("rmi"),
                    OsString::from("--no-prune"),
                    OsString::from(&image.local),
                ])
                .await;
        }
    }
}

/// A Compose service name as it can appear in an image name or tag.
fn image_safe(service: &str) -> String {
    service
        .to_ascii_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// The builder-side tag for a Compose service's image.
fn service_image_tag(application_id: &str, service: &str) -> String {
    format!(
        "litepod-app-{application_id}-{}:latest",
        image_safe(service)
    )
}

fn compose_services(document: &mut serde_yaml::Value) -> Result<&mut serde_yaml::Mapping, String> {
    document
        .get_mut("services")
        .and_then(serde_yaml::Value::as_mapping_mut)
        .ok_or_else(|| "Invalid Compose file: expected a services mapping".to_string())
}

/// The Compose services that build an image (`build:`); the rest only pull one.
fn built_services(content: &str) -> Result<Vec<String>, String> {
    let mut document: serde_yaml::Value =
        serde_yaml::from_str(content).map_err(|error| format!("Invalid Compose file: {error}"))?;
    Ok(compose_services(&mut document)?
        .iter()
        .filter(|(_, config)| config.get("build").is_some())
        .filter_map(|(name, _)| name.as_str().map(str::to_string))
        .collect())
}

/// Points each service in `images` at its image. `drop_build` also removes its `build:`
/// (the deploy side, which must run the delivered image and never build it again).
fn with_images(
    content: &str,
    images: &[(String, String)],
    drop_build: bool,
) -> Result<String, String> {
    let mut document: serde_yaml::Value =
        serde_yaml::from_str(content).map_err(|error| format!("Invalid Compose file: {error}"))?;
    let services = compose_services(&mut document)?;
    for (service, image) in images {
        let config = services
            .get_mut(service.as_str())
            .and_then(serde_yaml::Value::as_mapping_mut)
            .ok_or_else(|| format!("Compose service '{service}' was not found"))?;
        config.insert("image".into(), image.as_str().into());
        if drop_build {
            config.remove("build");
        }
    }
    serde_yaml::to_string(&document).map_err(|error| error.to_string())
}

/// Builds the services of a Git Compose application that have `build:` on the builder,
/// each tagged with `service_image_tag`. Services with only `image:` are left to the
/// server that runs the application, which pulls them as before.
async fn compose_build(
    state: &SharedState,
    application_id: &str,
    run_id: &str,
    source: &SourceConfigDto,
    repo: &Path,
    builder: &ServerRuntime,
) -> Result<Vec<(Option<String>, String)>, String> {
    let compose_path = source
        .compose_file_path
        .as_deref()
        .ok_or("Compose file path is missing")?;
    let content = std::fs::read_to_string(repo.join(compose_path))
        .map_err(|error| format!("Could not read Compose file {compose_path}: {error}"))?;
    // Tenant input, about to run on the builder: the same check the deploy applies.
    crate::deployment::compose::validate_tenant_compose(&content, false)?;
    let services = built_services(&content)?;
    if services.is_empty() {
        let _ = crate::deployment::history::append(
            state,
            run_id,
            "info",
            "Every Compose service uses a ready-made image, so there is nothing to build",
        )
        .await;
        return Ok(Vec::new());
    }
    let images: Vec<(String, String)> = services
        .iter()
        .map(|service| (service.clone(), service_image_tag(application_id, service)))
        .collect();
    let build_file = crate::deployment::compose_standalone::prepare_remote_compose(&with_images(
        &content, &images, false,
    )?)?;
    const BUILD_FILE: &str = "litepod.build.compose.yml";
    let parent = Path::new(compose_path)
        .parent()
        .unwrap_or_else(|| Path::new(""));
    std::fs::write(repo.join(parent).join(BUILD_FILE), build_file)
        .map_err(|error| format!("Could not write the build Compose file: {error}"))?;
    // The clone's .git can carry the source URL; it never leaves litepod.
    let _ = std::fs::remove_dir_all(repo.join(".git"));
    let workspace = builder
        .ensure_workspace(application_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or("Build server workspace was not created")?;
    builder
        .upload_workspace_tree(application_id, repo, "repo")
        .await
        .map_err(|error| error.to_string())?
        .ok_or("The repository was not uploaded to the build server")?;
    let remote_file = super::compose::remote_generated_path(&workspace, compose_path, BUILD_FILE)?;
    let environment = crate::deployment::decrypted_environment(state, application_id, "runtime")
        .await
        .map_err(|error| error.to_string())?;
    let env_file =
        crate::servers::ssh::SecretFile::write(&super::compose::env_file_contents(&environment)?)
            .map_err(|error| error.to_string())?;
    let remote_env = builder
        .upload_workspace_file(application_id, env_file.path(), "litepod.build.env")
        .await
        .map_err(|error| error.to_string())?
        .ok_or("The build environment was not uploaded")?;
    let mut args = vec![
        OsString::from("compose"),
        OsString::from("--project-name"),
        OsString::from(format!("litepod-build-{application_id}")),
        OsString::from("--env-file"),
        OsString::from(&remote_env),
        OsString::from("--file"),
        OsString::from(remote_file),
        OsString::from("build"),
    ];
    args.extend(services.iter().map(OsString::from));
    let _ = crate::deployment::history::append(
        state,
        run_id,
        "info",
        &format!("Building Compose services {}", services.join(", ")),
    )
    .await;
    let built = stream_podman_step(
        builder,
        &args,
        state,
        run_id,
        Duration::from_secs(30 * 60),
        "Compose build on the build server failed",
    )
    .await;
    let _ = builder
        .remote_command(&[
            OsString::from("rm"),
            OsString::from("-f"),
            OsString::from(remote_env),
        ])
        .await;
    built?;
    Ok(images
        .into_iter()
        .map(|(service, image)| (Some(service), image))
        .collect())
}

/// Builds on the build server (once per run, see `SharedBuild`), delivers the image to
/// the server this replica deploys to, then rolls it out there.
pub(crate) async fn run(
    state: &SharedState,
    application_id: &str,
    run_id: &str,
    source: &SourceConfigDto,
    repo: &Path,
    target: BuildTarget,
    builder: ServerRuntime,
) -> Result<(), String> {
    // shortcut: the replicas that wait still cloned the repository before getting here;
    // skip their clone if group deploys of large repositories get slow.
    let mut built_here = false;
    let built = shared_build(run_id)
        .get_or_init(|| {
            built_here = true;
            build(
                state,
                application_id,
                run_id,
                source,
                repo,
                &target,
                &builder,
            )
        })
        .await
        .clone()?;
    if !built_here {
        let _ = crate::deployment::history::append(
            state,
            run_id,
            "info",
            "Using the images already built for this deploy",
        )
        .await;
    }

    let runtime = ServerRuntime::for_application(state, application_id)
        .await
        .map_err(|error| error.to_string())?;
    for image in &built {
        match &target.repository {
            Some(_) => {
                crate::deployment::image::pull(
                    state,
                    application_id,
                    run_id,
                    &runtime,
                    &image.delivered,
                    target.credentials.clone(),
                )
                .await?
            }
            None => transfer(state, run_id, &builder, &runtime, &image.local).await?,
        }
    }
    let _ = crate::deployment::history::set_status(state, run_id, "deploying").await;
    if source.build_method == "compose" {
        // Each built service now runs the delivered image instead of building again.
        let compose_path = source
            .compose_file_path
            .as_deref()
            .ok_or("Compose file path is missing")?;
        let images: Vec<(String, String)> = built
            .iter()
            .filter_map(|image| Some((image.service.clone()?, image.delivered.clone())))
            .collect();
        let file = repo.join(compose_path);
        let content = std::fs::read_to_string(&file)
            .map_err(|error| format!("Could not read Compose file {compose_path}: {error}"))?;
        std::fs::write(&file, with_images(&content, &images, true)?)
            .map_err(|error| format!("Could not write Compose file {compose_path}: {error}"))?;
        return super::compose::run(state, application_id, source, repo).await;
    }
    let image = built.first().ok_or("The build produced no image")?;
    crate::deployment::rollout::deploy_image(
        state,
        application_id,
        source,
        &image.delivered,
        &runtime,
    )
    .await
}

/// Builds on the builder and, for registry delivery, publishes every image.
async fn build(
    state: &SharedState,
    application_id: &str,
    run_id: &str,
    source: &SourceConfigDto,
    repo: &Path,
    target: &BuildTarget,
    builder: &ServerRuntime,
) -> Result<Vec<Built>, String> {
    let server = builder
        .remote_server_name()
        .ok_or("The build server must be a remote server")?;
    let _ = crate::deployment::history::append(
        state,
        run_id,
        "info",
        &format!("Building on build server {server}"),
    )
    .await;
    let images = match source.build_method.as_str() {
        "docker" => {
            remote_docker_build(state, application_id, run_id, source, repo, builder).await?;
            vec![(None, image_tag(application_id))]
        }
        "railpack" => {
            remote_railpack_build(state, application_id, run_id, source, repo, builder).await?;
            vec![(None, image_tag(application_id))]
        }
        "compose" => compose_build(state, application_id, run_id, source, repo, builder).await?,
        _ => return Err("Unsupported build method".into()),
    };
    let mut built = Vec::with_capacity(images.len());
    for (service, local) in images {
        let delivered = match &target.repository {
            Some(repository) => {
                publish(
                    state,
                    application_id,
                    run_id,
                    builder,
                    repository,
                    service.as_deref(),
                    &local,
                    target,
                )
                .await?
            }
            None => local.clone(),
        };
        built.push(Built {
            service,
            local,
            delivered,
        });
    }
    Ok(built)
}

/// Registry delivery: pushes `<repository>:[<service>-]<commit>` from the builder and
/// returns `<repository>@<digest>`, the exact image every server then pulls.
#[allow(clippy::too_many_arguments)]
async fn publish(
    state: &SharedState,
    application_id: &str,
    run_id: &str,
    builder: &ServerRuntime,
    repository: &str,
    service: Option<&str>,
    local: &str,
    target: &BuildTarget,
) -> Result<String, String> {
    // Uploaded sources have no commit; the run id still names the build uniquely.
    let commit = crate::deployment::history::commit_sha(state, run_id)
        .await
        .map_err(|error| error.to_string())?
        .unwrap_or_else(|| run_id.to_string());
    let tag = match service {
        // One repository holds every service; the tag tells them apart.
        Some(service) => format!("{}-{commit}", image_safe(service)),
        None => commit,
    };
    let reference = format!("{repository}:{tag}");
    builder
        .podman(&[
            OsString::from("tag"),
            OsString::from(local),
            OsString::from(&reference),
        ])
        .await
        .map_err(|error| format!("Could not tag the image as {reference}: {error}"))?;
    let _ =
        crate::deployment::history::append(state, run_id, "info", &format!("Pushing {reference}"))
            .await;
    let pushed = push(
        state,
        application_id,
        run_id,
        builder,
        repository,
        target.credentials.as_ref(),
        &reference,
    )
    .await;
    let _ = builder
        .podman(&[
            OsString::from("rmi"),
            OsString::from("--no-prune"),
            OsString::from(&reference),
        ])
        .await;
    let image = format!("{repository}@{}", pushed?);
    let _ =
        crate::deployment::history::append(state, run_id, "info", &format!("Published {image}"))
            .await;
    Ok(image)
}

async fn image_id(server: &ServerRuntime, image: &str) -> Result<String, String> {
    server
        .podman(&[
            OsString::from("image"),
            OsString::from("inspect"),
            OsString::from("--format"),
            OsString::from("{{.Id}}"),
            OsString::from(image),
        ])
        .await
        .map(|output| output.stdout.trim().to_string())
        .map_err(|error| format!("Could not inspect {image}: {error}"))
}

/// Transfer delivery, no registry needed: `podman save` on the builder piped straight
/// into `podman load` where the application runs, through litepod and never to disk.
/// The image id is compared afterwards so a cut-off copy is never deployed.
async fn transfer(
    state: &SharedState,
    run_id: &str,
    builder: &ServerRuntime,
    runtime: &ServerRuntime,
    image: &str,
) -> Result<(), String> {
    use tokio::io::AsyncReadExt;
    let built = image_id(builder, image).await?;
    let destination = runtime.remote_server_name().unwrap_or("this server");
    let _ = crate::deployment::history::append(
        state,
        run_id,
        "info",
        &format!("Copying the image to {destination}"),
    )
    .await;
    let mut save = builder
        .podman_stream(&[OsString::from("save"), OsString::from(image)])
        .await
        .map_err(|error| format!("Could not export the image: {error}"))?;
    let mut load = runtime
        .podman_stream_with_stdin(&[OsString::from("load")])
        .await
        .map_err(|error| format!("Could not start the image import on {destination}: {error}"))?;
    let (Some(mut from), Some(mut to)) = (save.take_stdout(), load.take_stdin()) else {
        return Err("Could not open the image copy streams".into());
    };
    let (mut save_errors, mut load_errors) = (save.take_stderr(), load.take_stderr());
    let copied = tokio::time::timeout(Duration::from_secs(30 * 60), async {
        let bytes = tokio::io::copy(&mut from, &mut to).await;
        drop(to); // EOF tells `podman load` the archive is complete.
        let saved = save.wait().await.map_err(|error| error.to_string())?;
        let loaded = load.wait().await.map_err(|error| error.to_string())?;
        Ok::<_, String>((bytes, saved, loaded))
    })
    .await
    .map_err(|_| "Copying the image timed out after 30 minutes".to_string())??;
    let mut detail = String::new();
    for stream in [save_errors.as_mut(), load_errors.as_mut()]
        .into_iter()
        .flatten()
    {
        let _ = stream.read_to_string(&mut detail).await;
    }
    let (bytes, saved, loaded) = copied;
    let bytes = bytes.map_err(|error| format!("The image copy was cut off: {error}. {detail}"))?;
    if !saved.success() || !loaded.success() {
        return Err(format!(
            "Could not copy the image to {destination}: {}",
            detail.trim()
        ));
    }
    let arrived = image_id(runtime, image).await?;
    if arrived != built {
        return Err(format!(
            "The image that arrived on {destination} ({arrived}) is not the one built ({built}); nothing was deployed"
        ));
    }
    let _ = crate::deployment::history::append(
        state,
        run_id,
        "info",
        &format!("Copied {} MiB to {destination}", bytes / (1024 * 1024)),
    )
    .await;
    Ok(())
}

/// `podman push` on the build server; returns the pushed manifest digest.
async fn push(
    state: &SharedState,
    application_id: &str,
    run_id: &str,
    builder: &ServerRuntime,
    repository: &str,
    credentials: Option<&(String, String)>,
    reference: &str,
) -> Result<String, String> {
    let workspace = builder
        .ensure_workspace(application_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or("Build server workspace was not created")?;
    let digest_file = format!("{workspace}/push-{run_id}.digest");
    let mut args = vec![
        OsString::from("push"),
        OsString::from("--digestfile"),
        OsString::from(&digest_file),
    ];
    let mut cleanup = vec![
        OsString::from("rm"),
        OsString::from("-f"),
        OsString::from(&digest_file),
    ];
    if let Some((username, password)) = credentials {
        let secret = crate::servers::ssh::SecretFile::write(&auth_file_contents(
            repository, username, password,
        ))
        .map_err(|error| error.to_string())?;
        let auth_file = builder
            .upload_workspace_file(
                application_id,
                secret.path(),
                &format!("registry-auth-{run_id}.json"),
            )
            .await
            .map_err(|error| error.to_string())?
            .ok_or("Registry credentials were not uploaded")?;
        args.extend([OsString::from("--authfile"), OsString::from(&auth_file)]);
        cleanup.push(OsString::from(auth_file));
    }
    args.push(OsString::from(reference));
    let pushed = stream_podman_step(
        builder,
        &args,
        state,
        run_id,
        Duration::from_secs(30 * 60),
        &format!("Could not push {reference}"),
    )
    .await;
    let digest = match pushed {
        Ok(()) => builder
            .remote_command(&[OsString::from("cat"), OsString::from(&digest_file)])
            .await
            .map(|output| output.stdout.trim().to_string())
            .map_err(|error| format!("Could not read the pushed image digest: {error}")),
        // Most registries create the repository on the first push; the rest need it made by hand.
        Err(error) => {
            let login = if credentials.is_some() {
                String::new()
            } else {
                format!(
                    " No login is saved for {}; add one under Registries in the organization settings if it needs one.",
                    registry_host(repository)
                )
            };
            Err(format!(
                "{error}\nIf the registry does not create repositories on push, create {repository} there and deploy again.{login}"
            ))
        }
    };
    let _ = builder.remote_command(&cleanup).await;
    let digest = digest?;
    if !digest.starts_with("sha256:") {
        return Err(format!(
            "The registry returned an unexpected digest: {digest}"
        ));
    }
    Ok(digest)
}

#[cfg(test)]
mod tests {
    use super::{Machine, auth_file_contents, check_compatible, validate_repository};

    fn machine(
        name: &str,
        arch: Option<&str>,
        cpu: Option<f64>,
        memory_mib: Option<u64>,
    ) -> Machine {
        Machine {
            name: name.into(),
            arch: arch.map(Into::into),
            cpu_cores: cpu,
            memory_bytes: memory_mib.map(|mib| mib * 1024 * 1024),
        }
    }

    #[test]
    fn a_build_must_run_where_it_deploys_and_fit_on_the_builder() {
        // The build asks for 8 cores and 16 GiB.
        let check = |builder: &Machine, deploy: &Machine| {
            check_compatible(builder, deploy, Some(8.0), Some(16_384))
        };
        let web = machine("web", Some("x86_64"), None, None);
        let builder = machine("big", Some("amd64"), Some(16.0), Some(32_768));

        // FreeBSD's amd64 is Linux's x86_64.
        assert!(check(&builder, &web).is_ok());
        assert!(check(&builder, &machine("pi", Some("aarch64"), None, None)).is_err());
        // Unknown facts are not guessed at.
        assert!(check(&builder, &machine("new", None, None, None)).is_ok());

        let few_cores = machine("small", Some("x86_64"), Some(4.0), Some(32_768));
        assert!(check(&few_cores, &web).is_err());
        let little_ram = machine("short", Some("x86_64"), Some(16.0), Some(8_192));
        assert!(check(&little_ram, &web).is_err());
    }

    #[test]
    fn repository_needs_a_registry_host_and_no_tag() {
        assert!(validate_repository("registry.example.com/team/my-app").is_ok());
        assert!(validate_repository("localhost:5000/app").is_ok());
        assert!(validate_repository("ghcr.io/acme/app").is_ok());
        assert!(validate_repository("team/my-app").is_err());
        assert!(validate_repository("ghcr.io/acme/app:latest").is_err());
        assert!(validate_repository("ghcr.io/acme/App").is_err());
        assert!(validate_repository("ghcr.io/acme//app").is_err());
        assert!(validate_repository("ghcr.io/").is_err());
        assert!(validate_repository("ghcr.io/acme/app@sha256:abc").is_err());
    }

    #[tokio::test]
    async fn only_a_build_server_builds_and_no_application_runs_on_one() {
        use crate::dtos::deployment::UpdateBuildServerRequest;
        let (state, path) = crate::testing::test_state().await;
        let (_, _, app) = crate::testing::seed_application(&state, "organization-a").await;
        sqlx::query(
            "UPDATE applications SET deploy_type = 'git', build_method = 'docker' WHERE id = ?1",
        )
        .bind(&app)
        .execute(&state.db)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO servers (id, kind, name, organization_id, ingress_mode, state, ssh_host, ssh_port, ssh_user, created_at, updated_at, role) \
             VALUES ('builder', 'ssh', 'builder', 'organization-a', 'private_only', 'ready', '198.51.100.2', 22, 'litepod', 0, 0, 'build')",
        )
        .execute(&state.db)
        .await
        .unwrap();
        let request = |server: &str| UpdateBuildServerRequest {
            build_server_id: Some(server.into()),
            delivery: None,
            image_repository: Some("ghcr.io/acme/app".into()),
        };

        assert!(
            super::save(
                &state,
                "organization-a",
                &app,
                request("organization-a-server")
            )
            .await
            .is_err()
        );
        let saved = super::save(&state, "organization-a", &app, request("builder"))
            .await
            .unwrap();
        assert_eq!(saved.build_server_id.as_deref(), Some("builder"));
        // Transfer is the default and has no registry repository to keep.
        assert_eq!(saved.delivery, "transfer");
        assert_eq!(saved.image_repository, None);
        let registry = UpdateBuildServerRequest {
            delivery: Some("registry".into()),
            ..request("builder")
        };
        let saved = super::save(&state, "organization-a", &app, registry)
            .await
            .unwrap();
        assert_eq!(saved.image_repository.as_deref(), Some("ghcr.io/acme/app"));

        // An image built on an ARM builder would not start on an x86 server.
        sqlx::query(
            "UPDATE servers SET facts_json = '{\"architecture\":\"aarch64\"}' WHERE id = 'builder'",
        )
        .execute(&state.db)
        .await
        .unwrap();
        sqlx::query("UPDATE servers SET facts_json = '{\"architecture\":\"x86_64\"}' WHERE id = 'organization-a-server'")
            .execute(&state.db)
            .await
            .unwrap();
        let error = super::save(&state, "organization-a", &app, request("builder"))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("aarch64"), "{error}");

        // The builder is never offered for placement, and the database refuses it anyway.
        let placeable =
            crate::servers::repository::list_ready_for_organization(&state, "organization-a")
                .await
                .unwrap();
        assert!(placeable.iter().all(|server| server.id != "builder"));
        assert!(
            sqlx::query("UPDATE applications SET server_id = 'builder' WHERE id = ?1")
                .bind(&app)
                .execute(&state.db)
                .await
                .is_err()
        );
        crate::testing::cleanup_database(&path);
    }

    #[tokio::test]
    async fn replicas_of_one_deploy_share_a_single_build() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let builds = AtomicUsize::new(0);
        let build = || async {
            builds.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            Ok::<_, String>(vec![super::Built {
                service: None,
                local: "image".into(),
                delivered: "image".into(),
            }])
        };
        let replica = || async {
            let built = super::shared_build("run-shared")
                .get_or_init(build)
                .await
                .clone()
                .unwrap();
            built[0].delivered.clone()
        };
        let (a, b, c) = tokio::join!(replica(), replica(), replica());
        assert_eq!(
            (a.as_str(), b.as_str(), c.as_str()),
            ("image", "image", "image")
        );
        assert_eq!(builds.load(Ordering::SeqCst), 1);

        // Another deploy builds again.
        super::shared_build("run-other").get_or_init(build).await;
        assert_eq!(builds.load(Ordering::SeqCst), 2);
        let mut cache = super::BUILDS.lock().unwrap();
        cache.remove("run-shared");
        cache.remove("run-other");
    }

    #[test]
    fn only_compose_services_with_build_go_to_the_builder() {
        let compose = "services:\n  web:\n    build: .\n    ports: ['8080']\n  Worker_1:\n    build:\n      context: ./worker\n    image: acme/worker\n  db:\n    image: postgres:17\n";
        assert_eq!(
            super::built_services(compose).unwrap(),
            vec!["web", "Worker_1"]
        );
        assert_eq!(
            super::service_image_tag("app", "Worker_1"),
            "litepod-app-app-worker_1:latest"
        );

        // The deploy side runs what was delivered and never builds again.
        let images = vec![
            ("web".to_string(), "ghcr.io/acme/app@sha256:abc".to_string()),
            (
                "Worker_1".to_string(),
                "litepod-app-app-worker_1:latest".to_string(),
            ),
        ];
        let deployed: serde_yaml::Value =
            serde_yaml::from_str(&super::with_images(compose, &images, true).unwrap()).unwrap();
        let services = &deployed["services"];
        assert_eq!(services["web"]["image"], "ghcr.io/acme/app@sha256:abc");
        assert!(services["web"].get("build").is_none());
        assert_eq!(services["web"]["ports"][0], "8080");
        assert_eq!(
            services["Worker_1"]["image"],
            "litepod-app-app-worker_1:latest"
        );
        assert!(services["Worker_1"].get("build").is_none());
        assert_eq!(services["db"]["image"], "postgres:17");

        // The builder keeps `build:` and only learns the tag to give each image.
        let building: serde_yaml::Value =
            serde_yaml::from_str(&super::with_images(compose, &images, false).unwrap()).unwrap();
        assert_eq!(building["services"]["web"]["build"], ".");
        assert!(super::with_images(compose, &[("missing".into(), "x".into())], true).is_err());
    }

    #[test]
    fn auth_file_is_keyed_by_registry_host() {
        let json: serde_json::Value =
            serde_json::from_str(&auth_file_contents("ghcr.io/acme/app", "robot", "s3:cret"))
                .unwrap();
        // base64("robot:s3:cret")
        assert_eq!(json["auths"]["ghcr.io"]["auth"], "cm9ib3Q6czM6Y3JldA==");
    }
}
