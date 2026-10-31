use std::{
    ffi::OsString,
    io::{Cursor, Read},
    path::{Component, Path, PathBuf},
    time::{Duration, Instant},
};

use futures_util::{StreamExt, TryStreamExt};
use podman_api::{
    Podman,
    conn::TtyChunk,
    models,
    opts::{ContainerCreateOpts, ContainerDeleteOpts, ExecCreateOpts, PullOpts},
};
use tar::{Archive, Builder, Header};
use uuid::Uuid;

use crate::{
    dtos::{
        deployment::{WorkspaceEntryDto, WorkspaceFileDto},
        response::AppError,
    },
    servers::runtime::ServerRuntime,
    state::state::SharedState,
};

pub enum WorkspaceRoot {
    Host(PathBuf),
    NamedVolume(String),
    /// A named volume which belongs to an application placed on an SSH server.
    /// The application id is both the authorization scope and the sole source
    /// of the remote runtime; callers cannot select a host themselves.
    RemoteNamedVolume {
        application_id: String,
        volume: String,
    },
    /// A named volume on an SSH server, reached from the container console.
    /// The server id is the authorization scope and the source of the runtime.
    RemoteServerVolume {
        server_id: String,
        volume: String,
    },
}

impl WorkspaceRoot {
    fn clone_root(&self) -> WorkspaceRoot {
        match self {
            WorkspaceRoot::Host(path) => WorkspaceRoot::Host(path.clone()),
            WorkspaceRoot::NamedVolume(name) => WorkspaceRoot::NamedVolume(name.clone()),
            WorkspaceRoot::RemoteNamedVolume {
                application_id,
                volume,
            } => WorkspaceRoot::RemoteNamedVolume {
                application_id: application_id.clone(),
                volume: volume.clone(),
            },
            WorkspaceRoot::RemoteServerVolume { server_id, volume } => {
                WorkspaceRoot::RemoteServerVolume {
                    server_id: server_id.clone(),
                    volume: volume.clone(),
                }
            }
        }
    }
}

#[derive(serde::Serialize)]
pub struct WorkspaceListing {
    pub entries: Vec<WorkspaceEntryDto>,
    pub logs: Vec<String>,
}

const HELPER_IMAGE: &str = "docker.io/library/alpine:3.21";

struct HelperLifecycleTiming {
    image_preparation_ms: u64,
    create_ms: u64,
    start_ms: u64,
}

pub fn canonical_path(value: &str) -> Result<PathBuf, AppError> {
    let path = Path::new(value);
    if value.is_empty() || path.is_absolute() {
        return Err(AppError::ValidationError(
            "Use a non-empty relative file path without traversal".into(),
        ));
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(AppError::ValidationError(
                    "Use a non-empty relative file path without traversal".into(),
                ));
            }
        }
    }
    if normalized.as_os_str().is_empty() {
        return Err(AppError::ValidationError(
            "Use a non-empty relative file path without traversal".into(),
        ));
    }
    Ok(normalized)
}

fn host_root(root: &Path) -> Result<PathBuf, AppError> {
    std::fs::canonicalize(root)
        .map_err(|error| AppError::InternalError(format!("Could not resolve volume root: {error}")))
}

fn existing_host_path(root: &Path, path: &Path) -> Result<PathBuf, AppError> {
    let root = host_root(root)?;
    let resolved = std::fs::canonicalize(root.join(path)).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            AppError::ResourceNotFound
        } else {
            AppError::InternalError(error.to_string())
        }
    })?;
    if !resolved.starts_with(&root) {
        return Err(AppError::ValidationError(
            "File path escapes the mounted volume".into(),
        ));
    }
    Ok(resolved)
}

fn writable_host_path(root: &Path, path: &Path) -> Result<PathBuf, AppError> {
    let root = host_root(root)?;
    let mut current = root.clone();
    let parents: Vec<_> = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part),
            _ => None,
        })
        .collect();
    let (name, directories) = parents.split_last().ok_or_else(|| {
        AppError::ValidationError("Use a non-empty relative file path without traversal".into())
    })?;
    for directory in directories {
        current.push(directory);
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(AppError::ValidationError(
                    "File path escapes the mounted volume".into(),
                ));
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err(AppError::ValidationError(
                    "A parent path is not a directory".into(),
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(&current)
                    .map_err(|error| AppError::InternalError(error.to_string()))?
            }
            Err(error) => return Err(AppError::InternalError(error.to_string())),
        }
    }
    let target = current.join(name);
    if std::fs::symlink_metadata(&target)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(AppError::ValidationError(
            "File path escapes the mounted volume".into(),
        ));
    }
    Ok(target)
}

pub fn mime(path: &Path) -> Option<String> {
    match path
        .extension()
        .and_then(|item| item.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "html" | "htm" => Some("text/html"),
        "css" => Some("text/css"),
        "js" | "mjs" => Some("text/javascript"),
        "ts" => Some("text/typescript"),
        "json" => Some("application/json"),
        "md" => Some("text/markdown"),
        "txt" | "log" | "env" => Some("text/plain"),
        "yaml" | "yml" => Some("application/yaml"),
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "svg" => Some("image/svg+xml"),
        "pdf" => Some("application/pdf"),
        _ => None,
    }
    .map(str::to_owned)
}

pub fn is_text_file(path: &Path, bytes: &[u8]) -> bool {
    !bytes.contains(&0)
        && (mime(path)
            .as_deref()
            .map(|kind| {
                kind.starts_with("text/")
                    || matches!(
                        kind,
                        "application/json" | "application/yaml" | "image/svg+xml"
                    )
            })
            .unwrap_or_else(|| std::str::from_utf8(bytes).is_ok()))
}

fn entry(path: &Path, directory: bool, size: Option<u64>) -> WorkspaceEntryDto {
    WorkspaceEntryDto {
        path: path.to_string_lossy().to_string(),
        name: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string(),
        kind: if directory { "directory" } else { "file" }.into(),
        mime_type: if directory { None } else { mime(path) },
        byte_size: size,
    }
}

fn list_host(
    root: &Path,
    relative: &Path,
    depth: usize,
) -> Result<Vec<WorkspaceEntryDto>, AppError> {
    fn visit(
        root: &Path,
        dir: &Path,
        remaining_depth: usize,
        out: &mut Vec<WorkspaceEntryDto>,
    ) -> Result<(), AppError> {
        if remaining_depth == 0 {
            return Ok(());
        }
        for child in
            std::fs::read_dir(dir).map_err(|error| AppError::InternalError(error.to_string()))?
        {
            let child = child.map_err(|error| AppError::InternalError(error.to_string()))?;
            let metadata = child
                .path()
                .symlink_metadata()
                .map_err(|error| AppError::InternalError(error.to_string()))?;
            if metadata.file_type().is_symlink() {
                continue;
            }
            let relative = child
                .path()
                .strip_prefix(root)
                .map_err(|error| AppError::InternalError(error.to_string()))?
                .to_path_buf();
            out.push(entry(
                &relative,
                metadata.is_dir(),
                (!metadata.is_dir()).then_some(metadata.len()),
            ));
            if metadata.is_dir() {
                visit(root, &child.path(), remaining_depth - 1, out)?;
            }
        }
        Ok(())
    }
    let mut result = Vec::new();
    let root = host_root(root)?;
    let directory = if relative.as_os_str().is_empty() {
        root.clone()
    } else {
        existing_host_path(&root, relative)?
    };
    if !directory.is_dir() {
        return Err(AppError::ValidationError("Path is not a directory".into()));
    }
    visit(&root, &directory, depth, &mut result)?;
    result.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(result)
}

/// Reports whether a mountpoint podman advertised is actually reachable from this
/// process. It is not on macOS, where podman runs inside a VM, nor when the API is
/// containerized without the storage bind-mounted, nor when permissions deny it.
fn mountpoint_is_readable(mountpoint: &Path) -> bool {
    mountpoint.is_dir() && std::fs::read_dir(mountpoint).is_ok()
}

async fn named_volume_mountpoint(state: &SharedState, volume: &str) -> Option<PathBuf> {
    let podman = Podman::new(&state.podman_socket).ok()?;
    let inspected = podman.volumes().get(volume).inspect().await.ok()?;
    Some(inspected.mountpoint)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// Turns a named volume into a direct host path whenever its contents are reachable
/// without a helper container, so listings and reads use plain filesystem calls.
/// The decision only changes when the volume or the storage layout is recreated, so
/// it is cached per volume for the lifetime of the process.
async fn resolve_root(state: &SharedState, root: WorkspaceRoot) -> WorkspaceRoot {
    let WorkspaceRoot::NamedVolume(volume) = &root else {
        return root;
    };

    if let Some(cached) = state.volume_access.read().await.get(volume) {
        return match cached {
            Some(path) => WorkspaceRoot::Host(path.clone()),
            None => root,
        };
    }

    let started = Instant::now();
    let mountpoint = named_volume_mountpoint(state, volume).await;
    let native = mountpoint.filter(|path| mountpoint_is_readable(path));
    let (mode, reason) = match &native {
        Some(_) => ("native", "mountpoint readable"),
        None => ("helper", "mountpoint unreachable from the API host"),
    };
    tracing::info!(
        operation = "volume_workspace.access_mode",
        volume,
        mode,
        reason,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "volume workspace access mode"
    );
    state
        .volume_access
        .write()
        .await
        .insert(volume.clone(), native.clone());
    match native {
        Some(path) => WorkspaceRoot::Host(path),
        None => root,
    }
}

/// Drops a volume back to helper-container access, used when a native write is
/// rejected by the filesystem even though reads succeeded.
async fn demote_to_helper(state: &SharedState, volume: &str) {
    tracing::warn!(
        operation = "volume_workspace.access_mode",
        volume,
        mode = "helper",
        reason = "native write denied",
        "volume workspace access mode"
    );
    state
        .volume_access
        .write()
        .await
        .insert(volume.to_string(), None);
}

async fn with_helper<T>(
    state: &SharedState,
    volume: &str,
    operation: impl FnOnce(
        &podman_api::api::Container,
    ) -> futures_util::future::BoxFuture<'_, Result<T, AppError>>,
) -> Result<T, AppError> {
    with_helper_timing(state, volume, operation)
        .await
        .map(|(result, _)| result)
}

async fn with_helper_timing<T>(
    state: &SharedState,
    volume: &str,
    operation: impl FnOnce(
        &podman_api::api::Container,
    ) -> futures_util::future::BoxFuture<'_, Result<T, AppError>>,
) -> Result<(T, HelperLifecycleTiming), AppError> {
    let podman = Podman::new(&state.podman_socket)
        .map_err(|error| AppError::InternalError(format!("Podman is unavailable: {error}")))?;
    let images = podman.images();
    let image_preparation_started = Instant::now();
    if images.get(HELPER_IMAGE).inspect().await.is_err() {
        tracing::info!(
            operation = "volume_workspace.helper_container.image_inspect",
            elapsed_ms = image_preparation_started.elapsed().as_millis() as u64,
            outcome = "missing",
            "volume workspace timing"
        );
        let pull_started = Instant::now();
        let pull_options = PullOpts::builder().reference(HELPER_IMAGE).build();
        let mut pull = images.pull(&pull_options);
        while let Some(report) = pull.next().await {
            if let Err(error) = report {
                tracing::warn!(
                    operation = "volume_workspace.helper_container.image_pull",
                    elapsed_ms = pull_started.elapsed().as_millis() as u64,
                    outcome = "error",
                    "volume workspace timing"
                );
                return Err(AppError::InternalError(format!(
                    "Could not pull volume helper image: {error}"
                )));
            }
        }
        tracing::info!(
            operation = "volume_workspace.helper_container.image_pull",
            elapsed_ms = pull_started.elapsed().as_millis() as u64,
            outcome = "ok",
            "volume workspace timing"
        );
    } else {
        tracing::info!(
            operation = "volume_workspace.helper_container.image_inspect",
            elapsed_ms = image_preparation_started.elapsed().as_millis() as u64,
            outcome = "ok",
            "volume workspace timing"
        );
    }
    let image_preparation_ms = image_preparation_started.elapsed().as_millis() as u64;
    let name = format!("litepod-volume-workspace-{}", Uuid::now_v7());
    let named_volume = models::NamedVolume {
        dest: Some("/workspace".into()),
        is_anonymous: Some(false),
        name: Some(volume.to_string()),
        options: Some(vec!["rw".into()]),
    };
    let opts = ContainerCreateOpts::builder()
        .name(&name)
        .image(HELPER_IMAGE)
        .command(["sleep", "infinity"])
        .volumes(vec![named_volume])
        .build();
    let create_started = Instant::now();
    if let Err(error) = podman.containers().create(&opts).await {
        tracing::warn!(
            operation = "volume_workspace.helper_container.create",
            elapsed_ms = create_started.elapsed().as_millis() as u64,
            outcome = "error",
            "volume workspace timing"
        );
        return Err(AppError::InternalError(format!(
            "Could not mount volume: {error}"
        )));
    }
    tracing::info!(
        operation = "volume_workspace.helper_container.create",
        elapsed_ms = create_started.elapsed().as_millis() as u64,
        outcome = "ok",
        "volume workspace timing"
    );
    let create_ms = create_started.elapsed().as_millis() as u64;
    let container = podman.containers().get(&name);
    let result = async {
        let start_started = Instant::now();
        if let Err(error) = container.start(None).await {
            tracing::warn!(
                operation = "volume_workspace.helper_container.start",
                elapsed_ms = start_started.elapsed().as_millis() as u64,
                outcome = "error",
                "volume workspace timing"
            );
            return Err(AppError::InternalError(format!(
                "Could not start volume helper: {error}"
            )));
        }
        let start_ms = start_started.elapsed().as_millis() as u64;
        tracing::info!(
            operation = "volume_workspace.helper_container.start",
            elapsed_ms = start_ms,
            outcome = "ok",
            "volume workspace timing"
        );
        Ok((operation(&container).await?, start_ms))
    }
    .await;
    // The helper runs `sleep infinity`, which installs no SIGTERM handler, so a graceful
    // stop always burns podman's full 10s kill grace before every list/read returns.
    // Force-delete with timeout 0 sends SIGKILL immediately and removes in one call.
    let teardown_started = Instant::now();
    match container
        .delete(
            &ContainerDeleteOpts::builder()
                .force(true)
                .timeout(0_usize)
                .build(),
        )
        .await
    {
        Ok(()) => tracing::info!(
            operation = "volume_workspace.helper_container.teardown",
            elapsed_ms = teardown_started.elapsed().as_millis() as u64,
            outcome = "ok",
            "volume workspace timing"
        ),
        Err(_) => tracing::warn!(
            operation = "volume_workspace.helper_container.teardown",
            elapsed_ms = teardown_started.elapsed().as_millis() as u64,
            outcome = "error",
            "volume workspace timing"
        ),
    }
    result.map(|(result, start_ms)| {
        (
            result,
            HelperLifecycleTiming {
                image_preparation_ms,
                create_ms,
                start_ms,
            },
        )
    })
}

async fn list_named_volume_entries(
    state: &SharedState,
    volume: &str,
    relative: &Path,
    depth: usize,
) -> Result<WorkspaceListing, AppError> {
    let total_started = Instant::now();
    let relative = relative.to_path_buf();
    let listing = with_helper_timing(state, volume, |container| {
        Box::pin(async move {
            let start = Path::new("/workspace")
                .join(&relative)
                .to_string_lossy()
                .to_string();
            let mut out = Vec::new();
            let mut logs = Vec::new();
            for (file_type, directory) in [("d", true), ("f", false)] {
                let scan_started = Instant::now();
                let depth = depth.to_string();
                let entry_kind = if directory { "directory" } else { "file" };
                let exec = match container
                    .create_exec(
                        &ExecCreateOpts::builder()
                            .command([
                                "find",
                                &start,
                                "-mindepth",
                                "1",
                                "-maxdepth",
                                &depth,
                                "-type",
                                file_type,
                                "-print0",
                            ])
                            .attach_stdout(true)
                            .build(),
                    )
                    .await
                {
                    Ok(exec) => exec,
                    Err(error) => {
                        tracing::warn!(
                            operation = "volume_workspace.list.find",
                            entry_kind,
                            elapsed_ms = scan_started.elapsed().as_millis() as u64,
                            outcome = "error",
                            "volume workspace timing"
                        );
                        return Err(AppError::InternalError(error.to_string()));
                    }
                };
                let stream = match exec.start(&Default::default()).await {
                    Ok(Some(stream)) => stream,
                    Ok(None) => {
                        tracing::warn!(
                            operation = "volume_workspace.list.find",
                            entry_kind,
                            elapsed_ms = scan_started.elapsed().as_millis() as u64,
                            outcome = "error",
                            "volume workspace timing"
                        );
                        return Err(AppError::InternalError(
                            "Could not start volume directory listing".into(),
                        ));
                    }
                    Err(error) => {
                        tracing::warn!(
                            operation = "volume_workspace.list.find",
                            entry_kind,
                            elapsed_ms = scan_started.elapsed().as_millis() as u64,
                            outcome = "error",
                            "volume workspace timing"
                        );
                        return Err(AppError::InternalError(error.to_string()));
                    }
                };
                let mut stream = stream;
                let mut bytes = Vec::new();
                while let Some(chunk) = stream.next().await {
                    match chunk {
                        Ok(TtyChunk::StdOut(data)) => bytes.extend(data),
                        Ok(TtyChunk::StdErr(data)) => {
                            tracing::warn!(
                                operation = "volume_workspace.list.find",
                                entry_kind,
                                elapsed_ms = scan_started.elapsed().as_millis() as u64,
                                outcome = "error",
                                "volume workspace timing"
                            );
                            return Err(AppError::InternalError(
                                String::from_utf8_lossy(&data).to_string(),
                            ));
                        }
                        Ok(_) => {}
                        Err(error) => {
                            tracing::warn!(
                                operation = "volume_workspace.list.find",
                                entry_kind,
                                elapsed_ms = scan_started.elapsed().as_millis() as u64,
                                outcome = "error",
                                "volume workspace timing"
                            );
                            return Err(AppError::InternalError(error.to_string()));
                        }
                    }
                }
                let mut scan_entries = 0usize;
                for path in bytes
                    .split(|byte| *byte == 0)
                    .filter(|path| !path.is_empty())
                {
                    let path = match std::str::from_utf8(path) {
                        Ok(path) => path,
                        Err(error) => {
                            tracing::warn!(
                                operation = "volume_workspace.list.find",
                                entry_kind,
                                elapsed_ms = scan_started.elapsed().as_millis() as u64,
                                outcome = "error",
                                "volume workspace timing"
                            );
                            return Err(AppError::InternalError(error.to_string()));
                        }
                    };
                    let path = path
                        .strip_prefix("/workspace/")
                        .or_else(|| path.strip_prefix("/workspace"))
                        .unwrap_or(path)
                        .trim_start_matches('/');
                    if path.is_empty() {
                        continue;
                    }
                    let path = match canonical_path(path) {
                        Ok(path) => path,
                        Err(error) => {
                            tracing::warn!(
                                operation = "volume_workspace.list.find",
                                entry_kind,
                                elapsed_ms = scan_started.elapsed().as_millis() as u64,
                                outcome = "error",
                                "volume workspace timing"
                            );
                            return Err(error);
                        }
                    };
                    out.push(entry(&path, directory, None));
                    scan_entries += 1;
                }
                tracing::info!(
                    operation = "volume_workspace.list.find",
                    entry_kind,
                    entry_count = scan_entries,
                    elapsed_ms = scan_started.elapsed().as_millis() as u64,
                    outcome = "ok",
                    "volume workspace timing"
                );
                logs.push(format!(
                    "find {}: {}ms",
                    if directory { "directories" } else { "files" },
                    scan_started.elapsed().as_millis()
                ));
            }
            out.sort_by(|a, b| a.path.cmp(&b.path));
            Ok((out, logs))
        })
    })
    .await;
    let ((entries, mut logs), helper_timing) = match listing {
        Ok(listing) => listing,
        Err(error) => {
            tracing::warn!(
                operation = "volume_workspace.list.total",
                elapsed_ms = total_started.elapsed().as_millis() as u64,
                outcome = "error",
                "volume workspace timing"
            );
            return Err(error);
        }
    };
    logs.splice(
        0..0,
        [
            format!(
                "helper image preparation: {}ms",
                helper_timing.image_preparation_ms
            ),
            format!("helper container create: {}ms", helper_timing.create_ms),
            format!("helper container start: {}ms", helper_timing.start_ms),
        ],
    );
    logs.push(format!(
        "named-volume list total: {}ms",
        total_started.elapsed().as_millis()
    ));
    tracing::info!(
        operation = "volume_workspace.list.total",
        entry_count = entries.len(),
        elapsed_ms = total_started.elapsed().as_millis() as u64,
        outcome = "ok",
        "volume workspace timing"
    );
    Ok(WorkspaceListing { entries, logs })
}

fn remote_volume_mount(volume: &str) -> Result<OsString, AppError> {
    if volume.is_empty()
        || volume.contains(|character: char| {
            character.is_whitespace() || matches!(character, ':' | '/' | '\\' | '\0')
        })
    {
        return Err(AppError::InternalError(
            "Refusing an unsafe remote volume name".into(),
        ));
    }
    Ok(OsString::from(format!("{volume}:/workspace")))
}

/// Timeout for the tar-based content read (`remote_bytes_for`). A single
/// file can legitimately take longer than a probe-sized command, unlike the
/// `find`-based listing below which never transfers file content.
const REMOTE_VOLUME_READ_TIMEOUT: Duration = Duration::from_secs(180);
/// Timeout for the remote `find` listing. It only walks metadata, so it
/// stays well within a probe-sized budget regardless of volume size.
const REMOTE_VOLUME_LIST_TIMEOUT: Duration = Duration::from_secs(45);

/// Resolves either remote root variant to its runtime and the key for the SSH
/// staging directory (`apps/<uuid>` for an application, `console/srv-<id>` for
/// the container console).
async fn remote_target(
    state: &SharedState,
    root: &WorkspaceRoot,
) -> Result<(ServerRuntime, String), AppError> {
    match root {
        WorkspaceRoot::RemoteNamedVolume { application_id, .. } => Ok((
            ServerRuntime::for_application(state, application_id).await?,
            application_id.clone(),
        )),
        WorkspaceRoot::RemoteServerVolume { server_id, .. } => Ok((
            ServerRuntime::for_server(state, server_id).await?,
            format!("srv-{server_id}"),
        )),
        _ => Err(AppError::InternalError(
            "remote_target called on a non-remote root".into(),
        )),
    }
}

async fn remote_volume_archive(
    runtime: &ServerRuntime,
    volume: &str,
    path: &Path,
) -> Result<Vec<u8>, AppError> {
    if !runtime.is_remote() {
        return Err(AppError::InternalError(
            "Remote volume workspace resolved to the Local server".into(),
        ));
    }
    let requested = if path.as_os_str().is_empty() {
        OsString::from(".")
    } else {
        path.as_os_str().to_owned()
    };
    Ok(runtime
        .remote_podman_bytes(
            &[
                OsString::from("run"),
                OsString::from("--rm"),
                OsString::from("-v"),
                remote_volume_mount(volume)?,
                OsString::from(HELPER_IMAGE),
                OsString::from("tar"),
                OsString::from("-C"),
                OsString::from("/workspace"),
                OsString::from("-cf"),
                OsString::from("-"),
                OsString::from("--"),
                requested,
            ],
            REMOTE_VOLUME_READ_TIMEOUT,
        )
        .await?
        .stdout)
}

/// Depth-bounded remote listing. Mirrors the local `list_named_volume_entries`
/// two-pass `find` (dirs, then files) instead of tarring the whole subtree
/// (as `remote_volume_archive` does) just to read back file names — a full
/// tar of a large volume routinely blew the SSH command timeout to show a
/// directory listing.
async fn remote_find_entries(
    runtime: &ServerRuntime,
    volume: &str,
    relative: &Path,
    depth: usize,
) -> Result<Vec<WorkspaceEntryDto>, AppError> {
    if !runtime.is_remote() {
        return Err(AppError::InternalError(
            "Remote volume workspace resolved to the Local server".into(),
        ));
    }
    let start = Path::new("/workspace")
        .join(relative)
        .to_string_lossy()
        .to_string();
    let mut out = Vec::new();
    for (file_type, directory) in [("d", true), ("f", false)] {
        let bytes = runtime
            .remote_podman_bytes(
                &[
                    OsString::from("run"),
                    OsString::from("--rm"),
                    OsString::from("-v"),
                    remote_volume_mount(volume)?,
                    OsString::from(HELPER_IMAGE),
                    OsString::from("find"),
                    OsString::from(&start),
                    OsString::from("-mindepth"),
                    OsString::from("1"),
                    OsString::from("-maxdepth"),
                    OsString::from(depth.to_string()),
                    OsString::from("-type"),
                    OsString::from(file_type),
                    OsString::from("-print0"),
                ],
                REMOTE_VOLUME_LIST_TIMEOUT,
            )
            .await?
            .stdout;
        for path in bytes
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
        {
            let path = std::str::from_utf8(path)
                .map_err(|error| AppError::InternalError(error.to_string()))?;
            let path = path
                .strip_prefix("/workspace/")
                .or_else(|| path.strip_prefix("/workspace"))
                .unwrap_or(path)
                .trim_start_matches('/');
            if path.is_empty() {
                continue;
            }
            let path = canonical_path(path)?;
            out.push(entry(&path, directory, None));
        }
    }
    out.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(out)
}

async fn remote_list_entries(
    runtime: &ServerRuntime,
    volume: &str,
    path: &Path,
    depth: usize,
) -> Result<WorkspaceListing, AppError> {
    let started = Instant::now();
    let entries = remote_find_entries(runtime, volume, path, depth).await?;
    Ok(WorkspaceListing {
        entries,
        logs: vec![format!(
            "remote named-volume list total: {}ms",
            started.elapsed().as_millis()
        )],
    })
}

async fn remote_bytes_for(
    runtime: &ServerRuntime,
    volume: &str,
    path: &Path,
) -> Result<Vec<u8>, AppError> {
    let bytes = remote_volume_archive(runtime, volume, path).await?;
    let mut archive = Archive::new(Cursor::new(bytes));
    let mut entries = archive
        .entries()
        .map_err(|error| AppError::InternalError(error.to_string()))?;
    let mut file = entries
        .next()
        .ok_or(AppError::ResourceNotFound)?
        .map_err(|error| AppError::InternalError(error.to_string()))?;
    if file.header().entry_type().is_dir() || file.header().entry_type().is_symlink() {
        return Err(AppError::ResourceNotFound);
    }
    let mut output = Vec::new();
    file.read_to_end(&mut output)
        .map_err(|error| AppError::InternalError(error.to_string()))?;
    Ok(output)
}

async fn write_remote_archive(
    runtime: &ServerRuntime,
    staging_key: &str,
    volume: &str,
    archive: Vec<u8>,
) -> Result<(), AppError> {
    let workspace = runtime
        .ensure_workspace(staging_key)
        .await?
        .ok_or_else(|| AppError::InternalError("Remote volume write resolved to Local".into()))?;
    let archive_name = format!("volume-edit-{}.tar", Uuid::now_v7());
    let local = std::env::temp_dir().join(&archive_name);
    std::fs::write(&local, archive).map_err(|error| AppError::InternalError(error.to_string()))?;
    let upload = runtime
        .upload_workspace_file(staging_key, &local, &archive_name)
        .await;
    let _ = std::fs::remove_file(&local);
    upload?;
    let extracted = runtime
        .podman(&[
            OsString::from("run"),
            OsString::from("--rm"),
            OsString::from("-v"),
            remote_volume_mount(volume)?,
            OsString::from("-v"),
            OsString::from(format!("{workspace}:/staging:ro")),
            OsString::from(HELPER_IMAGE),
            OsString::from("tar"),
            OsString::from("-xf"),
            OsString::from(format!("/staging/{archive_name}")),
            OsString::from("-C"),
            OsString::from("/workspace"),
        ])
        .await;
    let cleanup = runtime
        .remote_command(&[
            OsString::from("rm"),
            OsString::from("-f"),
            OsString::from(format!("{workspace}/{archive_name}")),
        ])
        .await;
    extracted?;
    cleanup.map(|_| ())
}

pub async fn list_entries(
    state: &SharedState,
    root: WorkspaceRoot,
    raw_path: Option<&str>,
    depth: usize,
) -> Result<WorkspaceListing, AppError> {
    let started = Instant::now();
    let path = raw_path
        .map(canonical_path)
        .transpose()?
        .unwrap_or_default();
    let named = matches!(root, WorkspaceRoot::NamedVolume(_));
    match resolve_root(state, root).await {
        WorkspaceRoot::Host(root) => {
            let entries = list_host(&root, &path, depth)?;
            let label = if named {
                "native volume list total"
            } else {
                "host-volume list total"
            };
            Ok(WorkspaceListing {
                entries,
                logs: vec![format!("{label}: {}ms", started.elapsed().as_millis())],
            })
        }
        WorkspaceRoot::NamedVolume(volume) => {
            list_named_volume_entries(state, &volume, &path, depth).await
        }
        root @ (WorkspaceRoot::RemoteNamedVolume { .. }
        | WorkspaceRoot::RemoteServerVolume { .. }) => {
            let volume = match &root {
                WorkspaceRoot::RemoteNamedVolume { volume, .. }
                | WorkspaceRoot::RemoteServerVolume { volume, .. } => volume.clone(),
                _ => unreachable!(),
            };
            let (runtime, _key) = remote_target(state, &root).await?;
            remote_list_entries(&runtime, &volume, &path, depth).await
        }
    }
}

async fn bytes_for(
    state: &SharedState,
    root: WorkspaceRoot,
    path: &Path,
) -> Result<Vec<u8>, AppError> {
    match resolve_root(state, root).await {
        WorkspaceRoot::Host(root) => {
            std::fs::read(existing_host_path(&root, path)?).map_err(|error| {
                if error.kind() == std::io::ErrorKind::NotFound {
                    AppError::ResourceNotFound
                } else {
                    AppError::InternalError(error.to_string())
                }
            })
        }
        WorkspaceRoot::NamedVolume(volume) => {
            let container_path = Path::new("/workspace").join(path);
            with_helper(state, &volume, |container| {
                Box::pin(async move {
                    let bytes = container
                        .copy_from(container_path)
                        .try_concat()
                        .await
                        .map_err(|error| AppError::InternalError(error.to_string()))?;
                    let mut archive = Archive::new(Cursor::new(bytes));
                    let mut entries = archive
                        .entries()
                        .map_err(|error| AppError::InternalError(error.to_string()))?;
                    let mut file = entries
                        .next()
                        .ok_or(AppError::ResourceNotFound)?
                        .map_err(|error| AppError::InternalError(error.to_string()))?;
                    let mut out = Vec::new();
                    file.read_to_end(&mut out)
                        .map_err(|error| AppError::InternalError(error.to_string()))?;
                    Ok(out)
                })
            })
            .await
        }
        root @ (WorkspaceRoot::RemoteNamedVolume { .. }
        | WorkspaceRoot::RemoteServerVolume { .. }) => {
            let volume = match &root {
                WorkspaceRoot::RemoteNamedVolume { volume, .. }
                | WorkspaceRoot::RemoteServerVolume { volume, .. } => volume.clone(),
                _ => unreachable!(),
            };
            let (runtime, _key) = remote_target(state, &root).await?;
            remote_bytes_for(&runtime, &volume, path).await
        }
    }
}

pub async fn read_file(
    state: &SharedState,
    root: WorkspaceRoot,
    raw_path: &str,
) -> Result<WorkspaceFileDto, AppError> {
    let path = canonical_path(raw_path)?;
    let bytes = bytes_for(state, root, &path).await?;
    let is_text = is_text_file(&path, &bytes);
    Ok(WorkspaceFileDto {
        path: raw_path.into(),
        mime_type: mime(&path),
        is_text,
        content: is_text.then(|| String::from_utf8_lossy(&bytes).to_string()),
        byte_size: bytes.len() as u64,
    })
}

pub async fn read_file_bytes(
    state: &SharedState,
    root: WorkspaceRoot,
    raw_path: &str,
) -> Result<(Vec<u8>, Option<String>), AppError> {
    let path = canonical_path(raw_path)?;
    Ok((bytes_for(state, root, &path).await?, mime(&path)))
}

fn tar_one(path: &Path, bytes: &[u8], directory: bool) -> Result<Vec<u8>, AppError> {
    let mut builder = Builder::new(Vec::new());
    let mut header = Header::new_gnu();
    header.set_mode(if directory { 0o755 } else { 0o644 });
    header.set_size(if directory { 0 } else { bytes.len() as u64 });
    header.set_entry_type(if directory {
        tar::EntryType::Directory
    } else {
        tar::EntryType::Regular
    });
    header.set_cksum();
    builder
        .append_data(&mut header, path, Cursor::new(bytes))
        .map_err(|error| AppError::InternalError(error.to_string()))?;
    builder
        .into_inner()
        .map_err(|error| AppError::InternalError(error.to_string()))
}

/// Writes through whichever access mode `root` names. Native writes report
/// `PermissionDenied` separately so the caller can fall back to a helper container.
async fn write_through(
    state: &SharedState,
    root: WorkspaceRoot,
    path: &Path,
    bytes: Vec<u8>,
    directory: bool,
) -> Result<(), (AppError, bool)> {
    let internal = |error: std::io::Error| {
        (
            AppError::InternalError(error.to_string()),
            error.kind() == std::io::ErrorKind::PermissionDenied,
        )
    };
    match root {
        WorkspaceRoot::Host(root) => {
            let target = writable_host_path(&root, path).map_err(|error| (error, false))?;
            if directory {
                std::fs::create_dir(&target).map_err(internal)?;
            } else {
                std::fs::write(target, bytes).map_err(internal)?;
            }
        }
        WorkspaceRoot::NamedVolume(volume) => {
            let archive = tar_one(path, &bytes, directory).map_err(|error| (error, false))?;
            with_helper(state, &volume, |container| {
                Box::pin(async move {
                    container
                        .copy_to("/workspace", archive.into())
                        .await
                        .map_err(|error| AppError::InternalError(error.to_string()))
                })
            })
            .await
            .map_err(|error| (error, false))?;
        }
        root @ (WorkspaceRoot::RemoteNamedVolume { .. }
        | WorkspaceRoot::RemoteServerVolume { .. }) => {
            let volume = match &root {
                WorkspaceRoot::RemoteNamedVolume { volume, .. }
                | WorkspaceRoot::RemoteServerVolume { volume, .. } => volume.clone(),
                _ => unreachable!(),
            };
            let archive = tar_one(path, &bytes, directory).map_err(|error| (error, false))?;
            let (runtime, key) = remote_target(state, &root)
                .await
                .map_err(|error| (error, false))?;
            write_remote_archive(&runtime, &key, &volume, archive)
                .await
                .map_err(|error| (error, false))?;
        }
    }
    Ok(())
}

async fn write_new_or_existing(
    state: &SharedState,
    root: WorkspaceRoot,
    path: &Path,
    bytes: Vec<u8>,
    allow_existing: bool,
    directory: bool,
) -> Result<WorkspaceEntryDto, AppError> {
    let byte_size = bytes.len() as u64;
    let parent = path.parent().and_then(|parent| {
        (!parent.as_os_str().is_empty()).then_some(parent.to_string_lossy().to_string())
    });
    let existing = list_entries(state, root.clone_root(), parent.as_deref(), 1).await?;
    if existing
        .entries
        .iter()
        .any(|item| item.path == path.to_string_lossy())
        && !allow_existing
    {
        return Err(AppError::AlreadyExists);
    }
    let resolved = resolve_root(state, root.clone_root()).await;
    match write_through(state, resolved, path, bytes.clone(), directory).await {
        Ok(()) => {}
        // A readable mountpoint does not guarantee a writable one; retry once through
        // the helper container, which writes with the volume's own ownership.
        Err((error, true)) => match &root {
            WorkspaceRoot::NamedVolume(volume) => {
                demote_to_helper(state, volume).await;
                write_through(state, root.clone_root(), path, bytes, directory)
                    .await
                    .map_err(|(error, _)| error)?;
            }
            WorkspaceRoot::Host(_)
            | WorkspaceRoot::RemoteNamedVolume { .. }
            | WorkspaceRoot::RemoteServerVolume { .. } => return Err(error),
        },
        Err((error, _)) => return Err(error),
    }
    Ok(entry(path, directory, (!directory).then_some(byte_size)))
}

pub async fn write_text_file(
    state: &SharedState,
    root: WorkspaceRoot,
    raw_path: &str,
    content: String,
) -> Result<WorkspaceFileDto, AppError> {
    let path = canonical_path(raw_path)?;
    let existing = bytes_for(state, root.clone_root(), &path).await?;
    if !is_text_file(&path, &existing) {
        return Err(AppError::ValidationError(
            "Only existing text files can be edited".into(),
        ));
    }
    let bytes = content.into_bytes();
    write_new_or_existing(state, root, &path, bytes.clone(), true, false).await?;
    Ok(WorkspaceFileDto {
        path: raw_path.into(),
        mime_type: mime(&path),
        is_text: true,
        content: Some(String::from_utf8_lossy(&bytes).to_string()),
        byte_size: bytes.len() as u64,
    })
}

pub async fn create_entry(
    state: &SharedState,
    root: WorkspaceRoot,
    raw_path: &str,
    kind: &str,
    content: Option<String>,
) -> Result<WorkspaceEntryDto, AppError> {
    let path = canonical_path(raw_path)?;
    match kind {
        "file" => {
            write_new_or_existing(
                state,
                root,
                &path,
                content.unwrap_or_default().into_bytes(),
                false,
                false,
            )
            .await
        }
        "directory" => write_new_or_existing(state, root, &path, Vec::new(), false, true).await,
        _ => Err(AppError::ValidationError(
            "kind must be file or directory".into(),
        )),
    }
}
pub async fn upload_new_file(
    state: &SharedState,
    root: WorkspaceRoot,
    raw_path: &str,
    bytes: Vec<u8>,
) -> Result<WorkspaceEntryDto, AppError> {
    let path = canonical_path(raw_path)?;
    write_new_or_existing(state, root, &path, bytes, false, false).await
}

#[cfg(test)]
mod tests {
    use super::{
        canonical_path, existing_host_path, is_text_file, mountpoint_is_readable,
        remote_volume_mount,
    };
    use std::path::{Path, PathBuf};
    use uuid::Uuid;

    #[test]
    fn canonical_paths_reject_absolute_and_parent_paths() {
        assert!(canonical_path("/etc/passwd").is_err());
        assert!(canonical_path("assets/../secret").is_err());
        assert_eq!(
            canonical_path("./assets/site.css").unwrap(),
            PathBuf::from("assets/site.css")
        );
        assert_eq!(
            canonical_path("assets/site.css").unwrap(),
            PathBuf::from("assets/site.css")
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_host_paths_cannot_escape_the_volume_root() {
        use std::{fs, os::unix::fs::symlink};
        let root =
            std::env::temp_dir().join(format!("litepod-volume-workspace-test-{}", Uuid::now_v7()));
        let outside = std::env::temp_dir().join(format!(
            "litepod-volume-workspace-outside-{}",
            Uuid::now_v7()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("secret.txt"), "secret").unwrap();
        symlink(&outside, root.join("escaped")).unwrap();

        assert!(existing_host_path(&root, Path::new("escaped/secret.txt")).is_err());

        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[test]
    fn mountpoints_are_native_only_when_reachable_from_this_process() {
        let reachable =
            std::env::temp_dir().join(format!("litepod-volume-mountpoint-{}", Uuid::now_v7()));
        std::fs::create_dir_all(&reachable).unwrap();
        assert!(mountpoint_is_readable(&reachable));

        // The shape podman reports on macOS: a path that only exists inside the VM.
        assert!(!mountpoint_is_readable(&reachable.join("missing/_data")));

        let file = reachable.join("not-a-directory");
        std::fs::write(&file, b"x").unwrap();
        assert!(!mountpoint_is_readable(&file));

        std::fs::remove_dir_all(reachable).unwrap();
    }

    #[test]
    fn text_detection_rejects_binary_payloads() {
        assert!(is_text_file(Path::new("site.css"), b"body {}"));
        assert!(!is_text_file(Path::new("logo.png"), b"\x89PNG\0"));
    }

    #[test]
    fn remote_volume_helpers_keep_mounts_and_paths_scoped() {
        assert_eq!(
            remote_volume_mount("litepod-app-data").unwrap(),
            "litepod-app-data:/workspace"
        );
        assert!(remote_volume_mount("data:/host").is_err());
    }
}
