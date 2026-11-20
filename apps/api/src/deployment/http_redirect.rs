//! Plain HTTP for the central Caddy: HTTPS domains get a 308 to `https://`, the rest
//! are served as before.
//!
//! Every route lives in `srv0`, which also holds the TLS connection policies. Caddy
//! puts TLS on each of a server's listeners except the HTTP port, and an inherited
//! socket (`fd/3`) has no port, so with socket activation port 80 spoke TLS and every
//! `http://` visit got "Client sent an HTTP request to an HTTPS server". Without
//! sockets `:80` served the application in clear text with no redirect, because Caddy
//! only adds redirects to servers that listen on the HTTP port alone.
//!
//! So the HTTP listener moves to its own server, rebuilt from `srv0` on every config
//! edit: the same routes, except that an HTTPS host answers with a redirect unless
//! its domain turned the redirect off.

use std::collections::HashSet;

use serde_json::{Map, Value, json};

use crate::{dtos::response::AppError, state::state::SharedState};

pub const SERVER: &str = "litepod_http";

/// Hosts with HTTPS whose domain turned the redirect off. Path domains follow
/// their root.
pub async fn load_opt_outs(state: &SharedState) -> Result<HashSet<String>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query_scalar(
        "SELECT DISTINCT host FROM application_domains \
         WHERE https = 1 AND https_redirect = 0 AND path IS NULL",
    )
    .fetch_all(&conn)
    .await
    .map(|hosts| hosts.into_iter().collect())
    .map_err(|e| AppError::DatabaseError(e.to_string()))
}

/// `fd/3` is the HTTP socket by the installer's order.
fn is_http_listener(address: &Value) -> bool {
    address
        .as_str()
        .is_some_and(|a| a == "fd/3" || a.ends_with(":80"))
}

pub fn sync(config: &mut Value, opt_outs: &HashSet<String>) {
    let Some(servers) = config
        .pointer_mut("/apps/http/servers")
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    let Some(mut srv0) = servers.get("srv0").and_then(Value::as_object).cloned() else {
        return;
    };
    let mut http_listen: Vec<Value> = servers
        .get(SERVER)
        .and_then(|server| server["listen"].as_array())
        .cloned()
        .unwrap_or_default();

    let listen = srv0
        .get("listen")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if listen.iter().all(is_http_listener) {
        // No HTTPS listener, nothing to redirect: give the HTTP listeners back.
        if servers.remove(SERVER).is_some() {
            let mut merged = listen;
            for address in http_listen {
                if !merged.contains(&address) {
                    merged.insert(0, address);
                }
            }
            srv0.insert("listen".into(), Value::Array(merged));
            srv0.remove("listen_protocols");
            super::caddy_admin::adapt_inherited_sockets(&mut srv0, false);
            servers.insert("srv0".into(), Value::Object(srv0));
        }
        return;
    }
    let protocols = srv0.get("listen_protocols").and_then(Value::as_array).cloned();
    let mut kept = Vec::new();
    let mut kept_protocols = Vec::new();
    for (index, address) in listen.into_iter().enumerate() {
        if is_http_listener(&address) {
            if !http_listen.contains(&address) {
                http_listen.push(address);
            }
        } else {
            kept.push(address);
            if let Some(protocol) = protocols.as_ref().and_then(|p| p.get(index)) {
                kept_protocols.push(protocol.clone());
            }
        }
    }
    if http_listen.is_empty() {
        return;
    }
    let inherited = super::caddy_admin::inherits_sockets(&http_listen);
    srv0.insert("listen".into(), Value::Array(kept));
    if protocols.is_some() {
        srv0.insert("listen_protocols".into(), Value::Array(kept_protocols));
    }
    // A config split before the installer moved to sockets still says `:443`; the
    // HTTPS socket has to be there too or HTTPS stops answering.
    if inherited && !super::caddy_admin::inherits_sockets(srv0["listen"].as_array().unwrap()) {
        srv0["listen"].as_array_mut().unwrap().push("fd/4".into());
        super::caddy_admin::adapt_inherited_sockets(&mut srv0, true);
    }

    let https_hosts: HashSet<&str> = srv0
        .get("tls_connection_policies")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|policy| policy.pointer("/match/sni").and_then(Value::as_array))
        .flatten()
        .filter_map(Value::as_str)
        .filter(|host| !opt_outs.contains(*host))
        .collect();
    let routes: Vec<Value> = srv0
        .get("routes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|route| {
            if redirects(route, &https_hosts) {
                json!({
                    "match": route["match"],
                    "handle": [{
                        "handler": "static_response",
                        "status_code": 308,
                        "headers": { "Location": ["https://{http.request.host}{http.request.uri}"] }
                    }],
                    "terminal": true
                })
            } else {
                without_ids(route)
            }
        })
        .collect();

    let mut http = Map::new();
    http.insert("listen".into(), Value::Array(http_listen));
    http.insert("routes".into(), Value::Array(routes));
    if let Some(errors) = srv0.get("errors") {
        http.insert("errors".into(), without_ids(errors));
    }
    if let Some(logs) = srv0.get("logs") {
        http.insert("logs".into(), logs.clone());
    }
    // Caddy cannot tell `fd/3` is the HTTP port and would otherwise manage TLS here.
    http.insert("automatic_https".into(), json!({ "disable": true }));
    super::caddy_admin::adapt_inherited_sockets(&mut http, false);
    servers.insert("srv0".into(), Value::Object(srv0));
    servers.insert(SERVER.into(), Value::Object(http));
}

/// A route redirects when every host it matches has HTTPS with the redirect on.
fn redirects(route: &Value, https_hosts: &HashSet<&str>) -> bool {
    let Some(matchers) = route["match"].as_array().filter(|m| !m.is_empty()) else {
        return false;
    };
    matchers.iter().all(|matcher| {
        matcher["host"].as_array().is_some_and(|hosts| {
            !hosts.is_empty()
                && hosts
                    .iter()
                    .all(|host| host.as_str().is_some_and(|h| https_hosts.contains(h)))
        })
    })
}

/// Caddy rejects a config where two objects share an `@id`.
fn without_ids(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(key, _)| key.as_str() != "@id")
                .map(|(key, value)| (key.clone(), without_ids(value)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(without_ids).collect()),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(listen: Value) -> Value {
        json!({ "apps": { "http": { "servers": { "srv0": {
            "listen": listen,
            "listen_protocols": [["h1","h2"],["h1","h2"],["h3"],["h1","h2"]],
            "routes": [
                { "@id": "a", "match": [{ "host": ["a.example.com"] }], "handle": [{ "handler": "reverse_proxy" }], "terminal": true },
                { "@id": "b", "match": [{ "host": ["b.example.com"] }], "handle": [{ "handler": "reverse_proxy" }], "terminal": true },
                { "@id": "c", "match": [{ "host": ["c.example.com"] }], "handle": [{ "handler": "reverse_proxy" }], "terminal": true },
                { "handle": [{ "handler": "reverse_proxy" }] }
            ],
            "tls_connection_policies": [
                { "match": { "sni": ["a.example.com"] } },
                { "match": { "sni": ["c.example.com"] } }
            ]
        } } } } })
    }

    #[test]
    fn moves_the_http_socket_out_and_redirects_https_hosts() {
        let mut cfg = config(json!(["fd/3", "fd/4", "fdgram/5", ":443"]));
        let opt_outs = HashSet::from(["c.example.com".to_string()]);
        sync(&mut cfg, &opt_outs);
        let servers = &cfg["apps"]["http"]["servers"];
        assert_eq!(servers["srv0"]["listen"], json!(["fd/4", "fdgram/5", ":443"]));
        assert_eq!(
            servers["srv0"]["listen_protocols"],
            json!([["h1","h2"],["h3"],["h1","h2"]])
        );
        let http = &servers[SERVER];
        assert_eq!(http["listen"], json!(["fd/3"]));
        assert_eq!(http["listen_protocols"], json!([["h1", "h2"]]));
        assert_eq!(http["automatic_https"]["disable"], json!(true));
        let routes = http["routes"].as_array().unwrap();
        // a: HTTPS -> redirect; b: HTTP only -> served; c: opted out -> served; catch-all served.
        assert_eq!(routes[0]["handle"][0]["status_code"], json!(308));
        assert_eq!(routes[1]["handle"][0]["handler"], json!("reverse_proxy"));
        assert_eq!(routes[2]["handle"][0]["handler"], json!("reverse_proxy"));
        assert_eq!(routes[3]["handle"][0]["handler"], json!("reverse_proxy"));
        assert!(routes.iter().all(|route| route.get("@id").is_none()));

        // Idempotent: the 30s recovery loop must not reload Caddy every time.
        let before = cfg.clone();
        sync(&mut cfg, &opt_outs);
        assert_eq!(cfg, before);
    }

    #[test]
    fn plain_ports_split_the_same_way() {
        let mut cfg = config(json!([":80", ":443"]));
        cfg["apps"]["http"]["servers"]["srv0"]
            .as_object_mut()
            .unwrap()
            .remove("listen_protocols");
        sync(&mut cfg, &HashSet::new());
        let servers = &cfg["apps"]["http"]["servers"];
        assert_eq!(servers["srv0"]["listen"], json!([":443"]));
        assert_eq!(servers[SERVER]["listen"], json!([":80"]));
        assert!(servers[SERVER].get("listen_protocols").is_none());
    }

    #[test]
    fn http_only_instances_are_left_alone() {
        let mut cfg = config(json!([":80"]));
        let before = cfg.clone();
        sync(&mut cfg, &HashSet::new());
        assert_eq!(cfg, before);
    }
}
