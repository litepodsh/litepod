use super::super::{basic_auth, domains};

use crate::{dtos::response::AppError, state::state::SharedState};

/// `container_port` is the port Caddy dials inside the container, which a
/// domain may override. When `https` is set this also opens the `:443`
/// listener, adds a TLS connection policy for `hostname`, and enables Caddy's
/// automatic HTTPS (certificate issuance + the implicit HTTP->HTTPS redirect)
/// for that host. Posting a single route via `/config/.../routes` (the old
/// approach) skips all of that — Caddyfile adaptation normally wires it up,
/// but raw JSON posted straight to the admin API does not, so HTTPS domains
/// had no TLS listener anywhere and every browser saw ERR_SSL_PROTOCOL_ERROR.
///
/// If the domain is in the application's basicauth protected set, an
/// `authentication` handler is prepended to the route's handle chain. The
/// accounts come from `basic_auth::load_payload_for_domain`.
pub(crate) async fn caddy_route(
    state: &SharedState,
    id: &str,
    hostname: &str,
    container_name: &str,
    container_port: i64,
    https: bool,
) -> Result<(), AppError> {
    // The Caddy `@id` for an application domain is `litepod-route-<domain_id>`.
    // Strip the prefix back off so we can ask basic_auth about the domain.
    let domain_id = id.strip_prefix("litepod-route-").unwrap_or(id);
    let payload = basic_auth::load_payload_for_domain(state, domain_id).await?;
    let certificate = crate::deployment::certificates::load(state, id).await?;
    edit_caddy_config(state, |config| {
        apply_app_domain_to_config(
            config,
            id,
            hostname,
            container_name,
            container_port,
            https,
            payload.as_ref(),
        )?;
        crate::deployment::certificates::apply(config, id, hostname, https, &certificate, None)
    })
    .await
}

/// Changes all application upstreams in one Caddy load, preserving TLS and auth.
pub(crate) async fn caddy_application_routes(
    state: &SharedState,
    application_id: &str,
    targets: &[(String, String, i64)],
) -> Result<(), AppError> {
    let mut prepared = Vec::new();
    for (domain_id, container, port) in targets {
        let domain = domains::get(state, application_id, domain_id).await?;
        if domain.is_local {
            continue;
        }
        let id = domains::route_id(domain_id);
        let auth = basic_auth::load_payload_for_domain(state, domain_id).await?;
        let certificate = crate::deployment::certificates::load(state, &id).await?;
        prepared.push((domain, id, container, port, auth, certificate));
    }
    edit_caddy_config(state, |config| {
        for (domain, id, container, port, auth, certificate) in prepared {
            apply_app_domain_to_config(
                config,
                &id,
                &domain.host,
                container,
                *port,
                domain.https,
                auth.as_ref(),
            )?;
            crate::deployment::certificates::apply(
                config,
                &id,
                &domain.host,
                domain.https,
                &certificate,
                None,
            )?;
        }
        Ok(())
    })
    .await
}

/// Re-runs `caddy_route` for a single domain. Used by `basic_auth` after
/// enabled/realm/users/protected set changes, because the authentication
/// handler is part of the route handle chain and must be re-pushed when any
/// of those change. Best-effort: logs and continues on Caddy failure so the
/// DB write that triggered this still succeeds.
pub(crate) async fn reconcile_caddy_basicauth(state: &SharedState, domain_id: &str) {
    let Ok(domain) = domains::get_by_id(state, domain_id).await else {
        return;
    };
    let Ok(Some((container_name, container_port))) =
        domains::caddy_target(state, &domain.application_id, &domain).await
    else {
        return;
    };
    if let Err(error) = caddy_route(
        state,
        &route_id_from_domain(domain_id),
        &domain.host,
        &container_name,
        container_port,
        domain.https,
    )
    .await
    {
        tracing::warn!(
            domain_id,
            host = %domain.host,
            %error,
            "could not reconcile Caddy route after basicauth change"
        );
    }
}

pub(super) fn route_id_from_domain(domain_id: &str) -> String {
    format!("litepod-route-{domain_id}")
}

// One central Caddy per API process. All writers, including settings and recovery,
// must serialize their GET/mutate/POST so concurrent deploys cannot lose routes.
// ponytail: process-wide lock; use per-proxy locks if one process manages multiple Caddies.
static CADDY_CONFIG_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Read the live Caddy admin config, let `mutate` edit the JSON tree in place,
/// then load it back. Every route change litepod makes to the central Caddy —
/// app domains, manual routes, orphan pruning — goes through this one
/// GET-`/config/` → mutate → POST-`/load` round trip.
pub(crate) async fn edit_caddy_config<F>(state: &SharedState, mutate: F) -> Result<(), AppError>
where
    F: FnOnce(&mut serde_json::Value) -> Result<(), AppError>,
{
    let _guard = CADDY_CONFIG_LOCK.lock().await;
    edit_caddy_config_unlocked(state, mutate).await
}

// The caller must hold CADDY_CONFIG_LOCK, including while reading recovery inputs.
async fn edit_caddy_config_unlocked<F>(state: &SharedState, mutate: F) -> Result<(), AppError>
where
    F: FnOnce(&mut serde_json::Value) -> Result<(), AppError>,
{
    let (client, base) = crate::deployment::caddy_admin::client(
        &state.caddy_admin_url,
        Some(std::time::Duration::from_secs(10)),
    )
    .map_err(AppError::InternalError)?;
    let response = client
        .get(format!("{base}/config/"))
        .send()
        .await
        .map_err(|e| AppError::InternalError(format!("Could not reach Caddy admin API: {e}")))?;
    if !response.status().is_success() {
        return Err(AppError::InternalError(format!(
            "Caddy returned {}",
            response.status()
        )));
    }
    let mut config: serde_json::Value = response
        .json()
        .await
        .map_err(|e| AppError::InternalError(format!("Invalid Caddy configuration: {e}")))?;
    let previous = config.clone();
    mutate(&mut config)?;
    // Request analytics; this round trip runs every 30s (route recovery), so a restarted
    // Caddy gets its access log back without a separate job.
    if let Some(dial) = crate::metrics_history::requests::access_log_dial() {
        crate::metrics_history::requests::ensure_access_log(&mut config, dial);
    }
    if config == previous {
        return Ok(());
    }
    let response = client
        .post(format!("{base}/load"))
        .json(&config)
        .send()
        .await
        .map_err(|e| {
            AppError::InternalError(format!("Could not apply Caddy configuration: {e}"))
        })?;
    if !response.status().is_success() {
        let status = response.status();
        return Err(AppError::InternalError(format!(
            "Caddy rejected route update ({status}); inspect the Caddy service logs"
        )));
    }
    Ok(())
}

/// Retry after slow boots, transient proxy failures and independent Caddy restarts.
/// This repairs ingress only: it never starts containers or removes existing routes.
pub(crate) fn spawn_caddy_recovery(state: SharedState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if let Err(error) = recover_missing_caddy_routes(&state).await {
                tracing::warn!(%error, "Caddy route recovery failed; will retry");
            }
        }
    });
}

pub(crate) async fn recover_missing_caddy_routes(state: &SharedState) -> Result<(), AppError> {
    let _guard = CADDY_CONFIG_LOCK.lock().await;
    let settings = crate::handler::settings::load(state).await?;
    let mut apps = Vec::new();
    for route in domains::expected_routes(state).await? {
        if route.dial.is_some() {
            // Restore the authentication handler along with the route, never a
            // temporarily public proxy for an application protected by basic auth.
            let auth = basic_auth::load_payload_for_domain(state, &route.domain_id).await?;
            let certificate = crate::deployment::certificates::load(state, &route.route_id).await?;
            apps.push((route, auth, certificate));
        }
    }
    let manual = super::super::manual_routes::list_all(state).await?;
    edit_caddy_config_unlocked(state, |config| {
        if let Some(host) = &settings.domain
            && (!route_present(config, "litepod-server-domain", host, settings.https)
                || !crate::deployment::certificates::matches_config(
                    config,
                    "litepod-server-domain",
                    settings.https,
                    &settings.certificate,
                ))
        {
            crate::handler::settings::apply_server_domain_to_config(
                config,
                &settings,
                &state.caddy_server_upstream,
            )?;
        }
        for (route, auth, certificate) in &apps {
            if route_present(config, &route.route_id, &route.host, route.https)
                && crate::deployment::certificates::matches_config(
                    config,
                    &route.route_id,
                    route.https,
                    certificate,
                )
            {
                continue;
            }
            let Some((container, port)) =
                route.dial.as_deref().and_then(|dial| dial.rsplit_once(':'))
            else {
                continue;
            };
            let port = port
                .parse::<i64>()
                .map_err(|error| AppError::InternalError(error.to_string()))?;
            apply_app_domain_to_config(
                config,
                &route.route_id,
                &route.host,
                container,
                port,
                route.https,
                auth.as_ref(),
            )?;
            crate::deployment::certificates::apply(
                config,
                &route.route_id,
                &route.host,
                route.https,
                certificate,
                None,
            )?;
        }
        for route in &manual {
            let id = super::super::manual_routes::route_id(&route.id);
            if !route_present(config, &id, &route.host, route.https) {
                super::super::manual_routes::apply_manual_route_to_config(config, route)?;
            }
        }
        Ok(())
    })
    .await
}

fn route_present(config: &serde_json::Value, id: &str, host: &str, https: bool) -> bool {
    let has_route = config
        .pointer("/apps/http/servers/srv0/routes")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|routes| routes.iter().any(|route| route["@id"] == id));
    if !has_route || !https {
        return has_route;
    }
    let has_listener = config
        .pointer("/apps/http/servers/srv0/listen")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|listeners| listeners.iter().any(|listener| listener == ":443"));
    let has_sni = config
        .pointer("/apps/http/servers/srv0/tls_connection_policies")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|policies| {
            policies.iter().any(|policy| {
                policy
                    .pointer("/match/sni")
                    .and_then(serde_json::Value::as_array)
                    .is_some_and(|names| names.iter().any(|name| name == host))
            })
        });
    let has_automation = config
        .pointer("/apps/tls/automation/policies")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|policies| {
            policies.iter().any(|policy| {
                policy["subjects"]
                    .as_array()
                    .is_some_and(|names| names.iter().any(|name| name == host))
            })
        });
    let tag = format!("{id}-certificate");
    let has_custom = config
        .pointer("/apps/tls/certificates/load_pem")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|certificates| {
            certificates.iter().any(|certificate| {
                certificate["tags"]
                    .as_array()
                    .is_some_and(|tags| tags.iter().any(|value| value == &tag))
            })
        });
    has_listener && has_sni && (has_automation || has_custom)
}

pub(super) fn apply_app_domain_to_config(
    config: &mut serde_json::Value,
    id: &str,
    hostname: &str,
    container_name: &str,
    container_port: i64,
    https: bool,
    basicauth: Option<&basic_auth::BasicAuthPayload>,
) -> Result<(), AppError> {
    let tls_policy_id = format!("{id}-tls");
    let apps = config
        .as_object_mut()
        .and_then(|root| root.get_mut("apps"))
        .and_then(serde_json::Value::as_object_mut)
        .ok_or_else(|| AppError::InternalError("Caddy configuration has no apps section".into()))?;
    let http = apps
        .entry("http")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or_else(|| AppError::InternalError("Caddy HTTP app is invalid".into()))?;
    let servers = http
        .entry("servers")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or_else(|| AppError::InternalError("Caddy HTTP servers are invalid".into()))?;
    let server = servers
        .entry("srv0")
        .or_insert_with(|| serde_json::json!({ "listen": [":80"], "routes": [] }))
        .as_object_mut()
        .ok_or_else(|| AppError::InternalError("Caddy srv0 is invalid".into()))?;

    let routes = server
        .entry("routes")
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()
        .ok_or_else(|| AppError::InternalError("Caddy routes are invalid".into()))?;
    routes.retain(|route| route.get("@id").and_then(serde_json::Value::as_str) != Some(id));
    // Caddy evaluates handlers in array order. When basicauth is enabled the
    // `authentication` handler must run before the `reverse_proxy` so the
    // upstream is never dialed for an unauthenticated request. When disabled
    // we emit just the proxy handle so older routes with a stale auth handler
    // get cleaned up.
    let mut handles = Vec::with_capacity(2);
    if let Some(payload) = basicauth {
        handles.push(basic_auth::authentication_handle(payload)?);
    }
    handles.push(serde_json::json!({
        "handler": "reverse_proxy",
        "upstreams": [{ "dial": format!("{container_name}:{container_port}") }]
    }));
    // Caddy evaluates routes in order. The bootstrap Caddyfile installs
    // litepod's catch-all UI/API proxy first, so application host routes must be
    // prepended or they will never be reached.
    routes.insert(
        0,
        serde_json::json!({
            "@id": id,
            "match": [{ "host": [hostname] }],
            "handle": handles,
            "terminal": true
        }),
    );

    // Raw JSON posted to the admin API skips Caddyfile-to-JSON adaptation, so
    // the :443 listener and TLS connection policy have to be added by hand or
    // Caddy never negotiates TLS on the domain.
    let listen = server
        .entry("listen")
        .or_insert_with(|| serde_json::json!([":80"]))
        .as_array_mut()
        .ok_or_else(|| AppError::InternalError("Caddy listen addresses are invalid".into()))?;
    if https && !listen.iter().any(|addr| addr == ":443") {
        listen.push(serde_json::Value::String(":443".into()));
    }

    let conn_policies = server
        .entry("tls_connection_policies")
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()
        .ok_or_else(|| {
            AppError::InternalError("Caddy TLS connection policies are invalid".into())
        })?;
    conn_policies.retain(|policy| {
        policy
            .get("match")
            .and_then(|m| m.get("sni"))
            .and_then(serde_json::Value::as_array)
            .is_none_or(|values| !values.iter().any(|value| value.as_str() == Some(hostname)))
    });
    if https {
        conn_policies.push(serde_json::json!({ "match": { "sni": [hostname] } }));
    }

    // Automatic HTTPS also supplies the implicit HTTP->HTTPS redirect, so a
    // domain with https=true doesn't need a hand-written redirect route.
    let automatic_https = server
        .entry("automatic_https")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or_else(|| AppError::InternalError("Caddy automatic HTTPS is invalid".into()))?;
    automatic_https.remove("disable");
    let skipped_hosts = automatic_https
        .entry("skip")
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()
        .ok_or_else(|| AppError::InternalError("Caddy HTTPS skip list is invalid".into()))?;
    skipped_hosts.retain(|host| host.as_str() != Some(hostname));
    if !https {
        skipped_hosts.push(serde_json::Value::String(hostname.into()));
    }
    let remove_automatic_https = automatic_https.is_empty()
        || (automatic_https.len() == 1
            && automatic_https
                .get("skip")
                .is_some_and(|value| value.as_array().is_some_and(Vec::is_empty)));
    if remove_automatic_https {
        server.remove("automatic_https");
    }

    let tls = apps
        .entry("tls")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or_else(|| AppError::InternalError("Caddy TLS app is invalid".into()))?;
    let automation = tls
        .entry("automation")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or_else(|| AppError::InternalError("Caddy TLS automation is invalid".into()))?;
    let policies = automation
        .entry("policies")
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()
        .ok_or_else(|| AppError::InternalError("Caddy TLS policies are invalid".into()))?;
    // Drop our previous policy for this route AND any other policy that already
    // claims this host. Caddy refuses to load a config with two automation
    // policies matching the same host ("cannot apply more than one automation
    // policy to host: …"), so a stale policy left by an earlier delete would
    // otherwise wedge every future `/load`.
    policies.retain(|policy| {
        let same_id =
            policy.get("@id").and_then(serde_json::Value::as_str) == Some(tls_policy_id.as_str());
        let claims_host = policy
            .get("subjects")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|subjects| subjects.iter().any(|s| s.as_str() == Some(hostname)));
        !same_id && !claims_host
    });
    if https {
        policies.insert(
            0,
            serde_json::json!({ "@id": tls_policy_id, "subjects": [hostname] }),
        );
    }
    Ok(())
}

/// Removes an app domain's route and every piece of Caddy config `caddy_route`
/// added for it (TLS policy, connection policy, automatic-HTTPS skip entry).
/// Leaves the shared `:443` listener in place since other domains may still
/// need it.
pub(crate) async fn remove_caddy_route_config(
    state: &SharedState,
    id: &str,
    hostname: &str,
) -> Result<(), AppError> {
    // Best-effort: a transient Caddy outage must not block a domain delete; the
    // next reconcile or an orphan prune clears a route left behind.
    let _ = edit_caddy_config(state, |config| {
        apply_app_domain_removal_to_config(config, id, hostname)
    })
    .await;
    Ok(())
}

/// Rebuilds the live Caddy config keeping only the litepod-owned routes / TLS
/// policies whose id is in `keep_route_ids` (plus everything litepod does not
/// own). This is the console's "Prune orphans" button: it clears routes and TLS
/// automation policies left behind by a deleted application — the state that
/// makes every subsequent `/load` fail with "cannot apply more than one
/// automation policy to host". Returns the number of `srv0` routes dropped.
pub async fn prune_orphaned_caddy_config(
    state: &SharedState,
    keep_route_ids: &std::collections::HashSet<String>,
    keep_hosts: &std::collections::HashSet<String>,
) -> Result<usize, AppError> {
    let mut dropped = 0usize;
    edit_caddy_config(state, |config| {
        dropped = prune_orphans_in_config(config, keep_route_ids, keep_hosts);
        Ok(())
    })
    .await?;
    Ok(dropped)
}

pub(super) fn prune_orphans_in_config(
    config: &mut serde_json::Value,
    keep_route_ids: &std::collections::HashSet<String>,
    keep_hosts: &std::collections::HashSet<String>,
) -> usize {
    let is_ours = |id: &str| {
        id.starts_with("litepod-route-")
            || id.starts_with("litepod-manual-")
            || id.starts_with("litepod-server-domain")
    };
    let route_hosts = |route: &serde_json::Value| -> Vec<String> {
        route
            .get("match")
            .and_then(serde_json::Value::as_array)
            .map(|matchers| {
                matchers
                    .iter()
                    .filter_map(|m| m.get("host").and_then(serde_json::Value::as_array))
                    .flatten()
                    .filter_map(|h| h.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    };

    let mut dropped = 0usize;
    let mut dropped_hosts: std::collections::HashSet<String> = std::collections::HashSet::new();

    if let Some(server) = config
        .pointer_mut("/apps/http/servers/srv0")
        .and_then(serde_json::Value::as_object_mut)
    {
        if let Some(routes) = server
            .get_mut("routes")
            .and_then(serde_json::Value::as_array_mut)
        {
            routes.retain(|route| {
                let keep = match route.get("@id").and_then(serde_json::Value::as_str) {
                    Some(id) if is_ours(id) => keep_route_ids.contains(id),
                    _ => true,
                };
                if !keep {
                    dropped += 1;
                    dropped_hosts.extend(route_hosts(route));
                }
                keep
            });
        }
        // Only prune host-keyed entries for hosts that lost their route and are
        // not still claimed by something we keep.
        dropped_hosts.retain(|host| !keep_hosts.contains(host));
        if let Some(conn) = server
            .get_mut("tls_connection_policies")
            .and_then(serde_json::Value::as_array_mut)
        {
            conn.retain(|policy| {
                policy
                    .get("match")
                    .and_then(|m| m.get("sni"))
                    .and_then(serde_json::Value::as_array)
                    .is_none_or(|snis| {
                        !snis
                            .iter()
                            .any(|s| s.as_str().is_some_and(|h| dropped_hosts.contains(h)))
                    })
            });
        }
        if let Some(skip) = server
            .get_mut("automatic_https")
            .and_then(|value| value.get_mut("skip"))
            .and_then(serde_json::Value::as_array_mut)
        {
            skip.retain(|h| h.as_str().is_none_or(|h| !dropped_hosts.contains(h)));
        }
    }

    if let Some(policies) = config
        .pointer_mut("/apps/tls/automation/policies")
        .and_then(serde_json::Value::as_array_mut)
    {
        policies.retain(|policy| {
            let Some(id) = policy.get("@id").and_then(serde_json::Value::as_str) else {
                return true;
            };
            if !id.starts_with("litepod-") {
                return true;
            }
            let base = id.strip_suffix("-tls").unwrap_or(id);
            keep_route_ids.contains(base) || keep_route_ids.contains(id)
        });
    }

    dropped
}

pub(super) fn apply_app_domain_removal_to_config(
    config: &mut serde_json::Value,
    id: &str,
    hostname: &str,
) -> Result<(), AppError> {
    let tls_policy_id = format!("{id}-tls");
    let Some(server) = config
        .as_object_mut()
        .and_then(|root| root.get_mut("apps"))
        .and_then(|apps| apps.get_mut("http"))
        .and_then(|http| http.get_mut("servers"))
        .and_then(|servers| servers.get_mut("srv0"))
        .and_then(serde_json::Value::as_object_mut)
    else {
        return Ok(());
    };
    if let Some(routes) = server
        .get_mut("routes")
        .and_then(serde_json::Value::as_array_mut)
    {
        routes.retain(|route| route.get("@id").and_then(serde_json::Value::as_str) != Some(id));
    }
    if let Some(conn_policies) = server
        .get_mut("tls_connection_policies")
        .and_then(serde_json::Value::as_array_mut)
    {
        conn_policies.retain(|policy| {
            policy
                .get("match")
                .and_then(|m| m.get("sni"))
                .and_then(serde_json::Value::as_array)
                .is_none_or(|values| !values.iter().any(|value| value.as_str() == Some(hostname)))
        });
    }
    if let Some(skip) = server
        .get_mut("automatic_https")
        .and_then(|value| value.get_mut("skip"))
        .and_then(serde_json::Value::as_array_mut)
    {
        skip.retain(|host| host.as_str() != Some(hostname));
    }
    if let Some(policies) = config
        .get_mut("apps")
        .and_then(|apps| apps.get_mut("tls"))
        .and_then(|tls| tls.get_mut("automation"))
        .and_then(|automation| automation.get_mut("policies"))
        .and_then(serde_json::Value::as_array_mut)
    {
        policies.retain(|policy| {
            policy.get("@id").and_then(serde_json::Value::as_str) != Some(tls_policy_id.as_str())
        });
    }
    crate::deployment::certificates::remove_from_config(config, id, hostname);
    Ok(())
}
