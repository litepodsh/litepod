//! What may become a project or application icon: a name from [`NAMES`], an
//! uploaded image that survives [`prepare`], or a URL that passes [`check_value`].

use rand::RngExt;

use super::{MAX_ICON_BYTES, sanitize_filename, sanitize_svg, sniff_image_kind};
use crate::dtos::response::AppError;

/// The Lucide icons the web can draw by name. Same list as
/// `ORGANIZATION_ICON_MAP` in the UI; `organization-icons.test.ts` compares them.
pub const NAMES: &[&str] = &[
    "shield",
    "gallery-vertical-end",
    "audio-waveform",
    "command",
    "square-terminal",
    "bot",
    "book-open",
    "chart-pie",
    "frame",
    "map",
    "settings-2",
    "users",
    "building",
    "globe",
    "layers",
    "box",
    "rocket",
    "server",
    "database",
    "code",
    "cloud",
    "zap",
    "package",
    "cpu",
    "mail",
    "shopping-cart",
    "image",
    "music",
    "video",
    "message-circle",
    "lock",
    "activity",
    "heart",
    "star",
    "flame",
    "leaf",
    "gamepad-2",
    "camera",
    "wrench",
    "brain",
    "briefcase",
];

const MAX_URL_CHARS: usize = 2048;

pub fn random_name() -> &'static str {
    NAMES[rand::rng().random_range(0..NAMES.len())]
}

/// The bytes to keep and the extension to keep them under, or why not.
pub fn prepare(bytes: &[u8]) -> Result<(&'static str, Vec<u8>), AppError> {
    if bytes.len() > MAX_ICON_BYTES {
        return Err(AppError::ValidationError(format!(
            "Icon exceeds the {}MiB upload limit",
            MAX_ICON_BYTES / (1024 * 1024)
        )));
    }
    let kind = sniff_image_kind(bytes)?;
    let stored = if kind == "svg" {
        sanitize_svg(bytes)?
    } else {
        bytes.to_vec()
    };
    Ok((kind, stored))
}

/// A value saved as an icon: a known name, this entity's own stored icon
/// (`own_path` ends in `/icon/`), or an HTTPS address without credentials. Any
/// other same-site path is refused: every viewer's browser would request it
/// with their session.
pub fn check_value(value: &str, own_path: &str) -> Result<(), AppError> {
    if NAMES.contains(&value) {
        return Ok(());
    }
    if value
        .strip_prefix(own_path)
        .is_some_and(|filename| sanitize_filename(filename).is_ok())
    {
        return Ok(());
    }
    if value.chars().count() > MAX_URL_CHARS {
        return Err(AppError::ValidationError("The icon URL is too long".into()));
    }
    crate::security::ssrf::parse_https_url_with_port(value)
        .map(drop)
        .map_err(|error| AppError::ValidationError(format!("Icon: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWN: &str = "/api/v1/organizations/o/projects/p/icon/";

    #[test]
    fn names_own_icons_and_https_are_accepted() {
        assert!(check_value("rocket", OWN).is_ok());
        assert!(check_value(&format!("{OWN}a.png"), OWN).is_ok());
        assert!(check_value("https://cdn.example.com/a.png", OWN).is_ok());
    }

    #[test]
    fn everything_else_is_refused() {
        for bad in [
            "",
            "not-an-icon",
            "http://cdn.example.com/a.png",
            "javascript:alert(1)",
            "data:image/svg+xml,<svg/>",
            "file:///etc/passwd",
            "https://user:pass@example.com/a.png",
            "https://10.0.0.1/a.png",
            "/api/v1/auth/logout",
            "/api/v1/organizations/other/projects/p/icon/a.png",
            "/api/v1/organizations/o/projects/p/icon/../../secret",
        ] {
            assert!(check_value(bad, OWN).is_err(), "{bad}");
        }
        let long = format!("https://example.com/{}", "a".repeat(MAX_URL_CHARS));
        assert!(check_value(&long, OWN).is_err());
    }

    #[test]
    fn a_random_name_is_one_of_the_names() {
        assert!(NAMES.contains(&random_name()));
    }

    #[test]
    fn prepare_cleans_an_svg_and_refuses_what_is_no_image() {
        let (kind, cleaned) =
            prepare(b"<svg><script>alert(1)</script><path d=\"M0 0\"/></svg>").unwrap();
        assert_eq!(kind, "svg");
        assert!(!String::from_utf8(cleaned).unwrap().contains("script"));
        assert!(prepare(b"<html></html>").is_err());
        assert!(prepare(&vec![0x89; MAX_ICON_BYTES + 1]).is_err());
    }
}
