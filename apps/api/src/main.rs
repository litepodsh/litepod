use dotenvy::dotenv;
use std::net::SocketAddr;
use std::str::FromStr;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use crate::router::router::create_router;
use crate::state::state::{AppState, SharedState};

mod assistant;
mod backup;
mod billing;
mod cli;
mod cqrs;
mod database_backup;
mod deployment;
mod dtos;
mod git;
mod handler;
mod health_checks;
mod icons;
mod loki;
mod mcp;
mod metrics_history;
mod migrations;
mod notifications;
mod onboarding;
mod podman;
mod router;
mod scheduled_tasks;
mod security;
mod servers;
mod ee;
mod state;
mod templates;
#[cfg(test)]
mod testing;
mod updater;
mod users;
mod utils;

fn main() {
    // Operator subcommands (reset-password, reset-2fa, change-email,
    // list-sysadmins) are dispatched at the very top of `main()` so the
    // process never binds the HTTP listener, opens Dragonfly, or starts
    // any reconciler when the only thing the operator needs is a single
    // SQL update against the database. See `cli::run`.
    let argv: Vec<String> = std::env::args().collect();
    if argv.len() > 1 {
        std::process::exit(cli::run(argv));
    }

    eprintln!("[boot] starting");

    // Both `ring` and `aws-lc-rs` are in the dependency tree (reqwest,
    // rustls-platform-verifier, ...), so rustls can't auto-select a process-wide
    // crypto provider — install one explicitly, before any TLS connection can be
    // attempted.
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("[boot] failed to install rustls crypto provider");

    if let Err(e) = dotenv() {
        eprintln!("[boot] no .env file: {e}");
    }

    eprintln!(
        "[boot] env: APP_ENV={:?} PORT={:?} DATABASE_PATH={:?} SENTRY={:?} TRACES_SAMPLE_RATE={:?}",
        std::env::var("APP_ENV").ok(),
        std::env::var("PORT").ok(),
        std::env::var("DATABASE_PATH").ok(),
        std::env::var("SENTRY_DSN").ok(),
        std::env::var("SENTRY_TRACES_SAMPLE_RATE").ok(),
    );

    let sentry_dsn = std::env::var("SENTRY_DSN")
        .ok()
        .filter(|dsn| !dsn.trim().is_empty());

    // Self-hosted (GlitchTip or Sentry self-hosted) vs Sentry SaaS, purely by DSN host.
    let sentry_kind = sentry_dsn.as_deref().map(|dsn| {
        if dsn.contains("sentry.io") {
            "cloud"
        } else {
            "self-hosted"
        }
    });

    let _sentry_guard = sentry_dsn.as_ref().map(|dsn| {
        let sentry_env = std::env::var("APP_ENV").unwrap_or_else(|_| "dev".to_string());
        let traces_sample_rate = std::env::var("SENTRY_TRACES_SAMPLE_RATE")
            .ok()
            .and_then(|value| value.parse::<f32>().ok())
            .unwrap_or_else(|| match sentry_env.as_str() {
                "dev" => 1.0,
                "qa" | "staging" => 0.5,
                _ => 0.1,
            });

        sentry::init(
            sentry::ClientOptions::new()
                .dsn(dsn.as_str())
                .maybe_release(sentry::release_name!())
                .environment(sentry_env.clone())
                .send_default_pii(true)
                .traces_sample_rate(traces_sample_rate),
        )
    });

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("[boot] failed to build tokio runtime");

    rt.block_on(async_main(sentry_kind));
}

/// The name Podman knows this container by.
///
/// This has to be a *name*, not an id: the self-updater recreates the container
/// under whatever this returns (`updater::apply::pull_and_recreate`), and Caddy's
/// bootstrap catch-all dials `litepod-api:6001` by DNS name. Reading `HOSTNAME`
/// gives the container's short id instead, so a single self-update used to
/// rename the container to a hex string and permanently break that catch-all -
/// every host without an explicit route started answering 502.
///
/// Resolution order: `LITEPOD_CONTAINER_NAME`, then the real name from
/// `podman inspect`, then `litepod-api`. An install already renamed to its own id
/// is renamed back, which is metadata-only and keeps the container running.
async fn resolve_self_container_name(podman_socket: &str) -> String {
    const DEFAULT: &str = "litepod-api";

    if let Ok(configured) = std::env::var("LITEPOD_CONTAINER_NAME")
        && !configured.trim().is_empty()
    {
        return configured;
    }
    let Ok(hostname) = std::env::var("HOSTNAME") else {
        return DEFAULT.to_string();
    };
    let Ok(podman) = podman_api::Podman::new(podman_socket) else {
        return DEFAULT.to_string();
    };
    let Ok(info) = podman.containers().get(&hostname).inspect().await else {
        // Not running under Podman, or the socket is not reachable yet.
        return DEFAULT.to_string();
    };
    let name = info
        .name
        .map(|name| name.trim_start_matches('/').to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| hostname.clone());
    if name != hostname {
        return name;
    }

    // The container is named after its own id, so a previous update already lost
    // the DNS name. Take `litepod-api` back if nothing else holds it.
    if podman.containers().get(DEFAULT).inspect().await.is_ok() {
        tracing::warn!(
            container = %name,
            "this container is named after its own id and {DEFAULT} is taken; \
             Caddy's catch-all will not resolve"
        );
        return name;
    }
    match podman.containers().get(&name).rename(DEFAULT).await {
        Ok(()) => {
            tracing::warn!(from = %name, to = %DEFAULT, "renamed self back to the expected container name");
            DEFAULT.to_string()
        }
        Err(error) => {
            tracing::warn!(container = %name, %error, "could not rename self back to {DEFAULT}");
            name
        }
    }
}

async fn async_main(sentry_kind: Option<&'static str>) {
    let version = std::env::var("LITEPOD_VERSION").unwrap_or_else(|_| "dev".to_string());
    let git_sha = std::env::var("LITEPOD_GIT_SHA").unwrap_or_else(|_| "unknown".to_string());
    let is_cloud = matches!(std::env::var("IS_CLOUD").as_deref(), Ok("true"));
    let polar = crate::billing::config::PolarConfig::from_env()
        .unwrap_or_else(|error| panic!("[boot] invalid Polar configuration: {error}"));
    tracing::info!(
        configured = polar.is_some(),
        "Polar Billing configuration loaded"
    );
    eprintln!("[boot] version={version} sha={git_sha}");

    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let stdout_layer = tracing_subscriber::fmt::layer()
        .json()
        .flatten_event(true)
        .with_current_span(true)
        .with_span_list(false);

    // Temporarily disabled regardless of LOKI_URL — revert this hunk to
    // restore normal Loki shipping.
    let (loki_layer, loki_enabled): (Option<crate::loki::LokiLayer>, bool) = (None, false);

    tracing_subscriber::registry()
        .with(env_filter)
        .with(stdout_layer)
        .with(loki_layer)
        .with(sentry::integrations::tracing::layer())
        .init();

    match sentry_kind {
        Some(kind) => tracing::info!(kind, "sentry enabled"),
        None => tracing::info!("sentry disabled (no SENTRY_DSN)"),
    }
    if loki_enabled {
        tracing::info!("Loki log shipping enabled");
    } else {
        tracing::info!("Loki log shipping disabled");
    }

    let configured_update_channel = std::env::var("LITEPOD_UPDATE_CHANNEL");
    let update_channel = crate::updater::UpdateChannel::parse(
        &configured_update_channel.as_deref().unwrap_or("stable"),
    );
    if let Ok(value) = &configured_update_channel
        && !matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "stable" | "alpha"
        )
    {
        tracing::warn!(
            value = %value,
            "invalid LITEPOD_UPDATE_CHANNEL; falling back to stable"
        );
    }

    let database_path =
        std::env::var("DATABASE_PATH").unwrap_or_else(|_| "./data/litepod.db".to_string());
    let database_path_ref = std::path::Path::new(&database_path);
    if let Some(parent) = database_path_ref
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)
            .unwrap_or_else(|error| panic!("[boot] failed to create database directory: {error}"));
    }
    let connect_options = sqlx::sqlite::SqliteConnectOptions::from_str(&database_path)
        .unwrap_or_else(|error| panic!("[boot] invalid DATABASE_PATH {database_path}: {error}"))
        .create_if_missing(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .foreign_keys(true)
        .busy_timeout(std::time::Duration::from_millis(5000));
    let db_pool = sqlx::sqlite::SqlitePoolOptions::new()
        // SQLite serializes writers regardless of pool size; a handful of readers
        // plus headroom for one writer in flight is enough - this isn't a
        // networked DB where more connections buy more throughput.
        .max_connections(8)
        .connect_with(connect_options)
        .await
        .expect("[boot] failed to open SQLite database");
    tracing::info!("[boot] using local SQLite database");

    // Run migrations
    {
        tracing::info!("[boot] running migrations");
        migrations::run(&db_pool).await;
    }

    let rate_limit_config = security::rate_limit::load_config(&db_pool).await;

    // Container name, not the `caddy` Compose service name: deployed apps join the
    // same network and a service named `caddy` would claim that DNS alias.
    let caddy_url =
        std::env::var("CADDY_URL").unwrap_or_else(|_| "http://litepod-caddy:2019".to_string());

    let caddy_admin_url = std::env::var("CADDY_ADMIN_URL")
        .unwrap_or_else(|_| "http://litepod-caddy:2019".to_string());

    let caddyfile_path = std::env::var("CADDYFILE_PATH")
        .unwrap_or_else(|_| "../Caddyfile/Caddyfile".to_string())
        .into();

    let resolved_podman_socket = podman::podman_socket::resolve_process_podman_socket();
    tracing::info!(source = %resolved_podman_socket.source, "podman endpoint resolved");
    let podman_socket = resolved_podman_socket.endpoint;

    let self_container_name = resolve_self_container_name(&podman_socket).await;
    tracing::info!(container = %self_container_name, "resolved own container name");

    let public_ip = std::env::var("LITEPOD_PUBLIC_IP").expect(
        "[boot] LITEPOD_PUBLIC_IP must be set (used to build auto-assigned *.sslip.io domains)",
    );

    // These used to silently fall back to hardcoded values baked into the binary
    // (a known encryption key, a known password pepper, admin/secret Basic Auth)
    // whenever the operator forgot to set them. That's a silent full compromise of
    // every encrypted AI provider key and every password hash, so refuse to boot
    // instead. `scripts/install-staging.sh` already generates real values for all
    // four; local dev must set them in `api/.env` the same way.
    let encryption_key = std::env::var("ENCRYPTION_KEY").unwrap_or_default();
    if encryption_key.len() != 64 || !encryption_key.bytes().all(|b| b.is_ascii_hexdigit()) {
        eprintln!(
            "[boot] ENCRYPTION_KEY must be set to 64 hex characters (see scripts/install-staging.sh)"
        );
        std::process::exit(1);
    }
    if std::env::var("HASH_SECRET")
        .unwrap_or_default()
        .trim()
        .is_empty()
    {
        eprintln!("[boot] HASH_SECRET must be set to a non-empty value");
        std::process::exit(1);
    }
    if std::env::var("METRICS_USERNAME")
        .unwrap_or_default()
        .is_empty()
        || std::env::var("METRICS_PASSWORD")
            .unwrap_or_default()
            .is_empty()
    {
        eprintln!("[boot] METRICS_USERNAME and METRICS_PASSWORD must both be set");
        std::process::exit(1);
    }

    let api_port: i64 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(6001);

    let port_range = (
        std::env::var("LITEPOD_PORT_RANGE_START")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(10_000),
        std::env::var("LITEPOD_PORT_RANGE_END")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(20_000),
    );

    // Infrastructure ports that must not be handed out to application services.
    let mut reserved_ports: std::collections::HashSet<i64> =
        [80, 443, 2019, api_port].into_iter().collect();
    if let Ok(extra) = std::env::var("LITEPOD_RESERVED_PORTS") {
        reserved_ports.extend(
            extra
                .split(',')
                .filter_map(|p| p.trim().parse::<i64>().ok()),
        );
    }

    let data_dir = std::path::PathBuf::from(
        std::env::var("LITEPOD_DATA_DIR").unwrap_or_else(|_| "./data".to_string()),
    );
    if let Err(error) = std::fs::create_dir_all(&data_dir) {
        tracing::warn!("could not create data dir {}: {error}", data_dir.display());
    }
    let host_data_dir = std::path::PathBuf::from(
        std::env::var("LITEPOD_HOST_DATA_DIR")
            .unwrap_or_else(|_| data_dir.to_string_lossy().into_owned()),
    );
    // Podman resolves relative bind-mount sources against its own cwd, not ours.
    let host_data_dir = std::fs::canonicalize(&host_data_dir).unwrap_or(host_data_dir);
    let data_volume = std::env::var("LITEPOD_DATA_VOLUME")
        .ok()
        .filter(|value| !value.trim().is_empty());

    let shared_state: SharedState = std::sync::Arc::new(AppState {
        started_at: chrono::Local::now(),
        db: db_pool,
        caddy_url,
        caddy_admin_url,
        caddyfile_path,
        // Container name, not the `app` Compose service alias: any deployed app
        // on this network could claim `app` and hijack the dashboard. Same
        // reasoning as the catch-all in `Caddyfile/Caddyfile.prod`.
        caddy_server_upstream: std::env::var("CADDY_SERVER_UPSTREAM")
            .unwrap_or_else(|_| "litepod-api:6001".to_string()),
        podman_socket,
        pending_execs: std::sync::Mutex::new(std::collections::HashMap::new()),
        data_dir,
        data_volume,
        host_data_dir,
        podman_network: std::env::var("LITEPOD_PODMAN_NETWORK")
            .unwrap_or_else(|_| "litepod-network".to_string()),
        // GitHub must reach this URL for App callbacks and webhooks, so it is a tunnel in dev
        // and a real domain in production. PUBLIC_URL stays supported for existing deployments.
        public_url: std::env::var("LITEPOD_PUBLIC_URL")
            .or_else(|_| std::env::var("PUBLIC_URL"))
            .unwrap_or_else(|_| "http://localhost:6001".to_string()),
        oauth_states: std::sync::Mutex::new(std::collections::HashMap::new()),
        public_ip,
        is_cloud,
        polar,
        dev_mode: std::env::var("LITEPOD_DEV_MODE")
            .is_ok_and(|value| matches!(value.trim(), "1" | "true")),
        port_range,
        reserved_ports,
        volume_access: Default::default(),
        podman_host: Default::default(),
        queue_runtime: Default::default(),
        version,
        git_sha,
        image_repo: std::env::var("LITEPOD_IMAGE_REPO")
            .unwrap_or_else(|_| "docker.io/litepod/litepod".to_string()),
        image_tag: std::env::var("LITEPOD_IMAGE_TAG").unwrap_or_else(|_| "latest".to_string()),
        update_channel,
        version_manifest_url: std::env::var("LITEPOD_VERSION_MANIFEST_URL")
            .unwrap_or_else(|_| "https://litepod.sh/version.json".to_string()),
        update_interval_secs: std::env::var("LITEPOD_UPDATE_INTERVAL")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3600),
        container_name: self_container_name,
        caddy_container: std::env::var("CADDY_CONTAINER")
            .unwrap_or_else(|_| "litepod-caddy".to_string()),
        stack_env_file: std::env::var("LITEPOD_STACK_ENV_FILE")
            .ok()
            .map(std::path::PathBuf::from)
            .filter(|path| path.is_file()),
        update_state: Default::default(),
        backup_lock: Default::default(),
        rate_limiter: tokio::sync::RwLock::new(
            crate::security::rate_limit::RateLimiterState::from_config(rate_limit_config),
        ),
        ephemeral: crate::scheduled_tasks::ephemeral::EphemeralStore::from_env(),
    });

    if let Err(error) = crate::servers::bootstrap(&shared_state).await {
        tracing::error!(error = %error, "could not initialize deployment servers");
        std::process::exit(1);
    }

    if let Err(error) = crate::servers::repository::fail_interrupted_jobs(&shared_state).await {
        tracing::warn!(error = %error, "could not reconcile interrupted server setup jobs");
    }
    // Real container state before any "interrupted" bookkeeping: an application whose
    // container finished starting right before the restart must not be marked failed.
    if let Err(error) = crate::servers::monitor::reconcile_at_boot(&shared_state).await {
        tracing::warn!(error = %error, "could not reconcile container state at boot");
    }
    if let Err(error) = crate::deployment::history::fail_interrupted(&shared_state).await {
        tracing::warn!(error = %error, "could not reconcile interrupted deployments");
    }
    if let Err(error) = crate::deployment::queue::recover_interrupted(&shared_state).await {
        tracing::warn!(error = %error, "could not reconcile interrupted queue jobs");
    }
    if let Err(error) = crate::backup::run::fail_interrupted(&shared_state).await {
        tracing::warn!(error = %error, "could not reconcile interrupted backups");
    }
    if let Err(error) = crate::backup::restore::fail_interrupted(&shared_state).await {
        tracing::warn!(error = %error, "could not reconcile interrupted restores");
    }
    crate::deployment::queue::start_coordinator(shared_state.clone()).await;
    crate::deployment::dns_validation::spawn_cdn_range_refresh();

    let restore_state = shared_state.clone();
    tokio::spawn(async move {
        match crate::servers::ingress::reconcile_running(&restore_state).await {
            Ok((reconciled, failed)) => {
                tracing::info!(reconciled, failed, "reconciled remote ingress")
            }
            Err(error) => {
                tracing::warn!(error = %error, "could not reconcile remote ingress")
            }
        }
    });

    crate::updater::spawn(shared_state.clone());
    crate::backup::scheduler::spawn(shared_state.clone());
    crate::database_backup::scheduler::spawn(shared_state.clone());
    crate::scheduled_tasks::scheduler::spawn(shared_state.clone());
    crate::health_checks::spawn(shared_state.clone());
    crate::git::repositories::audit_log::spawn(shared_state.clone());
    crate::billing::audit::spawn(shared_state.clone());
    crate::servers::monitor::spawn(shared_state.clone());
    crate::metrics_history::spawn(shared_state.clone());
    crate::servers::ingress::spawn_relay_supervisor(shared_state.clone());
    crate::deployment::operations::spawn_caddy_recovery(shared_state.clone());
    crate::notifications::spawn(shared_state.clone());
    if shared_state.is_cloud {
        crate::users::email_verification::spawn_cleanup(shared_state.clone());
    }

    let app = create_router(shared_state);

    let port = api_port.to_string();
    let base_url = format!("http://localhost:{}", port);

    eprintln!("[boot] binding [::]:{port}");
    let listener = tokio::net::TcpListener::bind(&format!("[::]:{}", port))
        .await
        .unwrap_or_else(|e| {
            eprintln!("[boot] bind failed: {e}");
            std::process::exit(1);
        });

    eprintln!();
    eprintln!("  litepod API");
    eprintln!("  Server   -> [::]:{}", port);
    eprintln!("  Health   -> {}/health", base_url);
    eprintln!("  Docs     -> {}/docs", base_url);
    eprintln!("  Metrics  -> {}/metrics", base_url);
    eprintln!();

    if let Err(e) = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    {
        eprintln!("[boot] serve error: {e}");
        eprintln!("[boot] serve error: {e}");
        std::process::exit(1);
    }
}
