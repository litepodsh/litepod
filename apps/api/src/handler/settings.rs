use axum::{
    Extension, Json,
    extract::{Path, State},
    response::IntoResponse,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::podman::podman_socket::podman_client as podman;
use crate::security::middleware::auth::AuthUser;
use crate::{
    dtos::response::{AppError, ok},
    state::state::SharedState,
};

#[derive(Debug, Serialize)]
pub struct BuildConfigurationDto {
    pub max_concurrent_builds: i64,
    pub active_builds: i64,
}

#[derive(Debug, Deserialize)]
pub struct UpdateBuildConfigurationRequest {
    pub max_concurrent_builds: i64,
}

pub async fn get_build_configuration_handler(
    State(state): State<SharedState>,
    Extension(user): Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    if !user.is_sysadmin {
        return Err(AppError::InsufficientPermissions);
    }
    use sqlx::Row;
    let conn = crate::git::repositories::db::get_conn(&state).await;
    let limit = sqlx::query("SELECT max_concurrent_builds FROM build_configuration WHERE id = 1")
        .fetch_optional(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?
        .ok_or(AppError::DatabaseError(
            "Build configuration is missing".into(),
        ))?
        .try_get::<i64, _>(0)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let active_builds = sqlx::query(
        "SELECT COUNT(*) FROM deployment_jobs WHERE status IN ('building', 'deploying')",
    )
    .fetch_optional(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?
    .map(|row| row.try_get::<i64, _>(0))
    .transpose()
    .map_err(|e| AppError::DatabaseError(e.to_string()))?
    .unwrap_or(0);
    Ok(ok(BuildConfigurationDto {
        max_concurrent_builds: limit,
        active_builds,
    }))
}

pub async fn update_build_configuration_handler(
    State(state): State<SharedState>,
    Extension(user): Extension<AuthUser>,
    Json(request): Json<UpdateBuildConfigurationRequest>,
) -> Result<impl IntoResponse, AppError> {
    if !user.is_sysadmin {
        return Err(AppError::InsufficientPermissions);
    }
    if !(1..=64).contains(&request.max_concurrent_builds) {
        return Err(AppError::ValidationError(
            "Maximum concurrent builds must be between 1 and 64".into(),
        ));
    }
    let conn = crate::git::repositories::db::get_conn(&state).await;
    sqlx::query("UPDATE build_configuration SET max_concurrent_builds = ?1 WHERE id = 1")
        .bind(request.max_concurrent_builds)
        .execute(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    state.queue_runtime.notify.notify_one();
    let _ = crate::security::security_events::record(
        &state,
        None,
        &user,
        "settings.build_config_updated",
        Some(&request.max_concurrent_builds.to_string()),
    )
    .await;
    get_build_configuration_handler(State(state), Extension(user)).await
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ScheduledTaskHistorySettings {
    pub history_retention_days: i64,
}

pub async fn get_scheduled_task_history_handler(
    State(state): State<SharedState>,
    Extension(user): Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    if !user.is_sysadmin {
        return Err(AppError::InsufficientPermissions);
    }
    Ok(ok(ScheduledTaskHistorySettings {
        history_retention_days: crate::scheduled_tasks::history_retention_days(&state).await?,
    }))
}

pub async fn update_scheduled_task_history_handler(
    State(state): State<SharedState>,
    Extension(user): Extension<AuthUser>,
    Json(request): Json<ScheduledTaskHistorySettings>,
) -> Result<impl IntoResponse, AppError> {
    if !user.is_sysadmin {
        return Err(AppError::InsufficientPermissions);
    }
    if !(0..=3650).contains(&request.history_retention_days) {
        return Err(AppError::ValidationError(
            "History retention must be between 0 and 3650 days".into(),
        ));
    }
    let conn = crate::git::repositories::db::get_conn(&state).await;
    sqlx::query("UPDATE scheduled_task_settings SET history_retention_days = ?1 WHERE id = 1")
        .bind(request.history_retention_days)
        .execute(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    // apply a shortened window right away instead of waiting for the next hourly purge
    if let Err(error) = crate::scheduled_tasks::purge_expired_history(&state).await {
        tracing::warn!(%error, "scheduled tasks history purge failed");
    }
    let _ = crate::security::security_events::record(
        &state,
        None,
        &user,
        "settings.scheduled_task_history_updated",
        Some(&request.history_retention_days.to_string()),
    )
    .await;
    get_scheduled_task_history_handler(State(state), Extension(user)).await
}

pub async fn get_rate_limit_handler(
    State(state): State<SharedState>,
    Extension(user): Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    if !user.is_sysadmin {
        return Err(AppError::InsufficientPermissions);
    }
    let config = state.rate_limiter.read().await.config;
    Ok(ok(config))
}

pub async fn update_rate_limit_handler(
    State(state): State<SharedState>,
    Extension(user): Extension<AuthUser>,
    Json(request): Json<crate::security::rate_limit::RateLimitConfig>,
) -> Result<impl IntoResponse, AppError> {
    if !user.is_sysadmin {
        return Err(AppError::InsufficientPermissions);
    }
    if request.global_per_second == 0
        || request.global_burst == 0
        || request.auth_per_second == 0
        || request.auth_burst == 0
        || request.auth_max_attempts == 0
        || request.auth_ban_minutes <= 0
    {
        return Err(AppError::ValidationError(
            "Rate limit values must be positive".into(),
        ));
    }
    let conn = crate::git::repositories::db::get_conn(&state).await;
    sqlx::query(
        "UPDATE rate_limit_settings SET enabled = ?1, global_per_second = ?2, global_burst = ?3, auth_per_second = ?4, auth_burst = ?5, auth_max_attempts = ?6, auth_ban_minutes = ?7, updated_at = datetime('now') WHERE id = 1",
    )
    .bind(request.enabled as i64)
    .bind(request.global_per_second as i64)
    .bind(request.global_burst as i64)
    .bind(request.auth_per_second as i64)
    .bind(request.auth_burst as i64)
    .bind(request.auth_max_attempts as i64)
    .bind(request.auth_ban_minutes)
    .execute(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    state.rate_limiter.write().await.apply_config(request);
    let _ = crate::security::security_events::record(
        &state,
        None,
        &user,
        "settings.rate_limits_updated",
        None,
    )
    .await;
    get_rate_limit_handler(State(state), Extension(user)).await
}

/// Container name for each restartable infra target, overridable so a
/// renamed compose project (or staging's fixed `litepod-*` names) still resolves.
pub(crate) fn infrastructure_container_name(target: &str) -> Result<String, AppError> {
    let (default, env_var) = match target {
        "caddy" => ("litepod-caddy", "CADDY_CONTAINER"),
        "api" => ("litepod-api", "API_CONTAINER"),
        _ => return Err(AppError::ResourceNotFound),
    };
    Ok(std::env::var(env_var).unwrap_or_else(|_| default.to_string()))
}

pub async fn restart_infrastructure_handler(
    State(state): State<SharedState>,
    Extension(user): Extension<AuthUser>,
    Path(target): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    if !user.is_sysadmin {
        return Err(AppError::InsufficientPermissions);
    }
    let name = infrastructure_container_name(&target)?;
    let client = podman(&state)?;
    let container = client.containers().get(&name);
    container.inspect().await.map_err(|_| {
        AppError::ValidationError(format!(
            "Container '{name}' was not found — it may not run in this environment (dev API runs on the host, not in a container)"
        ))
    })?;

    // Queued before the restart, not after: restarting the API kills this
    // process, and a notification written to the queue survives that while an
    // in-flight HTTP request would not.
    crate::notifications::notify_restart(
        &state,
        &name,
        Some(crate::handler::common::actor_label(&user).as_str()),
    )
    .await;

    let _ = crate::security::security_events::record(
        &state,
        None,
        &user,
        "infrastructure.restarted",
        Some(&target),
    )
    .await;

    if target == "api" {
        // Restarting our own container kills this process mid-response. Ack first and
        // let Podman actually cycle the container shortly after the response flushes.
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            let _ = container.restart().await;
        });
        return Ok(ok(json!({ "target": target, "restarting": true })));
    }

    container
        .restart()
        .await
        .map_err(|error| AppError::InternalError(error.to_string()))?;
    Ok(ok(json!({ "target": target, "restarting": false })))
}

static SERVER_DOMAIN_WRITE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const SERVER_DOMAIN_ROUTE_ID: &str = "litepod-server-domain";
const SERVER_DOMAIN_TLS_POLICY_ID: &str = "litepod-server-domain-tls";

#[derive(Debug, Serialize)]
pub struct ServerDomainSettings {
    pub domain: Option<String>,
    pub email: Option<String>,
    pub https: bool,
    #[serde(flatten)]
    pub certificate: crate::deployment::certificates::CertificateSettings,
}

#[derive(Debug, Deserialize)]
pub struct UpdateServerDomainSettings {
    #[serde(flatten)]
    pub certificate: crate::deployment::certificates::CertificateRequest,
    pub domain: String,
    pub email: String,
    pub https: bool,
}

pub async fn get_server_domain_handler(
    State(state): State<SharedState>,
    Extension(user): Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    if !user.is_sysadmin {
        return Err(AppError::InsufficientPermissions);
    }
    Ok(ok(load(&state).await?))
}

pub async fn update_server_domain_handler(
    State(state): State<SharedState>,
    Extension(user): Extension<AuthUser>,
    Json(request): Json<UpdateServerDomainSettings>,
) -> Result<impl IntoResponse, AppError> {
    if !user.is_sysadmin {
        return Err(AppError::InsufficientPermissions);
    }
    let _guard = SERVER_DOMAIN_WRITE_LOCK.lock().await;
    let _certificates = crate::deployment::certificates::INVENTORY_LOCK.lock().await;
    let domain = normalize_domain(&request.domain)?;
    let email = normalize_email(&request.email)?;
    let previous = load(&state).await?;
    let certificate = crate::deployment::certificates::prepare_for_state(
        &state,
        &previous.certificate,
        &request.certificate,
        &domain,
        request.https,
    )
    .await?;
    let settings = ServerDomainSettings {
        domain: Some(domain.clone()),
        email: Some(email.clone()),
        https: request.https,
        certificate,
    };

    // Apply Caddy first. A failed update must never leave the UI claiming that
    // the server is reachable at a domain Caddy is not serving.
    sync_caddy(&state, &settings).await?;
    let conn = crate::git::repositories::db::get_conn(&state).await;
    let saved = async {
    let mut tx = conn.begin().await.map_err(|error| AppError::DatabaseError(error.to_string()))?;
    sqlx::query(
        "INSERT INTO server_domain_settings (id, domain, email, https, updated_at) VALUES (1, ?1, ?2, ?3, ?4)\
         ON CONFLICT(id) DO UPDATE SET domain = excluded.domain, email = excluded.email, https = excluded.https, updated_at = excluded.updated_at",
    )
    .bind(&domain)
    .bind(email)
    .bind(request.https as i64)
    .bind(Utc::now().timestamp_millis())
    .execute(&mut *tx)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    crate::deployment::certificates::save_on(&mut tx, SERVER_DOMAIN_ROUTE_ID, &settings.certificate).await?;
    tx.commit().await.map_err(|error| AppError::DatabaseError(error.to_string()))
    }.await;
    if let Err(error) = saved {
        let _ = sync_caddy(&state, &previous).await;
        return Err(error);
    }

    let _ = crate::security::security_events::record(
        &state,
        None,
        &user,
        "settings.server_domain_updated",
        Some(&domain),
    )
    .await;
    Ok(ok(settings))
}

pub async fn load(state: &SharedState) -> Result<ServerDomainSettings, AppError> {
    use sqlx::Row;
    let conn = crate::git::repositories::db::get_conn(state).await;
    let Some(row) =
        sqlx::query("SELECT domain, email, https FROM server_domain_settings WHERE id = 1")
            .fetch_optional(&conn)
            .await
            .map_err(|error| AppError::DatabaseError(error.to_string()))?
    else {
        return Ok(ServerDomainSettings {
            domain: None,
            email: None,
            https: true,
            certificate: Default::default(),
        });
    };
    Ok(ServerDomainSettings {
        domain: Some(
            row.try_get(0)
                .map_err(|error| AppError::DatabaseError(error.to_string()))?,
        ),
        email: Some(
            row.try_get(1)
                .map_err(|error| AppError::DatabaseError(error.to_string()))?,
        ),
        https: row
            .try_get::<i64, _>(2)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?
            != 0,
        certificate: crate::deployment::certificates::load(state, SERVER_DOMAIN_ROUTE_ID).await?,
    })
}

/// The URL the outside world reaches this server on: the configured server domain when one is
/// set, otherwise the LITEPOD_PUBLIC_URL/PUBLIC_URL fallback. Used for anything an external service
/// has to call back into — GitHub App callbacks, webhooks, deploy hooks.
pub async fn public_base_url(state: &SharedState) -> String {
    match load(state).await {
        Ok(settings) => match settings.domain.filter(|domain| !domain.trim().is_empty()) {
            Some(domain) => format!(
                "{}://{}",
                if settings.https { "https" } else { "http" },
                domain.trim().trim_end_matches('/')
            ),
            None => state.public_url.trim_end_matches('/').to_string(),
        },
        // Settings are unreadable rather than unset: fall back rather than break the caller.
        Err(_) => state.public_url.trim_end_matches('/').to_string(),
    }
}

/// Re-apply the persisted route after Caddy restarts. Errors are deliberately
/// logged by the caller: the API can still start while Caddy is coming up.
pub async fn restore_caddy(state: &SharedState) -> Result<(), AppError> {
    let settings = load(state).await?;
    if settings.domain.is_some() {
        sync_caddy(state, &settings).await?;
    }
    Ok(())
}

async fn sync_caddy(state: &SharedState, settings: &ServerDomainSettings) -> Result<(), AppError> {
    crate::deployment::operations::edit_caddy_config(state, |config| {
        apply_server_domain_to_config(config, settings, &state.caddy_server_upstream)
    })
    .await
}

pub(crate) fn apply_server_domain_to_config(
    config: &mut Value,
    settings: &ServerDomainSettings,
    upstream: &str,
) -> Result<(), AppError> {
    let apps = config
        .as_object_mut()
        .and_then(|root| root.get_mut("apps"))
        .and_then(Value::as_object_mut)
        .ok_or_else(|| AppError::InternalError("Caddy configuration has no apps section".into()))?;
    let http = apps
        .entry("http")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| AppError::InternalError("Caddy HTTP app is invalid".into()))?;
    let servers = http
        .entry("servers")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| AppError::InternalError("Caddy HTTP servers are invalid".into()))?;
    let server = servers
        .entry("srv0")
        .or_insert_with(|| json!({ "listen": [":80"], "routes": [] }))
        .as_object_mut()
        .ok_or_else(|| AppError::InternalError("Caddy srv0 is invalid".into()))?;
    let routes = server
        .entry("routes")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or_else(|| AppError::InternalError("Caddy routes are invalid".into()))?;
    let previous_domain = routes
        .iter()
        .find(|route| route.get("@id").and_then(Value::as_str) == Some(SERVER_DOMAIN_ROUTE_ID))
        .and_then(|route| route.get("match"))
        .and_then(Value::as_array)
        .and_then(|matches| matches.first())
        .and_then(|matcher| matcher.get("host"))
        .and_then(Value::as_array)
        .and_then(|hosts| hosts.first())
        .and_then(Value::as_str)
        .map(str::to_owned);
    routes.retain(|route| route.get("@id").and_then(Value::as_str) != Some(SERVER_DOMAIN_ROUTE_ID));

    if let Some(domain) = &settings.domain {
        routes.insert(
            0,
            json!({
                "@id": SERVER_DOMAIN_ROUTE_ID,
                "match": [{ "host": [domain] }],
                "handle": [{ "handler": "reverse_proxy", "upstreams": [{ "dial": upstream }] }],
                "terminal": true,
            }),
        );
    }

    // Caddyfile adaptation auto-adds the :443 listener and a matching TLS
    // connection policy for any hostname it finds in a route — but that only
    // happens during Caddyfile-to-JSON adaptation. This endpoint POSTs raw JSON
    // straight to the admin API, which skips that step entirely, so both have
    // to be added explicitly or Caddy accepts the TLS automation policy below
    // but never actually listens for/negotiates TLS on the domain.
    let listen = server
        .entry("listen")
        .or_insert_with(|| json!([":80"]))
        .as_array_mut()
        .ok_or_else(|| AppError::InternalError("Caddy listen addresses are invalid".into()))?;
    let want_https = settings.https && settings.domain.is_some();
    let inherited = crate::deployment::caddy_admin::inherits_sockets(listen);
    if want_https && !inherited && !listen.iter().any(|addr| addr == ":443") {
        listen.push(Value::String(":443".into()));
    }
    crate::deployment::caddy_admin::adapt_inherited_sockets(server, want_https);

    let conn_policies = server
        .entry("tls_connection_policies")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or_else(|| {
            AppError::InternalError("Caddy TLS connection policies are invalid".into())
        })?;
    let policy_matches = |policy: &Value, domain: &str| {
        policy
            .get("match")
            .and_then(|m| m.get("sni"))
            .and_then(Value::as_array)
            .is_some_and(|values| values.iter().any(|value| value.as_str() == Some(domain)))
    };
    conn_policies.retain(|policy| {
        !previous_domain
            .as_deref()
            .is_some_and(|domain| policy_matches(policy, domain))
            && !settings
                .domain
                .as_deref()
                .is_some_and(|domain| policy_matches(policy, domain))
    });
    if settings.https {
        if let Some(domain) = &settings.domain {
            conn_policies.push(json!({ "match": { "sni": [domain] } }));
        }
    }

    // The project previously disabled Caddy automatic HTTPS globally. Removing
    // that server-level flag makes the host route above eligible for Caddy's
    // built-in certificate automation and HTTPS redirects.
    let automatic_https = server
        .entry("automatic_https")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| AppError::InternalError("Caddy automatic HTTPS is invalid".into()))?;
    automatic_https.remove("disable");
    let skipped_hosts = automatic_https
        .entry("skip")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or_else(|| AppError::InternalError("Caddy HTTPS skip list is invalid".into()))?;
    skipped_hosts.retain(|host| {
        let host = host.as_str();
        host != previous_domain.as_deref() && host != settings.domain.as_deref()
    });
    if !settings.https {
        if let Some(domain) = &settings.domain {
            skipped_hosts.push(Value::String(domain.clone()));
        }
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
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| AppError::InternalError("Caddy TLS app is invalid".into()))?;
    let automation = tls
        .entry("automation")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| AppError::InternalError("Caddy TLS automation is invalid".into()))?;
    let policies = automation
        .entry("policies")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or_else(|| AppError::InternalError("Caddy TLS policies are invalid".into()))?;
    policies.retain(|policy| {
        policy.get("@id").and_then(Value::as_str) != Some(SERVER_DOMAIN_TLS_POLICY_ID)
    });
    if settings.https && settings.domain.is_some() {
        let domain = settings
            .domain
            .as_ref()
            .expect("validated server-domain settings include a domain");
        let email = settings
            .email
            .as_ref()
            .expect("validated server-domain settings include an email");
        // No issuer is hard-coded here. Caddy's standard issuer policy includes
        // ZeroSSL and ACME, and avoids requiring a ZeroSSL API key when one has
        // not been configured on the Caddy server. Keep the contact address in
        // litepod's settings so a ZeroSSL-only policy can be configured later.
        let _ = email;
        policies.insert(
            0,
            json!({ "@id": SERVER_DOMAIN_TLS_POLICY_ID, "subjects": [domain] }),
        );
    }
    if let Some(previous) = previous_domain.as_deref() {
        crate::deployment::certificates::remove_from_config(
            config,
            SERVER_DOMAIN_ROUTE_ID,
            previous,
        );
    }
    if let Some(domain) = &settings.domain {
        crate::deployment::certificates::apply(
            config,
            SERVER_DOMAIN_ROUTE_ID,
            domain,
            settings.https,
            &settings.certificate,
            settings.email.as_deref(),
        )?;
    }
    Ok(())
}

fn normalize_domain(value: &str) -> Result<String, AppError> {
    let domain = value.trim().trim_end_matches('.').to_ascii_lowercase();
    if domain.is_empty()
        || domain.len() > 253
        || domain.contains("//")
        || !domain
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'))
    {
        return Err(AppError::ValidationError(
            "Enter a valid domain name without a scheme, path, or port".into(),
        ));
    }
    if !domain.contains('.')
        || domain.split('.').any(|label| {
            label.is_empty() || label.len() > 63 || label.starts_with('-') || label.ends_with('-')
        })
    {
        return Err(AppError::ValidationError(
            "Enter a valid fully-qualified domain name".into(),
        ));
    }
    Ok(domain)
}

fn normalize_email(value: &str) -> Result<String, AppError> {
    let email = value.trim().to_ascii_lowercase();
    if email.is_empty()
        || email.len() > 254
        || !email.contains('@')
        || email.starts_with('@')
        || email.ends_with('@')
    {
        return Err(AppError::ValidationError(
            "Enter a valid certificate contact email".into(),
        ));
    }
    Ok(email)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_domain_custom_certificate_and_issuer_switch_are_complete() {
        let (pem, key) = crate::deployment::certificates::tests::pair("litepod.example.com", false);
        let custom = crate::deployment::certificates::prepare(
            &Default::default(),
            &crate::deployment::certificates::CertificateRequest {
                certificate_provider: Some("custom".into()),
                certificate_pem: Some(pem),
                private_key_pem: Some(key),
                certificate_id: None,
            },
            "litepod.example.com",
            true,
        )
        .unwrap();
        let mut settings = ServerDomainSettings {
            domain: Some("litepod.example.com".into()),
            email: Some("admin@example.com".into()),
            https: true,
            certificate: custom,
        };
        let mut config = json!({"apps": {}});
        apply_server_domain_to_config(&mut config, &settings, "app:6001").unwrap();
        assert_eq!(
            config["apps"]["tls"]["certificates"]["load_pem"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        settings.certificate = crate::deployment::certificates::CertificateSettings {
            certificate_provider: "letsencrypt".into(),
            ..Default::default()
        };
        apply_server_domain_to_config(&mut config, &settings, "app:6001").unwrap();
        assert!(
            config["apps"]["tls"]["certificates"]["load_pem"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            config["apps"]["tls"]["automation"]["policies"][0]["issuers"][0]["ca"],
            "https://acme-v02.api.letsencrypt.org/directory"
        );
        assert!(
            config["apps"]["http"]["servers"]["srv0"]["automatic_https"]["skip_certificates"]
                .as_array()
                .is_none_or(Vec::is_empty)
        );
    }

    #[test]
    fn adds_a_server_route_and_tls_policy() {
        let mut config = json!({ "apps": { "http": { "servers": { "srv0": { "listen": [":80"], "automatic_https": { "disable": true }, "routes": [] } } } } });
        let settings = ServerDomainSettings {
            domain: Some("litepod.example.com".into()),
            email: Some("ops@example.com".into()),
            https: true,
            certificate: Default::default(),
        };
        apply_server_domain_to_config(&mut config, &settings, "app:6001").unwrap();
        assert_eq!(
            config["apps"]["http"]["servers"]["srv0"]["routes"][0]["match"][0]["host"][0],
            "litepod.example.com"
        );
        assert!(
            config["apps"]["http"]["servers"]["srv0"]
                .get("automatic_https")
                .is_none()
        );
        assert_eq!(
            config["apps"]["tls"]["automation"]["policies"][0]["subjects"][0],
            "litepod.example.com"
        );
        let listen = config["apps"]["http"]["servers"]["srv0"]["listen"]
            .as_array()
            .unwrap();
        assert!(listen.iter().any(|addr| addr == ":443"));
        assert_eq!(
            config["apps"]["http"]["servers"]["srv0"]["tls_connection_policies"][0]["match"]["sni"]
                [0],
            "litepod.example.com"
        );
    }

    #[test]
    fn skips_certificate_automation_when_https_is_disabled() {
        let mut config = json!({ "apps": { "http": { "servers": { "srv0": { "routes": [] } } } } });
        let settings = ServerDomainSettings {
            domain: Some("litepod.example.com".into()),
            email: Some("ops@example.com".into()),
            https: false,
            certificate: Default::default(),
        };
        apply_server_domain_to_config(&mut config, &settings, "app:6001").unwrap();
        assert_eq!(
            config["apps"]["http"]["servers"]["srv0"]["automatic_https"]["skip"][0],
            "litepod.example.com"
        );
        assert!(
            config
                .get("apps")
                .and_then(|apps| apps.get("tls"))
                .is_some()
        );
        let listen = config["apps"]["http"]["servers"]["srv0"]["listen"]
            .as_array()
            .unwrap();
        assert!(!listen.iter().any(|addr| addr == ":443"));
        assert!(
            config["apps"]["http"]["servers"]["srv0"]["tls_connection_policies"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn removes_stale_tls_connection_policy_when_domain_changes() {
        let mut config = json!({ "apps": { "http": { "servers": { "srv0": {
            "listen": [":80", ":443"],
            "routes": [{ "@id": SERVER_DOMAIN_ROUTE_ID, "match": [{ "host": ["old.example.com"] }] }],
            "tls_connection_policies": [{ "match": { "sni": ["old.example.com"] } }],
        } } } } });
        let settings = ServerDomainSettings {
            domain: Some("new.example.com".into()),
            email: Some("ops@example.com".into()),
            https: true,
            certificate: Default::default(),
        };
        apply_server_domain_to_config(&mut config, &settings, "app:6001").unwrap();
        let policies = config["apps"]["http"]["servers"]["srv0"]["tls_connection_policies"]
            .as_array()
            .unwrap();
        assert_eq!(policies.len(), 1);
        assert_eq!(policies[0]["match"]["sni"][0], "new.example.com");
    }

    #[test]
    fn rejects_urls_and_invalid_domains() {
        assert!(normalize_domain("https://litepod.example.com").is_err());
        assert!(normalize_domain("litepod.example.com").is_ok());
        assert!(normalize_email("ops@example.com").is_ok());
    }
}
