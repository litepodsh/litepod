//! The one way litepod talks to the central Caddy's admin API.
//!
//! `CADDY_ADMIN_URL` is normally `http://litepod-caddy:2019`. A
//! `unix:///path/admin.sock` address is also accepted, for a Caddy whose admin
//! listens on a socket. Clients never follow redirects: the admin API does not
//! send them, and following one would take an admin request somewhere else.

use std::time::Duration;

/// Prefix of a unix-socket admin address, as in `unix:///run/caddy-admin/admin.sock`.
const UNIX_PREFIX: &str = "unix://";

/// The host sent over the socket. Caddy accepts `localhost` on a unix listener.
const SOCKET_BASE: &str = "http://localhost";

/// True when Caddy listens on sockets inherited from systemd (rootless installs):
/// the real client address survives, unlike rootlessport's published ports.
pub fn inherits_sockets(listen: &[serde_json::Value]) -> bool {
    listen
        .iter()
        .any(|addr| addr.as_str().is_some_and(|a| a.starts_with("fd/")))
}

/// Keeps an inherited-socket server consistent. `fd/3` is HTTP, `fd/4` HTTPS and
/// `fdgram/5` the UDP socket for HTTP/3 (the installer's socket order). Caddy refuses
/// HTTP/3 on a TCP `fd` listener, so each listener gets its own protocols.
pub fn adapt_inherited_sockets(
    server: &mut serde_json::Map<String, serde_json::Value>,
    https: bool,
) {
    use serde_json::{Value, json};
    let Some(listen) = server.get_mut("listen").and_then(Value::as_array_mut) else {
        return;
    };
    if !inherits_sockets(listen) {
        return;
    }
    if https {
        for addr in ["fd/4", "fdgram/5"] {
            if !listen.iter().any(|existing| existing == addr) {
                listen.push(Value::String(addr.into()));
            }
        }
    }
    let protocols: Vec<Value> = listen
        .iter()
        .map(|addr| match addr.as_str() {
            Some(a) if a.starts_with("fdgram/") => json!(["h3"]),
            _ => json!(["h1", "h2"]),
        })
        .collect();
    server.insert("listen_protocols".into(), Value::Array(protocols));
}

/// A client for the admin API, and the base URL to join `/config/`, `/load`
/// and `/id/...` onto.
pub fn client(
    admin_url: &str,
    timeout: Option<Duration>,
) -> Result<(reqwest::Client, String), String> {
    let mut builder = reqwest::Client::builder()
        // The admin API never redirects; following one would leave the socket.
        .redirect(reqwest::redirect::Policy::none());
    if let Some(timeout) = timeout {
        builder = builder.timeout(timeout);
    }
    let base = match socket_path(admin_url) {
        Some(path) => {
            builder = builder.unix_socket(path);
            SOCKET_BASE.to_string()
        }
        None => admin_url.trim_end_matches('/').to_string(),
    };
    let client = builder
        .build()
        .map_err(|error| format!("Could not create Caddy client: {error}"))?;
    Ok((client, base))
}

fn socket_path(admin_url: &str) -> Option<&str> {
    admin_url
        .strip_prefix(UNIX_PREFIX)
        .filter(|path| path.starts_with('/'))
}

#[cfg(test)]
mod tests {
    #[test]
    fn inherited_sockets_get_https_and_h3_listeners() {
        use serde_json::json;
        assert!(!inherits_sockets(&[json!(":80")]));
        let mut server = json!({ "listen": ["fd/3"] }).as_object().unwrap().clone();
        adapt_inherited_sockets(&mut server, true);
        assert_eq!(server["listen"], json!(["fd/3", "fd/4", "fdgram/5"]));
        assert_eq!(
            server["listen_protocols"],
            json!([["h1", "h2"], ["h1", "h2"], ["h3"]])
        );
        let mut plain = json!({ "listen": [":80"] }).as_object().unwrap().clone();
        adapt_inherited_sockets(&mut plain, true);
        assert_eq!(plain["listen"], json!([":80"]));
        assert!(!plain.contains_key("listen_protocols"));
    }

    use super::*;

    #[test]
    fn a_unix_address_is_a_socket_path() {
        assert_eq!(
            socket_path("unix:///run/caddy-admin/admin.sock"),
            Some("/run/caddy-admin/admin.sock")
        );
        let (_, base) = client("unix:///run/caddy-admin/admin.sock", None).unwrap();
        assert_eq!(base, SOCKET_BASE);
    }

    #[test]
    fn a_tcp_address_is_used_as_is() {
        assert_eq!(socket_path("http://litepod-caddy:2019"), None);
        let (_, base) = client("http://litepod-caddy:2019/", None).unwrap();
        assert_eq!(base, "http://litepod-caddy:2019");
    }

    #[test]
    fn a_relative_socket_path_is_not_guessed_at() {
        assert_eq!(socket_path("unix://admin.sock"), None);
    }

    #[tokio::test]
    async fn requests_go_through_the_socket() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let path = std::env::temp_dir().join(format!("caddy-admin-{}.sock", uuid::Uuid::new_v4()));
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = vec![0; 1024];
            let n = stream.read(&mut buf).await.unwrap();
            let request = String::from_utf8_lossy(&buf[..n]).to_string();
            stream
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}")
                .await
                .unwrap();
            request
        });

        let (client, base) = client(&format!("unix://{}", path.display()), None).unwrap();
        let response = client.get(format!("{base}/config/")).send().await.unwrap();
        assert!(response.status().is_success());
        let request = server.await.unwrap();
        assert!(request.starts_with("GET /config/ HTTP/1.1"));
        let _ = std::fs::remove_file(&path);
    }
}
