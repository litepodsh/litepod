use axum::response::sse::{Event, KeepAlive, Sse};
use axum::{
    Extension, Json,
    extract::{Multipart, Path, Query, State},
    http::header,
    response::{IntoResponse, Response},
};
use chrono::DateTime;
use futures_util::Stream;
use sqlx::Row;
use std::{convert::Infallible, ffi::OsString, pin::Pin, time::Duration};

use crate::{
    deployment::{
        self,
        compose::{catalog::catalog_from_yaml, store::ComposeFileStore},
        managed_volumes,
    },
    dtos::{
        deployment::*,
        response::{AppError, created, ok},
    },
    git::repositories::audit_log,
    podman::containers::volume_workspace::{self, WorkspaceRoot},
    security::authz::extract::{RequireApp, RequireOrg, perm},
    security::authz::{Permission, Target},
    security::middleware::auth::AuthUser,
    security::middleware::org_scope::AppScope,
    servers::runtime::ServerRuntime,
    state::state::SharedState,
};

const MAX_ARTIFACT_BYTES: usize = 100 * 1024 * 1024;

/// Moved to `handler::common` so `middleware::org_scope` can use it without
/// depending on a handler module. Re-exported here because ~60 call sites in
/// this crate reach for it through this path.
pub(crate) use crate::handler::common::check_application;

/// Container names this application actually owns — the single row in
/// `application_deployments` for non-compose apps, or every service container
/// for a compose app. Used to check a caller-supplied `container_id` isn't
/// reaching into a container that belongs to a different organization/application
/// (e.g. before opening a terminal exec session).
pub(crate) async fn valid_container_names(
    state: &SharedState,
    organization_id: &str,
    project_id: &str,
    environment_id: &str,
    application_id: &str,
) -> Result<Vec<String>, AppError> {
    check_application(
        state,
        organization_id,
        project_id,
        environment_id,
        application_id,
    )
    .await?;
    container_names(state, application_id).await
}

type DomainPath = (String, String, String, String, String);
type WorkspacePath = (String, String, String, String, String);

#[derive(serde::Deserialize)]
pub struct WorkspacePathQuery {
    pub path: String,
}

#[derive(serde::Deserialize, Default)]
pub struct WorkspaceListQuery {
    pub path: Option<String>,
}

async fn application_workspace_root(
    state: &SharedState,
    application_id: &str,
    source: &str,
) -> Result<WorkspaceRoot, AppError> {
    let mounts = deployment::volumes(state, application_id).await?;
    let mount = mounts
        .into_iter()
        .find(|mount| mount.source == source)
        .ok_or(AppError::ResourceNotFound)?;
    if mount.mount_type == "volume" {
        if ServerRuntime::for_application(state, application_id)
            .await?
            .is_remote()
        {
            Ok(WorkspaceRoot::RemoteNamedVolume {
                application_id: application_id.to_string(),
                volume: mount.source,
            })
        } else {
            Ok(WorkspaceRoot::NamedVolume(mount.source))
        }
    } else {
        if ServerRuntime::for_application(state, application_id)
            .await?
            .is_remote()
        {
            return Err(AppError::ValidationError(
                "Remote application bind mounts cannot be edited from litepod".into(),
            ));
        }
        Ok(WorkspaceRoot::Host(std::path::PathBuf::from(mount.source)))
    }
}

async fn managed_workspace_root(
    state: &SharedState,
    application_id: &str,
    volume_id: &str,
) -> Result<WorkspaceRoot, AppError> {
    managed_volumes::workspace_root(state, application_id, volume_id).await
}

pub async fn get_deployment_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppRead>,
) -> Result<impl IntoResponse, AppError> {
    let application_id = app.application_id;
    Ok(ok(deployment::deployment(&state, &application_id).await?))
}

pub async fn port_constraints_handler(
    State(state): State<SharedState>,
    RequireApp(_, _): RequireApp<perm::AppRead>,
) -> Result<impl IntoResponse, AppError> {
    Ok(ok(deployment::domains::port_constraints(&state).await?))
}

pub async fn get_health_check_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppRead>,
) -> Result<impl IntoResponse, AppError> {
    Ok(ok(
        crate::health_checks::get(&state, &app.application_id).await?
    ))
}

pub async fn update_health_check_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppUpdate>,
    axum::Json(req): axum::Json<crate::dtos::health_check::UpdateHealthCheckRequest>,
) -> Result<impl IntoResponse, AppError> {
    Ok(ok(crate::health_checks::upsert(
        &state,
        &app.application_id,
        &req,
    )
    .await?))
}

pub async fn get_boot_policy_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppRead>,
) -> Result<impl IntoResponse, AppError> {
    Ok(ok(deployment::boot::get_policy(
        &state,
        &app.application_id,
    )
    .await?))
}

pub async fn update_boot_policy_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppUpdate>,
    axum::Json(req): axum::Json<deployment::boot::BootPolicyDto>,
) -> Result<impl IntoResponse, AppError> {
    Ok(ok(deployment::boot::set_policy(
        &state,
        &app.application_id,
        &req.boot_policy,
    )
    .await?))
}

/// The keyless deploy URL for this application, created on first request.
///
/// Editor, not Viewer: `POST /deploy/{token}` is unauthenticated, so whoever can
/// read this token can trigger deploys. Handing it to a read-only member would
/// hand them Editor's main power.
pub async fn get_deploy_hook_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppUpdate>,
) -> Result<impl IntoResponse, AppError> {
    let application_id = app.application_id;
    let token = deployment::deploy_hook::ensure_token(&state, &application_id).await?;
    Ok(ok(
        serde_json::json!({ "token": token, "url": deployment::deploy_hook::hook_url(&crate::handler::settings::public_base_url(&state).await, &token) }),
    ))
}

/// Rotates the token, immediately invalidating the previous URL.
pub async fn rotate_deploy_hook_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppUpdate>,
) -> Result<impl IntoResponse, AppError> {
    let application_id = app.application_id;
    let token = deployment::deploy_hook::rotate_token(&state, &application_id).await?;
    Ok(ok(
        serde_json::json!({ "token": token, "url": deployment::deploy_hook::hook_url(&crate::handler::settings::public_base_url(&state).await, &token) }),
    ))
}

pub async fn get_source_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppRead>,
) -> Result<impl IntoResponse, AppError> {
    let application_id = app.application_id;
    Ok(ok(deployment::source::get(&state, &application_id).await?))
}

pub async fn list_deployment_runs_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppRead>,
) -> Result<impl IntoResponse, AppError> {
    let application_id = app.application_id;
    Ok(ok(deployment::history::list(&state, &application_id).await?))
}

#[derive(serde::Deserialize, Default)]
pub struct DeploymentRunLogsQuery {
    pub tail: Option<usize>,
}

pub async fn get_deployment_run_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppRead>,
    // The extractor covers the leading segments.
    Path((_, _, _, _, run_id)): Path<(String, String, String, String, String)>,
    Query(query): Query<DeploymentRunLogsQuery>,
) -> Result<impl IntoResponse, AppError> {
    let application_id = app.application_id;
    Ok(ok(deployment::history::get(
        &state,
        &application_id,
        &run_id,
        query.tail,
    )
    .await?))
}

pub async fn delete_deployment_run_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppUpdate>,
    // The extractor covers the leading segments.
    Path((_, _, _, _, run_id)): Path<(String, String, String, String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let application_id = app.application_id;
    deployment::history::delete_run(&state, &application_id, &run_id).await?;
    Ok(ok(()))
}

pub async fn delete_deployment_runs_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppUpdate>,
) -> Result<impl IntoResponse, AppError> {
    let application_id = app.application_id;
    deployment::history::delete_all(&state, &application_id).await?;
    Ok(ok(()))
}

#[derive(serde::Deserialize, Default)]
pub struct QueueListQuery {
    pub limit: Option<usize>,
}

pub async fn list_queue_for_organization_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgRead>,
    Query(query): Query<QueueListQuery>,
) -> Result<impl IntoResponse, AppError> {
    let jobs = deployment::queue::list_for_organization(
        &state,
        &org.organization_id,
        query.limit.unwrap_or(50),
    )
    .await?;
    // ponytail: LIMIT applies before this filter, so a narrowly-scoped user can see fewer than
    // `limit` rows; push the filter into SQL if that ever matters.
    Ok(ok(jobs
        .into_iter()
        .filter(|job| {
            org.permissions.allows(
                Permission::AppRead,
                Target::Environment {
                    project_id: &job.project_id,
                    environment_id: &job.environment_id,
                },
            )
        })
        .collect::<Vec<_>>()))
}

pub async fn get_queue_handler(
    State(state): State<SharedState>,
    Path(job_id): Path<String>,
    Extension(user): Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    deployment::queue::ensure_user_access(&state, &user, &job_id, Permission::AppRead).await?;
    Ok(ok(deployment::queue::get(&state, &job_id).await?))
}

pub async fn cancel_queue_handler(
    State(state): State<SharedState>,
    Path(job_id): Path<String>,
    Extension(user): Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    // Cancelling changes job state, not just observes it — same floor as any other write.
    deployment::queue::ensure_user_access(&state, &user, &job_id, Permission::AppDeploy).await?;
    let job = deployment::queue::request_cancel(&state, &job_id).await?;
    audit_log::record(
        &state,
        &job.organization_id,
        &user,
        "deployment.canceled",
        Some("application"),
        Some(&job.application_id),
        Some(&job.application_name),
        None,
        None,
        None,
        None,
    )
    .await
    .ok();
    Ok(ok(job))
}

const STREAM_ACCESS_TTL: Duration = Duration::from_secs(5);

/// Per-connection memo of `ensure_user_access` decisions. Without it the queue
/// stream runs one database query per log line per subscriber, on the same
/// connection the build is writing its logs through. The TTL keeps a revoked
/// membership from being honoured for the whole life of a long stream.
struct StreamAccessCache {
    ttl: Duration,
    entries: std::collections::HashMap<String, (std::time::Instant, bool)>,
}

impl StreamAccessCache {
    fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            entries: std::collections::HashMap::new(),
        }
    }

    fn get(&self, job_id: &str, now: std::time::Instant) -> Option<bool> {
        self.entries
            .get(job_id)
            .filter(|(checked_at, _)| now.duration_since(*checked_at) < self.ttl)
            .map(|(_, allowed)| *allowed)
    }

    fn insert(&mut self, job_id: &str, allowed: bool, now: std::time::Instant) {
        self.entries.insert(job_id.to_string(), (now, allowed));
    }
}

pub async fn stream_queue_handler(
    State(state): State<SharedState>,
    Extension(user): Extension<AuthUser>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let mut receiver = state.queue_runtime.events.subscribe();
    let stream = Box::pin(async_stream::stream! {
        let mut access = StreamAccessCache::new(STREAM_ACCESS_TTL);
        loop {
            match receiver.recv().await {
                Ok(event) => {
                    let now = std::time::Instant::now();
                    let allowed = match access.get(&event.job_id, now) {
                        Some(allowed) => allowed,
                        None => {
                            let allowed = deployment::queue::ensure_user_access(&state, &user, &event.job_id, Permission::AppRead).await.is_ok();
                            access.insert(&event.job_id, allowed, now);
                            allowed
                        }
                    };
                    if allowed {
                        let kind = event.kind.clone();
                        yield Ok(Event::default().event(kind).data(serde_json::to_string(&event).unwrap_or_default()));
                    }
                }
                // The lag error does not say which job lost events, so this
                // carries no job id and skips the access check: it reveals only
                // that this subscriber fell behind.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    let event = deployment::queue::QueueEvent {
                        job_id: String::new(),
                        kind: "resync".into(),
                        sequence: None,
                        level: None,
                        payload: skipped.to_string(),
                        server_id: None,
                    };
                    yield Ok(Event::default().event("resync").data(serde_json::to_string(&event).unwrap_or_default()));
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

pub async fn save_source_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppUpdate>,
    Json(request): Json<UpdateSourceRequest>,
) -> Result<impl IntoResponse, AppError> {
    let application_id = app.application_id;
    Ok(ok(deployment::source::save(
        &state,
        &application_id,
        request,
    )
    .await?))
}

type BasicAuthUserPath = (String, String, String, String, String);

pub async fn get_basic_auth_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppRead>,
) -> Result<impl IntoResponse, AppError> {
    let application_id = app.application_id;
    Ok(ok(
        deployment::basic_auth::get(&state, &application_id).await?
    ))
}

pub async fn update_basic_auth_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppUpdate>,
    Json(request): Json<UpdateBasicAuthRequest>,
) -> Result<impl IntoResponse, AppError> {
    let application_id = app.application_id;
    Ok(ok(deployment::basic_auth::update(
        &state,
        &application_id,
        request,
    )
    .await?))
}

pub async fn add_basic_auth_user_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppUpdate>,
    Json(request): Json<AddBasicAuthUserRequest>,
) -> Result<impl IntoResponse, AppError> {
    let application_id = app.application_id;
    Ok(created(
        deployment::basic_auth::add_user(
            &state,
            &application_id,
            request.username,
            request.password,
        )
        .await?,
    ))
}

pub async fn update_basic_auth_user_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppUpdate>,
    Path((_, _, _, _, user_id)): Path<BasicAuthUserPath>,
    Json(request): Json<UpdateBasicAuthUserRequest>,
) -> Result<impl IntoResponse, AppError> {
    let application_id = app.application_id;
    Ok(ok(deployment::basic_auth::update_user(
        &state,
        &application_id,
        &user_id,
        request.username,
        request.password,
    )
    .await?))
}

pub async fn delete_basic_auth_user_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppUpdate>,
    Path((_, _, _, _, user_id)): Path<BasicAuthUserPath>,
) -> Result<impl IntoResponse, AppError> {
    let application_id = app.application_id;
    deployment::basic_auth::delete_user(&state, &application_id, &user_id).await?;
    Ok(ok(()))
}

pub async fn get_maintenance_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppRead>,
) -> Result<impl IntoResponse, AppError> {
    Ok(ok(
        deployment::maintenance::get(&state, &app.application_id).await?
    ))
}

/// Keeps an uploaded ZIP as the application's maintenance site.
pub async fn upload_maintenance_site_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppUpdate>,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, AppError> {
    let field = multipart
        .next_field()
        .await
        .map_err(|e| AppError::ValidationError(format!("Invalid multipart upload: {e}")))?
        .ok_or_else(|| AppError::ValidationError("Choose a ZIP to upload".into()))?;
    if field.name() != Some("archive") {
        return Err(AppError::ValidationError(
            "Upload field must be named archive".into(),
        ));
    }
    let bytes = field
        .bytes()
        .await
        .map_err(|e| AppError::ValidationError(format!("Could not read upload: {e}")))?;
    let maintenance =
        deployment::maintenance::upload_site(&state, &app.application_id, &bytes).await?;
    // The files are not logged: only that a site was uploaded, and which one.
    let metadata = serde_json::json!({ "site": maintenance.site.as_ref().map(|site| &site.id) })
        .to_string();
    audit_log::record(
        &state,
        &app.organization_id,
        &app.user,
        "maintenance.site_uploaded",
        Some("application"),
        Some(&app.application_id),
        Some(&app.application_name),
        Some(&metadata),
        None,
        None,
        None,
    )
    .await
    .ok();
    Ok(created(maintenance))
}

pub async fn delete_maintenance_site_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppUpdate>,
) -> Result<impl IntoResponse, AppError> {
    let maintenance = deployment::maintenance::remove_site(&state, &app.application_id).await?;
    audit_log::record(
        &state,
        &app.organization_id,
        &app.user,
        "maintenance.site_removed",
        Some("application"),
        Some(&app.application_id),
        Some(&app.application_name),
        None,
        None,
        None,
        None,
    )
    .await
    .ok();
    Ok(ok(maintenance))
}

pub async fn update_maintenance_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppUpdate>,
    Json(request): Json<UpdateMaintenanceRequest>,
) -> Result<impl IntoResponse, AppError> {
    let previous = deployment::maintenance::get(&state, &app.application_id).await?;
    let page_changed = request.page_html.is_some();
    let maintenance =
        deployment::maintenance::update(&state, &app.application_id, request).await?;
    if maintenance.enabled != previous.enabled || page_changed {
        // The page body is not logged: only that the mode or the page changed.
        let metadata =
            serde_json::json!({ "enabled": maintenance.enabled, "page_changed": page_changed })
                .to_string();
        audit_log::record(
            &state,
            &app.organization_id,
            &app.user,
            if maintenance.enabled != previous.enabled {
                if maintenance.enabled {
                    "maintenance.enabled"
                } else {
                    "maintenance.disabled"
                }
            } else {
                "maintenance.page_updated"
            },
            Some("application"),
            Some(&app.application_id),
            Some(&app.application_name),
            Some(&metadata),
            None,
            None,
            None,
        )
        .await
        .ok();
    }
    Ok(ok(maintenance))
}

pub async fn list_domains_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppRead>,
) -> Result<impl IntoResponse, AppError> {
    let application_id = app.application_id;
    Ok(ok(deployment::domains::list(&state, &application_id).await?))
}

pub async fn validate_domain_dns_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppRead>,
    // The extractor covers the leading segments.
    Path((_, _, _, _, domain_id)): Path<DomainPath>,
) -> Result<impl IntoResponse, AppError> {
    let application_id = app.application_id;
    Ok(ok(deployment::domains::validate_dns(
        &state,
        &application_id,
        &domain_id,
    )
    .await?))
}

pub async fn validate_host_dns_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppRead>,
    Json(request): Json<ValidateHostDnsRequest>,
) -> Result<impl IntoResponse, AppError> {
    Ok(ok(deployment::domains::validate_host_dns(
        &state,
        &app.application_id,
        &request.host,
        request.server_id.as_deref(),
    )
    .await?))
}

pub async fn random_sslip_host_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppDomainsManage>,
) -> Result<impl IntoResponse, AppError> {
    let application_id = app.application_id;
    let host = deployment::domains::random_sslip_host(&state, &application_id).await?;
    Ok(ok(serde_json::json!({ "host": host })))
}

fn authorize_certificate_assignment(
    admin: bool,
    request: &crate::deployment::certificates::CertificateRequest,
    current: Option<&str>,
) -> Result<(), AppError> {
    if !admin
        && (request.certificate_pem.is_some()
            || request.private_key_pem.is_some()
            || request
                .certificate_id
                .as_deref()
                .is_some_and(|id| Some(id) != current))
    {
        return Err(AppError::InsufficientPermissions);
    }
    Ok(())
}

pub async fn create_domain_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppDomainsManage>,
    Json(request): Json<CreateDomainRequest>,
) -> Result<impl IntoResponse, AppError> {
    authorize_certificate_assignment(app.user.is_sysadmin, &request.certificate, None)?;
    let application_id = app.application_id;
    Ok(created(
        deployment::domains::create(&state, &application_id, request).await?,
    ))
}

pub async fn generate_domain_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppDomainsManage>,
) -> Result<impl IntoResponse, AppError> {
    let application_id = app.application_id;
    Ok(created(
        deployment::domains::generate(&state, &application_id).await?,
    ))
}

pub async fn update_domain_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppDomainsManage>,
    // The extractor covers the leading segments.
    Path((_, _, _, _, domain_id)): Path<DomainPath>,
    Json(request): Json<UpdateDomainRequest>,
) -> Result<impl IntoResponse, AppError> {
    let current = deployment::domains::get(&state, &app.application_id, &domain_id).await?;
    authorize_certificate_assignment(
        app.user.is_sysadmin,
        &request.certificate,
        current.certificate_id.as_deref(),
    )?;
    let application_id = app.application_id;
    Ok(ok(deployment::domains::update(
        &state,
        &application_id,
        &domain_id,
        request,
    )
    .await?))
}

pub async fn delete_domain_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppDomainsManage>,
    // The extractor covers the leading segments.
    Path((_, _, _, _, domain_id)): Path<DomainPath>,
) -> Result<impl IntoResponse, AppError> {
    let application_id = app.application_id;
    deployment::domains::delete(&state, &application_id, &domain_id).await?;
    Ok(ok(()))
}

pub async fn upload_artifact_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppUpdate>,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, AppError> {
    let application_id = app.application_id;
    let field = multipart
        .next_field()
        .await
        .map_err(|e| AppError::ValidationError(format!("Invalid multipart upload: {e}")))?
        .ok_or_else(|| AppError::ValidationError("Choose one artifact to upload".into()))?;
    if field.name() != Some("artifact") {
        return Err(AppError::ValidationError(
            "Upload field must be named artifact".into(),
        ));
    }
    let filename = field.file_name().unwrap_or("application").to_owned();
    let bytes = field
        .bytes()
        .await
        .map_err(|e| AppError::ValidationError(format!("Could not read upload: {e}")))?;
    if bytes.len() > MAX_ARTIFACT_BYTES {
        return Err(AppError::ValidationError(
            "Artifact exceeds the 100 MiB upload limit".into(),
        ));
    }
    if multipart
        .next_field()
        .await
        .map_err(|e| AppError::ValidationError(format!("Invalid multipart upload: {e}")))?
        .is_some()
    {
        return Err(AppError::ValidationError(
            "Upload exactly one artifact".into(),
        ));
    }
    Ok(created(
        deployment::save_artifact(&state, &application_id, filename, bytes.to_vec()).await?,
    ))
}

pub async fn download_artifact_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppRead>,
) -> Result<Response, AppError> {
    let (filename, bytes, mime) =
        deployment::download_artifact(&state, &app.application_id).await?;
    let mut response = Response::new(axum::body::Body::from(bytes));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static(mime),
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        axum::http::HeaderValue::from_str(&format!(
            "attachment; filename=\"{}\"",
            filename
                .chars()
                .map(
                    |c| if c.is_ascii_graphic() && c != '"' && c != '\\' || c == ' ' {
                        c
                    } else {
                        '_'
                    }
                )
                .collect::<String>()
        ))
        .map_err(|error| AppError::InternalError(error.to_string()))?,
    );
    Ok(response)
}

/// Keeps an uploaded ZIP as the application's source tree. The next deploy
/// unpacks it where a clone would go.
pub async fn upload_source_archive_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppUpdate>,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, AppError> {
    let field = multipart
        .next_field()
        .await
        .map_err(|e| AppError::ValidationError(format!("Invalid multipart upload: {e}")))?
        .ok_or_else(|| AppError::ValidationError("Choose a ZIP to upload".into()))?;
    if field.name() != Some("archive") {
        return Err(AppError::ValidationError(
            "Upload field must be named archive".into(),
        ));
    }
    let bytes = field
        .bytes()
        .await
        .map_err(|e| AppError::ValidationError(format!("Could not read upload: {e}")))?;
    deployment::source_archive::store(&state, &app.application_id, &bytes).await?;
    Ok(created(
        deployment::source::get(&state, &app.application_id).await?,
    ))
}

pub async fn download_source_archive_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppRead>,
) -> Result<Response, AppError> {
    let bytes = tokio::fs::read(deployment::source_archive::archive_file(
        &state,
        &app.application_id,
    ))
    .await
    .map_err(|_| AppError::ValidationError("No source archive uploaded".into()))?;
    let mut response = Response::new(axum::body::Body::from(bytes));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/zip"),
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        axum::http::HeaderValue::from_static("attachment; filename=\"source.zip\""),
    );
    Ok(response)
}

pub async fn delete_artifact_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppUpdate>,
) -> Result<impl IntoResponse, AppError> {
    let application_id = app.application_id;
    Ok(ok(
        deployment::delete_artifact(&state, &application_id).await?
    ))
}

/// The one route here whose permission depends on the query: listing the
/// masked variables needs `app.read`, revealing the plaintext `app.env.read`.
/// The extractor already loaded the caller's permissions, so the extra step is
/// a lookup rather than a second database round-trip.
pub async fn get_environment_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppRead>,
    Query(query): Query<EnvironmentScopeQuery>,
) -> Result<impl IntoResponse, AppError> {
    let target = Target::Environment {
        project_id: &app.project_id,
        environment_id: &app.environment_id,
    };
    if query.reveal && !app.permissions.allows(Permission::AppEnvRead, target) {
        return Err(AppError::InsufficientPermissions);
    }
    Ok(ok(deployment::environment(
        &state,
        &app.application_id,
        query.scope.as_deref().unwrap_or("runtime"),
        query.reveal,
    )
    .await?))
}

#[derive(serde::Deserialize, Default)]
pub struct EnvironmentScopeQuery {
    pub scope: Option<String>,
    #[serde(default)]
    pub reveal: bool,
}

/// The runtime env's `{{url}}`-style markers and what each resolves to now.
/// Only marker text and public addresses leave here, never variable values, so
/// `app.read` is enough.
pub async fn domain_markers_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppRead>,
) -> Result<impl IntoResponse, AppError> {
    Ok(ok(crate::deployment::compose::marker_routes::describe(
        &state,
        &app.application_id,
    )
    .await?))
}

pub async fn update_environment_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppEnvUpdate>,
    Json(request): Json<UpdateEnvironmentRequest>,
) -> Result<impl IntoResponse, AppError> {
    let application_id = app.application_id;
    let scope = request.scope.as_deref().unwrap_or("runtime");
    // Expands any `{{password_24}}`-style marker the user typed or pasted but never
    // accepted from the editor's autocomplete, so a literal marker never reaches a
    // container. Same resolver catalog template creation uses.
    let content = crate::templates::secrets::substitute_placeholder_secrets(&request.content);
    let parsed = deployment::parse_environment_content(&content)?;
    if scope == "runtime" {
        crate::deployment::compose::marker_routes::validate_env(&state, &application_id, &parsed)
            .await?;
    } else if crate::deployment::compose::domain_markers::has_markers(&parsed) {
        // Build variables are never resolved, so a marker would reach the build literally.
        return Err(AppError::ValidationError(
            "Domain markers ({{url}}, {{domain}}) only work in runtime variables".into(),
        ));
    }
    let result = deployment::save_environment(&state, &application_id, scope, &content).await?;
    let metadata = serde_json::json!({ "scope": scope }).to_string();
    audit_log::record(
        &state,
        &app.organization_id,
        &app.user,
        "environment_variables.updated",
        Some("application"),
        Some(&application_id),
        Some(&app.application_name),
        Some(&metadata),
        None,
        None,
        None,
    )
    .await
    .ok();
    Ok(ok(result))
}

pub async fn deploy_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppDeploy>,
    request: Option<Json<DeployRequest>>,
) -> Result<impl IntoResponse, AppError> {
    let AppScope {
        organization_id,
        application_id,
        application_name,
        user,
        ..
    } = app;
    if let Some(clean_build) = request.and_then(|Json(request)| request.clean_build) {
        deployment::source::apply_clean_build_for_deploy(&state, &application_id, clean_build)
            .await?;
    }
    let job = deployment::enqueue_for_application(
        &state,
        &application_id,
        "manual_deploy",
        Some(&user.user_id),
    )
    .await?;
    crate::handler::dashboard::invalidate_organization_dashboard(&state, &organization_id).await;
    audit_log::record(
        &state,
        &organization_id,
        &user,
        "deployment.triggered",
        Some("application"),
        Some(&application_id),
        Some(&application_name),
        None,
        None,
        None,
        None,
    )
    .await
    .ok();
    Ok(Json(serde_json::json!({ "error": null, "data": job })))
}

/// Re-deploys one replica of a group-targeted application, leaving the other
/// servers as they are.
pub async fn retry_replica_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppDeploy>,
    Path((_, _, _, _, server_id)): Path<(String, String, String, String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let AppScope {
        organization_id,
        application_id,
        application_name,
        user,
        ..
    } = app;
    let targets = crate::servers::groups::resolve_target_servers(&state, &application_id).await?;
    if !targets.iter().any(|server| server.id == server_id) {
        return Err(AppError::ResourceNotFound);
    }
    let job = deployment::queue::pinned_to(
        &server_id,
        deployment::enqueue_for_application(
            &state,
            &application_id,
            "manual_deploy",
            Some(&user.user_id),
        ),
    )
    .await?;
    crate::handler::dashboard::invalidate_organization_dashboard(&state, &organization_id).await;
    audit_log::record(
        &state,
        &organization_id,
        &user,
        "deployment.triggered",
        Some("application"),
        Some(&application_id),
        Some(&application_name),
        None,
        None,
        None,
        None,
    )
    .await
    .ok();
    Ok(ok(job))
}

pub async fn rollback_deployment_run_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppDeploy>,
    // The extractor covers the leading segments.
    Path((_, _, _, _, run_id)): Path<(String, String, String, String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let AppScope {
        organization_id,
        application_id,
        application_name,
        user,
        ..
    } = app;
    let job = deployment::enqueue_rollback_for_application(
        &state,
        &application_id,
        &run_id,
        Some(&user.user_id),
    )
    .await?;
    crate::handler::dashboard::invalidate_organization_dashboard(&state, &organization_id).await;
    audit_log::record(
        &state,
        &organization_id,
        &user,
        "deployment.rolled_back",
        Some("application"),
        Some(&application_id),
        Some(&application_name),
        None,
        None,
        None,
        None,
    )
    .await
    .ok();
    Ok(Json(serde_json::json!({ "error": null, "data": job })))
}

pub async fn set_clean_build_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppUpdate>,
    Json(request): Json<SetCleanBuildRequest>,
) -> Result<impl IntoResponse, AppError> {
    let application_id = app.application_id;
    Ok(ok(deployment::source::set_clean_build(
        &state,
        &application_id,
        request.clean_build,
    )
    .await?))
}

pub async fn stop_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppDeploy>,
) -> Result<impl IntoResponse, AppError> {
    let AppScope {
        organization_id,
        application_id,
        application_name,
        user,
        ..
    } = app;
    let result = deployment::stop(&state, &application_id, Some(&user.user_id)).await?;
    audit_log::record(
        &state,
        &organization_id,
        &user,
        "deployment.stopped",
        Some("application"),
        Some(&application_id),
        Some(&application_name),
        None,
        None,
        None,
        None,
    )
    .await
    .ok();
    Ok(ok(result))
}

pub async fn volumes_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppRead>,
) -> Result<impl IntoResponse, AppError> {
    let application_id = app.application_id;
    Ok(ok(deployment::volumes(&state, &application_id).await?))
}

pub async fn list_managed_volumes_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppRead>,
) -> Result<impl IntoResponse, AppError> {
    let application_id = app.application_id;
    Ok(ok(managed_volumes::list(&state, &application_id).await?))
}

pub async fn create_managed_volume_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppUpdate>,
    Json(request): Json<CreateManagedVolumeRequest>,
) -> Result<impl IntoResponse, AppError> {
    let application_id = app.application_id;
    let source = deployment::source::get(&state, &application_id).await?;
    let is_compose = source.deploy_type == "compose"
        || (source.deploy_type == "git" && source.build_method == "compose");
    let services = if source.deploy_type == "compose" {
        let content = ComposeFileStore::new(&state.data_dir, &application_id)
            .read_content()
            .await
            .map_err(AppError::InternalError)?;
        catalog_from_yaml(&content, false)?
            .services
            .into_iter()
            .map(|service| service.name)
            .collect()
    } else if is_compose {
        source.compose_public_service.into_iter().collect()
    } else {
        Vec::new()
    };
    let volume =
        managed_volumes::create_record(&state, &application_id, is_compose, request, &services)
            .await?;
    audit_log::record(
        &state,
        &app.organization_id,
        &app.user,
        "volume.created",
        Some("volume"),
        Some(&volume.id),
        Some(&volume.name),
        None,
        None,
        None,
        None,
    )
    .await
    .ok();
    Ok(created(volume))
}

pub async fn attach_managed_volume_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppUpdate>,
    // The extractor covers the leading segments.
    Path((_, _, _, _, volume_id)): Path<WorkspacePath>,
) -> Result<impl IntoResponse, AppError> {
    let application_id = app.application_id;
    Ok(ok(managed_volumes::attach(
        &state,
        &application_id,
        &volume_id,
    )
    .await?))
}

pub async fn detach_managed_volume_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppUpdate>,
    // The extractor covers the leading segments.
    Path((_, _, _, _, volume_id)): Path<WorkspacePath>,
) -> Result<impl IntoResponse, AppError> {
    let application_id = app.application_id;
    Ok(ok(managed_volumes::detach(
        &state,
        &application_id,
        &volume_id,
    )
    .await?))
}

pub async fn delete_managed_volume_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppUpdate>,
    // The extractor covers the leading segments.
    Path((_, _, _, _, volume_id)): Path<WorkspacePath>,
) -> Result<impl IntoResponse, AppError> {
    let application_id = app.application_id;
    managed_volumes::delete(&state, &application_id, &volume_id).await?;
    audit_log::record(
        &state,
        &app.organization_id,
        &app.user,
        "volume.deleted",
        Some("volume"),
        Some(&volume_id),
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .ok();
    Ok(ok(()))
}

pub async fn managed_workspace_list_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppRead>,
    // The extractor covers the leading segments.
    Path((_, _, _, _, volume_id)): Path<WorkspacePath>,
    Query(query): Query<WorkspaceListQuery>,
) -> Result<impl IntoResponse, AppError> {
    let application_id = app.application_id;
    let depth = if query.path.is_some() { 1 } else { 2 };
    Ok(ok(volume_workspace::list_entries(
        &state,
        managed_workspace_root(&state, &application_id, &volume_id).await?,
        query.path.as_deref(),
        depth,
    )
    .await?))
}

pub async fn managed_workspace_file_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppRead>,
    // The extractor covers the leading segments.
    Path((_, _, _, _, volume_id)): Path<WorkspacePath>,
    Query(query): Query<WorkspacePathQuery>,
) -> Result<impl IntoResponse, AppError> {
    let application_id = app.application_id;
    Ok(ok(volume_workspace::read_file(
        &state,
        managed_workspace_root(&state, &application_id, &volume_id).await?,
        &query.path,
    )
    .await?))
}

pub async fn managed_workspace_save_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppUpdate>,
    // The extractor covers the leading segments.
    Path((_, _, _, _, volume_id)): Path<WorkspacePath>,
    Query(query): Query<WorkspacePathQuery>,
    Json(request): Json<UpdateWorkspaceTextRequest>,
) -> Result<impl IntoResponse, AppError> {
    let application_id = app.application_id;
    let result = volume_workspace::write_text_file(
        &state,
        managed_workspace_root(&state, &application_id, &volume_id).await?,
        &query.path,
        request.content,
    )
    .await?;
    audit_log::record(
        &state,
        &app.organization_id,
        &app.user,
        "volume.file_updated",
        Some("volume"),
        Some(&volume_id),
        Some(&query.path),
        None,
        None,
        None,
        None,
    )
    .await
    .ok();
    Ok(ok(result))
}

pub async fn managed_workspace_create_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppUpdate>,
    // The extractor covers the leading segments.
    Path((_, _, _, _, volume_id)): Path<WorkspacePath>,
    Json(request): Json<CreateWorkspaceEntryRequest>,
) -> Result<impl IntoResponse, AppError> {
    let application_id = app.application_id;
    Ok(ok(volume_workspace::create_entry(
        &state,
        managed_workspace_root(&state, &application_id, &volume_id).await?,
        &request.path,
        &request.kind,
        request.content,
    )
    .await?))
}

pub async fn managed_workspace_upload_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppUpdate>,
    // The extractor covers the leading segments.
    Path((_, _, _, _, volume_id)): Path<WorkspacePath>,
    Query(query): Query<WorkspacePathQuery>,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, AppError> {
    let application_id = app.application_id;
    let field = multipart
        .next_field()
        .await
        .map_err(|error| AppError::ValidationError(error.to_string()))?
        .ok_or_else(|| AppError::ValidationError("Choose one file to upload".into()))?;
    let bytes = field
        .bytes()
        .await
        .map_err(|error| AppError::InternalError(error.to_string()))?;
    Ok(ok(volume_workspace::upload_new_file(
        &state,
        managed_workspace_root(&state, &application_id, &volume_id).await?,
        &query.path,
        bytes.to_vec(),
    )
    .await?))
}

pub async fn managed_workspace_download_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppRead>,
    // The extractor covers the leading segments.
    Path((_, _, _, _, volume_id)): Path<WorkspacePath>,
    Query(query): Query<WorkspacePathQuery>,
) -> Result<Response, AppError> {
    let application_id = app.application_id;
    let (bytes, mime) = volume_workspace::read_file_bytes(
        &state,
        managed_workspace_root(&state, &application_id, &volume_id).await?,
        &query.path,
    )
    .await?;
    let filename = std::path::Path::new(&query.path)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let mut response = Response::new(axum::body::Body::from(bytes));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        axum::http::HeaderValue::from_str(
            &mime.unwrap_or_else(|| "application/octet-stream".into()),
        )
        .map_err(|error| AppError::InternalError(error.to_string()))?,
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        axum::http::HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
            .map_err(|error| AppError::InternalError(error.to_string()))?,
    );
    Ok(response)
}

pub async fn workspace_list_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppRead>,
    // The extractor covers the leading segments.
    Path((_, _, _, _, source)): Path<WorkspacePath>,
    Query(query): Query<WorkspaceListQuery>,
) -> Result<impl IntoResponse, AppError> {
    let application_id = app.application_id;
    let depth = if query.path.is_some() { 1 } else { 2 };
    Ok(ok(volume_workspace::list_entries(
        &state,
        application_workspace_root(&state, &application_id, &source).await?,
        query.path.as_deref(),
        depth,
    )
    .await?))
}
pub async fn workspace_file_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppRead>,
    // The extractor covers the leading segments.
    Path((_, _, _, _, source)): Path<WorkspacePath>,
    Query(query): Query<WorkspacePathQuery>,
) -> Result<impl IntoResponse, AppError> {
    let application_id = app.application_id;
    Ok(ok(volume_workspace::read_file(
        &state,
        application_workspace_root(&state, &application_id, &source).await?,
        &query.path,
    )
    .await?))
}
pub async fn workspace_save_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppUpdate>,
    // The extractor covers the leading segments.
    Path((_, _, _, _, source)): Path<WorkspacePath>,
    Query(query): Query<WorkspacePathQuery>,
    Json(request): Json<UpdateWorkspaceTextRequest>,
) -> Result<impl IntoResponse, AppError> {
    let application_id = app.application_id;
    Ok(ok(volume_workspace::write_text_file(
        &state,
        application_workspace_root(&state, &application_id, &source).await?,
        &query.path,
        request.content,
    )
    .await?))
}
pub async fn workspace_create_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppUpdate>,
    // The extractor covers the leading segments.
    Path((_, _, _, _, source)): Path<WorkspacePath>,
    Json(request): Json<CreateWorkspaceEntryRequest>,
) -> Result<impl IntoResponse, AppError> {
    let application_id = app.application_id;
    Ok(ok(volume_workspace::create_entry(
        &state,
        application_workspace_root(&state, &application_id, &source).await?,
        &request.path,
        &request.kind,
        request.content,
    )
    .await?))
}
pub async fn workspace_upload_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppUpdate>,
    // The extractor covers the leading segments.
    Path((_, _, _, _, source)): Path<WorkspacePath>,
    Query(query): Query<WorkspacePathQuery>,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, AppError> {
    let application_id = app.application_id;
    let field = multipart
        .next_field()
        .await
        .map_err(|error| AppError::ValidationError(error.to_string()))?
        .ok_or_else(|| AppError::ValidationError("Choose one file to upload".into()))?;
    let bytes = field
        .bytes()
        .await
        .map_err(|error| AppError::InternalError(error.to_string()))?;
    Ok(ok(volume_workspace::upload_new_file(
        &state,
        application_workspace_root(&state, &application_id, &source).await?,
        &query.path,
        bytes.to_vec(),
    )
    .await?))
}
pub async fn workspace_download_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppRead>,
    // The extractor covers the leading segments.
    Path((_, _, _, _, source)): Path<WorkspacePath>,
    Query(query): Query<WorkspacePathQuery>,
) -> Result<Response, AppError> {
    let application_id = app.application_id;
    let (bytes, mime) = volume_workspace::read_file_bytes(
        &state,
        application_workspace_root(&state, &application_id, &source).await?,
        &query.path,
    )
    .await?;
    let filename = std::path::Path::new(&query.path)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let mut response = Response::new(axum::body::Body::from(bytes));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        axum::http::HeaderValue::from_str(
            &mime.unwrap_or_else(|| "application/octet-stream".into()),
        )
        .map_err(|error| AppError::InternalError(error.to_string()))?,
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        axum::http::HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
            .map_err(|error| AppError::InternalError(error.to_string()))?,
    );
    Ok(response)
}

/// Podman with .timestamps(true) prepends each line with RFC3339Nano + space.
/// Parse the timestamp prefix; if it fails, treat the whole line as text.
fn parse_log_line(line: &str) -> (String, Option<String>) {
    if let Some(space_pos) = line.find(' ') {
        let candidate = &line[..space_pos];
        if DateTime::parse_from_rfc3339(candidate).is_ok() {
            return (
                line[space_pos + 1..].to_string(),
                Some(candidate.to_string()),
            );
        }
    }
    (line.to_string(), None)
}

/// The `podman logs` argv both log handlers share, so the follow and snapshot
/// reads cannot drift in the flags that shape the output `parse_log_line` expects.
pub(crate) fn logs_argv(container_name: &str, tail: usize, follow: bool) -> Vec<OsString> {
    let mut args = vec![OsString::from("logs")];
    if follow {
        args.push(OsString::from("--follow"));
    }
    args.extend([
        OsString::from("--timestamps"),
        OsString::from("--tail"),
        OsString::from(tail.to_string()),
        OsString::from(container_name),
    ]);
    args
}

/// A container/network Podman cannot find. The wording depends on the command and the
/// Podman version: `inspect` says `no such object`, the libpod API says `no such container`,
/// `network inspect` says `no such network`.
pub(crate) fn missing_podman_resource(error: &AppError) -> bool {
    let message = error.to_string().to_ascii_lowercase();
    message.contains("no such container")
        || message.contains("no such object")
        || message.contains("no such network")
}

/// A container Podman cannot find is a missing resource, not a server fault. Without
/// this the caller sees a 500 carrying Podman's raw 404 text.
fn map_missing_container(error: AppError) -> AppError {
    if missing_podman_resource(&error) {
        return AppError::ResourceNotFound;
    }
    error
}

/// Reads one pipe of a streaming child line by line into a shared channel. The
/// task ends when the pipe closes, which happens as soon as the child is killed.
fn forward_lines<R>(reader: R, sender: tokio::sync::mpsc::Sender<String>)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        use tokio::io::AsyncBufReadExt;
        let mut lines = tokio::io::BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if sender.send(line).await.is_err() {
                break;
            }
        }
    });
}

#[derive(serde::Deserialize, Default)]
pub struct LogsQuery {
    pub tail: Option<usize>,
    pub container: Option<String>,
    /// Replica pin for a group-targeted application.
    pub server_id: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RuntimeLogTarget {
    pub container_name: String,
    pub service: Option<String>,
    pub status: String,
}

pub(crate) fn is_running_container(status: &str) -> bool {
    matches!(status.to_ascii_lowercase().as_str(), "running" | "up")
        || status.to_ascii_lowercase().starts_with("running")
        || status.to_ascii_lowercase().starts_with("up")
}

/// Container names only, with no live state lookup. The metrics stream asks for these
/// every 8s, so it must never pay for the inspect that `log_targets` does.
pub(crate) async fn container_names(
    state: &SharedState,
    application_id: &str,
) -> Result<Vec<String>, AppError> {
    let source = deployment::source::get(state, application_id).await?;
    if source.deploy_type == "compose" {
        return Ok(
            deployment::compose_standalone::status(state, application_id)
                .await?
                .into_iter()
                .map(|service| service.container_name)
                .collect(),
        );
    }
    let conn = crate::git::repositories::db::get_conn(state).await;
    let name =
        sqlx::query("SELECT container_name FROM application_deployments WHERE application_id = ?1")
            .bind(application_id)
            .fetch_optional(&conn)
            .await
            .map_err(|e| AppError::DatabaseError(e.to_string()))?
            .map(|row| {
                row.try_get::<String, _>(0)
                    .map_err(|e| AppError::DatabaseError(e.to_string()))
            })
            .transpose()?;
    Ok(name.into_iter().filter(|name| !name.is_empty()).collect())
}

pub(crate) async fn log_targets(
    state: &SharedState,
    application_id: &str,
) -> Result<Vec<RuntimeLogTarget>, AppError> {
    let source = deployment::source::get(state, application_id).await?;
    let mut targets: Vec<RuntimeLogTarget> = if source.deploy_type == "compose" {
        deployment::compose_standalone::status(state, application_id)
            .await?
            .into_iter()
            .map(|service| RuntimeLogTarget {
                container_name: service.container_name,
                service: Some(service.service),
                status: service.status,
            })
            .collect()
    } else {
        let conn = crate::git::repositories::db::get_conn(state).await;
        let target = sqlx::query(
            "SELECT container_name, status FROM application_deployments WHERE application_id = ?1",
        )
        .bind(application_id)
        .fetch_optional(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?
        .map(|row| {
            Ok::<_, AppError>((
                row.try_get::<String, _>(0)
                    .map_err(|e| AppError::DatabaseError(e.to_string()))?,
                row.try_get::<String, _>(1)
                    .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            ))
        })
        .transpose()?;
        let mut resolved = Vec::new();
        if let Some((container_name, recorded_status)) = target {
            // The column only records what the last deploy intended. After a restart
            // the container may have outlived it in either direction, so Podman
            // decides; the recorded intent is the fallback for an unreachable server.
            let status = match ServerRuntime::for_application(state, application_id).await {
                Ok(runtime) => inspect_container_state(&runtime, &container_name).await,
                Err(_) => recorded_status,
            };
            resolved.push(RuntimeLogTarget {
                container_name,
                service: None,
                status,
            });
        }
        resolved
    };

    targets.sort_by(|left, right| {
        is_running_container(&right.status)
            .cmp(&is_running_container(&left.status))
            .then_with(|| left.service.cmp(&right.service))
            .then_with(|| left.container_name.cmp(&right.container_name))
    });
    Ok(targets)
}

async fn resolve_log_target(
    state: &SharedState,
    application_id: &str,
    requested_container: Option<&str>,
) -> Result<Option<RuntimeLogTarget>, AppError> {
    let targets = log_targets(state, application_id).await?;
    match requested_container {
        Some(container) => targets
            .into_iter()
            .find(|target| target.container_name == container)
            .map(Some)
            .ok_or_else(|| AppError::ResourceNotFound),
        None => Ok(targets.into_iter().next()),
    }
}

pub async fn list_log_targets_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppLogsRead>,
) -> Result<impl IntoResponse, AppError> {
    let application_id = app.application_id;
    Ok(ok(log_targets(&state, &application_id).await?))
}

/// Metrics for this application only. The target list is derived from the
/// application's runtime records, then Podman is reached through ServerRuntime
/// so an SSH deployment can never read the control-plane engine.
pub async fn container_metrics_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppRead>,
) -> Result<impl IntoResponse, AppError> {
    let application_id = app.application_id;
    let filters = container_names(&state, &application_id).await?;
    let runtime = ServerRuntime::for_application(&state, &application_id).await?;
    Ok(ok(
        crate::podman::podman_metrics::collect_runtime_container_metrics(&runtime, &filters)
            .await?,
    ))
}

pub async fn container_metrics_stream_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppRead>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let application_id = app.application_id;
    let stream = async_stream::stream! {
        let mut interval = tokio::time::interval(Duration::from_secs(8));
        loop {
            interval.tick().await;
            let filters = match container_names(&state, &application_id).await {
                Ok(filters) => filters,
                Err(error) => {
                    yield Ok(Event::default().event("metrics-error").data(serde_json::json!({ "message": error.to_string() }).to_string()));
                    continue;
                }
            };
            let snapshot = match ServerRuntime::for_application(&state, &application_id).await {
                Ok(runtime) => crate::podman::podman_metrics::collect_runtime_container_metrics(&runtime, &filters).await,
                Err(error) => Err(error),
            };
            match snapshot {
                Ok(snapshot) => match serde_json::to_string(&snapshot) {
                    Ok(payload) => yield Ok(Event::default().event("metrics").data(payload)),
                    Err(error) => yield Ok(Event::default().event("metrics-error").data(serde_json::json!({ "message": error.to_string() }).to_string())),
                },
                Err(error) => yield Ok(Event::default().event("metrics-error").data(serde_json::json!({ "message": error.to_string() }).to_string())),
            }
        }
    };
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

pub async fn stream_logs_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppLogsRead>,
    Query(query): Query<LogsQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let application_id = app.application_id;
    let target = resolve_log_target(&state, &application_id, query.container.as_deref()).await?;

    let tail = query.tail.unwrap_or(200).clamp(1, 1_000);

    let stream: Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send>> = match target {
        None => Box::pin(async_stream::stream! {
            yield Ok(Event::default().data("No application containers are available."));
        }),
        Some(target) if !is_running_container(&target.status) => Box::pin(async_stream::stream! {
            yield Ok(Event::default().data("Container is not running."));
        }),
        Some(target) => {
            let runtime = ServerRuntime::for_application(&state, &application_id).await?;
            let mut child = runtime
                .podman_stream(&logs_argv(&target.container_name, tail, true))
                .await?;

            // Podman writes the container's stdout and stderr to the matching pipes; both are
            // log output, so they are merged back into one channel. Podman's own failure text
            // (a missing container, for instance) arrives on stderr and reaches the log pane
            // as a line rather than being dropped into an empty stream.
            let (sender, mut receiver) = tokio::sync::mpsc::channel::<String>(256);
            if let Some(stdout) = child.take_stdout() {
                forward_lines(stdout, sender.clone());
            }
            if let Some(stderr) = child.take_stderr() {
                forward_lines(stderr, sender.clone());
            }
            drop(sender);

            Box::pin(async_stream::stream! {
                // The stream owns the child: when the client disconnects, the SSE response is
                // dropped, which kills the process and ends the remote `podman logs --follow`.
                let _child = child;
                let mut filter = deployment::log_filter::LogFilter::default();
                while let Some(line) = receiver.recv().await {
                    let (log_text, ts) = parse_log_line(&line);
                    let Some(log_text) = filter.filter(&log_text) else { continue; };
                    let clean_text = deployment::log_filter::strip_ansi_sequences(log_text);
                    let payload = serde_json::json!({ "text": deployment::log_filter::truncate_line(&clean_text), "ts": ts });
                    yield Ok(Event::default().data(payload.to_string()));
                }
            })
        }
    };

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

pub async fn fetch_logs_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppLogsRead>,
    Query(query): Query<LogsQuery>,
) -> Result<impl IntoResponse, AppError> {
    let application_id = app.application_id;
    let tail = query.tail.unwrap_or(200).clamp(1, 1_000);
    let fetched = crate::handler::containers::in_replica(
        &state,
        &application_id,
        query.server_id.as_deref(),
        async {
            let Some(target) =
                resolve_log_target(&state, &application_id, query.container.as_deref()).await?
            else {
                return Ok(None);
            };
            let runtime = ServerRuntime::for_application(&state, &application_id).await?;
            let output = runtime
                .podman(&logs_argv(&target.container_name, tail, false))
                .await
                .map_err(map_missing_container)?;
            Ok(Some((target, output)))
        },
    )
    .await?;
    let Some((target, output)) = fetched else {
        return Ok(ok(
            serde_json::json!({ "lines": [], "total": 0, "status": "not_deployed" }),
        ));
    };

    let mut lines = Vec::new();
    let mut filter = deployment::log_filter::LogFilter::default();
    // Podman splits the container's two streams across stdout and stderr; both are log output.
    for line in output.stdout.lines().chain(output.stderr.lines()) {
        let (log_text, ts) = parse_log_line(line);
        if let Some(log_text) = filter.filter(&log_text) {
            let clean_text = deployment::log_filter::strip_ansi_sequences(log_text);
            lines.push(
                serde_json::json!({ "text": deployment::log_filter::truncate_line(&clean_text), "ts": ts }),
            );
        }
    }

    let total = lines.len();
    Ok(ok(
        serde_json::json!({ "lines": lines, "total": total, "status": target.status }),
    ))
}

/// Real-time container state from Podman. Returns the actual Podman state
/// regardless of what the DB says — no DB writes.
pub async fn get_container_status_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppRead>,
) -> Result<impl IntoResponse, AppError> {
    let application_id = app.application_id;
    // Compose deploys store `application_deployments.container_name` as the bare
    // project prefix, not a real container — `container_names()` already resolves
    // the actual per-service name(s) for compose and falls back to the raw column
    // for everything else (see log_targets/container_metrics_handler).
    let container_name = container_names(&state, &application_id)
        .await?
        .into_iter()
        .next();

    let Some(cn) = container_name else {
        return Ok(ok(ContainerStatusDto {
            status: "not_deployed".into(),
            label: "Not deployed".into(),
            container_name: None,
            internal_host: None,
            internal_port: None,
        }));
    };
    let internal_host = deployment::identity(&state, &application_id)
        .await
        .ok()
        .map(|(name, _)| name);
    let internal_port = deployment::runtime_endpoints::get(&state, &application_id)
        .await
        .ok()
        .flatten()
        .map(|endpoint| endpoint.container_port);

    // "We could not ask" is a different fact from "Podman does not know", and only the
    // first one means the container may well still be up on an unreachable server.
    let Ok(runtime) = ServerRuntime::for_application(&state, &application_id).await else {
        return Ok(ok(ContainerStatusDto {
            status: "unavailable".into(),
            label: "Unavailable".into(),
            container_name: Some(cn),
            internal_host,
            internal_port,
        }));
    };

    let status = inspect_container_state(&runtime, &cn).await;
    Ok(ok(ContainerStatusDto {
        label: container_status_label(&status).into(),
        status,
        container_name: Some(cn),
        internal_host,
        internal_port,
    }))
}

/// Live Podman state for one container. `not_found` means Podman answered and the
/// container does not exist; callers decide what an unreachable runtime means.
pub(crate) async fn inspect_container_state(
    runtime: &ServerRuntime,
    container_name: &str,
) -> String {
    let inspect = runtime
        .podman(&[
            OsString::from("inspect"),
            OsString::from("--format"),
            OsString::from("{{.State.Status}}"),
            OsString::from(container_name),
        ])
        .await;
    match inspect {
        Ok(output) => match output.stdout.trim() {
            "" => "unknown".to_string(),
            status => status.to_string(),
        },
        Err(_) => "not_found".to_string(),
    }
}

/// The human label for a raw Podman state; single source of truth for the UI badge.
pub(crate) fn container_status_label(status: &str) -> &'static str {
    match status {
        "running" => "Running",
        "paused" => "Paused",
        "exited" | "stopped" | "removing" | "dead" | "configured" => "Stopped",
        "created" => "Created",
        "not_found" => "Not deployed",
        "unavailable" => "Unavailable",
        _ => "Unknown",
    }
}

pub async fn start_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppDeploy>,
) -> Result<impl IntoResponse, AppError> {
    let AppScope {
        organization_id,
        application_id,
        application_name,
        user,
        ..
    } = app;
    let result = deployment::start(&state, &application_id, Some(&user.user_id)).await?;
    audit_log::record(
        &state,
        &organization_id,
        &user,
        "deployment.started",
        Some("application"),
        Some(&application_id),
        Some(&application_name),
        None,
        None,
        None,
        None,
    )
    .await
    .ok();
    Ok(ok(result))
}

pub async fn remove_runtime_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppDeploy>,
) -> Result<impl IntoResponse, AppError> {
    let AppScope {
        application_id,
        user,
        ..
    } = app;
    Ok(ok(deployment::remove_runtime(
        &state,
        &application_id,
        Some(&user.user_id),
    )
    .await?))
}

/// Live CPU/memory usage for the container backing this application. Best-effort:
/// a stopped app or an unreachable Podman socket returns `status` with the usage
/// fields left `None` instead of failing the request, since this backs a dashboard
/// widget rather than a control action.
pub async fn get_stats_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppRead>,
) -> Result<impl IntoResponse, AppError> {
    let application_id = app.application_id;
    let empty = ApplicationStatsDto {
        status: "stopped".into(),
        cpu_percent: None,
        mem_usage_bytes: None,
        mem_limit_bytes: None,
    };

    // Compose deploys store `application_deployments.container_name` as the bare
    // project prefix, not a real container — `container_names()` already resolves
    // the actual per-service name(s) for compose and falls back to the raw column
    // for everything else (see log_targets/container_metrics_handler).
    let Some(container_name) = container_names(&state, &application_id)
        .await?
        .into_iter()
        .next()
    else {
        return Ok(ok(empty));
    };

    let Ok(runtime) = ServerRuntime::for_application(&state, &application_id).await else {
        return Ok(ok(empty));
    };

    // `podman stats` fails on a container that is missing or not running, so its failure is the
    // liveness check and no separate inspect round trip is needed. Reuse the tolerant parser
    // shared with the system metrics endpoint because Podman versions vary between numeric and
    // human-formatted JSON fields.
    let Ok(output) = runtime
        .podman(&[
            OsString::from("stats"),
            OsString::from("--no-stream"),
            OsString::from("--format"),
            OsString::from("json"),
            OsString::from(&container_name),
        ])
        .await
    else {
        return Ok(ok(empty));
    };

    let metric = serde_json::from_str(&output.stdout).ok().and_then(|value| {
        crate::podman::podman_metrics::normalize_stats(
            value,
            &[crate::podman::podman_metrics::ContainerIdentity::running(
                &container_name,
                &container_name,
            )],
            chrono::Utc::now(),
        )
        .metrics
        .into_iter()
        .next()
    });

    Ok(ok(match metric {
        Some(metric) => ApplicationStatsDto {
            status: "running".into(),
            cpu_percent: metric.cpu_percent,
            mem_usage_bytes: metric.memory_usage_bytes,
            mem_limit_bytes: metric.memory_limit_bytes,
        },
        None => ApplicationStatsDto {
            status: "running".into(),
            ..empty
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::{AppError, StreamAccessCache, logs_argv, missing_podman_resource, parse_log_line};
    use std::time::{Duration, Instant};

    #[test]
    fn recognizes_every_wording_podman_uses_for_a_resource_it_cannot_find() {
        for message in [
            r#"SSH command failed: Error: no such object: "static-prod-statcoc-a24343""#,
            "no such container",
            "Error: no such network litepod-net",
        ] {
            assert!(missing_podman_resource(&AppError::ValidationError(
                message.into()
            )));
        }
        assert!(!missing_podman_resource(&AppError::ValidationError(
            "SSH command failed: permission denied".into()
        )));
    }

    #[test]
    fn splits_the_timestamp_podman_prepends_and_tolerates_lines_without_one() {
        let (text, ts) = parse_log_line("2026-08-07T10:11:12.123456789Z listening on :3000");
        assert_eq!(text, "listening on :3000");
        assert_eq!(ts.as_deref(), Some("2026-08-07T10:11:12.123456789Z"));

        // Podman's own failure text reaches the log pane unstamped and must survive intact.
        let (text, ts) = parse_log_line("Error: no such container");
        assert_eq!(text, "Error: no such container");
        assert_eq!(ts, None);
    }

    #[test]
    fn follow_only_adds_the_follow_flag_to_a_shared_argv() {
        let snapshot = logs_argv("app-prod-web-abc123", 200, false);
        let follow = logs_argv("app-prod-web-abc123", 200, true);

        assert_eq!(snapshot[0], "logs");
        assert!(snapshot.iter().any(|arg| arg == "--timestamps"));
        assert!(snapshot.iter().any(|arg| arg == "200"));
        assert_eq!(snapshot.last().unwrap(), "app-prod-web-abc123");
        assert!(!snapshot.iter().any(|arg| arg == "--follow"));

        assert_eq!(follow.len(), snapshot.len() + 1);
        assert!(follow.iter().any(|arg| arg == "--follow"));
    }

    #[test]
    fn returns_a_cached_decision_until_the_ttl_expires() {
        let now = Instant::now();
        let mut cache = StreamAccessCache::new(Duration::from_secs(30));
        cache.insert("job-1", true, now);

        assert_eq!(
            cache.get("job-1", now + Duration::from_secs(29)),
            Some(true)
        );
        assert_eq!(cache.get("job-1", now + Duration::from_secs(31)), None);
    }

    #[test]
    fn caches_denials_too_and_ignores_unknown_jobs() {
        let now = Instant::now();
        let mut cache = StreamAccessCache::new(Duration::from_secs(30));
        cache.insert("job-1", false, now);

        assert_eq!(cache.get("job-1", now), Some(false));
        assert_eq!(cache.get("job-2", now), None);
    }
}

#[cfg(test)]
mod certificate_assignment_tests {
    use super::*;
    #[test]
    fn only_admin_can_assign_another_inventory_certificate() {
        let request = crate::deployment::certificates::CertificateRequest {
            certificate_id: Some("company".into()),
            ..Default::default()
        };
        assert!(authorize_certificate_assignment(false, &request, None).is_err());
        assert!(authorize_certificate_assignment(false, &request, Some("other")).is_err());
        assert!(authorize_certificate_assignment(false, &request, Some("company")).is_ok());
        assert!(authorize_certificate_assignment(true, &request, None).is_ok());
    }
}
