//! The response for a file read out of a volume or container explorer, as a download or, for
//! the media the explorer previews, shown inline.

use axum::{
    http::{HeaderValue, header},
    response::Response,
};

use crate::dtos::response::AppError;

/// Types the browser renders without running anything from the file. HTML and SVG stay
/// downloads: shown inline from the API origin they could run script with the user's session.
fn previewable(mime: &str) -> bool {
    (mime.starts_with("image/") && mime != "image/svg+xml")
        || mime.starts_with("audio/")
        || mime.starts_with("video/")
        || mime == "application/pdf"
}

pub fn file_response(
    bytes: Vec<u8>,
    mime: Option<String>,
    path: &str,
    inline: bool,
) -> Result<Response, AppError> {
    let mime = mime.unwrap_or_else(|| "application/octet-stream".into());
    let inline = inline && previewable(&mime);
    let filename = std::path::Path::new(path)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .replace(['"', '\\', '\r', '\n'], "_");
    let header_value = |value: &str| {
        HeaderValue::from_str(value).map_err(|e| AppError::InternalError(e.to_string()))
    };
    let mut response = Response::new(axum::body::Body::from(bytes));
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, header_value(&mime)?);
    headers.insert(
        header::CONTENT_DISPOSITION,
        header_value(&format!(
            "{}; filename=\"{filename}\"",
            if inline { "inline" } else { "attachment" }
        ))?,
    );
    if inline {
        headers.insert(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        );
        // Browsers refuse to render a PDF in a sandboxed document, so only media gets `sandbox`.
        if mime != "application/pdf" {
            headers.insert(
                header::CONTENT_SECURITY_POLICY,
                HeaderValue::from_static("sandbox"),
            );
        }
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disposition(mime: &str, inline: bool) -> String {
        file_response(b"x".to_vec(), Some(mime.into()), "dir/a\"b.bin", inline)
            .unwrap()
            .headers()[header::CONTENT_DISPOSITION]
            .to_str()
            .unwrap()
            .to_owned()
    }

    #[test]
    fn only_media_and_pdf_are_shown_inline() {
        for mime in ["image/png", "audio/mpeg", "video/mp4", "application/pdf"] {
            assert!(disposition(mime, true).starts_with("inline;"), "{mime}");
        }
        for mime in ["text/html", "image/svg+xml", "application/octet-stream"] {
            assert!(disposition(mime, true).starts_with("attachment;"), "{mime}");
        }
        assert!(disposition("image/png", false).starts_with("attachment;"));
        assert_eq!(
            disposition("image/png", false),
            "attachment; filename=\"a_b.bin\""
        );
    }

    #[test]
    fn inline_media_is_sandboxed_and_never_sniffed() {
        let response = file_response(vec![], Some("audio/ogg".into()), "a.ogg", true).unwrap();
        assert_eq!(
            response.headers()[header::CONTENT_SECURITY_POLICY],
            "sandbox"
        );
        assert_eq!(
            response.headers()[header::X_CONTENT_TYPE_OPTIONS],
            "nosniff"
        );
        let pdf = file_response(vec![], Some("application/pdf".into()), "a.pdf", true).unwrap();
        assert!(pdf.headers().get(header::CONTENT_SECURITY_POLICY).is_none());
    }
}
