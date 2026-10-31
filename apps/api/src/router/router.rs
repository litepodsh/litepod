use axum::body::Body;
use axum::http::{Method, Request, header};
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    middleware,
    response::IntoResponse,
    routing::{delete, get, patch, post, put},
};
use axum_prometheus::PrometheusMetricLayerBuilder;
use serde::Serialize;
use tower::ServiceExt;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::services::ServeDir;
use utoipa::OpenApi;
use utoipa_scalar::{Scalar, Servable};

use crate::security::middleware::auth::auth_middleware;
use crate::security::middleware::csrf::{cors_allowed_origins, origin_guard};
use crate::security::middleware::metrics_basic_auth_middleware::metrics_basic_auth_middleware;
use crate::security::middleware::rate_limit::{
    auth_rate_limit_middleware, global_rate_limit_middleware, login_ban_middleware,
};
use crate::security::middleware::request_context::request_context_middleware;
use crate::security::middleware::sentry_transaction::sentry_transaction_middleware;
use crate::state::state::SharedState;
use crate::{
    ee::handler::{branding, sso},
    handler::{
        access, account, application_network_probe, application_network_stats,
        application_networks, assistant, auth, backup, billing, caddy_console, catalog_templates,
        compose, containers, dashboard, database_backup, debug, deployments, git_providers,
        github_app, icons, invitations, maintenance, maintenance_site, mcp, notifications, oauth,
        onboarding,
        organization_apps, organizations, permissions, personal_access_tokens, podman_console,
        projects, runtime, scheduled_tasks, server_groups, servers, sessions, settings, system,
        templates, users, webhooks,
    },
    podman::terminal,
};

// axum-prometheus 0.10 matches these with matchit 0.8, whose catch-all syntax is
// `{*name}` (the old `*name` is parsed as a literal segment and silently matches
// nothing but `/health`). Covers every non-UI route; UI/asset paths hit the
// fallback and stay out of the metrics.
const METRICS_ALLOWED_PATHS: &[&str] = &[
    "/api/v1/{*path}",
    "/deploy/{*path}",
    "/health",
    "/exception",
    "/docs",
];

#[derive(OpenApi)]
#[openapi(paths(
    health_check,
    auth::setup_check_handler,
    auth::registration_status_handler,
    auth::setup_handler,
    auth::register_handler,
    auth::invited_register_handler,
    auth::login_handler,
    auth::login_totp_handler,
    auth::password_reset_request_handler,
    auth::password_reset_confirm_handler,
    auth::logout_handler,
    auth::me_handler,
    runtime::get_runtime_handler,
    users::list_users_handler,
    users::get_user_handler,
    users::update_user_handler,
    users::delete_user_handler,
    users::set_user_disabled_handler,
    invitations::list_handler,
    invitations::create_handler,
    invitations::revoke_handler,
    invitations::regenerate_handler,
    invitations::inspect_handler,
    organizations::list_organizations_handler,
    organizations::create_organization_handler,
    organizations::get_organization_handler,
    organizations::update_organization_handler,
    organizations::delete_organization_handler,
    organizations::list_members_handler,
    organizations::update_member_handler,
    organizations::remove_member_handler,
    organizations::upload_icon_handler,
    projects::list_projects_handler,
    dashboard::organization_dashboard_handler,
    organization_apps::list_organization_apps_handler,
    permissions::my_permissions_handler,
    access::catalog_handler,
    access::list_roles_handler,
    access::create_role_handler,
    access::update_role_handler,
    access::delete_role_handler,
    access::list_groups_handler,
    access::get_group_handler,
    access::create_group_handler,
    access::update_group_handler,
    access::delete_group_handler,
    access::set_group_members_handler,
    access::list_assignments_handler,
    access::create_assignment_handler,
    access::delete_assignment_handler,
    access::effective_permissions_handler,
    sso::start_handler,
    sso::callback_handler,
    sso::backchannel_logout_handler,
    sso::list_connections_handler,
    sso::create_connection_handler,
    sso::update_connection_handler,
    sso::delete_connection_handler,
    sso::list_mappings_handler,
    sso::set_mappings_handler,
    sso::list_domains_handler,
    sso::create_domain_handler,
    sso::verify_domain_handler,
    sso::assign_domain_handler,
    sso::delete_domain_handler,
    sso::rotate_scim_token_handler,
    sso::delete_scim_token_handler,
    sso::get_settings_handler,
    sso::set_enforcement_handler,
    projects::create_project_handler,
    projects::get_project_handler,
    projects::update_project_handler,
    projects::delete_project_handler,
    projects::upload_project_icon_handler,
    projects::serve_project_icon_handler,
    assistant::send_message_handler,
    assistant::list_providers_handler,
    assistant::list_connections_handler,
    assistant::create_connection_handler,
    assistant::update_connection_handler,
    assistant::refresh_connection_handler,
    assistant::delete_connection_handler,
    assistant::get_web_search_handler,
    assistant::save_web_search_handler,
    assistant::update_web_search_handler,
    assistant::delete_web_search_handler,
    assistant::list_conversations_handler,
    assistant::read_conversation_handler,
    assistant::rename_conversation_handler,
    assistant::settle_action_handler,
    mcp::mcp_handler,
    projects::create_environment_handler,
    projects::update_environment_handler,
    projects::delete_environment_handler,
    projects::create_application_handler,
    projects::get_application_handler,
    projects::update_application_handler,
    projects::delete_application_handler,
    icons::upload_application_icon_handler,
    account::get_account_handler,
    account::patch_account_handler,
    account::deactivate_account_handler,
    account::change_password_handler,
    account::totp_setup_handler,
    account::totp_verify_handler,
    account::totp_disable_handler,
    account::regenerate_codes_handler,
    account::get_sessions_handler,
    account::delete_session_handler,
    account::delete_other_sessions_handler,
    personal_access_tokens::list_tokens_handler,
    personal_access_tokens::grantable_permissions_handler,
    personal_access_tokens::create_token_handler,
    personal_access_tokens::revoke_token_handler,
    personal_access_tokens::revoke_all_tokens_handler,
    sessions::list_sessions_handler,
    sessions::revoke_current_session_handler,
    sessions::revoke_all_sessions_handler,
    sessions::delete_session_handler,
    terminal::create_exec_handler,
    terminal::exec_ws_handler,
    git_providers::list_git_providers_handler,
    git_providers::create_git_provider_handler,
    git_providers::update_git_provider_handler,
    git_providers::delete_git_provider_handler,
    git_providers::test_repositories_handler,
    git_providers::list_repositories_handler,
    git_providers::list_branches_handler,
    git_providers::list_public_branches_handler,
    github_app::begin_manifest_handler,
    github_app::get_github_app_handler,
    github_app::install_url_handler,
    github_app::manifest_callback_handler,
    github_app::install_callback_handler,
    git_providers::gitlab_oauth_status_handler,
    git_providers::gitlab_oauth_begin_handler,
    git_providers::gitlab_oauth_disconnect_handler,
    git_providers::bitbucket_oauth_status_handler,
    git_providers::bitbucket_oauth_begin_handler,
    git_providers::bitbucket_oauth_disconnect_handler,
    git_providers::git_webhook_status_handler,
    system::get_version_handler,
    system::post_check_update_handler,
    system::post_apply_update_handler
), tags(
    (name = "health", description = "Service health"),
    (name = "auth", description = "Authentication"),
    (name = "users", description = "User management"),
    (name = "organizations", description = "Organization management"),
    (name = "git-providers", description = "Git provider credentials"),
    (name = "github-app", description = "GitHub App creation and installation"),
    (name = "webhooks", description = "Inbound provider webhooks"),
    (name = "projects", description = "Project, environment, and application management"),
    (name = "account", description = "Account management"),
    (name = "sessions", description = "Session management"),
    (name = "terminal", description = "Container terminal access"),
    (name = "debug", description = "Development-only operations"),
    (name = "system", description = "Build identity and self-update state"),
    (name = "assistant", description = "Assistant proposals, applied only on confirm")
))]
struct ApiDoc;

pub fn create_router(state: SharedState) -> Router {
    let (prometheus_layer, metrics_handle) = PrometheusMetricLayerBuilder::new()
        .with_allow_patterns(METRICS_ALLOWED_PATHS)
        .with_default_metrics()
        .build_pair();

    // Emit the series at 0 from boot so `increase()` sees the first registration.
    metrics::counter!(crate::cqrs::commands::auth::USER_REGISTRATIONS_METRIC).increment(0);

    let metrics_routes = Router::new()
        .route(
            "/metrics",
            get(move || async move {
                let handle = metrics_handle.clone();
                handle.render()
            }),
        )
        .layer(middleware::from_fn(metrics_basic_auth_middleware));

    // The OpenAPI spec is the complete map of every route, parameter and body
    // shape in the product. It is not a credential, but handing it to anyone who
    // can register an account removes all the work from finding a weak endpoint —
    // during the A01 period it would have listed the seven vulnerable routes
    // outright. Same gate and same layer order as `debug_routes` below:
    // `require_sysadmin` is applied first so it ends up *inside*
    // `auth_middleware`, which is what populates the `AuthUser` it reads.
    let docs_route = Router::new()
        .merge(Scalar::with_url("/docs", ApiDoc::openapi()))
        .layer(middleware::from_fn(
            crate::security::middleware::require_role::require_sysadmin(),
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));
    // Login gets its own failed-attempt ban tracker on top of the shared auth throttle below —
    // a request-rate limit alone doesn't stop a slow brute-force that stays under the rate.
    let login_route = Router::new()
        .route("/api/v1/auth/login", post(auth::login_handler))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            login_ban_middleware,
        ));
    let public_auth_routes = Router::new()
        .route(
            "/api/v1/auth/setup",
            get(auth::setup_check_handler).post(auth::setup_handler),
        )
        .route(
            "/api/v1/auth/registration-status",
            get(auth::registration_status_handler),
        )
        .route("/api/v1/auth/register", post(auth::register_handler))
        .route(
            "/api/v1/auth/email-verification/confirm",
            post(auth::confirm_email_verification_handler),
        )
        .route(
            "/api/v1/auth/email-verification/resend",
            post(auth::resend_email_verification_handler),
        )
        .route(
            "/api/v1/auth/invited-register",
            post(auth::invited_register_handler),
        )
        .route(
            "/api/v1/invitations/{token}",
            get(invitations::inspect_handler),
        )
        .route("/api/v1/auth/logout", post(auth::logout_handler))
        // Second half of a 2FA login. Unauthenticated by necessity — there is no
        // session yet; the single-use challenge token issued by `/auth/login` is
        // the authorization, and it caps its own wrong-code attempts.
        .route("/api/v1/auth/login/totp", post(auth::login_totp_handler))
        .route(
            "/api/v1/auth/password-reset/request",
            post(auth::password_reset_request_handler),
        )
        .route(
            "/api/v1/auth/password-reset/confirm",
            post(auth::password_reset_confirm_handler),
        )
        .merge(login_route)
        // OAuth for the MCP endpoint: public, and throttled like the rest of auth.
        .merge(oauth::routes())
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_rate_limit_middleware,
        ));
    // GitHub + GitLab call these: the manifest/install callbacks arrive as cross-site browser
    // redirects (a SameSite cookie may not ride along) and the webhooks are server-to-server.
    // All are protected by their own single-use nonce or shared secret instead of a session.
    let public_provider_routes = Router::new()
        .route("/api/v1/webhooks/polar", post(billing::webhook_handler))
        .route(
            "/api/v1/github-app/manifest/callback",
            get(github_app::manifest_callback_handler),
        )
        .route(
            "/api/v1/github-app/install/callback",
            get(github_app::install_callback_handler),
        )
        .route(
            "/api/v1/git-providers/{provider_id}/gitlab-oauth/callback",
            get(git_providers::gitlab_oauth_callback_handler),
        )
        .route(
            "/api/v1/git-providers/{provider_id}/bitbucket-oauth/callback",
            get(git_providers::bitbucket_oauth_callback_handler),
        )
        .route(
            "/api/v1/git-providers/{provider_id}/gitea-oauth/callback",
            get(git_providers::gitea_oauth_callback_handler),
        )
        // Unified git webhook. The handler reads the provider's `provider_type` from
        // the database and dispatches to the right signature scheme (GitHub HMAC, GitLab
        // shared secret, Gitea/Forgejo HMAC).
        .route(
            "/api/v1/webhooks/git/{provider_id}",
            post(webhooks::git_webhook_handler),
        )
        // Keyless deploy hook: the token in the path is the credential.
        .route("/deploy/{token}", post(webhooks::deploy_hook_handler))
        // Same idea for an MCP agent's site or source ZIP; the site's own 10 MiB
        // cap is enforced when it is staged.
        .route(
            "/api/v1/assistant/uploads/{action_id}/{token}",
            axum::routing::put(webhooks::assistant_upload_handler)
                .post(webhooks::assistant_upload_handler)
                .layer(axum::extract::DefaultBodyLimit::max(
                    crate::deployment::source_archive::MAX_ARCHIVE_BYTES,
                )),
        )
        .with_state(state.clone());

    let debug_routes = debug::routes()
        .layer(middleware::from_fn(
            crate::security::middleware::require_role::require_sysadmin(),
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    let api_v1_routes = Router::new()
        .route("/api/v1/billing/overview", get(billing::overview_handler))
        .route("/api/v1/billing/checkout", post(billing::checkout_handler))
        .route("/api/v1/billing/servers/quantity", patch(billing::update_server_quantity_handler))
        .route("/api/v1/billing/invoices", get(billing::invoices_handler))
        .route("/api/v1/billing/invoices/{order_id}/pdf", get(billing::invoice_pdf_handler))
        .route("/api/v1/billing/portal", post(billing::portal_handler))
        .route("/api/v1/admin/billing/accounts", get(billing::admin_accounts_handler))
        .route("/api/v1/admin/billing/external-records", get(billing::admin_external_records_handler))
        .route("/api/v1/admin/billing/reconcile", post(billing::admin_reconcile_handler))
        .route("/api/v1/admin/billing/external-records/{id}/assign", post(billing::admin_assign_external_record_handler))
        .route("/api/v1/organizations/{organization_id}/server-groups", get(server_groups::list_handler).post(server_groups::create_handler))
        .route("/api/v1/organizations/{organization_id}/server-groups/{group_id}", get(server_groups::get_handler).patch(server_groups::update_handler).delete(server_groups::delete_handler))
        .route("/api/v1/organizations/{organization_id}/server-groups/{group_id}/servers", get(server_groups::list_members_handler).put(server_groups::replace_members_handler))
        .route("/api/v1/organizations/{organization_id}/servers", get(servers::list_handler).post(servers::create_handler))
        .route("/api/v1/organizations/{organization_id}/servers/remote-capacity", get(servers::remote_capacity_handler))
        .route("/api/v1/organizations/{organization_id}/servers/available", get(servers::available_handler))
        .route("/api/v1/organizations/{organization_id}/servers/geo", get(servers::geo_handler))
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}", get(servers::get_handler).patch(servers::rename_handler).delete(servers::delete_handler))
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/public-key", get(servers::public_key_handler))
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/terminal-sessions", get(crate::handler::terminal_sessions::list_server_sessions_handler))
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/preflight", post(servers::preflight_handler))
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/confirm-host-key", post(servers::confirm_host_key_handler))
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/healthcheck", post(servers::healthcheck_handler))
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/maintenance/update-packages", post(servers::maintenance_update_packages_handler))
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/maintenance/restart", post(servers::maintenance_restart_handler))
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/maintenance/prune-images", post(servers::maintenance_prune_images_handler))
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/maintenance/restart-podman", post(servers::maintenance_restart_podman_handler))
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/maintenance/check-disk", post(servers::maintenance_check_disk_handler))
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/maintenance/create-swap", post(servers::maintenance_create_swap_handler))
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/maintenance/update-helper", post(servers::maintenance_update_helper_handler))
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/test-deploy", post(servers::test_deploy_handler))
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/setup", post(servers::setup_handler))
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/repair", post(servers::repair_handler))
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/jobs", get(servers::list_jobs_handler))
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/applications", get(servers::assigned_applications_handler))
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/ports", get(servers::ports_handler))
        // Per-server Podman console (replaces the sysadmin /debug/podman surface).
        // Path layout keeps the generic `resources/{resource}` branch free of
        // static siblings so matchit never has to fall back across the two.
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/podman/overview", get(podman_console::overview_handler))
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/podman/system/df", get(podman_console::system_df_handler))
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/podman/system/prune-build-cache", post(podman_console::prune_build_cache_handler))
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/podman/events", get(podman_console::events_handler))
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/podman/events/stream", get(podman_console::events_stream_handler))
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/podman/metrics/stream", get(podman_console::metrics_stream_handler))
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/podman/metrics/history", get(crate::metrics_history::server_history_handler))
        .route("/api/v1/organizations/{organization_id}/metrics/retention", get(crate::metrics_history::get_retention_handler).put(crate::metrics_history::update_retention_handler))
        .route("/api/v1/organizations/{organization_id}/analytics", get(crate::metrics_history::requests::organization_analytics_handler))
        .route("/api/v1/organizations/{organization_id}/analytics/sparklines", get(crate::metrics_history::requests::organization_sparklines_handler))
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/podman/logs/{id}", get(podman_console::container_logs_handler))
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/podman/logs/{id}/stream", get(podman_console::container_logs_stream_handler))
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/podman/top/{id}", get(podman_console::container_top_handler))
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/podman/container/{id}/{action}", post(podman_console::container_action_handler))
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/podman/pod/{id}/{action}", post(podman_console::pod_action_handler))
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/podman/image/pull", post(podman_console::image_pull_handler))
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/podman/volume/create", post(podman_console::volume_create_handler))
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/podman/volume/{volume}/workspace", get(podman_console::volume_workspace_list_handler))
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/podman/volume/{volume}/workspace/file", get(podman_console::volume_workspace_file_handler).put(podman_console::volume_workspace_save_handler))
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/podman/volume/{volume}/workspace/download", get(podman_console::volume_workspace_download_handler))
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/podman/volume/{volume}/workspace/entries", post(podman_console::volume_workspace_create_handler))
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/podman/volume/{volume}/workspace/upload", post(podman_console::volume_workspace_upload_handler))
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/podman/network/create", post(podman_console::network_create_handler))
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/podman/network/{id}/{action}", post(podman_console::network_member_handler))
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/podman/resources/{resource}", get(podman_console::list_handler))
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/podman/resources/{resource}/prune", post(podman_console::prune_handler))
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/podman/resources/{resource}/{id}", get(podman_console::inspect_handler).delete(podman_console::delete_handler))
        // Per-server Caddy console (replaces the sysadmin /debug/caddy surface).
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/caddy/routes", get(caddy_console::routes_handler))
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/caddy/routes/reconcile", post(caddy_console::reconcile_handler))
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/caddy/routes/prune", post(caddy_console::prune_handler))
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/caddy/manual", get(caddy_console::list_manual_handler).post(caddy_console::create_manual_handler))
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/caddy/manual/{id}", put(caddy_console::update_manual_handler).delete(caddy_console::delete_manual_handler))
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/caddy/config", get(caddy_console::config_handler))
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/caddy/status", get(caddy_console::status_handler))
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/caddy/health", get(caddy_console::health_handler))
        .route("/api/v1/organizations/{organization_id}/servers/{server_id}/caddy/edge-routes", get(caddy_console::edge_routes_handler))
        .route("/api/v1/deployment-queue/events", get(deployments::stream_queue_handler))
        .route("/api/v1/deployment-queue/{job_id}", get(deployments::get_queue_handler).post(deployments::cancel_queue_handler))
        .route("/api/v1/auth/me", get(auth::me_handler))
        .route("/api/v1/runtime", get(runtime::get_runtime_handler))
        .route("/api/v1/settings/certificates", get(crate::handler::certificates::list_handler).post(crate::handler::certificates::create_handler))
        .route("/api/v1/settings/certificates/test", axum::routing::post(crate::handler::certificates::test_handler))
        .route("/api/v1/settings/certificates/{id}", get(crate::handler::certificates::detail_handler).put(crate::handler::certificates::update_handler).delete(crate::handler::certificates::delete_handler))
        .route("/api/v1/settings/server-domain", get(settings::get_server_domain_handler).put(settings::update_server_domain_handler))
        .route("/api/v1/settings/build-configuration", get(settings::get_build_configuration_handler).patch(settings::update_build_configuration_handler))
        .route("/api/v1/settings/scheduled-task-history", get(settings::get_scheduled_task_history_handler).patch(settings::update_scheduled_task_history_handler))
        .route("/api/v1/settings/rate-limit", get(settings::get_rate_limit_handler).patch(settings::update_rate_limit_handler))
        .route("/api/v1/settings/telemetry", get(crate::telemetry::get_handler).patch(crate::telemetry::update_handler))
        .route("/api/v1/settings/telemetry/acknowledge", post(crate::telemetry::acknowledge_handler))
        .route("/api/v1/settings/infrastructure/{target}/restart", post(settings::restart_infrastructure_handler))
        .route("/api/v1/settings/backup/s3-config", get(backup::get_s3_config_handler).put(backup::update_s3_config_handler))
        .route("/api/v1/organizations/{organization_id}/backup-targets", get(backup::list_s3_targets_handler).post(backup::create_s3_target_handler))
        .route("/api/v1/organizations/{organization_id}/backup-targets/meta", get(backup::list_s3_target_meta_handler))
        .route("/api/v1/organizations/{organization_id}/backup-targets/{target_id}", put(backup::update_s3_target_handler).delete(backup::delete_s3_target_handler))
        .route("/api/v1/maintenance/backup/history", get(backup::list_backup_history_handler))
        .route("/api/v1/maintenance/backup/run", post(backup::trigger_backup_handler))
        .route("/api/v1/maintenance/backup/{id}/restore", post(backup::trigger_restore_handler))
        .route("/api/v1/maintenance/backup/{id}/download", get(backup::download_backup_handler))
        .route("/api/v1/maintenance/backup/restores", get(backup::list_restore_history_handler))
        .route("/api/v1/maintenance/prune", post(maintenance::prune_handler))
        .route("/api/v1/maintenance/deployment-history", post(maintenance::delete_deployment_history_handler))
        .route("/api/v1/users", get(users::list_users_handler))
        .route(
            "/api/v1/users/{user_id}",
            get(users::get_user_handler)
                .patch(users::update_user_handler)
                .delete(users::delete_user_handler),
        )
        .route("/api/v1/users/{user_id}/disabled", put(users::set_user_disabled_handler))
        .route("/api/v1/invitations", get(invitations::list_handler).post(invitations::create_handler))
        .route("/api/v1/invitations/organizations", get(invitations::eligible_organizations_handler))
        .route("/api/v1/invitations/{id}/revoke", post(invitations::revoke_handler))
        .route("/api/v1/invitations/{id}/regenerate", post(invitations::regenerate_handler))
        .route(
            "/api/v1/organizations",
            get(organizations::list_organizations_handler).post(organizations::create_organization_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}",
            get(organizations::get_organization_handler)
                .patch(organizations::update_organization_handler)
                .delete(organizations::delete_organization_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/members",
            get(organizations::list_members_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/members/{user_id}",
            patch(organizations::update_member_handler).delete(organizations::remove_member_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/audit-logs",
            get(organizations::list_audit_logs_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/audit-logs/export",
            get(organizations::export_audit_logs_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/icon",
            post(organizations::upload_icon_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/icon/{filename}",
            get(organizations::serve_icon_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/scheduled-tasks",
            get(scheduled_tasks::list_scheduled_tasks_handler).post(scheduled_tasks::create_scheduled_task_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/deployment-queue",
            get(deployments::list_queue_for_organization_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/scheduled-tasks/{task_id}",
            get(scheduled_tasks::get_scheduled_task_handler)
                .patch(scheduled_tasks::update_scheduled_task_handler)
                .delete(scheduled_tasks::delete_scheduled_task_handler),
        )
        .route("/api/v1/organizations/{organization_id}/scheduled-tasks/test", post(scheduled_tasks::test_scheduled_task_handler))
        .route("/api/v1/organizations/{organization_id}/scheduled-tasks/{task_id}/run", post(scheduled_tasks::run_scheduled_task_handler))
        .route("/api/v1/organizations/{organization_id}/scheduled-tasks/{task_id}/history", delete(scheduled_tasks::clear_scheduled_task_history_handler))
        .route("/api/v1/organizations/{organization_id}/scheduled-tasks/{task_id}/executions", get(scheduled_tasks::list_scheduled_task_executions_handler))
        .route("/api/v1/organizations/{organization_id}/scheduled-tasks/{task_id}/executions/{execution_id}", get(scheduled_tasks::get_scheduled_task_execution_handler))
        .route("/api/v1/organizations/{organization_id}/scheduled-tasks/{task_id}/executions/{execution_id}/stream", get(scheduled_tasks::stream_scheduled_task_execution_handler))
        .route("/api/v1/organizations/{organization_id}/scheduled-tasks/{task_id}/executions/{execution_id}/cancel", post(scheduled_tasks::cancel_scheduled_task_execution_handler))
        .route(
            "/api/v1/organizations/{organization_id}/notifications",
            get(notifications::list_handler).post(notifications::create_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/onboarding",
            get(onboarding::get_handler).patch(onboarding::action_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/notifications/catalog",
            get(notifications::catalog_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/notifications/test",
            post(notifications::test_unsaved_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/notifications/{provider_id}",
            put(notifications::update_handler).delete(notifications::delete_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/notifications/{provider_id}/test",
            post(notifications::test_saved_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/notifications/{provider_id}/deliveries",
            get(notifications::deliveries_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/git-providers",
            get(git_providers::list_git_providers_handler).post(git_providers::create_git_provider_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/git-providers/{provider_id}",
            axum::routing::patch(git_providers::update_git_provider_handler).delete(git_providers::delete_git_provider_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/git-providers/test-repositories",
            post(git_providers::test_repositories_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/git-providers/branches",
            post(git_providers::list_public_branches_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/git-providers/github-app/manifest",
            post(github_app::begin_manifest_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/git-providers/{provider_id}/github-app",
            get(github_app::get_github_app_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/git-providers/{provider_id}/github-app/install-url",
            post(github_app::install_url_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/git-providers/{provider_id}/repositories",
            get(git_providers::list_repositories_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/git-providers/{provider_id}/branches",
            post(git_providers::list_branches_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/git-providers/{provider_id}/gitlab-oauth",
            get(git_providers::gitlab_oauth_status_handler).delete(git_providers::gitlab_oauth_disconnect_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/git-providers/{provider_id}/bitbucket-oauth",
            get(git_providers::bitbucket_oauth_status_handler).delete(git_providers::bitbucket_oauth_disconnect_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/git-providers/{provider_id}/gitea-oauth",
            get(git_providers::gitea_oauth_status_handler).delete(git_providers::gitea_oauth_disconnect_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/git-providers/{provider_id}/git-webhook",
            get(git_providers::git_webhook_status_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/git-providers/{provider_id}/gitlab-oauth/begin",
            post(git_providers::gitlab_oauth_begin_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/git-providers/{provider_id}/bitbucket-oauth/begin",
            post(git_providers::bitbucket_oauth_begin_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/git-providers/{provider_id}/gitea-oauth/begin",
            post(git_providers::gitea_oauth_begin_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects",
            get(projects::list_projects_handler).post(projects::create_project_handler),
        )
        .route("/api/v1/organizations/{organization_id}/dashboard", get(dashboard::organization_dashboard_handler))
        .route(
            "/api/v1/organizations/{organization_id}/apps/summary",
            get(organization_apps::list_organization_apps_handler),
        )
        .route("/api/v1/organizations/{organization_id}/me/permissions", get(permissions::my_permissions_handler))
        .route("/api/v1/organizations/{organization_id}/permissions/catalog", get(access::catalog_handler))
        .route(
            "/api/v1/organizations/{organization_id}/roles",
            get(access::list_roles_handler).post(access::create_role_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/roles/{role_id}",
            patch(access::update_role_handler).delete(access::delete_role_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/groups",
            get(access::list_groups_handler).post(access::create_group_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/groups/{group_id}",
            get(access::get_group_handler)
                .patch(access::update_group_handler)
                .delete(access::delete_group_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/groups/{group_id}/members",
            put(access::set_group_members_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/assignments",
            get(access::list_assignments_handler).post(access::create_assignment_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/assignments/{assignment_id}",
            delete(access::delete_assignment_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/members/{user_id}/effective-permissions",
            get(access::effective_permissions_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}",
            get(projects::get_project_handler)
                .patch(projects::update_project_handler)
                .delete(projects::delete_project_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/icon",
            post(projects::upload_project_icon_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/icon/{filename}",
            get(projects::serve_project_icon_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments",
            post(projects::create_environment_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}",
            patch(projects::update_environment_handler).delete(projects::delete_environment_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications",
            post(projects::create_application_handler),
        )
        .route("/api/v1/catalog-templates", get(catalog_templates::list_handler))
        .route("/api/v1/catalog-templates/{template_id}", get(catalog_templates::get_handler))
        .route("/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/catalog-templates/{template_id}", post(catalog_templates::create_handler))
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/templates/postgres/preview",
            post(templates::preview_postgres_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/templates/postgres",
            post(templates::create_postgres_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/templates/mariadb/preview",
            post(templates::preview_mariadb_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/templates/mariadb",
            post(templates::create_mariadb_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/templates/dragonflydb/preview",
            post(templates::preview_dragonflydb_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/templates/dragonflydb",
            post(templates::create_dragonflydb_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/templates/keydb/preview",
            post(templates::preview_keydb_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/templates/keydb",
            post(templates::create_keydb_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/templates/sqlserver/preview",
            post(templates::preview_sqlserver_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/templates/sqlserver",
            post(templates::create_sqlserver_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/templates/postgres/resource",
            get(templates::get_postgres_resource_handler)
                .patch(templates::update_postgres_resource_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/templates/postgres/resource/rotate-password",
            post(templates::rotate_postgres_password_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/templates/mongodb/preview",
            post(templates::preview_mongodb_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/templates/mongodb",
            post(templates::create_mongodb_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/templates/mongodb/resource",
            get(templates::get_mongodb_resource_handler)
                .patch(templates::update_mongodb_resource_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/templates/mongodb/resource/rotate-password",
            post(templates::rotate_mongodb_password_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/templates/mariadb/resource",
            get(templates::get_mariadb_resource_handler)
                .patch(templates::update_mariadb_resource_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/templates/mariadb/resource/rotate-password",
            post(templates::rotate_mariadb_password_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/templates/dragonflydb/resource",
            get(templates::get_dragonflydb_resource_handler)
                .patch(templates::update_dragonflydb_resource_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/templates/dragonflydb/resource/rotate-password",
            post(templates::rotate_dragonflydb_password_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/templates/keydb/resource",
            get(templates::get_keydb_resource_handler)
                .patch(templates::update_keydb_resource_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/templates/keydb/resource/rotate-password",
            post(templates::rotate_keydb_password_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/templates/sqlserver/resource",
            get(templates::get_sqlserver_resource_handler)
                .patch(templates::update_sqlserver_resource_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/templates/sqlserver/resource/rotate-password",
            post(templates::rotate_sqlserver_password_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}",
            get(projects::get_application_handler)
                .patch(projects::update_application_handler)
                .delete(projects::delete_application_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/terminal-sessions",
            get(crate::handler::terminal_sessions::list_application_sessions_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/resource-baseline",
            get(projects::sample_resource_baseline_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/deployment",
            get(deployments::get_deployment_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/deploy-hook",
            get(deployments::get_deploy_hook_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/deploy-hook/rotate",
            post(deployments::rotate_deploy_hook_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/source",
            get(deployments::get_source_handler).put(deployments::save_source_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/source/clean-build",
            axum::routing::patch(deployments::set_clean_build_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/deployment-runs",
            get(deployments::list_deployment_runs_handler).delete(deployments::delete_deployment_runs_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/deployment-runs/{run_id}",
            get(deployments::get_deployment_run_handler).delete(deployments::delete_deployment_run_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/deployment-runs/{run_id}/rollback",
            post(deployments::rollback_deployment_run_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/deployment/port-constraints",
            get(deployments::port_constraints_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/health-check",
            get(deployments::get_health_check_handler).put(deployments::update_health_check_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/boot-policy",
            get(deployments::get_boot_policy_handler).put(deployments::update_boot_policy_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/backup-policy",
            get(database_backup::get_backup_policy_handler)
                .put(database_backup::update_backup_policy_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/backups",
            get(database_backup::list_backups_handler)
                .post(database_backup::run_backup_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/backups/{backup_id}/restore",
            post(database_backup::restore_backup_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/backups/{backup_id}",
            axum::routing::delete(database_backup::delete_backup_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/domains",
            get(deployments::list_domains_handler).post(deployments::create_domain_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/domains/random-sslip-host",
            get(deployments::random_sslip_host_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/domains/generate",
            post(deployments::generate_domain_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/domains/validate-dns",
            post(deployments::validate_host_dns_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/domains/{domain_id}/validate-dns",
            post(deployments::validate_domain_dns_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/domains/{domain_id}",
            axum::routing::patch(deployments::update_domain_handler).delete(deployments::delete_domain_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/basic-auth",
            get(deployments::get_basic_auth_handler).put(deployments::update_basic_auth_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/maintenance",
            get(deployments::get_maintenance_handler).put(deployments::update_maintenance_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/maintenance/site",
            post(deployments::upload_maintenance_site_handler)
                .delete(deployments::delete_maintenance_site_handler)
                .layer(axum::extract::DefaultBodyLimit::max(
                    crate::deployment::maintenance::MAX_SITE_ARCHIVE_BYTES + 1024 * 1024,
                )),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/basic-auth/users",
            post(deployments::add_basic_auth_user_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/basic-auth/users/{user_id}",
            axum::routing::patch(deployments::update_basic_auth_user_handler)
                .delete(deployments::delete_basic_auth_user_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/artifact",
            get(deployments::download_artifact_handler)
                .post(deployments::upload_artifact_handler)
                .delete(deployments::delete_artifact_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/source-archive",
            get(deployments::download_source_archive_handler)
                .post(deployments::upload_source_archive_handler)
                .layer(axum::extract::DefaultBodyLimit::max(
                    crate::deployment::source_archive::MAX_ARCHIVE_BYTES + 1024 * 1024,
                )),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/icon",
            post(icons::upload_application_icon_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/icon/{filename}",
            get(icons::serve_application_icon_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/environment",
            get(deployments::get_environment_handler).put(deployments::update_environment_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/environment/domain-markers",
            get(deployments::domain_markers_handler),
        )
        // The whole assistant is three routes. What it can do is a tool list, not
        // a route table, so a new capability never adds an endpoint here. Writes
        // come back as actions; settling one is the only thing that applies them,
        // and it carries the user's own session.
        .route(
            "/api/v1/organizations/{organization_id}/assistant/messages",
            post(assistant::send_message_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/assistant/conversations",
            get(assistant::list_conversations_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/assistant/conversations/{conversation_id}",
            get(assistant::read_conversation_handler)
                .patch(assistant::rename_conversation_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/assistant/actions/{action_id}",
            post(assistant::settle_action_handler),
        )
        // MCP: the same two tools, for the user's own agent. One route, like
        // the assistant's; what it can do is still the tool list.
        .route(
            "/api/v1/organizations/{organization_id}/mcp",
            post(mcp::mcp_handler),
        )
        // The connections behind the assistant: an organization's own keys and
        // endpoints. Not a route per provider - what providers exist is a table
        // in `assistant::config`, served by the first route here.
        .route(
            "/api/v1/assistant/providers",
            get(assistant::list_providers_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/assistant/connections",
            get(assistant::list_connections_handler).post(assistant::create_connection_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/assistant/connections/{connection_id}",
            patch(assistant::update_connection_handler)
                .delete(assistant::delete_connection_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/assistant/connections/{connection_id}/refresh",
            post(assistant::refresh_connection_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/assistant/web-search",
            get(assistant::get_web_search_handler)
                .post(assistant::save_web_search_handler)
                .patch(assistant::update_web_search_handler)
                .delete(assistant::delete_web_search_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/replicas/{server_id}/deploy",
            post(deployments::retry_replica_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/deploy",
            post(deployments::deploy_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/stop",
            post(deployments::stop_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/volumes",
            get(deployments::volumes_handler),
        )
        .route("/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/managed-volumes", get(deployments::list_managed_volumes_handler).post(deployments::create_managed_volume_handler))
        .route("/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/managed-volumes/{volume_id}/attach", post(deployments::attach_managed_volume_handler))
        .route("/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/managed-volumes/{volume_id}/detach", post(deployments::detach_managed_volume_handler))
        .route("/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/managed-volumes/{volume_id}", delete(deployments::delete_managed_volume_handler))
        .route("/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/containers/{container_id}/files", get(crate::handler::container_files::list_handler))
        .route("/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/containers/{container_id}/files/file", get(crate::handler::container_files::read_handler).put(crate::handler::container_files::save_handler))
        .route("/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/containers/{container_id}/files/download", get(crate::handler::container_files::download_handler))
        .route("/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/containers/{container_id}/files/entries", post(crate::handler::container_files::create_handler))
        .route("/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/containers/{container_id}/files/upload", post(crate::handler::container_files::upload_handler))
        .route("/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/managed-volumes/{volume_id}/workspace", get(deployments::managed_workspace_list_handler))
        .route("/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/managed-volumes/{volume_id}/workspace/file", get(deployments::managed_workspace_file_handler).put(deployments::managed_workspace_save_handler))
        .route("/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/managed-volumes/{volume_id}/workspace/download", get(deployments::managed_workspace_download_handler))
        .route("/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/managed-volumes/{volume_id}/workspace/entries", post(deployments::managed_workspace_create_handler))
        .route("/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/managed-volumes/{volume_id}/workspace/upload", post(deployments::managed_workspace_upload_handler))
        .route("/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/volumes/{source}/workspace", get(deployments::workspace_list_handler))
        .route("/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/volumes/{source}/workspace/file", get(deployments::workspace_file_handler).put(deployments::workspace_save_handler))
        .route("/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/volumes/{source}/workspace/download", get(deployments::workspace_download_handler))
        .route("/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/volumes/{source}/workspace/entries", post(deployments::workspace_create_handler))
        .route("/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/volumes/{source}/workspace/upload", post(deployments::workspace_upload_handler))
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/log-targets",
            get(deployments::list_log_targets_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/logs",
            get(deployments::fetch_logs_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/logs/stream",
            get(deployments::stream_logs_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/stats",
            get(deployments::get_stats_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/containers/metrics",
            get(deployments::container_metrics_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/containers/metrics/stream",
            get(deployments::container_metrics_stream_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/containers/metrics/history",
            get(crate::metrics_history::application_history_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/analytics",
            get(crate::metrics_history::requests::application_analytics_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/container-status",
            get(deployments::get_container_status_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/replicas/containers",
            get(containers::replica_containers_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/containers/{container_name}/start",
            post(containers::container_start_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/containers/{container_name}/stop",
            post(containers::container_stop_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/containers/{container_name}/restart",
            post(containers::container_restart_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/containers/{container_name}/inspect",
            get(containers::container_inspect_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/networks",
            get(application_networks::application_networks_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/resource-networks",
            get(application_networks::resource_networks_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/resource-networks/{resource_id}",
            post(application_networks::attach_resource_network_handler)
                .delete(application_networks::detach_resource_network_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/network-probe",
            post(application_network_probe::application_network_probe_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/network-stats",
            get(application_network_stats::application_network_stats_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/start",
            post(deployments::start_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/remove-runtime",
            post(deployments::remove_runtime_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/compose/content",
            get(compose::compose_content_get_handler).put(compose::compose_content_put_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/compose/validate",
            post(compose::compose_validate_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/compose/proxy-routes",
            get(compose::compose_proxy_routes_list_handler).post(compose::compose_proxy_routes_upsert_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/compose/proxy-routes/{domain_id}",
            axum::routing::delete(compose::compose_proxy_routes_delete_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/compose/deploy",
            post(compose::compose_deploy_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/compose/stop",
            post(compose::compose_stop_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/compose/start",
            post(compose::compose_start_handler),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/environments/{environment_id}/applications/{application_id}/compose/status",
            get(compose::compose_status_handler),
        )
        .route(
            "/api/v1/account/tokens",
            get(personal_access_tokens::list_tokens_handler)
                .post(personal_access_tokens::create_token_handler)
                .delete(personal_access_tokens::revoke_all_tokens_handler),
        )
        .route(
            "/api/v1/account/tokens/permissions",
            get(personal_access_tokens::grantable_permissions_handler),
        )
        .route(
            "/api/v1/account/tokens/{id}",
            delete(personal_access_tokens::revoke_token_handler),
        )
        .route(
            "/api/v1/account",
            get(account::get_account_handler)
                .patch(account::patch_account_handler)
                .delete(account::deactivate_account_handler),
        )
        .route(
            "/api/v1/account/change-password",
            post(account::change_password_handler),
        )
        .route(
            "/api/v1/account/totp/setup",
            post(account::totp_setup_handler),
        )
        .route(
            "/api/v1/account/totp/verify",
            post(account::totp_verify_handler),
        )
        .route(
            "/api/v1/account/totp/disable",
            post(account::totp_disable_handler),
        )
        .route(
            "/api/v1/account/recovery-codes",
            post(account::regenerate_codes_handler),
        )
        .route(
            "/api/v1/account/sessions",
            get(account::get_sessions_handler)
                .delete(account::delete_all_sessions_handler),
        )
        .route(
            "/api/v1/account/sessions/{session_id}",
            delete(account::delete_session_handler),
        )
        .route(
            "/api/v1/account/sessions/others",
            delete(account::delete_other_sessions_handler),
        )
        .route(
            "/api/v1/sessions",
            get(sessions::list_sessions_handler),
        )
        .route(
            "/api/v1/sessions/current/revoke",
            post(sessions::revoke_current_session_handler),
        )
        .route(
            "/api/v1/sessions/revoke-all",
            post(sessions::revoke_all_sessions_handler),
        )
        .route(
            "/api/v1/sessions/{session_id}",
            delete(sessions::delete_session_handler),
        )
        .route(
            "/api/v1/system/version",
            get(system::get_version_handler),
        )
        .route(
            "/api/v1/system/update/check",
            post(system::post_check_update_handler),
        )
        .route(
            "/api/v1/system/update/apply",
            post(system::post_apply_update_handler),
        )
        .merge(terminal::routes())
        .merge(oauth::session_routes())
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    // The rate limiter is layered before `.fallback(ui_fallback)` is added below, so it only
    // wraps the API/docs/health routes above and not the static UI assets served by the
    // fallback. Static JS/CSS chunks are fetched in bursts far larger than any sane API quota
    // (e.g. dozens of chunks on a single page load), so counting them against the same limiter
    // as API calls causes spurious 429s on asset loads right after login.
    // CORS is for API consumers. Applying it to the static UI made a bad
    // CORS_ORIGINS value block SvelteKit's own `/_app/version.json` check.
    let sso_routes = sso_routes_for_edition(state.clone());

    let api_routes = public_auth_routes
        .merge(public_provider_routes)
        .merge(debug_routes)
        .merge(api_v1_routes)
        .merge(sso_routes)
        // Inside CORS, so a 429 still carries `Access-Control-Allow-Origin`:
        // without it the browser hides the status and reports "Failed to fetch".
        .layer(middleware::from_fn_with_state(
            state.clone(),
            global_rate_limit_middleware,
        ))
        .layer(
            CorsLayer::new()
                .allow_origin(build_cors_origins())
                .allow_methods([
                    Method::GET,
                    Method::POST,
                    Method::PUT,
                    Method::PATCH,
                    Method::DELETE,
                    Method::OPTIONS,
                ])
                .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION, header::ACCEPT])
                .allow_credentials(true),
        );

    let app = Router::new()
        .route("/health", get(health_check))
        .route("/exception", get(exception_handler))
        .merge(metrics_routes)
        .merge(docs_route)
        // .merge(debug_route)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            global_rate_limit_middleware,
        ))
        .merge(api_routes)
        // Added after the rate limiter's layer on purpose (see the handler): Caddy sends
        // every visitor of every application in maintenance through here.
        .route(
            "/__litepod/maintenance/{application_id}",
            get(maintenance_site::serve_site_handler),
        )
        .route(
            "/__litepod/maintenance/{application_id}/",
            get(maintenance_site::serve_site_handler),
        )
        .route(
            "/__litepod/maintenance/{application_id}/{*path}",
            get(maintenance_site::serve_site_handler),
        )
        .fallback(ui_fallback)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            oauth::mcp_challenge,
        ))
        .layer(middleware::from_fn_with_state(state.clone(), origin_guard))
        .layer(middleware::from_fn(sentry_transaction_middleware))
        .layer(prometheus_layer)
        // Outermost: stamp IP/UA/request-id before any other layer so even
        // short-circuited responses (rate-limit 429, auth 401, fallback 404)
        // still echo `X-Request-Id`, and audit_log::record reads populated
        // RequestContext from extensions after auth_middleware runs.
        .layer(middleware::from_fn(request_context_middleware))
        .with_state(state);

    app
}

/// Enterprise identity (spec 3) is self-hosted only: on cloud these routes do
/// not exist (404), so the cloud control plane never acts for a customer IdP.
pub(crate) fn sso_routes_for_edition(state: SharedState) -> Router<SharedState> {
    if state.is_cloud {
        Router::new()
    } else {
        sso_routes(state.clone()).merge(branding_routes(state))
    }
}

/// White-label: public read for the login page, sysadmin-only write.
fn branding_routes(state: SharedState) -> Router<SharedState> {
    let manage = Router::new()
        .route("/api/v1/settings/branding", put(branding::update_handler))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));
    Router::new()
        .route("/api/v1/branding", get(branding::get_handler))
        .merge(manage)
}

/// SSO login, org SSO management and SCIM.
fn sso_routes(state: SharedState) -> Router<SharedState> {
    let login = Router::new()
        .route("/api/v1/auth/sso/start", post(sso::start_handler))
        .route("/api/v1/auth/sso/callback", get(sso::callback_handler));
    // SAML needs native xmlsec, so it only exists in builds with the `saml` feature.
    #[cfg(feature = "saml")]
    let login = login
        .route("/api/v1/auth/saml/acs", post(sso::saml_acs_handler))
        .route(
            "/api/v1/auth/saml/{connection_id}/metadata",
            get(sso::saml_metadata_handler),
        );
    let login = login.layer(middleware::from_fn_with_state(
        state.clone(),
        auth_rate_limit_middleware,
    ));
    let base = "/api/v1/organizations/{organization_id}/sso";
    let manage = Router::new()
        .route(
            &format!("{base}/connections"),
            get(sso::list_connections_handler).post(sso::create_connection_handler),
        )
        .route(
            &format!("{base}/connections/{{connection_id}}"),
            patch(sso::update_connection_handler).delete(sso::delete_connection_handler),
        )
        .route(
            &format!("{base}/connections/{{connection_id}}/group-mappings"),
            get(sso::list_mappings_handler).put(sso::set_mappings_handler),
        )
        .route(
            &format!("{base}/domains"),
            get(sso::list_domains_handler).post(sso::create_domain_handler),
        )
        .route(
            &format!("{base}/domains/{{domain_id}}"),
            patch(sso::assign_domain_handler).delete(sso::delete_domain_handler),
        )
        .route(
            &format!("{base}/domains/{{domain_id}}/verify"),
            post(sso::verify_domain_handler),
        )
        .route(
            &format!("{base}/scim-token"),
            post(sso::rotate_scim_token_handler).delete(sso::delete_scim_token_handler),
        )
        .route(&format!("{base}/settings"), get(sso::get_settings_handler))
        .route(
            &format!("{base}/enforcement"),
            put(sso::set_enforcement_handler),
        )
        .layer(middleware::from_fn_with_state(state, auth_middleware));
    // The IdP calls server to server, one request per signed-out user: a bulk
    // offboarding must not trip the per-IP login limiter (global limiter still applies).
    let backchannel = Router::new().route(
        "/api/v1/auth/sso/backchannel/{connection_id}",
        post(sso::backchannel_logout_handler),
    );
    login
        .merge(backchannel)
        .merge(manage)
        // Bearer-token API for the IdP's provisioning service; no session.
        .nest("/scim/v2/{organization_id}", scim_routes())
}

/// SCIM 2.0 subset, relative to `/scim/v2/{organization_id}`.
pub fn scim_routes() -> Router<SharedState> {
    use crate::ee::sso::scim;
    Router::new()
        .route("/ServiceProviderConfig", get(scim::service_provider_config))
        .route("/ResourceTypes", get(scim::resource_types))
        .route("/Schemas", get(scim::schemas))
        .route(
            "/Users",
            get(scim::list_users).post(scim::create_user_handler),
        )
        .route(
            "/Users/{id}",
            get(scim::get_user)
                .put(scim::replace_user)
                .patch(scim::patch_user)
                .delete(scim::delete_user),
        )
        .route("/Groups", get(scim::list_groups).post(scim::create_group))
        .route(
            "/Groups/{id}",
            get(scim::get_group)
                .patch(scim::patch_group)
                .delete(scim::delete_group),
        )
}

async fn spa_shell_handler() -> impl IntoResponse {
    let ui_dir = std::env::var("UI_DIST_PATH").unwrap_or_else(|_| "./ui".to_string());
    match tokio::fs::read(format!("{ui_dir}/404.html")).await {
        Ok(body) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            body,
        )
            .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

const SECRET_PROBE_SEGMENTS: &[&str] = &[".git", ".aws", ".terraform"];
const SECRET_PROBE_BASENAMES: &[&str] = &[
    "wp-config",
    "docker-compose",
    "aws_keys",
    "appsettings.json",
];
const SECRET_PROBE_SUFFIXES: &[&str] = &[
    ".bak",
    ".old",
    ".swp",
    ".save",
    ".dist",
    ".properties",
    ".ftpconfig",
];

fn is_secret_probe_path(path: &str) -> bool {
    let decoded = urlencoding::decode(path).unwrap_or_else(|_| std::borrow::Cow::Borrowed(path));
    let normalized = decoded.to_ascii_lowercase();

    if normalized
        .split('/')
        .filter(|segment| !segment.is_empty())
        .any(|segment| {
            segment == ".env"
                || segment.starts_with(".env.")
                || segment == ".env~"
                || SECRET_PROBE_SEGMENTS.contains(&segment)
        })
    {
        return true;
    }

    let file_name = normalized
        .rsplit('/')
        .find(|segment| !segment.is_empty())
        .unwrap_or_default();

    SECRET_PROBE_BASENAMES.iter().any(|name| {
        file_name == *name
            || file_name
                .strip_prefix(name)
                .is_some_and(|suffix| suffix.starts_with('.'))
    }) || SECRET_PROBE_SUFFIXES
        .iter()
        .any(|suffix| file_name.ends_with(suffix))
}

/// Paths shaped like a static asset request (have a file extension, e.g. `_app/immutable/nodes/27.HASH.js`)
/// are never a real page navigation - if missing, a stale-hash reference from a build the user
/// is no longer on is far more likely than an actual route. Serving the SPA shell (200 + HTML) for
/// those masquerades a failure as success and confuses fetch()/dynamic import callers expecting JS/CSS;
/// a plain 404 is unambiguous and lets the client-side stale-chunk reload handle recovery.
fn is_static_asset_path(path: &str) -> bool {
    path.rsplit('/')
        .next()
        .is_some_and(|last_segment| last_segment.contains('.'))
}

pub(crate) async fn ui_fallback(req: Request<Body>) -> impl IntoResponse {
    let path = req.uri().path().to_string();
    // API paths are never UI routes: an unknown one is a real 404, not the SPA
    // shell. On cloud this is how the absent SSO/SCIM routes answer (spec 3).
    if is_secret_probe_path(&path) || path.starts_with("/api/") || path.starts_with("/scim/") {
        return StatusCode::NOT_FOUND.into_response();
    }

    let ui_dir = std::env::var("UI_DIST_PATH").unwrap_or_else(|_| "./ui".to_string());
    let res = ServeDir::new(&ui_dir)
        .precompressed_br()
        .precompressed_gzip()
        .oneshot(req)
        .await
        .expect("ServeDir is infallible");
    let mut res = if res.status() == StatusCode::NOT_FOUND {
        if is_static_asset_path(&path) {
            StatusCode::NOT_FOUND.into_response()
        } else {
            spa_shell_handler().await.into_response()
        }
    } else {
        let mut res = res.into_response();
        // Hashed filenames never change content, so the browser can skip revalidation entirely.
        if path.starts_with("/_app/immutable/") {
            res.headers_mut().insert(
                header::CACHE_CONTROL,
                header::HeaderValue::from_static("public, max-age=31536000, immutable"),
            );
        }
        res
    };
    // No page is ever framed: a hidden iframe over /oauth/consent would turn
    // a click on someone else's site into an approval.
    let headers = res.headers_mut();
    headers.insert(
        header::X_FRAME_OPTIONS,
        header::HeaderValue::from_static("DENY"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        header::HeaderValue::from_static("frame-ancestors 'none'"),
    );
    res
}

fn build_cors_origins() -> AllowOrigin {
    let origins: Vec<axum::http::HeaderValue> = cors_allowed_origins()
        .iter()
        .map(|s| s.parse().expect("Invalid CORS origin"))
        .collect();

    AllowOrigin::list(origins)
}

#[derive(Serialize, utoipa::ToSchema)]
struct HealthResponse {
    status: String,
    database: String,
    caddy: String,
}

#[utoipa::path(
    get,
    path = "/health",
    tag = "health",
    responses(
        (status = 200, description = "Service is healthy", body = HealthResponse),
    ),
)]
async fn health_check(State(state): State<SharedState>) -> impl IntoResponse {
    let db_status = match sqlx::query("SELECT 1").execute(&state.db).await {
        Ok(_) => "up".to_string(),
        Err(_) => "down".to_string(),
    };

    let caddy_up = match crate::deployment::caddy_admin::client(
        &state.caddy_admin_url,
        Some(std::time::Duration::from_secs(5)),
    ) {
        Ok((client, base)) => client
            .get(format!("{base}/config/"))
            .send()
            .await
            .is_ok_and(|resp| resp.status().is_success()),
        Err(_) => false,
    };
    let caddy_status = if caddy_up { "up" } else { "down" }.to_string();

    Json(HealthResponse {
        status: "ok".to_string(),
        database: db_status,
        caddy: caddy_status,
    })
}

async fn exception_handler() -> impl IntoResponse {
    sentry::capture_message("Something went wrong", sentry::Level::Info);
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "exception handler".to_string(),
    )
}

#[cfg(test)]
mod tests {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        response::IntoResponse,
    };

    use super::{METRICS_ALLOWED_PATHS, is_secret_probe_path, ui_fallback};

    #[test]
    fn metrics_allowlist_contains_only_product_routes() {
        assert_eq!(
            METRICS_ALLOWED_PATHS,
            &[
                "/api/v1/{*path}",
                "/deploy/{*path}",
                "/health",
                "/exception",
                "/docs",
            ],
        );
        assert!(!METRICS_ALLOWED_PATHS.contains(&"/metrics"));
        assert!(
            !METRICS_ALLOWED_PATHS
                .iter()
                .any(|path| path.starts_with("/_app"))
        );
        // matchit 0.8 catch-all form, not the 0.7 `*path`.
        assert!(
            METRICS_ALLOWED_PATHS
                .iter()
                .all(|path| !path.contains("/*"))
        );
    }

    #[test]
    fn secret_probe_classifier_blocks_known_scan_families() {
        for path in [
            "/.env",
            "/backend/.ENV.production",
            "/%2eaws/credentials",
            "/.git/config",
            "/app/.terraform/terraform.tfstate",
            "/api/wp-config.bak",
            "/api/docker-compose.old",
            "/aws_keys.properties",
            "/dashboard/.ftpconfig",
        ] {
            assert!(is_secret_probe_path(path), "{path}");
        }

        for path in [
            "/login",
            "/projects/project-1/environments/prod",
            "/api/v1/organizations",
            "/_app/immutable/entry/app.js",
            "/configuration",
        ] {
            assert!(!is_secret_probe_path(path), "{path}");
        }
    }

    #[tokio::test]
    async fn ui_fallback_returns_404_for_secret_probe() {
        let response = ui_fallback(
            Request::builder()
                .uri("/%2eenv")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    /// Every UI page refuses to be framed, so no other site can lay an
    /// invisible `/oauth/consent` over a button of its own.
    #[tokio::test]
    async fn ui_responses_cannot_be_framed() {
        for uri in ["/oauth/consent", "/"] {
            let response = ui_fallback(Request::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .into_response();
            assert_eq!(
                response.headers()[axum::http::header::X_FRAME_OPTIONS],
                "DENY",
                "{uri}"
            );
            assert_eq!(
                response.headers()[axum::http::header::CONTENT_SECURITY_POLICY],
                "frame-ancestors 'none'",
                "{uri}"
            );
        }
    }

    /// `/docs` renders the full OpenAPI spec — every route, parameter and body
    /// shape in the product. Exercised through the real router so the middleware
    /// layer order is what is actually being asserted, not a reimplementation of
    /// it: `require_sysadmin` has to run *after* `auth_middleware` populates the
    /// `AuthUser` it reads, and a wrong order would silently 403 everyone.
    #[tokio::test]
    async fn the_openapi_spec_is_reachable_only_by_a_sysadmin() {
        use crate::git::repositories::{session, user};
        use crate::testing::{cleanup_database, test_state};
        use crate::utils::security::generate_token;
        use tower::ServiceExt;

        let (state, path) = test_state().await;

        async fn session_for(state: &crate::state::state::SharedState, user_id: &str) -> String {
            let token = generate_token();
            session::create_session(
                state,
                &format!("session-{user_id}"),
                user_id,
                &token,
                "2999-01-01 00:00:00",
                None,
            )
            .await
            .unwrap();
            token
        }

        let viewer_token = session_for(&state, "viewer").await;
        let admin_token = session_for(&state, "editor").await;
        user::make_sysadmin(&state, "editor").await.unwrap();

        // Built once: `create_router` installs the global Prometheus recorder, and
        // a second install panics. `Router` is `Clone`, and `oneshot` consumes.
        let router = super::create_router(state.clone());
        let get_docs = |cookie: Option<String>| {
            let router = router.clone();
            async move {
                let mut request = Request::builder().uri("/docs");
                if let Some(token) = cookie {
                    request = request.header("cookie", format!("session_token={token}"));
                }
                router
                    .oneshot(request.body(Body::empty()).unwrap())
                    .await
                    .unwrap()
                    .status()
            }
        };

        assert_eq!(
            get_docs(None).await,
            StatusCode::UNAUTHORIZED,
            "an anonymous caller must not get the route map"
        );
        assert_eq!(
            get_docs(Some(viewer_token)).await,
            StatusCode::FORBIDDEN,
            "merely holding an account must not be enough"
        );
        assert_eq!(
            get_docs(Some(admin_token)).await,
            StatusCode::OK,
            "a sysadmin still needs the docs"
        );

        cleanup_database(&path);
    }
}
