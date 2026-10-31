//! File access to a running container's own filesystem, for the file explorer.
//!
//! Listing goes through `podman exec ls` (so the image needs `ls`), reads and writes go through
//! `podman cp`, which moves a tar stream and works without a shell. Paths from the client are
//! relative to `/` and cannot contain `..`; the container is the boundary, not a volume root, so
//! there is nothing to escape from beyond what `podman exec` already gives the same caller.

use std::{ffi::OsString, io::Read, time::Duration};

use tar::{Archive, Builder, EntryType, Header};

use crate::{
    dtos::{
        deployment::{WorkspaceEntryDto, WorkspaceFileDto},
        response::AppError,
    },
    podman::containers::volume_workspace::{
        WorkspaceListing, canonical_path, is_text_file, mime,
    },
    servers::runtime::ServerRuntime,
};

/// Largest file the explorer will read or write in one go.
pub const MAX_FILE_BYTES: usize = 25 * 1024 * 1024;
/// Entries returned for one directory; `/proc`-sized folders are cut here.
const MAX_ENTRIES: usize = 5000;
const EXEC_BUDGET: Duration = Duration::from_secs(20);
const COPY_BUDGET: Duration = Duration::from_secs(120);

/// Where a container file lives, with the tar metadata to put it back as it was.
struct Fetched {
    bytes: Vec<u8>,
    mode: u32,
    uid: u64,
    gid: u64,
}

/// A client path made absolute inside the container. `None` and `""` mean `/`.
fn absolute(raw: Option<&str>) -> Result<String, AppError> {
    match raw.map(str::trim).filter(|path| !path.is_empty()) {
        None => Ok("/".into()),
        Some(raw) => Ok(format!("/{}", canonical_path(raw)?.to_string_lossy())),
    }
}

fn parent_and_name(path: &str) -> Result<(String, String), AppError> {
    let (parent, name) = path
        .rsplit_once('/')
        .filter(|(_, name)| !name.is_empty())
        .ok_or_else(|| AppError::ValidationError("Use a path to a file or folder".into()))?;
    Ok((
        if parent.is_empty() { "/" } else { parent }.to_owned(),
        name.to_owned(),
    ))
}

fn exec(container: &str, argv: &[&str]) -> Vec<OsString> {
    let mut args = vec![OsString::from("exec"), OsString::from(container)];
    args.extend(argv.iter().map(OsString::from));
    args
}

/// Turns the output of `ls -1Ap` into entries. Directories end with `/`; everything else,
/// symlinks included, is a file (reading one follows the link).
fn parse_listing(parent: &str, stdout: &str) -> Vec<WorkspaceEntryDto> {
    let mut entries: Vec<WorkspaceEntryDto> = stdout
        .lines()
        .filter(|line| !line.is_empty())
        .take(MAX_ENTRIES)
        .map(|line| {
            let (name, directory) = match line.strip_suffix('/') {
                Some(name) => (name, true),
                None => (line, false),
            };
            let path = format!("{}/{name}", parent.trim_end_matches('/'));
            let path = path.trim_start_matches('/').to_owned();
            WorkspaceEntryDto {
                mime_type: if directory { None } else { mime(std::path::Path::new(name)) },
                path,
                name: name.to_owned(),
                kind: if directory { "directory" } else { "file" }.into(),
                byte_size: None,
            }
        })
        .collect();
    entries.sort_by(|a, b| {
        (b.kind == "directory")
            .cmp(&(a.kind == "directory"))
            .then_with(|| a.name.cmp(&b.name))
    });
    entries
}

pub async fn list(
    runtime: &ServerRuntime,
    container: &str,
    raw_path: Option<&str>,
) -> Result<WorkspaceListing, AppError> {
    let path = absolute(raw_path)?;
    let output = runtime
        .podman_with_timeout(&exec(container, &["ls", "-1Ap", "--", &path]), EXEC_BUDGET)
        .await
        .map_err(|error| {
            AppError::ValidationError(format!(
                "Could not list {path}. The container must be running and its image needs `ls`: {error}"
            ))
        })?;
    let mut logs = Vec::new();
    if output.stdout.lines().count() > MAX_ENTRIES {
        logs.push(format!("Showing the first {MAX_ENTRIES} entries of {path}"));
    }
    Ok(WorkspaceListing {
        entries: parse_listing(&path, &output.stdout),
        logs,
    })
}

async fn fetch(runtime: &ServerRuntime, container: &str, path: &str) -> Result<Fetched, AppError> {
    // Size first: `podman cp` holds the whole tar in memory. A missing `wc` only skips the guard.
    if let Ok(output) = runtime
        .podman_with_timeout(&exec(container, &["wc", "-c", "--", path]), EXEC_BUDGET)
        .await
        && let Some(size) = output
            .stdout
            .split_whitespace()
            .next()
            .and_then(|value| value.parse::<usize>().ok())
        && size > MAX_FILE_BYTES
    {
        return Err(AppError::ValidationError(format!(
            "The file is {size} bytes; the explorer opens files up to {MAX_FILE_BYTES} bytes"
        )));
    }
    let tar = runtime.podman_cp_from(container, path, COPY_BUDGET).await?;
    let mut archive = Archive::new(tar.as_slice());
    let entries = archive
        .entries()
        .map_err(|error| AppError::InternalError(format!("Could not read the file: {error}")))?;
    for entry in entries {
        let mut entry = entry
            .map_err(|error| AppError::InternalError(format!("Could not read the file: {error}")))?;
        let header = entry.header().clone();
        match header.entry_type() {
            EntryType::Regular | EntryType::Continuous => {
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes).map_err(|error| {
                    AppError::InternalError(format!("Could not read the file: {error}"))
                })?;
                return Ok(Fetched {
                    bytes,
                    mode: header.mode().unwrap_or(0o644),
                    uid: header.uid().unwrap_or(0),
                    gid: header.gid().unwrap_or(0),
                });
            }
            EntryType::Directory => {
                return Err(AppError::ValidationError("That path is a folder".into()));
            }
            _ => {}
        }
    }
    Err(AppError::ResourceNotFound)
}

pub async fn read_file(
    runtime: &ServerRuntime,
    container: &str,
    raw_path: &str,
) -> Result<WorkspaceFileDto, AppError> {
    let path = absolute(Some(raw_path))?;
    let fetched = fetch(runtime, container, &path).await?;
    let name = std::path::Path::new(&path);
    let is_text = is_text_file(name, &fetched.bytes);
    Ok(WorkspaceFileDto {
        path: raw_path.into(),
        mime_type: mime(name),
        is_text,
        content: is_text.then(|| String::from_utf8_lossy(&fetched.bytes).into_owned()),
        byte_size: fetched.bytes.len() as u64,
    })
}

pub async fn read_file_bytes(
    runtime: &ServerRuntime,
    container: &str,
    raw_path: &str,
) -> Result<(Vec<u8>, Option<String>), AppError> {
    let path = absolute(Some(raw_path))?;
    let fetched = fetch(runtime, container, &path).await?;
    Ok((fetched.bytes, mime(std::path::Path::new(&path))))
}

fn tar_entry(name: &str, bytes: &[u8], directory: bool, meta: (u32, u64, u64)) -> Result<Vec<u8>, AppError> {
    let mut header = Header::new_gnu();
    header.set_mode(meta.0);
    header.set_uid(meta.1);
    header.set_gid(meta.2);
    header.set_size(if directory { 0 } else { bytes.len() as u64 });
    header.set_entry_type(if directory { EntryType::Directory } else { EntryType::Regular });
    header.set_cksum();
    let mut builder = Builder::new(Vec::new());
    builder
        .append_data(&mut header, name, bytes)
        .map_err(|error| AppError::InternalError(error.to_string()))?;
    builder
        .into_inner()
        .map_err(|error| AppError::InternalError(error.to_string()))
}

async fn put(
    runtime: &ServerRuntime,
    application_id: &str,
    container: &str,
    path: &str,
    bytes: &[u8],
    directory: bool,
    meta: (u32, u64, u64),
) -> Result<(), AppError> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(AppError::ValidationError(format!(
            "Files over {MAX_FILE_BYTES} bytes cannot be written from the explorer"
        )));
    }
    let (parent, name) = parent_and_name(path)?;
    let tar = tar_entry(&name, bytes, directory, meta)?;
    runtime
        .podman_cp_to(application_id, container, &parent, tar, COPY_BUDGET)
        .await
}

async fn exists(runtime: &ServerRuntime, container: &str, path: &str) -> bool {
    runtime
        .podman_with_timeout(&exec(container, &["ls", "-d", "--", path]), EXEC_BUDGET)
        .await
        .is_ok()
}

/// Overwrites an existing text file, keeping its mode and owner.
pub async fn write_text_file(
    runtime: &ServerRuntime,
    application_id: &str,
    container: &str,
    raw_path: &str,
    content: String,
) -> Result<WorkspaceFileDto, AppError> {
    let path = absolute(Some(raw_path))?;
    let existing = fetch(runtime, container, &path).await?;
    if !is_text_file(std::path::Path::new(&path), &existing.bytes) {
        return Err(AppError::ValidationError(
            "Only existing text files can be edited".into(),
        ));
    }
    let bytes = content.into_bytes();
    put(
        runtime,
        application_id,
        container,
        &path,
        &bytes,
        false,
        (existing.mode, existing.uid, existing.gid),
    )
    .await?;
    Ok(WorkspaceFileDto {
        path: raw_path.into(),
        mime_type: mime(std::path::Path::new(&path)),
        is_text: true,
        content: Some(String::from_utf8_lossy(&bytes).into_owned()),
        byte_size: bytes.len() as u64,
    })
}

/// Creates a file or folder that does not exist yet; never overwrites.
pub async fn create_entry(
    runtime: &ServerRuntime,
    application_id: &str,
    container: &str,
    raw_path: &str,
    kind: &str,
    bytes: Vec<u8>,
) -> Result<WorkspaceEntryDto, AppError> {
    let directory = match kind {
        "file" => false,
        "directory" => true,
        _ => {
            return Err(AppError::ValidationError(
                "kind must be file or directory".into(),
            ));
        }
    };
    let path = absolute(Some(raw_path))?;
    if exists(runtime, container, &path).await {
        return Err(AppError::ValidationError(format!("{path} already exists")));
    }
    let meta = if directory { (0o755, 0, 0) } else { (0o644, 0, 0) };
    put(runtime, application_id, container, &path, &bytes, directory, meta).await?;
    let name = std::path::Path::new(&path);
    Ok(WorkspaceEntryDto {
        path: raw_path.trim_start_matches('/').into(),
        name: name
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        kind: kind.into(),
        mime_type: if directory { None } else { mime(name) },
        byte_size: (!directory).then_some(bytes.len() as u64),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_paths_are_made_absolute_and_traversal_is_refused() {
        assert_eq!(absolute(None).unwrap(), "/");
        assert_eq!(absolute(Some("")).unwrap(), "/");
        assert_eq!(absolute(Some("etc/nginx")).unwrap(), "/etc/nginx");
        assert_eq!(absolute(Some("./etc//nginx")).unwrap(), "/etc/nginx");
        assert!(absolute(Some("/etc/passwd")).is_err());
        assert!(absolute(Some("etc/../root")).is_err());
    }

    #[test]
    fn a_path_splits_into_parent_and_name() {
        assert_eq!(
            parent_and_name("/etc/nginx/nginx.conf").unwrap(),
            ("/etc/nginx".into(), "nginx.conf".into())
        );
        assert_eq!(parent_and_name("/app").unwrap(), ("/".into(), "app".into()));
        assert!(parent_and_name("/").is_err());
    }

    #[test]
    fn listings_put_folders_first_and_use_paths_relative_to_root() {
        let entries = parse_listing("/etc", "zeta.conf\nnginx/\nalpha\n.hidden\n\n");
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["nginx", ".hidden", "alpha", "zeta.conf"]);
        assert_eq!(entries[0].kind, "directory");
        assert_eq!(entries[0].path, "etc/nginx");
        assert_eq!(parse_listing("/", "bin/\n")[0].path, "bin");
    }

    #[test]
    fn written_tar_keeps_the_mode_owner_and_name() {
        let tar = tar_entry("app.conf", b"hello", false, (0o600, 1000, 1001)).unwrap();
        let mut archive = Archive::new(tar.as_slice());
        let mut entry = archive.entries().unwrap().next().unwrap().unwrap();
        assert_eq!(entry.path().unwrap().to_string_lossy(), "app.conf");
        assert_eq!(entry.header().mode().unwrap(), 0o600);
        assert_eq!(entry.header().uid().unwrap(), 1000);
        assert_eq!(entry.header().gid().unwrap(), 1001);
        let mut body = String::new();
        entry.read_to_string(&mut body).unwrap();
        assert_eq!(body, "hello");
    }

    #[test]
    fn a_folder_tar_has_no_body() {
        let tar = tar_entry("data", b"ignored", true, (0o755, 0, 0)).unwrap();
        let mut archive = Archive::new(tar.as_slice());
        let entry = archive.entries().unwrap().next().unwrap().unwrap();
        assert_eq!(entry.header().entry_type(), EntryType::Directory);
        assert_eq!(entry.header().size().unwrap(), 0);
    }
}
