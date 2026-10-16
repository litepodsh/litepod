//! Removes high-volume package-fetch and verbose curl diagnostics from logs.
//!
//! Build output can contain thousands of lines that describe HTTP transport
//! details rather than useful build progress. `LogFilter` is deliberately
//! stateful because a curl response's headers and wrapped cookie values do not
//! all carry their own curl marker.

pub const MAX_LOG_LINE_BYTES: usize = 8 * 1024;

/// Removes terminal ANSI control sequences from output intended for a plain
/// text log viewer. This includes CSI colors/styles and OSC terminal metadata,
/// without altering ordinary UTF-8 application output.
pub fn strip_ansi_sequences(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut output = String::with_capacity(input.len());
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] != 0x1b {
            let character = input[index..]
                .chars()
                .next()
                .expect("index is a UTF-8 boundary");
            output.push(character);
            index += character.len_utf8();
            continue;
        }

        index += 1;
        match bytes.get(index) {
            Some(b'[') => {
                index += 1;
                while index < bytes.len() {
                    let byte = bytes[index];
                    index += 1;
                    if (0x40..=0x7e).contains(&byte) {
                        break;
                    }
                }
            }
            Some(b']') => {
                index += 1;
                while index < bytes.len() {
                    if bytes[index] == 0x07 {
                        index += 1;
                        break;
                    }
                    if bytes[index] == 0x1b && bytes.get(index + 1) == Some(&b'\\') {
                        index += 2;
                        break;
                    }
                    index += 1;
                }
            }
            Some(_) => index += 1,
            None => {}
        }
    }

    output
}

/// Truncates oversized output without splitting a UTF-8 code point.
pub fn truncate_line(line: &str) -> String {
    if line.len() <= MAX_LOG_LINE_BYTES {
        return line.to_owned();
    }
    let mut end = MAX_LOG_LINE_BYTES;
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}… [log line truncated]", &line[..end])
}

/// Filters noisy deployment log lines while preserving ordinary stderr.
#[derive(Debug, Default)]
pub struct LogFilter {
    in_curl_trace: bool,
    in_wrapped_cookie: bool,
}

impl LogFilter {
    /// Returns `None` when `line` is a verbose curl or package-download
    /// diagnostic. The returned value always borrows the original line, so
    /// callers can append it without allocating another string.
    pub fn filter<'a>(&mut self, line: &'a str) -> Option<&'a str> {
        let (content, timestamped) = without_timestamp_prefix(line);

        if is_package_manifest_download(content) {
            return None;
        }

        if is_curl_trace_start(content) {
            self.in_curl_trace = true;
            self.in_wrapped_cookie = is_set_cookie_header(content);
            return None;
        }

        if self.in_curl_trace {
            if is_curl_trace_continuation(content) {
                self.in_wrapped_cookie = is_set_cookie_header(content);
                return None;
            }

            // Curl can wrap a long Set-Cookie header over several unprefixed
            // lines. A timestamp belongs to a new emitted log line, so it
            // safely terminates that wrapped value.
            if self.in_wrapped_cookie && !timestamped && !content.trim().is_empty() {
                return None;
            }

            self.in_curl_trace = false;
            self.in_wrapped_cookie = false;
        }

        Some(line)
    }
}

/// Removes the timestamp produced by the build logger for matching only. It
/// intentionally accepts only a clock-shaped bracket prefix so application
/// messages such as `[worker] ...` are never reclassified.
fn without_timestamp_prefix(line: &str) -> (&str, bool) {
    let trimmed = line.trim_start();
    let Some(end) = trimmed.find(']') else {
        return (trimmed, false);
    };
    let stamp = &trimmed[1..end];
    if !trimmed.starts_with('[')
        || stamp.is_empty()
        || !stamp
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b':' | b'.'))
    {
        return (trimmed, false);
    }
    (trimmed[end + 1..].trim_start(), true)
}

fn is_package_manifest_download(line: &str) -> bool {
    let line = line.trim_start();
    line.starts_with("Enqueue package manifest for download:")
        || (line.starts_with("Enqueue package ") && line.ends_with(" for download"))
}

fn is_curl_trace_start(line: &str) -> bool {
    let line = line.trim_start();
    is_curl_response_status(line) || is_curl_request_line(line) || is_curl_verbose_detail(line)
}

fn is_curl_trace_continuation(line: &str) -> bool {
    let line = line.trim_start();
    line.starts_with('<')
        || line.starts_with('>')
        || line.starts_with('{')
        || line.starts_with('}')
        || is_curl_verbose_detail(line)
}

fn is_curl_response_status(line: &str) -> bool {
    let Some(rest) = line.strip_prefix('<').map(str::trim_start) else {
        return false;
    };
    rest.starts_with("HTTP/")
        || (rest.as_bytes().get(0).is_some_and(u8::is_ascii_digit)
            && rest.as_bytes().get(1).is_some_and(u8::is_ascii_digit)
            && rest.as_bytes().get(2).is_some_and(u8::is_ascii_digit)
            && rest
                .as_bytes()
                .get(3)
                .is_some_and(|byte| byte.is_ascii_whitespace()))
}

fn is_curl_request_line(line: &str) -> bool {
    let Some(rest) = line.strip_prefix('>').map(str::trim_start) else {
        return false;
    };
    [
        "GET ", "POST ", "PUT ", "PATCH ", "DELETE ", "HEAD ", "OPTIONS ", "CONNECT ", "HTTP/",
    ]
    .iter()
    .any(|prefix| rest.starts_with(prefix))
}

fn is_curl_verbose_detail(line: &str) -> bool {
    let Some(detail) = line.strip_prefix('*').map(str::trim_start) else {
        return false;
    };
    let detail = detail.to_ascii_lowercase();
    [
        "uses proxy env variable",
        "host ",
        "trying ",
        "connected to ",
        "connect to ",
        "could not resolve host",
        "ipv4:",
        "ipv6:",
        "alpn:",
        "tls",
        "ssl",
        "server certificate",
        "certificate",
        "request completely sent off",
        "upload completely sent off",
        "connection #",
        "closing connection",
        "established connection",
        "found bundle",
        "re-using existing",
    ]
    .iter()
    .any(|prefix| detail.starts_with(prefix))
}

fn is_set_cookie_header(line: &str) -> bool {
    line.trim_start()
        .strip_prefix('<')
        .map(str::trim_start)
        .is_some_and(|header| header.to_ascii_lowercase().starts_with("set-cookie:"))
}

#[cfg(test)]
mod tests {
    use super::LogFilter;

    #[test]
    fn filters_a_verbose_curl_response_with_wrapped_cookies() {
        let mut filter = LogFilter::default();
        let output = [
            "[15:22:05.040] < 304 Not Modified",
            "[15:22:05.051] < Date: Sun, 19 Jul 2026 22:21:41 GMT",
            "[15:22:05.068] < set-cookie:",
            "__cf_bm=long-cookie-value; HttpOnly; SameSite=None; Secure; Path=/;",
            "Domain=npmjs.org; Expires=Sun, 19 Jul 2026 22:51:41 GMT",
            "[15:22:05.077] < CF-Ray: a1dd1edc387cda28-QRO",
            "[15:22:05.149] < Server: cloudflare",
            "[15:22:05.158] build step completed",
        ];

        let retained: Vec<_> = output
            .into_iter()
            .filter_map(|line| filter.filter(line))
            .collect();
        assert_eq!(retained, vec!["[15:22:05.158] build step completed"]);
    }

    #[test]
    fn filters_curl_request_and_transport_markers() {
        let mut filter = LogFilter::default();
        let output = [
            "* Host registry.npmjs.org:443 was resolved.",
            "*   Trying 104.16.25.34:443...",
            "> GET /package HTTP/1.1",
            "> Host: registry.npmjs.org",
            "< HTTP/1.1 200 OK",
            "< content-type: application/json",
            "application build output",
        ];

        let retained: Vec<_> = output
            .into_iter()
            .filter_map(|line| filter.filter(line))
            .collect();
        assert_eq!(retained, vec!["application build output"]);
    }

    #[test]
    fn filters_bun_manifest_download_diagnostics_without_a_curl_trace() {
        let mut filter = LogFilter::default();
        assert_eq!(
            filter.filter("Enqueue package manifest for download: micromark-extension-directive"),
            None
        );
        assert_eq!(
            filter.filter("[15:23:31.184] Enqueue package manifest for download: hast-util-format"),
            None
        );
    }

    #[test]
    fn preserves_normal_application_stderr() {
        let mut filter = LogFilter::default();
        let output = [
            "Error: database migration failed",
            "[15:23:31.184] WARN unable to connect to Redis",
            "* application emitted an asterisk-prefixed diagnostic",
            "<not a curl response>",
        ];

        let retained: Vec<_> = output
            .into_iter()
            .filter_map(|line| filter.filter(line))
            .collect();
        assert_eq!(retained, output);
    }

    #[test]
    fn stops_filtering_once_a_normal_timestamped_line_arrives() {
        let mut filter = LogFilter::default();
        assert_eq!(filter.filter("[15:22:05.040] < 304 Not Modified"), None);
        assert_eq!(
            filter.filter("[15:22:05.158] compiling application"),
            Some("[15:22:05.158] compiling application")
        );
    }

    #[test]
    fn truncates_long_utf8_lines_at_a_character_boundary() {
        let line = format!("{}é", "x".repeat(super::MAX_LOG_LINE_BYTES));
        let output = super::truncate_line(&line);
        assert!(output.ends_with("… [log line truncated]"));
        assert!(output.is_char_boundary(output.len()));
    }

    #[test]
    fn strips_terminal_color_and_style_sequences() {
        assert_eq!(
            super::strip_ansi_sequences("\x1b[4m>>>> Executing compose <<<<\x1b[0m"),
            ">>>> Executing compose <<<<",
        );
    }
}
