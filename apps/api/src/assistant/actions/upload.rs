//! The upload half of `deploy_static_site` and `deploy_zip`, and of an icon on a
//! project or application action.
//!
//! The agent gets a URL with a one-time token when it proposes, and `curl`s
//! the site there. The bytes wait under the action's id until a person
//! confirms; nothing is created from them before that. The token is the only
//! credential, so it dies with the proposal: pending, unexpired, this action.

use std::path::PathBuf;

use sha2::{Digest, Sha256};
use sqlx::Row;

use crate::{dtos::response::AppError, state::state::SharedState};

use super::lifecycle::now_ms;
use super::types::*;

pub(super) fn hash_token(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn staging_dir(state: &SharedState, action_id: &str) -> PathBuf {
    state.data_dir.join("assistant_uploads").join(action_id)
}

/// Where the agent sends the site, ready to paste into `curl -T`.
pub async fn upload_url(state: &SharedState, action_id: &str, token: &str) -> String {
    format!(
        "{}/api/v1/assistant/uploads/{action_id}/{token}",
        crate::handler::settings::public_base_url(state)
            .await
            .trim_end_matches('/')
    )
}

/// What every upload-carrying proposal stores, whatever else it holds.
#[derive(serde::Deserialize)]
struct UploadToken {
    #[serde(default)]
    upload_token_hash: String,
}

/// The actions that may carry an icon.
const ICON_TOOLS: [&str; 4] = [
    CREATE_PROJECT,
    UPDATE_PROJECT,
    CREATE_APPLICATION,
    UPDATE_APPLICATION,
];
const ICON_FILE: &str = "icon";

/// Keeps an upload for a pending `deploy_static_site`, `deploy_zip` or icon. A
/// second upload replaces the first, so a wrong zip is fixed by sending the
/// right one.
pub async fn stage_upload(
    state: &SharedState,
    action_id: &str,
    token: &str,
    bytes: &[u8],
) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(
        "SELECT arguments, status, expires_at, tool FROM assistant_actions WHERE id = ?1 AND tool IN (?2, ?3, ?4, ?5, ?6, ?7)",
    )
    .bind(action_id)
    .bind(DEPLOY_STATIC_SITE)
    .bind(DEPLOY_ZIP)
    .bind(ICON_TOOLS[0])
    .bind(ICON_TOOLS[1])
    .bind(ICON_TOOLS[2])
    .bind(ICON_TOOLS[3])
    .fetch_optional(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?
    .ok_or(AppError::ResourceNotFound)?;
    let arguments: String = row
        .try_get(0)
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    let status: String = row
        .try_get(1)
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    let expires_at: i64 = row
        .try_get(2)
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    let tool: String = row
        .try_get(3)
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    let site: UploadToken = serde_json::from_str(&arguments)
        .map_err(|error| AppError::InternalError(error.to_string()))?;
    // A wrong token answers like a missing action: nothing to learn by guessing.
    if site.upload_token_hash.is_empty() || site.upload_token_hash != hash_token(token) {
        return Err(AppError::ResourceNotFound);
    }
    if status != "pending" || now_ms() > expires_at {
        return Err(AppError::ValidationError(
            "This upload link is no longer open. Propose the site again.".into(),
        ));
    }

    if ICON_TOOLS.contains(&tool.as_str()) {
        // Refused now, so the agent can fix it before anyone confirms.
        crate::icons::prepare(bytes)?;
        return write_staged(state, action_id, ICON_FILE, bytes).await;
    }

    let filename = match crate::deployment::file_kind("upload", bytes) {
        Ok("zip") => "site.zip",
        _ if tool == DEPLOY_ZIP => {
            return Err(AppError::ValidationError(
                "Send the project as a ZIP archive.".into(),
            ));
        }
        Ok(_) => "index.html",
        Err(error) => return Err(error),
    };
    // A source project is allowed to be bigger than a site.
    if tool == DEPLOY_STATIC_SITE && bytes.len() > MAX_SITE_BYTES {
        return Err(AppError::ValidationError(
            "The site is larger than 10 MB, the upload limit.".into(),
        ));
    }
    if bytes.len() > crate::deployment::source_archive::MAX_ARCHIVE_BYTES {
        return Err(AppError::ValidationError(
            "The ZIP is larger than 100 MB, the upload limit.".into(),
        ));
    }
    write_staged(state, action_id, filename, bytes).await
}

async fn write_staged(
    state: &SharedState,
    action_id: &str,
    filename: &str,
    bytes: &[u8],
) -> Result<(), AppError> {
    let dir = staging_dir(state, action_id);
    let _ = tokio::fs::remove_dir_all(&dir).await;
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|error| AppError::InternalError(error.to_string()))?;
    tokio::fs::write(dir.join(filename), bytes)
        .await
        .map_err(|error| AppError::InternalError(error.to_string()))?;
    Ok(())
}

/// How a tool call wants to set an icon, checked before anything is stored.
#[derive(Default)]
pub struct IconRequest {
    /// `icon` or `icon_url`, kept in the proposal.
    pub value: Option<String>,
    pub upload_token_hash: String,
    token: String,
    inline: Option<Vec<u8>>,
}

impl IconRequest {
    /// Reads `icon`, `icon_url`, `icon_base64` and `icon_upload`: one at most.
    pub fn parse(arguments: &serde_json::Value) -> Result<Self, AppError> {
        use base64::Engine;

        let text = |key: &str| arguments.get(key).and_then(|value| value.as_str());
        let name = text("icon");
        let url = text("icon_url").map(str::trim);
        let encoded = text("icon_base64");
        let upload = arguments
            .get("icon_upload")
            .and_then(|value| value.as_bool())
            == Some(true);
        let given = usize::from(name.is_some())
            + usize::from(url.is_some())
            + usize::from(encoded.is_some())
            + usize::from(upload);
        if given > 1 {
            return Err(AppError::ValidationError(
                "Pass only one of `icon`, `icon_url`, `icon_base64` or `icon_upload`".into(),
            ));
        }
        if let Some(name) = name
            && !crate::icons::NAMES.contains(&name)
        {
            return Err(AppError::ValidationError(format!(
                "`{name}` is not an icon. Use one of: {}",
                crate::icons::NAMES.join(", ")
            )));
        }
        if let Some(url) = url.filter(|url| !url.is_empty()) {
            crate::security::ssrf::parse_https_url(url)
                .map_err(|error| AppError::ValidationError(format!("`icon_url`: {error}")))?;
        }
        let inline = match encoded {
            None => None,
            Some(encoded) => {
                // Generators often hand back a data URL rather than bare base64.
                let bare = encoded
                    .split_once(";base64,")
                    .map_or(encoded, |(_, bare)| bare);
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(bare.trim())
                    .map_err(|error| {
                        AppError::ValidationError(format!("`icon_base64` is not base64: {error}"))
                    })?;
                crate::icons::prepare(&bytes)?;
                Some(bytes)
            }
        };
        let token = if upload || inline.is_some() {
            format!(
                "{}{}",
                uuid::Uuid::new_v4().simple(),
                uuid::Uuid::new_v4().simple()
            )
        } else {
            String::new()
        };
        Ok(Self {
            value: name.or(url).map(str::to_owned),
            upload_token_hash: if token.is_empty() {
                String::new()
            } else {
                hash_token(&token)
            },
            token,
            inline,
        })
    }

    /// Stages an inline image under the stored proposal. `Some` is the address
    /// the agent still has to send the image to.
    pub async fn finish(
        self,
        state: &SharedState,
        action_id: &str,
    ) -> Result<Option<String>, AppError> {
        if let Some(bytes) = &self.inline {
            stage_upload(state, action_id, &self.token, bytes).await?;
            return Ok(None);
        }
        if self.token.is_empty() {
            return Ok(None);
        }
        Ok(Some(upload_url(state, action_id, &self.token).await))
    }
}

/// What a confirmed action does to the icon.
pub(super) enum IconChange {
    Keep,
    Clear,
    Named(String),
    Image(Vec<u8>),
}

impl IconChange {
    /// The update request's tri-state field; an image goes through its own call.
    pub(super) fn field(&self) -> Option<Option<String>> {
        match self {
            Self::Clear => Some(None),
            Self::Named(name) => Some(Some(name.clone())),
            Self::Keep | Self::Image(_) => None,
        }
    }
}

/// Resolves the icon of a confirmed action, before it changes anything: a dead
/// link must not leave a project behind.
pub(super) async fn take_icon(
    state: &SharedState,
    action_id: &str,
    value: Option<&str>,
    upload_token_hash: &str,
) -> Result<IconChange, AppError> {
    if !upload_token_hash.is_empty() {
        let path = staging_dir(state, action_id).join(ICON_FILE);
        let Ok(bytes) = tokio::fs::read(&path).await else {
            return Err(AppError::ValidationError(
                "The icon has not been uploaded yet. The agent has to send it to the upload URL first."
                    .into(),
            ));
        };
        discard(state, action_id).await;
        return Ok(IconChange::Image(bytes));
    }
    match value.map(str::trim) {
        None => Ok(IconChange::Keep),
        Some("") => Ok(IconChange::Clear),
        Some(name) if crate::icons::NAMES.contains(&name) => Ok(IconChange::Named(name.to_owned())),
        Some(url) => fetch_icon(url)
            .await
            .map(IconChange::Image)
            .map_err(AppError::ValidationError),
    }
}

/// Downloads an image through `security::ssrf` and keeps it to the icon limit.
/// Litepod serves its own copy, so a link that expires or a third party that
/// logs visitors never reaches the people looking at the icon.
async fn fetch_icon(raw: &str) -> Result<Vec<u8>, String> {
    use futures_util::StreamExt;

    let url = crate::security::ssrf::parse_https_url(raw)?;
    let (_, response) =
        crate::assistant::web::get_public(url, "image/png, image/jpeg, image/webp, image/svg+xml")
            .await?;
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        body.extend_from_slice(&chunk.map_err(|_| "The image stopped downloading".to_string())?);
        if body.len() > crate::icons::MAX_ICON_BYTES {
            return Err("The image at `icon_url` is larger than the 2 MiB icon limit".into());
        }
    }
    crate::icons::prepare(&body).map_err(|error| error.to_string())?;
    Ok(body)
}

/// The upload route's own limit, so a site is refused the same either way.
const MAX_SITE_BYTES: usize = 10 * 1024 * 1024;

/// A site sent inside the tool call instead of to the upload URL, for an agent
/// whose sandbox cannot reach this host: `zip_base64`, or `files` as path to
/// text content, zipped here. `None` when the call carries neither.
pub fn inline_site(arguments: &serde_json::Value) -> Result<Option<Vec<u8>>, AppError> {
    inline_files(arguments, true)
}

/// The same for a source project: `files` needs no `index.html`, and the ZIP
/// may be up to the source upload limit.
pub fn inline_project(arguments: &serde_json::Value) -> Result<Option<Vec<u8>>, AppError> {
    inline_files(arguments, false)
}

fn inline_files(arguments: &serde_json::Value, site: bool) -> Result<Option<Vec<u8>>, AppError> {
    use base64::Engine;
    use std::io::{Cursor, Write};

    if let Some(encoded) = arguments.get("zip_base64").and_then(|value| value.as_str()) {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded.trim())
            .map_err(|error| {
                AppError::ValidationError(format!("`zip_base64` is not base64: {error}"))
            })?;
        return sized(bytes, site).map(Some);
    }
    let Some(files) = arguments.get("files").and_then(|value| value.as_object()) else {
        return Ok(None);
    };
    if site && !files.contains_key("index.html") {
        return Err(AppError::ValidationError(
            "`files` needs an `index.html` at the root".into(),
        ));
    }
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (path, content) in files {
        let Some(content) = content.as_str() else {
            return Err(AppError::ValidationError(format!(
                "`files.{path}` must be the file's text"
            )));
        };
        // Checked again when the site is unpacked; refusing here keeps a bad
        // path from ever becoming a proposal.
        if path.starts_with('/') || path.split(['/', '\\']).any(|part| part == "..") {
            return Err(AppError::ValidationError(format!(
                "`{path}` must be a relative path inside the site"
            )));
        }
        zip.start_file(path.as_str(), options)
            .map_err(|error| AppError::InternalError(error.to_string()))?;
        zip.write_all(content.as_bytes())
            .map_err(|error| AppError::InternalError(error.to_string()))?;
    }
    let bytes = zip
        .finish()
        .map_err(|error| AppError::InternalError(error.to_string()))?
        .into_inner();
    sized(bytes, site).map(Some)
}

fn sized(bytes: Vec<u8>, site: bool) -> Result<Vec<u8>, AppError> {
    if bytes.is_empty() {
        return Err(AppError::ValidationError("The upload is empty".into()));
    }
    if !site && bytes.len() > crate::deployment::source_archive::MAX_ARCHIVE_BYTES {
        return Err(AppError::ValidationError(
            "The project is larger than 100 MB, the upload limit.".into(),
        ));
    }
    if site && bytes.len() > MAX_SITE_BYTES {
        return Err(AppError::ValidationError(
            "The site is larger than 10 MB, the upload limit.".into(),
        ));
    }
    Ok(bytes)
}

/// The staged site, as `(filename, bytes)`; `None` until the agent uploads.
pub(super) async fn staged(state: &SharedState, action_id: &str) -> Option<(String, Vec<u8>)> {
    let dir = staging_dir(state, action_id);
    for filename in ["site.zip", "index.html"] {
        if let Ok(bytes) = tokio::fs::read(dir.join(filename)).await {
            return Some((filename.to_owned(), bytes));
        }
    }
    None
}

// shortcut: an upload whose proposal expires without anyone opening it again
// stays on disk; sweep `assistant_uploads` by age if that ever adds up.
pub(super) async fn discard(state: &SharedState, action_id: &str) {
    let _ = tokio::fs::remove_dir_all(staging_dir(state, action_id)).await;
}
