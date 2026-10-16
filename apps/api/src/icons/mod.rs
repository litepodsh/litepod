//! Icon uploads for organizations and applications, stored on local disk
//! under `data_dir/icons/{prefix}/{id}/{uuid}.{ext}` — the same volume every
//! install (self-hosted and cloud alike) already uses for the SQLite DB and
//! deployment artifacts (`deployment::operations::app_root`). No S3
//! dependency: the file is served back through an authenticated proxy route
//! (`handler::organizations::serve_icon_handler`,
//! `handler::icons::serve_application_icon_handler`), never a raw disk path
//! or bucket URL.
//!
//! Every upload is re-verified server-side by its magic bytes — the
//! client-declared Content-Type and filename are never trusted — and an SVG
//! is additionally run through an allowlist sanitizer before it ever touches
//! disk. See `sniff_image_kind` and `sanitize_svg`.

use std::collections::HashSet;

use uuid::Uuid;

use crate::dtos::response::AppError;
use crate::state::state::SharedState;

pub const MAX_ICON_BYTES: usize = 2 * 1024 * 1024;

fn icon_dir(state: &SharedState, prefix: &str, id: &str) -> std::path::PathBuf {
    state.data_dir.join("icons").join(prefix).join(id)
}

/// Rejects anything that isn't a real PNG/JPEG/WEBP/SVG, based on the actual
/// bytes — never the client's declared Content-Type or filename extension,
/// both of which are attacker-controlled.
pub fn sniff_image_kind(bytes: &[u8]) -> Result<&'static str, AppError> {
    const PNG_MAGIC: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    if bytes.len() >= 8 && bytes[0..8] == PNG_MAGIC {
        return Ok("png");
    }
    if bytes.len() >= 3 && bytes[0..3] == [0xFF, 0xD8, 0xFF] {
        return Ok("jpg");
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Ok("webp");
    }
    // SVG is text, not a fixed magic number — same leading-comment-skipping
    // shape as `deployment::operations::file_kind` uses for HTML uploads.
    let leading = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]);
    let mut rest = leading.trim_start_matches('\u{feff}').trim_start();
    while rest.starts_with("<!--") {
        match rest.find("-->") {
            Some(end) => rest = rest[end + 3..].trim_start(),
            None => break,
        }
    }
    let lower = rest.to_ascii_lowercase();
    if lower.starts_with("<?xml") || lower.starts_with("<svg") {
        return Ok("svg");
    }
    Err(AppError::ValidationError(
        "Upload a PNG, JPEG, WebP, or SVG image".into(),
    ))
}

// Structural SVG elements only — no `<script>`, `<foreignObject>`, `<image>`,
// `<a>`, `<style>`, or `<use>`. Dropping `<style>` closes off CSS-based
// exfiltration (`@import`, `background: url(...)`); dropping `<use>` and any
// href/xlink:href attribute closes off both `javascript:` hrefs and external
// resource fetches, at the cost of internal sprite-reuse (`<use href="#x">`)
// — an acceptable trade for a single-icon upload.
const ALLOWED_SVG_TAGS: [&str; 18] = [
    "svg",
    "path",
    "g",
    "defs",
    "symbol",
    "rect",
    "circle",
    "ellipse",
    "line",
    "polyline",
    "polygon",
    "text",
    "tspan",
    "title",
    "desc",
    "linearGradient",
    "radialGradient",
    "stop",
];

const ALLOWED_SVG_ATTRS: [&str; 28] = [
    "xmlns",
    "xmlns:xlink",
    "viewBox",
    "width",
    "height",
    "preserveAspectRatio",
    "d",
    "fill",
    "fill-rule",
    "fill-opacity",
    "stroke",
    "stroke-width",
    "stroke-linecap",
    "stroke-linejoin",
    "stroke-dasharray",
    "stroke-opacity",
    "opacity",
    "transform",
    "x",
    "y",
    "x1",
    "y1",
    "x2",
    "y2",
    "cx",
    "cy",
    "r",
    "id",
];

/// Strips everything not on the structural-SVG allowlist above — in
/// particular `<script>`, every `on*` event-handler attribute (never on the
/// attribute allowlist, so ammonia drops them regardless of tag), and any
/// `href`/`xlink:href` (also never allowlisted, closing off `javascript:`
/// URLs). The sanitized bytes, never the original upload, are what gets
/// written to disk.
fn sanitize_svg(bytes: &[u8]) -> Result<Vec<u8>, AppError> {
    let source = std::str::from_utf8(bytes)
        .map_err(|_| AppError::ValidationError("SVG must be valid UTF-8".into()))?;
    // ammonia doesn't parse a DOCTYPE at all, so a hand-crafted `<!ENTITY`
    // ahead of the root element would pass through untouched — reject
    // outright rather than trying to sanitize XXE away.
    let lower = source.to_ascii_lowercase();
    if lower.contains("<!doctype") || lower.contains("<!entity") {
        return Err(AppError::ValidationError(
            "SVG must not contain a DOCTYPE or ENTITY declaration".into(),
        ));
    }
    let cleaned = ammonia::Builder::default()
        .tags(HashSet::from(ALLOWED_SVG_TAGS))
        .generic_attributes(HashSet::from(ALLOWED_SVG_ATTRS))
        .link_rel(None)
        .clean(source)
        .to_string();
    Ok(cleaned.into_bytes())
}

/// Sniffs, sanitizes (SVG only), and writes the icon to
/// `data_dir/icons/{prefix}/{id}/{uuid}.{ext}`, returning the filename
/// (`{uuid}.{ext}`) callers use to build the proxy URL.
pub async fn store(
    state: &SharedState,
    prefix: &str,
    id: &str,
    bytes: &[u8],
) -> Result<String, AppError> {
    if bytes.len() > MAX_ICON_BYTES {
        return Err(AppError::ValidationError(format!(
            "Icon exceeds the {}MiB upload limit",
            MAX_ICON_BYTES / (1024 * 1024)
        )));
    }
    let kind = sniff_image_kind(bytes)?;
    let stored_bytes = if kind == "svg" {
        sanitize_svg(bytes)?
    } else {
        bytes.to_vec()
    };

    let dir = icon_dir(state, prefix, id);
    tokio::fs::create_dir_all(&dir).await.map_err(|error| {
        AppError::InternalError(format!("could not create icon directory: {error}"))
    })?;

    // ponytail: best-effort wipe of any previous icon(s) for this entity
    // before writing the new one — simpler than parsing the old proxy URL
    // back into a disk path, and self-heals any orphan left by a prior
    // failed write. A user who clears the icon field without ever
    // re-uploading still leaves one file behind; add a dedicated delete
    // endpoint if that ever matters in practice.
    if let Ok(mut entries) = tokio::fs::read_dir(&dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let _ = tokio::fs::remove_file(entry.path()).await;
        }
    }

    let filename = format!("{}.{kind}", Uuid::now_v7());
    tokio::fs::write(dir.join(&filename), &stored_bytes)
        .await
        .map_err(|error| AppError::InternalError(format!("could not write icon: {error}")))?;
    Ok(filename)
}

/// Rejects any filename that isn't a bare `name.ext` — no path separators, no
/// `.`/`..` — before it's ever joined onto a disk path.
fn sanitize_filename(filename: &str) -> Result<&str, AppError> {
    if filename.is_empty()
        || filename == "."
        || filename == ".."
        || filename.contains('/')
        || filename.contains('\\')
    {
        return Err(AppError::ResourceNotFound);
    }
    Ok(filename)
}

/// Reads a previously stored icon back off disk for the proxy serve routes.
pub async fn read(
    state: &SharedState,
    prefix: &str,
    id: &str,
    filename: &str,
) -> Result<Vec<u8>, AppError> {
    let filename = sanitize_filename(filename)?;
    let path = icon_dir(state, prefix, id).join(filename);
    tokio::fs::read(&path)
        .await
        .map_err(|_| AppError::ResourceNotFound)
}

/// The `Content-Type` to serve an icon with, derived from its own filename
/// extension — never from anything the client sends on the request.
pub fn content_type_for(filename: &str) -> &'static str {
    let ext = filename
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_png() {
        let bytes = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0];
        assert_eq!(sniff_image_kind(&bytes).unwrap(), "png");
    }

    #[test]
    fn sniffs_jpeg() {
        let bytes = [0xFF, 0xD8, 0xFF, 0xE0];
        assert_eq!(sniff_image_kind(&bytes).unwrap(), "jpg");
    }

    #[test]
    fn sniffs_webp() {
        let mut bytes = b"RIFF".to_vec();
        bytes.extend_from_slice(&[0, 0, 0, 0]);
        bytes.extend_from_slice(b"WEBP");
        assert_eq!(sniff_image_kind(&bytes).unwrap(), "webp");
    }

    #[test]
    fn sniffs_svg() {
        let bytes = b"<?xml version=\"1.0\"?><svg></svg>";
        assert_eq!(sniff_image_kind(bytes).unwrap(), "svg");
    }

    #[test]
    fn rejects_unknown_bytes() {
        let bytes = b"not an image";
        assert!(sniff_image_kind(bytes).is_err());
    }

    #[test]
    fn sanitize_svg_strips_script_and_event_handlers() {
        let malicious =
            b"<svg onload=\"alert(1)\"><script>alert(2)</script><circle r=\"1\" onclick=\"alert(3)\"/></svg>";
        let cleaned = sanitize_svg(malicious).unwrap();
        let cleaned = String::from_utf8(cleaned).unwrap();
        assert!(!cleaned.contains("script"), "{cleaned}");
        assert!(!cleaned.contains("onload"), "{cleaned}");
        assert!(!cleaned.contains("onclick"), "{cleaned}");
        assert!(cleaned.contains("circle"), "{cleaned}");
    }

    #[test]
    fn sanitize_svg_rejects_doctype() {
        let malicious = b"<!DOCTYPE svg [<!ENTITY x \"boom\">]><svg></svg>";
        assert!(sanitize_svg(malicious).is_err());
    }

    #[test]
    fn sanitize_filename_rejects_traversal() {
        assert!(sanitize_filename("../../etc/passwd").is_err());
        assert!(sanitize_filename("a/b.png").is_err());
        assert!(sanitize_filename("icon.png").is_ok());
    }
}
