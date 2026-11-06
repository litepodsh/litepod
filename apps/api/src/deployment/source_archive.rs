//! An uploaded ZIP as the source tree of a git-type application.
//!
//! The archive is kept as one file per application and unpacked into the same
//! workspace a clone would use, so every build method runs unchanged.

use std::{
    io::Cursor,
    path::{Component, Path, PathBuf},
};

use sha2::{Digest, Sha256};

use crate::{dtos::response::AppError, state::state::SharedState};

/// The upload limit for a source archive; sites keep their own, smaller one.
pub const MAX_ARCHIVE_BYTES: usize = 100 * 1024 * 1024;
const MAX_ENTRIES: usize = 20_000;
/// Declared sizes are checked before anything is written, so a small ZIP that
/// expands to gigabytes (a zip bomb) is refused.
const MAX_UNPACKED_BYTES: u64 = 1024 * 1024 * 1024;
/// How many past uploads are kept for rollback, newest first.
const KEPT_VERSIONS: usize = 10;
/// How long a one-time download link stays good when nobody uses it.
const DOWNLOAD_LINK_MS: i64 = 10 * 60 * 1000;

fn archive_path(state: &SharedState, application_id: &str) -> PathBuf {
    state
        .data_dir
        .join("applications")
        .join(application_id)
        .join("source.zip")
}

fn versions_dir(state: &SharedState, application_id: &str) -> PathBuf {
    archive_path(state, application_id)
        .parent()
        .expect("archive path has a parent")
        .join("source-archives")
}

/// The first 12 hex characters of the SHA-256: the id a deploy run records for
/// the upload it built, and the name its copy is kept under.
fn short_id(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .take(6)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The kept copy of the upload a run built, for a rollback. `None` when it was
/// pruned, or when `id` is not one this module could have produced.
pub fn version_file(state: &SharedState, application_id: &str, id: &str) -> Option<PathBuf> {
    if id.len() != 12 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let path = versions_dir(state, application_id).join(format!("{id}.zip"));
    path.is_file().then_some(path)
}

fn is_zip(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && bytes[0..2] == [0x50, 0x4b] && matches!(&bytes[2..4], [3, 4] | [5, 6])
}

/// Keeps `bytes` as the application's source and marks it as upload-sourced.
/// A second upload replaces the first.
pub async fn store(
    state: &SharedState,
    application_id: &str,
    bytes: &[u8],
) -> Result<(), AppError> {
    if bytes.is_empty() {
        return Err(AppError::ValidationError("The archive is empty".into()));
    }
    if bytes.len() > MAX_ARCHIVE_BYTES {
        return Err(AppError::ValidationError(
            "The archive is larger than 100 MB, the upload limit. Leave out node_modules, build output and other generated folders.".into(),
        ));
    }
    // The magic bytes only say "some ZIP container" (a .docx or .jar passes), so
    // the directory is read too: a broken, unsafe or empty archive fails here
    // instead of in the middle of a deploy.
    if !is_zip(bytes) {
        return Err(AppError::ValidationError(
            "The source must be a ZIP archive".into(),
        ));
    }
    inspect(bytes).map_err(AppError::ValidationError)?;
    let path = archive_path(state, application_id);
    let parent = path.parent().expect("archive path has a parent");
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|error| AppError::InternalError(error.to_string()))?;
    tokio::fs::write(&path, bytes)
        .await
        .map_err(|error| AppError::InternalError(error.to_string()))?;
    keep_version(state, application_id, bytes)
        .await
        .map_err(|error| AppError::InternalError(error.to_string()))?;
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE applications SET source_upload = 1, deploy_type = 'git', git_provider_id = NULL, git_url = NULL, git_branch = NULL, git_repo_full_name = NULL, git_credentials_username = NULL, git_credentials_password_encrypted = NULL, auto_deploy = 0, updated_at = ?1 WHERE id = ?2",
    )
    .bind(chrono::Utc::now().timestamp_millis())
    .bind(application_id)
    .execute(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    Ok(())
}

/// Keeps this upload under its id and drops the oldest beyond [`KEPT_VERSIONS`].
async fn keep_version(
    state: &SharedState,
    application_id: &str,
    bytes: &[u8],
) -> std::io::Result<()> {
    let dir = versions_dir(state, application_id);
    tokio::fs::create_dir_all(&dir).await?;
    tokio::fs::write(dir.join(format!("{}.zip", short_id(bytes))), bytes).await?;

    let mut kept = Vec::new();
    let mut entries = tokio::fs::read_dir(&dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let modified = entry.metadata().await?.modified()?;
        kept.push((modified, entry.path()));
    }
    kept.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, path) in kept.into_iter().skip(KEPT_VERSIONS) {
        let _ = tokio::fs::remove_file(path).await;
    }
    Ok(())
}

fn download_token_hash(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// A link to the application's stored ZIP, ready to `curl`, for the connection
/// behind `access_token_id`: an agent never sees its own token, so it cannot send
/// one. Links nobody used are swept here, so the table only holds live ones.
pub async fn create_download_link(
    state: &SharedState,
    application_id: &str,
    access_token_id: &str,
) -> Result<String, AppError> {
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let now = chrono::Utc::now().timestamp_millis();
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("DELETE FROM source_archive_downloads WHERE expires_at <= ?1")
        .bind(now)
        .execute(&conn)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    sqlx::query(
        "INSERT INTO source_archive_downloads (token_hash, application_id, access_token_id, expires_at) VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(download_token_hash(&token))
    .bind(application_id)
    .bind(access_token_id)
    .bind(now + DOWNLOAD_LINK_MS)
    .execute(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    Ok(format!(
        "{}/api/v1/assistant/downloads/{token}",
        crate::handler::settings::public_base_url(state)
            .await
            .trim_end_matches('/')
    ))
}

/// The application a download token is good for, spending the token. `None` when
/// it is unknown, already used or expired, or when the connection that asked for
/// it may no longer read the application. One statement, so two requests racing
/// on the same link cannot both get it.
pub async fn redeem_download(state: &SharedState, token: &str) -> Result<Option<String>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query_as::<_, (String, String)>(
        "DELETE FROM source_archive_downloads WHERE token_hash = ?1 AND expires_at > ?2 RETURNING application_id, access_token_id",
    )
    .bind(download_token_hash(token))
    .bind(chrono::Utc::now().timestamp_millis())
    .fetch_optional(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    let Some((application_id, access_token_id)) = row else {
        return Ok(None);
    };
    Ok(
        connection_may_read(state, &application_id, &access_token_id)
            .await?
            .then_some(application_id),
    )
}

/// The same checks a request with this token would pass: the token is open, its
/// user is active, MCP is on, and the member may still read this application.
async fn connection_may_read(
    state: &SharedState,
    application_id: &str,
    access_token_id: &str,
) -> Result<bool, AppError> {
    use crate::git::repositories::{db, personal_access_token, user};
    use crate::security::{
        authz::{Permission, extract::member_permissions, set::Target},
        middleware::auth::AuthUser,
    };

    let Some(token) = personal_access_token::find_active_by_id(state, access_token_id).await?
    else {
        return Ok(false);
    };
    let Some(user_row) = user::get_user_by_id_result(state, &token.user_id).await? else {
        return Ok(false);
    };
    if !user::is_active(&user_row) {
        return Ok(false);
    }
    let conn = db::get_conn(state).await;
    let Some((organization_id, project_id, environment_id)) =
        sqlx::query_as::<_, (String, String, String)>(
            "SELECT p.organization_id, p.id, e.id FROM applications a \
             JOIN environments e ON e.id = a.environment_id \
             JOIN projects p ON p.id = e.project_id WHERE a.id = ?1",
        )
        .bind(application_id)
        .fetch_optional(&conn)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?
    else {
        return Ok(false);
    };
    if token
        .mcp_organization_id
        .as_ref()
        .is_some_and(|pinned| *pinned != organization_id)
        || crate::handler::mcp::mcp_on(state, &organization_id)
            .await
            .is_err()
    {
        return Ok(false);
    }
    let actor = AuthUser {
        is_sysadmin: user::is_sysadmin(state, &user_row.id).await,
        user_id: user_row.id,
        email: user_row.email,
        name: user_row.name,
        token_id: Some(token.id),
        token_name: Some(token.name),
    };
    let scopes: Vec<String> = serde_json::from_str(&token.scopes).unwrap_or_default();
    // A member refused here (gone, or missing a 2FA now required) is a "no", not an error.
    let Ok((_, permissions)) =
        member_permissions(state, &actor, &organization_id, Some(&scopes)).await
    else {
        return Ok(false);
    };
    Ok(permissions.allows(
        Permission::AppRead,
        Target::Environment {
            project_id: &project_id,
            environment_id: &environment_id,
        },
    ))
}

/// Whether the application builds from an uploaded archive.
pub async fn is_upload(state: &SharedState, application_id: &str) -> Result<bool, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query_scalar::<_, i64>("SELECT source_upload FROM applications WHERE id = ?1")
        .bind(application_id)
        .fetch_optional(&conn)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?
        .map(|flag| flag == 1)
        .ok_or(AppError::ApplicationNotFound)
}

/// Unpacks the stored archive into `destination` (which must not exist) and
/// returns a short id for it, the first 12 hex characters of its SHA-256.
///
/// A ZIP whose files all sit under one folder (what GitHub and most "compress
/// folder" tools produce) is unpacked from inside that folder, so the project
/// path means the same thing as it does for a repository.
///
/// Blocking: call inside `tokio::task::spawn_blocking`.
pub fn unpack(archive: &Path, destination: &Path) -> Result<String, String> {
    let bytes = std::fs::read(archive)
        .map_err(|error| format!("The uploaded source is missing, upload it again: {error}"))?;
    unpack_bytes(&bytes, destination, &Limits::SOURCE)
}

/// How big an archive may be once opened. A source tree is allowed a lot; a page
/// somebody serves to visitors is not.
pub struct Limits {
    pub max_entries: usize,
    pub max_unpacked_bytes: u64,
}

impl Limits {
    pub const SOURCE: Limits = Limits {
        max_entries: MAX_ENTRIES,
        max_unpacked_bytes: MAX_UNPACKED_BYTES,
    };
}

/// [`unpack`] for an archive already in memory, with its own [`Limits`].
///
/// Blocking: call inside `tokio::task::spawn_blocking`.
pub fn unpack_bytes(bytes: &[u8], destination: &Path, limits: &Limits) -> Result<String, String> {
    let (mut zip, paths) = inspect_with(bytes, limits)?;
    let root = shared_root(&paths);

    std::fs::create_dir_all(destination).map_err(|error| error.to_string())?;
    for (index, path) in paths.iter().enumerate() {
        let relative: PathBuf = match &root {
            Some(root) => path.strip_prefix(root).unwrap_or(path).to_path_buf(),
            None => path.clone(),
        };
        if relative.as_os_str().is_empty() {
            continue;
        }
        let mut entry = zip.by_index(index).map_err(|error| error.to_string())?;
        let output = destination.join(&relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&output).map_err(|error| error.to_string())?;
            continue;
        }
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let mut file = std::fs::File::create(&output).map_err(|error| error.to_string())?;
        std::io::copy(&mut entry, &mut file).map_err(|error| error.to_string())?;
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            // Keeps scripts executable; the mask drops setuid/setgid/sticky.
            let _ =
                std::fs::set_permissions(&output, std::fs::Permissions::from_mode(mode & 0o777));
        }
    }

    Ok(short_id(bytes))
}

/// Reads the ZIP's directory and checks it without unpacking anything: valid,
/// within the entry and size limits, every path safe, and at least one file.
/// Returns the archive and the cleaned path of each entry, by index.
fn inspect(bytes: &[u8]) -> Result<(zip::ZipArchive<Cursor<&[u8]>>, Vec<PathBuf>), String> {
    inspect_with(bytes, &Limits::SOURCE)
}

fn inspect_with<'a>(
    bytes: &'a [u8],
    limits: &Limits,
) -> Result<(zip::ZipArchive<Cursor<&'a [u8]>>, Vec<PathBuf>), String> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| format!("The uploaded source is not a valid ZIP: {error}"))?;
    if zip.len() > limits.max_entries {
        return Err(format!(
            "The archive has more than {} entries",
            limits.max_entries
        ));
    }
    let mut declared = 0u64;
    let mut files = 0usize;
    let mut paths = Vec::with_capacity(zip.len());
    for index in 0..zip.len() {
        let entry = zip.by_index(index).map_err(|error| error.to_string())?;
        declared = declared.saturating_add(entry.size());
        let path = clean_path(entry.name())?;
        if entry.is_file() && !path.as_os_str().is_empty() {
            files += 1;
        }
        paths.push(path);
    }
    if declared > limits.max_unpacked_bytes {
        return Err(format!(
            "The archive unpacks to more than {}",
            human_bytes(limits.max_unpacked_bytes)
        ));
    }
    if files == 0 {
        return Err("The archive has no files in it".into());
    }
    Ok((zip, paths))
}

fn human_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{} GB", bytes / (1024 * 1024 * 1024))
    } else {
        format!("{} MB", bytes / (1024 * 1024))
    }
}

pub fn archive_file(state: &SharedState, application_id: &str) -> PathBuf {
    archive_path(state, application_id)
}

/// A relative path with no `..`, no root and no drive prefix. Platform
/// metadata folders are skipped by the caller through an empty path.
fn clean_path(name: &str) -> Result<PathBuf, String> {
    let path = Path::new(name);
    let mut clean = PathBuf::new();
    for part in path.components() {
        match part {
            Component::Normal(part) => clean.push(part),
            Component::CurDir => {}
            _ => return Err(format!("The archive contains an unsafe path: {name}")),
        }
    }
    if clean
        .components()
        .next()
        .is_some_and(|first| first.as_os_str() == "__MACOSX")
    {
        return Ok(PathBuf::new());
    }
    Ok(clean)
}

/// The single top-level folder every entry sits under, if there is one.
fn shared_root(paths: &[PathBuf]) -> Option<PathBuf> {
    let mut root: Option<&std::ffi::OsStr> = None;
    for path in paths.iter().filter(|path| !path.as_os_str().is_empty()) {
        let mut parts = path.components();
        let first = parts.next()?.as_os_str();
        // A file at the top level means there is no wrapping folder.
        parts.next()?;
        match root {
            None => root = Some(first),
            Some(existing) if existing == first => {}
            Some(_) => return None,
        }
    }
    root.map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip_of(files: &[(&str, &str)]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        for (name, content) in files {
            zip.start_file(*name, options).unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    #[tokio::test]
    async fn an_expired_download_link_is_refused() {
        let (state, path) = crate::testing::test_state().await;
        let (_, _, application_id) =
            crate::testing::seed_application(&state, "organization-a").await;
        crate::testing::seed_mcp_connection(&state, "organization-a", "editor", "connection").await;
        let link = create_download_link(&state, &application_id, "connection")
            .await
            .unwrap();
        let token = link.rsplit('/').next().unwrap();
        sqlx::query("UPDATE source_archive_downloads SET expires_at = 0")
            .execute(&state.db)
            .await
            .unwrap();

        assert_eq!(redeem_download(&state, token).await.unwrap(), None);
        crate::testing::cleanup_database(&path);
    }

    /// Each step leaves the connection good except for one thing, so a link only
    /// works while the person behind it could still make the same request.
    #[tokio::test]
    async fn a_download_link_dies_with_the_connection_behind_it() {
        let (state, path) = crate::testing::test_state().await;
        let (_, _, application_id) =
            crate::testing::seed_application(&state, "organization-a").await;
        crate::testing::seed_mcp_connection(&state, "organization-a", "editor", "connection").await;
        crate::testing::seed_mcp_connection(&state, "organization-a", "outsider", "stranger").await;
        crate::testing::seed_mcp_connection(&state, "organization-b", "editor", "elsewhere").await;
        let token_for = async |connection: &str| {
            let link = create_download_link(&state, &application_id, connection)
                .await
                .unwrap();
            link.rsplit('/').next().unwrap().to_owned()
        };

        let good = token_for("connection").await;
        assert_eq!(
            redeem_download(&state, &good).await.unwrap().as_deref(),
            Some(application_id.as_str())
        );

        for connection in ["stranger", "elsewhere"] {
            let token = token_for(connection).await;
            assert_eq!(
                redeem_download(&state, &token).await.unwrap(),
                None,
                "{connection}"
            );
        }

        let mcp_off = token_for("connection").await;
        sqlx::query("UPDATE organizations SET mcp_enabled = 0 WHERE id = 'organization-a'")
            .execute(&state.db)
            .await
            .unwrap();
        assert_eq!(redeem_download(&state, &mcp_off).await.unwrap(), None);
        sqlx::query("UPDATE organizations SET mcp_enabled = 1 WHERE id = 'organization-a'")
            .execute(&state.db)
            .await
            .unwrap();

        let revoked = token_for("connection").await;
        crate::git::repositories::personal_access_token::revoke(&state, "connection")
            .await
            .unwrap();
        assert_eq!(redeem_download(&state, &revoked).await.unwrap(), None);
        crate::testing::cleanup_database(&path);
    }

    fn scratch() -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("litepod-archive-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn unpacked(files: &[(&str, &str)]) -> (PathBuf, PathBuf) {
        let dir = scratch();
        let archive = dir.join("a.zip");
        std::fs::write(&archive, zip_of(files)).unwrap();
        let out = dir.join("out");
        unpack(&archive, &out).unwrap();
        (dir, out)
    }

    #[tokio::test]
    async fn an_earlier_upload_stays_available_for_rollback() {
        let (state, path) = crate::testing::test_state().await;
        let first = zip_of(&[("a.txt", "one")]);
        let second = zip_of(&[("a.txt", "two")]);
        store(&state, "app-1", &first).await.unwrap();
        store(&state, "app-1", &second).await.unwrap();

        // The current file is the latest; the first is still kept by its id.
        assert_eq!(
            std::fs::read(archive_file(&state, "app-1")).unwrap(),
            second
        );
        let kept = version_file(&state, "app-1", &short_id(&first)).expect("first is kept");
        let out = scratch().join("out");
        unpack(&kept, &out).unwrap();
        assert_eq!(std::fs::read_to_string(out.join("a.txt")).unwrap(), "one");

        // Only ids this module produced name a file.
        assert!(version_file(&state, "app-1", "../../etc/pa").is_none());
        assert!(version_file(&state, "app-1", "000000000000").is_none());
        crate::testing::cleanup_database(&path);
    }

    #[test]
    fn a_wrapping_folder_is_stripped() {
        let (_dir, out) = unpacked(&[("app-main/package.json", "{}"), ("app-main/src/a.js", "")]);
        assert!(out.join("package.json").is_file());
        assert!(out.join("src/a.js").is_file());
    }

    #[test]
    fn a_flat_archive_is_kept_as_is() {
        let (_dir, out) = unpacked(&[("package.json", "{}"), ("src/a.js", "")]);
        assert!(out.join("package.json").is_file());
        assert!(out.join("src/a.js").is_file());
    }

    #[test]
    fn several_top_level_folders_are_kept() {
        let (_dir, out) = unpacked(&[("api/a.js", ""), ("web/b.js", "")]);
        assert!(out.join("api/a.js").is_file());
        assert!(out.join("web/b.js").is_file());
    }

    #[test]
    fn an_archive_with_no_files_is_refused() {
        assert!(inspect(&zip_of(&[])).is_err());
        assert!(inspect(b"PK\x03\x04 not really a zip").is_err());
        assert!(inspect(&zip_of(&[("a.txt", "x")])).is_ok());
    }

    #[test]
    fn a_path_that_climbs_out_is_refused() {
        let dir = scratch();
        let archive = dir.join("a.zip");
        std::fs::write(&archive, zip_of(&[("../evil", "x")])).unwrap();
        assert!(unpack(&archive, &dir.join("out")).is_err());
    }
}
