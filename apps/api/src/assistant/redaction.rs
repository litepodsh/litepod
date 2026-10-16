//! Keeping secrets out of what the model is sent.
//!
//! `get environment` returns names only, but a log line prints whatever the
//! application printed and a compose file carries passwords in clear text. Both
//! leave this machine, so both are scrubbed first.
//!
//! Two passes, in this order:
//!
//! 1. **Exact values.** The application's own decrypted environment values are
//!    matched literally. This is the precise pass: it catches a DSN in a stack
//!    trace no pattern would recognise, because the string is known.
//! 2. **Shapes.** `KEY=value` and `"key": "value"` where the key looks like a
//!    secret, plus bearer tokens and URLs carrying credentials. This is the
//!    imprecise pass, for values that never were environment variables.
//!
//! Redaction is best effort and deliberately over-eager: a redacted line the
//! model cannot read costs one follow-up question, a leaked credential costs a
//! rotation.

const MASK: &str = "[redacted]";

/// Values shorter than this are matched too often by accident - a port, a
/// `true`, a one-word hostname - and redacting them would gut the logs.
const MIN_VALUE_LEN: usize = 8;

/// Key fragments that make a value worth hiding whatever it looks like.
const SECRET_KEYS: &[&str] = &[
    "password",
    "passwd",
    "secret",
    "token",
    "apikey",
    "api_key",
    "accesskey",
    "access_key",
    "privatekey",
    "private_key",
    "credential",
    "auth",
    "dsn",
    "session",
    "signature",
    "salt",
];

/// Replaces every occurrence of the application's own secret values.
///
/// Only values that are long enough to be distinctive are used, so a `PORT` of
/// `8080` does not blank every port number in the logs.
pub fn redact_values(text: &str, values: &[(String, String)]) -> String {
    let mut out = text.to_owned();
    for (name, value) in values {
        let value = value.trim();
        if value.len() < MIN_VALUE_LEN {
            continue;
        }
        if out.contains(value) {
            out = out.replace(value, &format!("[redacted {name}]"));
        }
    }
    out
}

/// Blanks values that look like credentials, whatever their source.
pub fn redact_shapes(text: &str) -> String {
    text.lines().map(redact_line).collect::<Vec<_>>().join("\n")
}

fn redact_line(line: &str) -> String {
    let mut out = redact_urls(line);
    out = redact_assignments(&out, '=');
    out = redact_assignments(&out, ':');
    out
}

/// `postgres://user:hunter2@host` keeps its shape and loses the password.
fn redact_urls(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;

    while let Some(at) = rest.find("://") {
        let (before, after) = rest.split_at(at + 3);
        out.push_str(before);
        // Credentials end at the `@`, and only within this authority section.
        let authority_end = after.find(['/', ' ', '"', '\'']).unwrap_or(after.len());
        match after[..authority_end].find('@') {
            Some(marker) => {
                let credentials = &after[..marker];
                match credentials.find(':') {
                    Some(colon) => {
                        out.push_str(&credentials[..=colon]);
                        out.push_str(MASK);
                    }
                    None => out.push_str(credentials),
                }
                rest = &after[marker..];
            }
            None => {
                out.push_str(&after[..authority_end]);
                rest = &after[authority_end..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// `SECRET_KEY=abc` and `"token": "abc"`, both reduced to the key.
fn redact_assignments(line: &str, separator: char) -> String {
    let Some(at) = line.find(separator) else {
        return line.to_owned();
    };
    let (key, value) = line.split_at(at);
    let looks_secret = {
        let key = key.to_ascii_lowercase();
        SECRET_KEYS.iter().any(|needle| key.contains(needle))
    };
    if !looks_secret || value.len() <= 1 {
        return line.to_owned();
    }
    // Everything after the separator goes, quotes and trailing comma included:
    // a partial mask is a hint, and a hint is enough to guess from.
    format!("{key}{separator}{MASK}")
}

/// Both passes, in order.
pub fn redact(text: &str, values: &[(String, String)]) -> String {
    redact_shapes(&redact_values(text, values))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values() -> Vec<(String, String)> {
        vec![
            (
                "SENTRY_DSN".into(),
                "https://8f21c4a@o450712.ingest.sentry.io/1988".into(),
            ),
            ("PORT".into(), "8080".into()),
        ]
    }

    #[test]
    fn a_known_value_goes_wherever_it_appears() {
        let line = "ERROR failed to reach https://8f21c4a@o450712.ingest.sentry.io/1988 twice";
        let out = redact_values(line, &values());
        assert!(out.contains("[redacted SENTRY_DSN]"));
        assert!(!out.contains("8f21c4a"));
    }

    #[test]
    fn a_short_value_is_left_alone() {
        // Redacting `8080` would blank every port number in the logs.
        let out = redact_values("listening on 8080", &values());
        assert_eq!(out, "listening on 8080");
    }

    #[test]
    fn a_password_in_a_url_goes_and_the_shape_stays() {
        let out = redact_shapes("url = postgres://gateway:hunter2@db-prod-3:5432/gateway");
        assert!(
            out.contains("postgres://gateway:[redacted]@db-prod-3:5432/gateway"),
            "{out}"
        );
    }

    #[test]
    fn a_url_without_credentials_is_untouched() {
        let line = "GET https://api.example.com/v1/health 200";
        assert_eq!(redact_shapes(line), line);
    }

    #[test]
    fn an_assignment_with_a_secret_looking_key_goes() {
        assert_eq!(
            redact_shapes("API_TOKEN=sk-live-9f2b41"),
            "API_TOKEN=[redacted]"
        );
        assert_eq!(
            redact_shapes("      \"password\": \"hunter2\","),
            "      \"password\":[redacted]"
        );
    }

    #[test]
    fn an_ordinary_assignment_survives() {
        assert_eq!(redact_shapes("LOG_LEVEL=debug"), "LOG_LEVEL=debug");
        assert_eq!(redact_shapes("replicas: 3"), "replicas: 3");
    }

    #[test]
    fn both_passes_run() {
        let text = "dsn=https://8f21c4a@o450712.ingest.sentry.io/1988\nPASSWORD=hunter2000";
        let out = redact(text, &values());
        assert!(!out.contains("8f21c4a"));
        assert!(!out.contains("hunter2000"));
    }
}
