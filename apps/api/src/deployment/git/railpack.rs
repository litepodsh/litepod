use std::{
    io::Cursor,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use futures_util::{StreamExt, TryStreamExt};
use podman_api::{
    Podman,
    conn::TtyChunk,
    models,
    opts::{
        ContainerCreateOpts, ContainerCreateOptsBuilder, ContainerDeleteOpts, ContainerLogsOpts,
        ContainerWaitOpts, ImageBuildOpts,
    },
};

use crate::{dtos::deployment::SourceConfigDto, state::state::SharedState};

use super::super::{BUILD_CPU_PERIOD_US, decrypted_environment, history, resource_limits};

/// Build logs are written to the run history one row per chunk; cap it so a
/// noisy build cannot flood the database.
const MAX_LOG_ROWS: usize = 400;
static BUILDER_IMAGE_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

/// Embedded at compile time so the builder image can be produced on any host —
/// including the packaged runtime container, which never COPYs these files in
/// — with no dependency on the API process's CWD or an out-of-band pre-build
/// step. Overridable via RAILPACK_BUILDER_DOCKERFILE for local iteration
/// without recompiling (see `builder_dockerfile_override`).
const EMBEDDED_DOCKERFILE: &str = include_str!("../../../../Dockerfile.railpack-builder");
const EMBEDDED_BUILD_SCRIPT: &str = include_str!("../../../../scripts/railpack-build.sh");
const EMBEDDED_PREPARE_SCRIPT: &str = include_str!("../../../../scripts/railpack-prepare.sh");

fn image_tag(application_id: &str) -> String {
    format!("litepod-app-{application_id}:latest")
}

pub(crate) fn builder_name(application_id: &str, run_id: &str) -> String {
    format!("litepod-railpack-build-{application_id}-{run_id}")
}

fn planner_name(application_id: &str, run_id: &str) -> String {
    format!("litepod-railpack-prepare-{application_id}-{run_id}")
}

fn workspace_dir(data_dir: &Path, run_id: &str) -> PathBuf {
    data_dir.join("railpack").join(run_id)
}

fn workspace_path(run_id: &str) -> String {
    format!("/data/railpack/{run_id}")
}

fn context_dir(application_id: &str, project_path: &str) -> String {
    let path = project_path.trim().trim_matches('/');
    if path.is_empty() || path == "." {
        format!("/data/repos/{application_id}")
    } else {
        format!("/data/repos/{application_id}/{path}")
    }
}

pub(crate) fn apply_start_command(environment: &mut Vec<(String, String)>, command: Option<&str>) {
    let Some(command) = command.map(str::trim).filter(|command| !command.is_empty()) else {
        return;
    };
    environment.retain(|(name, _)| name != "RAILPACK_START_CMD");
    environment.push(("RAILPACK_START_CMD".into(), command.into()));
}

fn builder_image() -> String {
    std::env::var("RAILPACK_BUILDER_IMAGE")
        .ok()
        .filter(|value| !value.trim().is_empty())
        // Fully qualified: Podman's short-name resolution would otherwise try a
        // remote registry for this purely local image.
        .unwrap_or_else(|| "localhost/litepod-railpack-builder:rootless-v2".to_string())
}

/// Disk path to use instead of the embedded builder Dockerfile, for local
/// iteration on the Dockerfile/scripts without a recompile. Unset (the
/// default, and what every packaged build runs with) always uses the
/// content embedded in the binary at compile time.
fn builder_dockerfile_override() -> Option<std::path::PathBuf> {
    std::env::var("RAILPACK_BUILDER_DOCKERFILE")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .map(std::path::PathBuf::from)
}

/// The only files the builder Dockerfile pulls from its context. Kept as constants
/// so staging fails loudly if the Dockerfile ever copies something else.
const BUILDER_CONTEXT_FILES: &[&str] =
    &["scripts/railpack-build.sh", "scripts/railpack-prepare.sh"];

/// Stages the builder Dockerfile and the scripts it needs into a throwaway
/// directory, and returns that directory as the build context.
///
/// `podman_api` tars and gzips the whole context in memory before posting it to the
/// socket. Handing it the repo root would compress `target/` and `node_modules/` -
/// gigabytes - on every deploy, pinning a core and ballooning the process heap.
pub(crate) fn stage_builder_context(run_id: &str) -> Result<PathBuf, String> {
    let staging = crate::servers::replica::scoped_path(
        std::env::temp_dir().join(format!("litepod-railpack-builder-context-{run_id}")),
    );
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(staging.join("scripts"))
        .map_err(|e| format!("Could not create builder build context: {e}"))?;

    match builder_dockerfile_override() {
        Some(dockerfile) => {
            let source_root = dockerfile.parent().ok_or_else(|| {
                format!("Builder Dockerfile has no parent: {}", dockerfile.display())
            })?;
            std::fs::copy(&dockerfile, staging.join("Dockerfile.railpack-builder"))
                .map_err(|e| format!("Could not stage {}: {e}", dockerfile.display()))?;
            for file in BUILDER_CONTEXT_FILES {
                std::fs::copy(source_root.join(file), staging.join(file))
                    .map_err(|e| format!("Could not stage {file}: {e}"))?;
            }
        }
        None => {
            std::fs::write(
                staging.join("Dockerfile.railpack-builder"),
                EMBEDDED_DOCKERFILE,
            )
            .map_err(|e| format!("Could not stage embedded builder Dockerfile: {e}"))?;
            std::fs::write(
                staging.join("scripts/railpack-build.sh"),
                EMBEDDED_BUILD_SCRIPT,
            )
            .map_err(|e| format!("Could not stage embedded railpack-build.sh: {e}"))?;
            std::fs::write(
                staging.join("scripts/railpack-prepare.sh"),
                EMBEDDED_PREPARE_SCRIPT,
            )
            .map_err(|e| format!("Could not stage embedded railpack-prepare.sh: {e}"))?;
        }
    }

    Ok(staging)
}

async fn ensure_builder_image(podman: &Podman, run_id: &str) -> Result<(), String> {
    let tag = builder_image();
    if podman.images().get(&tag).inspect().await.is_ok() {
        return Ok(());
    }
    let _guard = BUILDER_IMAGE_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;
    // Another concurrent Railpack deployment may have built the shared image
    // while this task waited for the provisioning lock.
    if podman.images().get(&tag).inspect().await.is_ok() {
        return Ok(());
    }
    let context = stage_builder_context(run_id)?;
    let result = async {
        let options = ImageBuildOpts::builder(context.to_string_lossy().to_string())
            .dockerfile("Dockerfile.railpack-builder")
            .tag(&tag)
            .build();

        let images = podman.images();
        let mut build = images
            .build(&options)
            .map_err(|e| format!("Could not start builder image build: {e}"))?;
        while let Some(chunk) = build.next().await {
            let chunk = chunk.map_err(|e| format!("Builder image build failed: {e}"))?;
            for line in chunk.stream.lines() {
                tracing::info!("[railpack-builder] {line}");
            }
        }
        podman
            .images()
            .get(&tag)
            .inspect()
            .await
            .map_err(|_| String::from("Builder image build produced no image"))?;
        Ok(())
    }
    .await;
    let _ = std::fs::remove_dir_all(&context);
    result
}

/// Deletes only the temporary resources belonging to one Railpack run. This is
/// called both from the normal runner and by the queue after cancellation,
/// because cancellation drops the in-flight deployment future before its local
/// cleanup code can run.
pub(crate) async fn cleanup_run(state: &SharedState, application_id: &str, run_id: &str) {
    // The build ran wherever the application is deployed (local or remote over
    // SSH) — cleanup must target the same host, or a canceled remote build
    // leaves its planner/builder containers orphaned on the remote server.
    match crate::servers::runtime::ServerRuntime::for_application(state, application_id).await {
        Ok(runtime) => {
            // Dropping the local `ssh` does not stop the command it started on
            // the server, so a canceled `podman build` of the worker image
            // would keep running there. The bracket keeps the pattern from
            // matching this command's own shell.
            if runtime.is_remote() {
                let _ = runtime
                    .remote_command(&[
                        std::ffi::OsString::from("pkill"),
                        std::ffi::OsString::from("-f"),
                        std::ffi::OsString::from(format!("[r]ailpack-builder-{run_id}")),
                    ])
                    .await;
            }
            // A remote build staged its inputs and output under the SSH user's
            // home, where nothing else would ever remove them.
            if runtime.is_remote() {
                let root = format!(".local/share/litepod/apps/{application_id}");
                let _ = runtime
                    .remote_command(&[
                        std::ffi::OsString::from("rm"),
                        std::ffi::OsString::from("-rf"),
                        std::ffi::OsString::from(format!("{root}/railpack/{run_id}")),
                        std::ffi::OsString::from(format!("{root}/railpack-builder-{run_id}")),
                        std::ffi::OsString::from(format!("{root}/railpack-builder-{run_id}.tar")),
                    ])
                    .await;
            }
            // `--time 0` skips the 10s SIGTERM grace (buildkit ignores it) and
            // the removals run together, so a cancel does not wait on each one.
            futures_util::future::join_all(
                [
                    planner_name(application_id, run_id),
                    builder_name(application_id, run_id),
                ]
                .map(|name| async {
                    let args = [
                        std::ffi::OsString::from("rm"),
                        std::ffi::OsString::from("--force"),
                        std::ffi::OsString::from("--time"),
                        std::ffi::OsString::from("0"),
                        std::ffi::OsString::from("--volumes"),
                        std::ffi::OsString::from(name),
                    ];
                    runtime.podman(&args).await
                }),
            )
            .await;
            let _ = runtime
                .podman(&[
                    std::ffi::OsString::from("rmi"),
                    std::ffi::OsString::from("--force"),
                    std::ffi::OsString::from(format!(
                        "litepod-railpack-builder-{application_id}:{run_id}"
                    )),
                ])
                .await;
        }
        Err(error) => {
            tracing::warn!(application_id, run_id, %error, "could not resolve server to clean up Railpack containers");
        }
    }
    let _ = std::fs::remove_dir_all(workspace_dir(&state.data_dir, run_id));
}

fn tail(lines: &[String]) -> String {
    lines
        .iter()
        .rev()
        .take(40)
        .cloned()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n")
}

/// The builder container sees the application data directory at `/data`, so the
/// same paths work whether the data lives on a bind mount or a named volume.
/// Named volumes go through the dedicated `volumes` field (`NamedVolume`), not
/// the generic OCI `mounts` field — the generic path skips libpod's
/// named-volume resolution (copy-up/chown), which crun then rejects with
/// "No such device" on rootless Podman.
fn data_mount(state: &SharedState) -> (Vec<models::ContainerMount>, Vec<models::NamedVolume>) {
    match &state.data_volume {
        Some(volume) => (
            vec![],
            vec![models::NamedVolume {
                dest: Some("/data".into()),
                is_anonymous: None,
                name: Some(volume.clone()),
                options: None,
            }],
        ),
        None => (
            vec![models::ContainerMount {
                destination: Some("/data".into()),
                options: None,
                source: Some(state.host_data_dir.to_string_lossy().to_string()),
                _type: Some("bind".into()),
                uid_mappings: None,
                gid_mappings: None,
            }],
            vec![],
        ),
    }
}

fn with_build_limits(
    mut options: ContainerCreateOptsBuilder,
    source: &SourceConfigDto,
) -> Result<ContainerCreateOptsBuilder, String> {
    if let Some(limits) = resource_limits(source)? {
        let memory = i64::try_from(limits.memory_bytes)
            .map_err(|_| "Build memory limit is too large".to_string())?;
        let memswap = i64::try_from(limits.memswap_bytes)
            .map_err(|_| "Build swap limit is too large".to_string())?;
        options = options
            .cpu_period(BUILD_CPU_PERIOD_US as u64)
            .cpu_quota(limits.cpu_quota_us as i64)
            .resource_limits(models::LinuxResources {
                block_io: None,
                cpu: None,
                devices: None,
                hugepage_limits: None,
                memory: Some(models::LinuxMemory {
                    disable_oom_killer: None,
                    kernel: None,
                    kernel_tcp: None,
                    limit: Some(memory),
                    reservation: None,
                    swap: Some(memswap),
                    swappiness: None,
                    use_hierarchy: None,
                }),
                network: None,
                pids: None,
                rdma: None,
                unified: None,
            });
    }
    Ok(options)
}

async fn collect_build_logs(
    state: &SharedState,
    run_id: &str,
    container: &podman_api::api::Container,
) -> Vec<String> {
    let log_options = ContainerLogsOpts::builder()
        .stdout(true)
        .stderr(true)
        .follow(true)
        .build();
    let mut logs = container.logs(&log_options);
    let mut collected = Vec::new();
    let mut rows = 0usize;
    while let Some(chunk) = logs.next().await {
        let Ok(chunk) = chunk else { continue };
        let bytes = match chunk {
            TtyChunk::StdOut(bytes) | TtyChunk::StdErr(bytes) => bytes,
            TtyChunk::StdIn(_) => continue,
        };
        let message = String::from_utf8_lossy(&bytes)
            .trim_end_matches('\n')
            .to_string();
        if message.is_empty() {
            continue;
        }
        for line in message.lines() {
            collected.push(line.to_string());
            if rows < MAX_LOG_ROWS {
                let _ = history::append(state, run_id, "info", line).await;
                rows += 1;
                if rows == MAX_LOG_ROWS {
                    let _ =
                        history::append(state, run_id, "info", "… build output truncated").await;
                }
            }
        }
        if collected.len() > 4000 {
            collected.drain(..collected.len() - 4000);
        }
    }
    let _ = container.wait(&ContainerWaitOpts::builder().build()).await;
    collected
}

async fn copy_from_container(
    container: &podman_api::api::Container,
    path: &str,
) -> Result<Vec<u8>, String> {
    container
        .copy_from(path)
        .try_concat()
        .await
        .map_err(|error| format!("Could not transfer build input {path}: {error}"))
}

fn copied_child_path(parent: &str, source_path: &str) -> Result<String, String> {
    let name = Path::new(source_path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| format!("Invalid build input path: {source_path}"))?;
    Ok(format!("{parent}/{name}"))
}

fn extract_archive_file(archive: Vec<u8>, destination: &Path) -> Result<(), String> {
    let parent = destination.parent().ok_or_else(|| {
        format!(
            "Built image archive has no parent directory: {}",
            destination.display()
        )
    })?;
    std::fs::create_dir_all(parent).map_err(|error| {
        format!(
            "Could not create built image archive directory {}: {error}",
            parent.display()
        )
    })?;
    let mut archive = tar::Archive::new(Cursor::new(archive));
    let mut entries = archive
        .entries()
        .map_err(|error| format!("Could not read build output archive: {error}"))?;
    while let Some(entry) = entries.next() {
        let mut entry =
            entry.map_err(|error| format!("Could not read build output entry: {error}"))?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let mut output = std::fs::File::create(destination).map_err(|error| {
            format!(
                "Could not create built image archive {}: {error}",
                destination.display()
            )
        })?;
        std::io::copy(&mut entry, &mut output)
            .map_err(|error| format!("Could not write built image archive: {error}"))?;
        return Ok(());
    }
    Err("Builder produced an empty image archive".into())
}

/// Prepares a Railpack plan with access to the repository, then transfers only
/// that plan and the requested project directory into an isolated rootless
/// BuildKit container. The runner has no `/data` mount, so application builds
/// cannot browse other application data.
async fn build_image(
    state: &SharedState,
    application_id: &str,
    run_id: &str,
    podman: &Podman,
    source: &SourceConfigDto,
) -> Result<std::path::PathBuf, String> {
    ensure_builder_image(podman, run_id).await?;
    let _ = history::append(state, run_id, "info", "Preparing Railpack build workspace").await;
    let out_dir = workspace_dir(&state.data_dir, run_id);
    std::fs::create_dir_all(&out_dir)
        .map_err(|error| format!("Could not create Railpack workspace: {error}"))?;

    // Build-time variables go through a file rather than argv so they are not
    // visible in the container's process table.
    let mut environment = decrypted_environment(state, application_id, "build")
        .await
        .map_err(|error| error.to_string())?;
    apply_start_command(&mut environment, source.railpack_start_command.as_deref());
    let build_env = environment
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(out_dir.join("build-env"), build_env)
        .map_err(|error| format!("Could not write Railpack build environment: {error}"))?;

    let context = context_dir(application_id, &source.railpack_project_path);
    let planner_name = planner_name(application_id, run_id);
    let planner = podman.containers().get(&planner_name);
    let _ = planner
        .delete(
            &ContainerDeleteOpts::builder()
                .force(true)
                .volumes(true)
                .build(),
        )
        .await;

    let mut networks = HashMap::new();
    networks.insert(
        state.podman_network.clone(),
        models::PerNetworkOptions {
            aliases: None,
            interface_name: None,
            static_ips: None,
            static_mac: None,
        },
    );
    let (bind_mounts, named_volumes) = data_mount(state);
    let planner_options = with_build_limits(
        ContainerCreateOpts::builder()
            .name(&planner_name)
            .image(builder_image())
            .env([
                ("CONTEXT_DIR", context.clone()),
                ("OUT_DIR", workspace_path(run_id)),
            ])
            .mounts(bind_mounts)
            .volumes(named_volumes)
            .net_namespace(models::Namespace {
                nsmode: Some("bridge".into()),
                value: None,
            })
            .networks(networks.clone()),
        source,
    )?
    .build();
    podman
        .containers()
        .create(&planner_options)
        .await
        .map_err(|error| format!("Could not create Railpack preparation container: {error}"))?;
    planner
        .start(None)
        .await
        .map_err(|error| format!("Could not start Railpack preparation container: {error}"))?;
    let _ = history::append(state, run_id, "info", "Railpack plan preparation started").await;

    let planner_logs = collect_build_logs(state, run_id, &planner).await;
    if !out_dir.join("railpack-plan.json").is_file() {
        let _ = planner
            .delete(&ContainerDeleteOpts::builder().force(true).build())
            .await;
        return Err(format!(
            "Railpack build preparation failed: {}",
            tail(&planner_logs)
        ));
    }

    let repository = copy_from_container(&planner, &context).await?;
    let inputs = copy_from_container(&planner, &workspace_path(run_id)).await?;
    let _ = planner
        .delete(&ContainerDeleteOpts::builder().force(true).build())
        .await;

    let name = builder_name(application_id, run_id);
    let builder = podman.containers().get(&name);
    let _ = builder
        .delete(&ContainerDeleteOpts::builder().force(true).build())
        .await;
    let runner_context = copied_child_path("/workspace/source", &context)?;
    let runner_plan = copied_child_path("/workspace/inputs", &workspace_path(run_id))?;
    let runner_options = with_build_limits(
        ContainerCreateOpts::builder()
            .name(&name)
            .image(builder_image())
            .entrypoint(["/usr/local/bin/railpack-build"])
            // BuildKit creates nested cgroups for `RUN` steps. Rootless Podman
            // keeps this privilege inside the caller's user namespace, while
            // the builder remains ephemeral, has no `/data` mount, and still
            // receives the application's outer CPU/RAM cgroup limits.
            .privileged(true)
            .env([
                ("IMAGE_TAG", image_tag(application_id)),
                ("CONTEXT_DIR", runner_context),
                ("PLAN_DIR", runner_plan),
                ("OUT_DIR", "/output".to_string()),
                ("RAILPACK_CLEAN_BUILD", source.clean_build.to_string()),
            ])
            .net_namespace(models::Namespace {
                nsmode: Some("bridge".into()),
                value: None,
            })
            .networks(networks),
        source,
    )?
    .build();
    podman
        .containers()
        .create(&runner_options)
        .await
        .map_err(|error| {
            format!("Could not create rootless Railpack builder container: {error}")
        })?;
    builder
        .copy_to("/workspace/source", repository.into())
        .await
        .map_err(|error| format!("Could not transfer repository to rootless builder: {error}"))?;
    builder
        .copy_to("/workspace/inputs", inputs.into())
        .await
        .map_err(|error| {
            format!("Could not transfer Railpack plan to rootless builder: {error}")
        })?;
    builder
        .start(None)
        .await
        .map_err(|error| format!("Could not start rootless Railpack builder container: {error}"))?;
    let _ = history::append(
        state,
        run_id,
        "info",
        "Rootless Railpack builder started; streaming build output",
    )
    .await;
    let build_logs = collect_build_logs(state, run_id, &builder).await;

    // buildctl only writes the tarball on success, so its presence is the
    // build's exit status. Podman's copy endpoint returns a tar stream, which
    // we unpack into the normal deployment workspace for image loading.
    let archive = out_dir.join("image.tar");
    let outcome = async {
        let output = match copy_from_container(&builder, "/output/image.tar").await {
            Ok(output) => output,
            // A failed build has no output archive. Preserve its streamed
            // Railpack/BuildKit tail instead of replacing it with a generic
            // Podman copy error.
            Err(_) => return Err(format!("Railpack build failed: {}", tail(&build_logs))),
        };
        extract_archive_file(output, &archive)?;
        match std::fs::metadata(&archive) {
            Ok(metadata) if metadata.len() > 0 => {
                let _ =
                    history::append(state, run_id, "info", "Railpack image build completed").await;
                Ok(archive)
            }
            _ => Err(format!("Railpack build failed: {}", tail(&build_logs))),
        }
    }
    .await;
    let _ = builder
        .delete(&ContainerDeleteOpts::builder().force(true).build())
        .await;
    outcome
}

pub async fn run(
    state: &SharedState,
    application_id: &str,
    run_id: &str,
    source: &SourceConfigDto,
) -> Result<(), String> {
    let podman = Podman::new(&state.podman_socket)
        .map_err(|error| format!("Invalid Podman socket: {error}"))?;
    let result = async {
        // The remote Railpack build is already capped at 30 minutes; the local
        // path had no equivalent, so a runaway build could hang the job forever
        // instead of surfacing an error.
        let archive = match tokio::time::timeout(
            std::time::Duration::from_secs(30 * 60),
            build_image(state, application_id, run_id, &podman, source),
        )
        .await
        {
            Ok(result) => result?,
            Err(_) => return Err("Railpack build timed out after 30 minutes".to_string()),
        };
        let _ = history::set_status(state, run_id, "deploying").await;
        let _ = history::append(state, run_id, "info", "Loading built image into Podman").await;
        let bytes = std::fs::read(&archive)
            .map_err(|error| format!("Could not read the built image: {error}"))?;
        podman
            .images()
            .load(&bytes)
            .await
            .map_err(|error| format!("Could not load the built image: {error}"))?;
        Ok::<(), String>(())
    }
    .await;
    cleanup_run(state, application_id, run_id).await;
    result?;

    let runtime = crate::servers::runtime::ServerRuntime::for_application(state, application_id)
        .await
        .map_err(|error| error.to_string())?;
    super::super::rollout::deploy_image(
        state,
        application_id,
        source,
        &image_tag(application_id),
        &runtime,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn builder_configuration_falls_back_to_the_compose_defaults() {
        assert_eq!(
            builder_image(),
            "localhost/litepod-railpack-builder:rootless-v2"
        );
    }

    #[test]
    fn explicit_start_command_overrides_advanced_build_variable() {
        let mut environment = vec![("RAILPACK_START_CMD".into(), "old".into())];
        apply_start_command(&mut environment, Some(" node ./dist/server/entry.mjs "));
        assert_eq!(
            environment,
            vec![(
                "RAILPACK_START_CMD".to_string(),
                "node ./dist/server/entry.mjs".to_string()
            )]
        );
    }

    #[test]
    fn empty_start_command_preserves_build_environment() {
        let mut environment = vec![("FOO".into(), "bar".into())];
        apply_start_command(&mut environment, Some("  "));
        assert_eq!(environment, vec![("FOO".to_string(), "bar".to_string())]);
    }

    #[test]
    fn log_tail_keeps_the_last_lines() {
        let lines: Vec<String> = (0..60).map(|n| n.to_string()).collect();
        let tail = tail(&lines);
        assert!(tail.starts_with("20\n21"));
        assert!(tail.ends_with("59"));
    }

    #[test]
    fn build_script_disables_buildkit_cache_reuse() {
        let script = EMBEDDED_BUILD_SCRIPT;
        assert!(script.contains("RAILPACK_CLEAN_BUILD"));
        assert!(script.contains("set -- \"$@\" --no-cache"));
        assert!(script.contains("buildkitd --rootless"));
        assert!(script.contains("--oci-worker-snapshotter=native"));
    }

    #[test]
    fn stage_builder_context_embeds_the_default_dockerfile_and_scripts() {
        unsafe { std::env::remove_var("RAILPACK_BUILDER_DOCKERFILE") };
        let run_id = format!("test-{}", uuid::Uuid::now_v7());
        let dir = stage_builder_context(&run_id).unwrap();
        assert!(dir.join("Dockerfile.railpack-builder").is_file());
        assert!(dir.join("scripts/railpack-build.sh").is_file());
        assert!(dir.join("scripts/railpack-prepare.sh").is_file());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn run_scoped_resources_do_not_collide() {
        let app = "019fb3cb-276e-79a1-8b90-df1036bbba73";
        let first_run = "019fb4cc-276e-79a1-8b90-df1036bbba73";
        let second_run = "019fb5dd-276e-79a1-8b90-df1036bbba73";
        let root = std::env::temp_dir();

        assert_ne!(builder_name(app, first_run), builder_name(app, second_run));
        assert_ne!(planner_name(app, first_run), planner_name(app, second_run));
        assert_ne!(
            workspace_dir(&root, first_run),
            workspace_dir(&root, second_run)
        );
        assert_eq!(
            workspace_path(first_run),
            format!("/data/railpack/{first_run}")
        );
    }

    #[test]
    fn archive_extraction_creates_its_run_workspace() {
        let destination = std::env::temp_dir()
            .join(format!("litepod-railpack-test-{}", uuid::Uuid::now_v7()))
            .join("workspace")
            .join("image.tar");
        let mut tarball = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tarball);
            let content = b"image";
            let mut header = tar::Header::new_gnu();
            header.set_size(content.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, "image.tar", Cursor::new(content))
                .unwrap();
            builder.finish().unwrap();
        }

        extract_archive_file(tarball, &destination).unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"image");
        let _ = std::fs::remove_dir_all(destination.parent().unwrap().parent().unwrap());
    }
}
use std::collections::HashMap;
