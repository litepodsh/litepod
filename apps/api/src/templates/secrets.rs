//! Auto-generates real secrets for catalog templates, driven by an explicit
//! marker in the template's `.env` — never by guessing from text like
//! "replace-with-a-password". Every `{{...}}` marker inside a value is replaced,
//! so a marker may sit within a larger value (`APP_KEY=base64:{{base64_32}}`,
//! `WEBUI_AUTH=admin:{{password_32}}`, `DSN=postgres://u:{{password_32}}@db`):
//!
//! - `{{password_N}}`          — N random alphanumeric chars (upper+lower+digit)
//! - `{{password_N_symbols}}`  — N random chars, alphanumeric + symbols
//! - `{{password_N_readable}}` — N random chars with no look-alikes (no `I O l 0 1`),
//!                               for passwords a person reads off a screen and retypes
//! - `{{openssl_rand_N}}`      — N random bytes, hex-encoded (2N chars) —
//!                               identical to `openssl rand -hex N`
//! - `{{hex_upper_N}}`         — the same, upper-case
//! - `{{base64_N}}`            — N random bytes, standard padded Base64
//! - `{{base64url_N}}`         — N random bytes, URL-safe padded Base64
//! - `{{uuid_v4}}`             — a random UUID
//! - `{{uuid_v7}}`             — a time-ordered UUID
//!
//! Anything else — a real value, an empty value, free-text like "changeme", an
//! unknown or out-of-range marker — is left exactly as written. No heuristics.

use base64::{Engine as _, engine::general_purpose};
use rand::RngExt;

const MIN_LEN: usize = 1;
const MAX_LEN: usize = 128;
const ALNUM: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
// No `$ # & : = " ' ` \ /` or whitespace — those break shell interpolation,
// URL/DSN parsing (`user:pass@host`), YAML/.env quoting, or job-control (`&`)
// when a generated password lands unquoted in a connection string or script.
const ALNUM_SYMBOLS: &[u8] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789!%^*-_+.,~";
// Near the alphabet the database template dialogs generate from in the UI
// (`ui/src/lib/components/projects/postgres-template-dialog.svelte`), minus `I O l 0 1`
// so a password read off a screen cannot be mistranscribed — and minus `=`, which is
// excluded above and would also hide where the password starts on a `KEY=VALUE` line.
const READABLE: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789-_!";

enum Marker {
    Password { len: usize, alphabet: &'static [u8] },
    Hex { bytes: usize, upper: bool },
    Base64 { bytes: usize, url_safe: bool },
    Uuid { v7: bool },
}

fn sized(rest: &str) -> Option<usize> {
    let size: usize = rest.parse().ok()?;
    (MIN_LEN..=MAX_LEN).contains(&size).then_some(size)
}

/// Parses the inside of a `{{...}}` marker. Returns `None` for anything else,
/// including a malformed or out-of-range marker — malformed input is left
/// untouched rather than guessed at.
fn parse_marker(inner: &str) -> Option<Marker> {
    let inner = inner.trim();

    match inner {
        "uuid_v4" => return Some(Marker::Uuid { v7: false }),
        "uuid_v7" => return Some(Marker::Uuid { v7: true }),
        _ => {}
    }
    if let Some(rest) = inner.strip_prefix("openssl_rand_") {
        return Some(Marker::Hex {
            bytes: sized(rest)?,
            upper: false,
        });
    }
    if let Some(rest) = inner.strip_prefix("hex_upper_") {
        return Some(Marker::Hex {
            bytes: sized(rest)?,
            upper: true,
        });
    }
    // `base64url_` must be tried before `base64_`, which is not a prefix of it.
    if let Some(rest) = inner.strip_prefix("base64url_") {
        return Some(Marker::Base64 {
            bytes: sized(rest)?,
            url_safe: true,
        });
    }
    if let Some(rest) = inner.strip_prefix("base64_") {
        return Some(Marker::Base64 {
            bytes: sized(rest)?,
            url_safe: false,
        });
    }
    if let Some(rest) = inner.strip_prefix("password_") {
        let (digits, alphabet) = match rest.strip_suffix("_symbols") {
            Some(digits) => (digits, ALNUM_SYMBOLS),
            None => match rest.strip_suffix("_readable") {
                Some(digits) => (digits, READABLE),
                None => (rest, ALNUM),
            },
        };
        return Some(Marker::Password {
            len: sized(digits)?,
            alphabet,
        });
    }
    None
}

fn random_byte() -> u8 {
    rand::rng().random::<u8>()
}

fn generate(marker: Marker) -> String {
    match marker {
        Marker::Hex { bytes, upper } => (0..bytes)
            .map(|_| {
                if upper {
                    format!("{:02X}", random_byte())
                } else {
                    format!("{:02x}", random_byte())
                }
            })
            .collect(),
        Marker::Base64 { bytes, url_safe } => {
            let random: Vec<u8> = (0..bytes).map(|_| random_byte()).collect();
            if url_safe {
                general_purpose::URL_SAFE.encode(random)
            } else {
                general_purpose::STANDARD.encode(random)
            }
        }
        Marker::Password { len, alphabet } => (0..len)
            .map(|_| alphabet[random_byte() as usize % alphabet.len()] as char)
            .collect(),
        Marker::Uuid { v7 } => if v7 {
            uuid::Uuid::now_v7()
        } else {
            uuid::Uuid::new_v4()
        }
        .to_string(),
    }
}

/// Replaces every recognized `{{...}}` marker inside a single value. Unknown or
/// malformed markers are copied through verbatim, braces included.
fn substitute_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(open) = rest.find("{{") {
        let (before, from_open) = rest.split_at(open);
        out.push_str(before);
        let Some(close) = from_open.find("}}") else {
            break;
        };
        match parse_marker(&from_open[2..close]) {
            Some(marker) => out.push_str(&generate(marker)),
            None => out.push_str(&from_open[..close + 2]),
        }
        rest = &from_open[close + 2..];
    }
    out.push_str(rest);
    out
}

/// Rewrites `{{...}}` secret markers in a template's raw `.env` content, line by
/// line. Preserves comments, blank lines, and ordering — only the value half of a
/// `KEY=VALUE` line changes, and only where it contains a recognized marker.
pub fn substitute_placeholder_secrets(content: &str) -> String {
    content
        .lines()
        .map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                return line.to_owned();
            }
            let Some((name, value)) = line.split_once('=') else {
                return line.to_owned();
            };
            match value.contains("{{") {
                true => format!("{name}={}", substitute_value(value)),
                false => line.to_owned(),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value_of(line: &str) -> String {
        substitute_placeholder_secrets(line)
            .split_once('=')
            .unwrap()
            .1
            .to_owned()
    }

    #[test]
    fn generates_alnum_password_of_requested_length() {
        let value = value_of("FORGEJO_DATABASE_PASSWORD={{password_24}}");
        assert_eq!(value.len(), 24);
        assert!(value.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn generates_password_with_symbols_when_flagged() {
        let value = value_of("APP_SECRET={{password_40_symbols}}");
        assert_eq!(value.len(), 40);
    }

    #[test]
    fn readable_password_omits_look_alike_characters() {
        for _ in 0..64 {
            let value = value_of("CODE_SERVER_PASSWORD={{password_32_readable}}");
            assert_eq!(value.len(), 32);
            assert!(
                !value.contains(['I', 'O', 'l', '0', '1']),
                "look-alike character in {value}"
            );
            assert!(
                !value.contains('='),
                "`=` would hide where the password starts on a KEY=VALUE line: {value}"
            );
        }
    }

    #[test]
    fn generates_hex_matching_openssl_rand_convention() {
        let value = value_of("API_KEY={{openssl_rand_32}}");
        assert_eq!(value.len(), 64);
        assert!(value.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn generates_upper_case_hex() {
        let value = value_of("API_KEY={{hex_upper_32}}");
        assert_eq!(value.len(), 64);
        assert!(
            value
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_lowercase())
        );
    }

    #[test]
    fn generates_standard_base64_from_requested_number_of_bytes() {
        use base64::Engine as _;

        let value = value_of("ENTE_KEY={{base64_32}}");
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&value)
            .unwrap();
        assert_eq!(decoded.len(), 32);
        assert!(value.ends_with('='));
    }

    #[test]
    fn generates_url_safe_base64_from_requested_number_of_bytes() {
        use base64::Engine as _;

        let value = value_of("ENTE_JWT_SECRET={{base64url_32}}");
        let decoded = base64::engine::general_purpose::URL_SAFE
            .decode(&value)
            .unwrap();
        assert_eq!(decoded.len(), 32);
        assert!(!value.contains('+'));
        assert!(!value.contains('/'));
        assert!(value.ends_with('='));
    }

    #[test]
    fn generates_uuids_of_the_requested_version() {
        let v4: uuid::Uuid = value_of("NODE_ID={{uuid_v4}}").parse().unwrap();
        assert_eq!(v4.get_version_num(), 4);
        let v7: uuid::Uuid = value_of("NODE_ID={{uuid_v7}}").parse().unwrap();
        assert_eq!(v7.get_version_num(), 7);
    }

    #[test]
    fn substitutes_markers_embedded_in_a_larger_value() {
        let value = value_of("GARAGE_WEBUI_AUTH_USER_PASS=admin:{{password_32}}");
        let password = value.strip_prefix("admin:").expect("prefix preserved");
        assert_eq!(password.len(), 32);

        let value = value_of("INVOICENINJA_APP_KEY=base64:{{base64_32}}");
        assert!(value.starts_with("base64:"));
        assert!(!value.contains('{'));
    }

    #[test]
    fn substitutes_every_marker_on_one_line_independently() {
        let value = value_of("DSN=postgres://{{password_8}}:{{password_8}}@db");
        let middle = value
            .strip_prefix("postgres://")
            .and_then(|rest| rest.strip_suffix("@db"))
            .expect("surrounding text preserved");
        let (user, password) = middle.split_once(':').unwrap();
        assert_eq!((user.len(), password.len()), (8, 8));
        assert_ne!(user, password, "each marker gets its own secret");
    }

    #[test]
    fn leaves_free_text_and_real_values_untouched() {
        let out = substitute_placeholder_secrets(
            "FORGEJO_DOMAIN=localhost\nSOME_PASSWORD=replace-with-a-long-unique-password\nOTHER=changeme\nSET=already-real-value",
        );
        assert_eq!(
            out,
            "FORGEJO_DOMAIN=localhost\nSOME_PASSWORD=replace-with-a-long-unique-password\nOTHER=changeme\nSET=already-real-value"
        );
    }

    #[test]
    fn leaves_malformed_or_out_of_range_markers_untouched() {
        let input = "A={{password_}}\nB={{password_99999}}\nC={{unknown_9}}\nD={{password_16}\nE=prefix-{{nope}}-suffix\nF={{uuid_v9}}";
        assert_eq!(substitute_placeholder_secrets(input), input);
    }

    #[test]
    fn preserves_comments_blank_lines_and_ordering() {
        let out =
            substitute_placeholder_secrets("# comment\n\nFIRST=a\nSECRET={{password_8}}\nLAST=z");
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "# comment");
        assert_eq!(lines[1], "");
        assert_eq!(lines[2], "FIRST=a");
        assert_eq!(lines[4], "LAST=z");
    }
}
