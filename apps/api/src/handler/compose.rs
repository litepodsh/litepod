use axum::{
    Json,
    extract::{Path, State},
};
use serde::{Deserialize, Serialize};
use tokio::process::Command;

use crate::{
    deployment::{
        compose::{
            self,
            catalog::{ComposeCatalog, catalog_from_yaml},
            store::{ComposeFileStore, PortMapping},
        },
        compose_standalone,
    },
    dtos::{
        deployment::RedeployRequest,
        response::{AppError, ok},
    },
    git::repositories::audit_log,
    git::repositories::project as project_repo,
    security::authz::extract::{RequireApp, perm},
    security::middleware::org_scope::AppScope,
    state::state::SharedState,
};

fn is_service_running(status: &str) -> bool {
    let normalized = status.to_ascii_lowercase();
    normalized == "running" || normalized == "up" || normalized.starts_with("running ")
}

#[derive(Debug, Deserialize)]
pub struct ComposeContentRequest {
    pub content: String,
    pub port_mappings: Vec<PortMapping>,
}

#[derive(Debug, Deserialize)]
pub struct ComposeValidationRequest {
    pub content: String,
}

#[derive(Debug, Serialize)]
pub struct ComposeContentResponse {
    pub content: String,
    pub port_mappings: Vec<PortMapping>,
    pub catalog: Option<ComposeCatalog>,
}

#[derive(Debug, Serialize)]
pub struct ValidationError {
    pub line: Option<usize>,
    pub column: Option<usize>,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct ValidationResponse {
    pub valid: bool,
    pub errors: Vec<ValidationError>,
    pub catalog: Option<ComposeCatalog>,
}

#[derive(Debug, Deserialize)]
pub struct ComposeProxyRouteRequest {
    pub domain_id: String,
    pub service: String,
    pub container_port: u16,
}

pub async fn compose_content_get_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppRead>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let store = ComposeFileStore::new(&state.data_dir, &app.application_id);
    let content = store
        .read_content()
        .await
        .map_err(|e| AppError::InternalError(e))?;
    let port_mappings = store
        .read_port_mappings()
        .await
        .map_err(|e| AppError::InternalError(e))?;
    let catalog = (!content.trim().is_empty())
        .then(|| catalog_from_yaml(&content, false))
        .transpose()?;
    Ok(ok(ComposeContentResponse {
        content,
        port_mappings,
        catalog,
    }))
}

pub async fn compose_content_put_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppUpdate>,
    Json(request): Json<ComposeContentRequest>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let application_id = app.application_id;
    if !request.content.trim().is_empty() {
        serde_yaml::from_str::<serde_yaml::Value>(&request.content)
            .map_err(|e| AppError::ValidationError(format!("Invalid YAML: {e}")))?;
        // Refused at save time so a reserved name never reaches a deploy, where it
        // would shadow a control-plane hostname on the shared network.
        let reserved = compose::reserved_services_in_compose(&request.content);
        if !reserved.is_empty() {
            return Err(AppError::ValidationError(
                compose::reserved_service_message(&reserved),
            ));
        }
        // Host paths, binds and anonymous volumes are refused for everyone on
        // the shared server; they cannot be written on rootless Podman and a
        // host path or the Podman socket would let a tenant reach the host.
        let mounts = compose::unnamed_mounts_in_compose(&request.content);
        if !mounts.is_empty() {
            return Err(AppError::ValidationError(
                compose::unnamed_mounts_message(&mounts),
            ));
        }
        // `privileged: true` escapes the user namespace, so only the instance
        // SysAdmin may deploy one; an Editor in any organization is refused.
        if !app.user.is_sysadmin {
            let privileged = compose::privileged_services_in_compose(&request.content);
            if !privileged.is_empty() {
                return Err(AppError::ValidationError(
                    compose::privileged_services_message(&privileged),
                ));
            }
        }
    }
    let store = ComposeFileStore::new(&state.data_dir, &application_id);
    store
        .write_content(&request.content)
        .await
        .map_err(|e| AppError::InternalError(e))?;
    // `write_port_mappings` itself validates protocol and the reserved
    // 42000-42999 range - both are user-fixable input mistakes, not server
    // failures, so they're reported as a `ValidationError`.
    store
        .write_port_mappings(&request.port_mappings)
        .await
        .map_err(AppError::ValidationError)?;
    audit_log::record(
        &state,
        &app.organization_id,
        &app.user,
        "compose.updated",
        Some("application"),
        Some(&application_id),
        Some(&app.application_name),
        None,
        None,
        None,
        None,
    )
    .await
    .ok();
    let content = store
        .read_content()
        .await
        .map_err(|e| AppError::InternalError(e))?;
    let port_mappings = store
        .read_port_mappings()
        .await
        .map_err(|e| AppError::InternalError(e))?;
    let catalog = (!content.trim().is_empty())
        .then(|| catalog_from_yaml(&content, false))
        .transpose()?;
    Ok(ok(ComposeContentResponse {
        content,
        port_mappings,
        catalog,
    }))
}

pub async fn compose_validate_handler(
    // Only read from the `target_os = "linux"` block that wires up the podman
    // socket; unused on a macOS dev box.
    #[cfg_attr(not(target_os = "linux"), allow(unused_variables))] State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppUpdate>,
    Json(request): Json<ComposeValidationRequest>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let application_id = app.application_id;
    let content = request.content;
    if content.trim().is_empty() {
        return Ok(ok(ValidationResponse {
            valid: false,
            errors: vec![ValidationError {
                line: None,
                column: None,
                message: "Compose file is empty".into(),
            }],
            catalog: None,
        }));
    }

    // Reported before `podman compose config` runs: the provider is perfectly
    // happy with these names, but the deploy would hijack a control-plane
    // hostname on the shared network.
    let reserved = compose::reserved_services_in_compose(&content);
    if !reserved.is_empty() {
        return Ok(ok(ValidationResponse {
            valid: false,
            errors: vec![ValidationError {
                line: None,
                column: None,
                message: compose::reserved_service_message(&reserved),
            }],
            catalog: None,
        }));
    }

    // Same host-escape guards as the save path: no host paths / binds /
    // anonymous volumes, and `privileged` only for the instance SysAdmin.
    let mounts = compose::unnamed_mounts_in_compose(&content);
    if !mounts.is_empty() {
        return Ok(ok(ValidationResponse {
            valid: false,
            errors: vec![ValidationError {
                line: None,
                column: None,
                message: compose::unnamed_mounts_message(&mounts),
            }],
            catalog: None,
        }));
    }
    if !app.user.is_sysadmin {
        let privileged = compose::privileged_services_in_compose(&content);
        if !privileged.is_empty() {
            return Ok(ok(ValidationResponse {
                valid: false,
                errors: vec![ValidationError {
                    line: None,
                    column: None,
                    message: compose::privileged_services_message(&privileged),
                }],
                catalog: None,
            }));
        }
    }

    let tmp_dir = std::env::temp_dir().join(format!(
        "litepod-validate-{application_id}-{}",
        uuid::Uuid::now_v7()
    ));
    let _ = tokio::fs::create_dir_all(&tmp_dir).await;
    let tmp_file = tmp_dir.join("docker-compose.yml");
    tokio::fs::write(&tmp_file, &content)
        .await
        .map_err(|e| AppError::InternalError(e.to_string()))?;

    let mut command = Command::new("podman");
    #[cfg(target_os = "linux")]
    {
        command.arg("--url").arg(&state.podman_socket);
        // The provider spawns its own `podman` children; they only find the
        // mounted socket through `CONTAINER_HOST`.
        command.env("CONTAINER_HOST", &state.podman_socket);
    }
    let output = command
        .arg("compose")
        .arg("--project-name")
        .arg(format!("litepod-validate-{application_id}"))
        .arg("--file")
        .arg(&tmp_file)
        .arg("config")
        .output()
        .await
        .map_err(|e| {
            AppError::InternalError(format!("Could not run podman compose config: {e}"))
        })?;

    let _ = tokio::fs::remove_dir_all(&tmp_dir).await;

    if output.status.success() {
        let catalog = catalog_from_yaml(&String::from_utf8_lossy(&output.stdout), false).ok();
        Ok(ok(ValidationResponse {
            valid: true,
            errors: vec![],
            catalog,
        }))
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let errors: Vec<ValidationError> = stderr
            .lines()
            .filter(|l| !l.is_empty())
            .map(|line| {
                let (line_num, col_num, msg) = if let Some(pos) = line.find("line ") {
                    let rest = &line[pos + 5..];
                    if let Some(colon) = rest.find(':') {
                        let line_part = rest[..colon].trim();
                        let rest2 = &rest[colon + 1..];
                        let (col, message) = if let Some(comma) = rest2.find(',') {
                            (
                                rest2[..comma].trim().parse::<usize>().ok(),
                                rest2[comma + 1..].trim(),
                            )
                        } else {
                            (None, rest2.trim())
                        };
                        (line_part.parse::<usize>().ok(), col, message.to_string())
                    } else {
                        (None, None, line.to_string())
                    }
                } else {
                    (None, None, line.to_string())
                };
                ValidationError {
                    line: line_num,
                    column: col_num,
                    message: msg,
                }
            })
            .collect();
        Ok(ok(ValidationResponse {
            valid: false,
            errors,
            catalog: None,
        }))
    }
}

pub async fn compose_deploy_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppDeploy>,
    body: Option<Json<RedeployRequest>>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let AppScope {
        organization_id,
        project_id,
        environment_id: env_id,
        application_id,
        application_name,
        user,
        ..
    } = app;
    // Live status is the only ground truth for "is this a redeploy": the cached
    // DB status lags and would let a stopped-then-restarted app bypass the gate.
    let services = compose_standalone::status(&state, &application_id).await?;
    let any_running = services.iter().any(|s| is_service_running(&s.status));
    if any_running {
        let application = project_repo::get_application(
            &state,
            &organization_id,
            &project_id,
            &env_id,
            &application_id,
        )
        .await
        .map_err(AppError::DatabaseError)?
        .ok_or(AppError::ApplicationNotFound)?;
        let provided = body
            .as_ref()
            .and_then(|b| b.confirmation.as_deref())
            .unwrap_or("");
        // The gate is a speed bump, not an input test: a pasted name carrying
        // stray whitespace or different casing still proves intent.
        if !provided
            .trim()
            .eq_ignore_ascii_case(application.name.trim())
        {
            return Err(AppError::ValidationError(format!(
                "Redeploy requires typing the application name ({}) to confirm.",
                application.name
            )));
        }
    }
    // The queued job carries the run ID the dashboard needs to stream the deploy.
    let result = compose_standalone::enqueue(
        &state,
        &application_id,
        "manual_deploy",
        Some(&user.user_id),
    )
    .await?;
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
    Ok(ok(result))
}

pub async fn compose_stop_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppDeploy>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let result =
        compose_standalone::stop(&state, &app.application_id, Some(&app.user.user_id)).await?;
    audit_log::record(
        &state,
        &app.organization_id,
        &app.user,
        "deployment.stopped",
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
    Ok(ok(result))
}

pub async fn compose_start_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppDeploy>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let result =
        compose_standalone::start(&state, &app.application_id, Some(&app.user.user_id)).await?;
    audit_log::record(
        &state,
        &app.organization_id,
        &app.user,
        "deployment.started",
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
    Ok(ok(result))
}

pub async fn compose_status_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppRead>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let result = compose_standalone::status(&state, &app.application_id).await?;
    Ok(ok(result))
}

pub async fn compose_proxy_routes_list_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppRead>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    Ok(ok(crate::deployment::compose::proxy_routes::list(
        &state,
        &app.application_id,
    )
    .await?))
}

pub async fn compose_proxy_routes_upsert_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppUpdate>,
    Json(request): Json<ComposeProxyRouteRequest>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let application_id = app.application_id;
    let content = ComposeFileStore::new(&state.data_dir, &application_id)
        .read_content()
        .await
        .map_err(AppError::InternalError)?;
    let catalog = catalog_from_yaml(&content, false)?;
    if !catalog
        .services
        .iter()
        .any(|service| service.name == request.service)
    {
        return Err(AppError::ValidationError(format!(
            "Compose service '{}' was not found",
            request.service
        )));
    }
    Ok(ok(crate::deployment::compose::proxy_routes::save(
        &state,
        &application_id,
        &request.domain_id,
        &request.service,
        request.container_port,
    )
    .await?))
}

pub async fn compose_proxy_routes_delete_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppUpdate>,
    // The extractor covers the first four segments; only `domain_id` is left.
    Path((_, _, _, _, domain_id)): Path<(String, String, String, String, String)>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    crate::deployment::compose::proxy_routes::remove(&state, &app.application_id, &domain_id)
        .await?;
    Ok(ok(()))
}
