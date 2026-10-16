use super::runner;
use std::str::FromStr;

use chrono::{Local, Utc};
use croner::Cron;
use uuid::Uuid;

use crate::{
    dtos::{
        response::AppError,
        scheduled_task::{
            CreateScheduledTaskRequest, ScheduledTaskDetailDto, ScheduledTaskDto,
            ScheduledTaskExecutionDetailDto, ScheduledTaskExecutionDto,
            ScheduledTaskExecutionLogDto, UpdateScheduledTaskRequest,
        },
    },
    security::middleware::auth::AuthUser,
    state::state::SharedState,
};

const MAX_TIMEOUT_SECONDS: i64 = 86_400;
const TASK_COLUMNS: &str = "id, organization_id, name, description, cron_expression, runner, image, shell, timeout_seconds, network_access, enabled, server_id, next_run_at, active_execution_id, created_at, updated_at, application_id, compose_service";
const EXECUTION_COLUMNS: &str = "id, task_id, trigger, status, runner, image, network_access, server_id, queued_at, started_at, finished_at, exit_code, error, logs_truncated";
pub const LOCAL_SERVER_ID: &str = "local";

/// Resolves a request's `server_id` against the current control plane:
/// - rejects empty / unknown servers,
/// - rejects non-ready servers,
/// - rejects `kind='local'` in cloud mode (the control plane is not
///   authorized to schedule workloads on itself),
/// - rejects a non-remote server for the `remote_shell` runner (it runs the
///   script directly on the remote host, so it has no local placement),
/// - in self-hosted mode defaults the value to `LOCAL_SERVER_ID` when
///   the client omitted it.
pub async fn resolve_server_id(
    state: &SharedState,
    organization_id: &str,
    requested: Option<&str>,
    runner: Option<&str>,
) -> Result<String, AppError> {
    let resolved = requested
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(LOCAL_SERVER_ID);
    if state.is_cloud && resolved == LOCAL_SERVER_ID {
        return Err(AppError::ValidationError(
            "Cloud mode requires a ready remote server for scheduled tasks".into(),
        ));
    }
    // Scoped lookup: a server owned by another organization reads as unknown,
    // so a task cannot be pinned onto another tenant's host.
    let server = crate::servers::repository::get_for_organization(state, resolved, organization_id)
        .await?
        .ok_or_else(|| AppError::ValidationError(format!("Server '{resolved}' was not found")))?;
    if server.state != crate::servers::model::ServerState::Ready {
        return Err(AppError::ValidationError(format!(
            "Server '{}' is not ready",
            server.name
        )));
    }
    if state.is_cloud && server.kind != "ssh" {
        return Err(AppError::ValidationError(
            "Cloud mode requires a ready remote server for scheduled tasks".into(),
        ));
    }
    if runner == Some("remote_shell") && server.kind != "ssh" {
        return Err(AppError::ValidationError(
            "The remote_shell runner requires a remote server".into(),
        ));
    }
    Ok(server.id)
}

/// Reject `litepod_api` runner when the control plane is in cloud mode:
/// the runner executes the script inside the API process, not on a server,
/// so it has no business running in a control plane that is forbidden to
/// run workloads locally.
pub fn validate_runner_for_cloud(state: &SharedState, runner: &str) -> Result<(), AppError> {
    if state.is_cloud && runner == "litepod_api" {
        return Err(AppError::ValidationError(
            "The litepod_api runner is not available in cloud mode".into(),
        ));
    }
    Ok(())
}

/// Boot-time guard: refuse to start the control plane in cloud mode while
/// any scheduled task still targets a non-remote server. Mirrors the
/// applications-on-local check in `servers::repository`.
pub async fn assert_no_local_scheduled_tasks(state: &SharedState) -> Result<(), AppError> {
    use sqlx::Row;
    if !state.is_cloud {
        return Ok(());
    }
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(
        "SELECT t.id, t.name, t.server_id
         FROM scheduled_tasks t
         JOIN servers s ON s.id = t.server_id
         WHERE s.kind != 'ssh'
         LIMIT 1",
    )
    .fetch_optional(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    let Some(row) = row else {
        return Ok(());
    };
    let id: String = row
        .try_get(0)
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    let name: String = row
        .try_get(1)
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    let server_id: String = row
        .try_get(2)
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    Err(AppError::ValidationError(format!(
        "IS_CLOUD=true cannot start while scheduled task '{name}' ({id}) is assigned to non-remote server '{server_id}'"
    )))
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

// Scheduled tasks run code on a host, so they are an admin-level capability
// inside the organization. That gate now lives in the route signature: every
// handler in `handler::scheduled_tasks` takes `RequireOrg<perm::ScheduledTaskManage>`,
// which cannot be skipped, so the old `ensure_manage_access` helper is gone.

fn read_execution(row: &sqlx::sqlite::SqliteRow) -> Result<ScheduledTaskExecutionDto, AppError> {
    use sqlx::Row;
    Ok(ScheduledTaskExecutionDto {
        id: row
            .try_get(0)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        task_id: row
            .try_get(1)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        trigger: row
            .try_get(2)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        status: row
            .try_get(3)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        runner: row
            .try_get(4)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        image: row
            .try_get(5)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        network_access: row
            .try_get::<i64, _>(6)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?
            != 0,
        server_id: row
            .try_get(7)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        queued_at: row
            .try_get(8)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        started_at: row
            .try_get(9)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        finished_at: row
            .try_get(10)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        exit_code: row
            .try_get(11)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        error: row
            .try_get(12)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        logs_truncated: row
            .try_get::<i64, _>(13)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?
            != 0,
    })
}

pub fn validate_request(
    req: &CreateScheduledTaskRequest,
    is_sysadmin: bool,
) -> Result<(), AppError> {
    if req.name.trim().is_empty() || req.name.len() > 120 {
        return Err(AppError::ValidationError(
            "Name must be between 1 and 120 characters".into(),
        ));
    }
    if req.script.trim().is_empty() {
        return Err(AppError::ValidationError("Script is required".into()));
    }
    if !matches!(req.shell.as_str(), "bash" | "sh" | "fish" | "nu" | "zsh") {
        return Err(AppError::ValidationError(
            "Shell must be bash, sh, fish, nu, or zsh".into(),
        ));
    }
    if !(1..=MAX_TIMEOUT_SECONDS).contains(&req.timeout_seconds) {
        return Err(AppError::ValidationError(
            "Timeout must be between 1 second and 24 hours".into(),
        ));
    }
    Cron::from_str(&req.cron_expression).map_err(|_| {
        AppError::ValidationError("Cron must be a valid five-field expression".into())
    })?;
    match req.runner.as_str() {
        "ephemeral"
            if req
                .image
                .as_deref()
                .is_some_and(|image| !image.trim().is_empty()) =>
        {
            Ok(())
        }
        "ephemeral" => Err(AppError::ValidationError(
            "An image is required for ephemeral jobs".into(),
        )),
        "remote_shell"
            if req
                .image
                .as_deref()
                .is_none_or(|image| image.trim().is_empty()) =>
        {
            Ok(())
        }
        "remote_shell" => Err(AppError::ValidationError(
            "remote_shell jobs cannot specify an image".into(),
        )),
        "litepod_api" if !is_sysadmin => Err(AppError::InsufficientPermissions),
        "litepod_api"
            if req
                .image
                .as_deref()
                .is_none_or(|image| image.trim().is_empty()) =>
        {
            Ok(())
        }
        "litepod_api" => Err(AppError::ValidationError(
            "litepod API jobs cannot specify an image".into(),
        )),
        "application_exec"
            if req
                .image
                .as_deref()
                .is_some_and(|image| !image.trim().is_empty()) =>
        {
            Err(AppError::ValidationError(
                "application_exec jobs cannot specify an image".into(),
            ))
        }
        "application_exec"
            if req
                .application_id
                .as_deref()
                .is_none_or(|value| value.trim().is_empty()) =>
        {
            Err(AppError::ValidationError(
                "An application is required for application_exec jobs".into(),
            ))
        }
        "application_exec" => Ok(()),
        _ => Err(AppError::ValidationError(
            "Runner must be ephemeral, remote_shell, litepod_api, or application_exec".into(),
        )),
    }
}

/// Confirms `application_id` really hangs off `organization_id`, the same
/// tenancy guarantee `handler::common::check_application` gives routes that
/// already know the project/environment in the URL. A scheduled task only
/// carries the application id, so this re-derives the chain with a join
/// instead of requiring the caller to look the rest up first.
async fn application_belongs_to_organization(
    state: &SharedState,
    organization_id: &str,
    application_id: &str,
) -> Result<bool, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(
        "SELECT 1 FROM applications a \
         INNER JOIN environments e ON e.id = a.environment_id \
         INNER JOIN projects p ON p.id = e.project_id \
         WHERE a.id = ?1 AND p.organization_id = ?2",
    )
    .bind(application_id)
    .bind(organization_id)
    .fetch_optional(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    Ok(row.is_some())
}

pub fn next_run_at(expression: &str) -> Result<i64, AppError> {
    let cron = Cron::from_str(expression).map_err(|_| {
        AppError::ValidationError("Cron must be a valid five-field expression".into())
    })?;
    cron.find_next_occurrence(&Local::now(), false)
        .map(|next| next.with_timezone(&Utc).timestamp_millis())
        .map_err(|_| AppError::ValidationError("Cron has no future occurrence".into()))
}

fn read_task(row: &sqlx::sqlite::SqliteRow) -> Result<ScheduledTaskDto, AppError> {
    use sqlx::Row;
    Ok(ScheduledTaskDto {
        id: row
            .try_get(0)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        organization_id: row
            .try_get(1)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        name: row
            .try_get(2)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        description: row
            .try_get(3)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        cron_expression: row
            .try_get(4)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        runner: row
            .try_get(5)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        image: row
            .try_get(6)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        shell: row
            .try_get(7)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        timeout_seconds: row
            .try_get(8)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        network_access: row
            .try_get::<i64, _>(9)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?
            != 0,
        enabled: row
            .try_get::<i64, _>(10)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?
            != 0,
        server_id: row
            .try_get(11)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        next_run_at: row
            .try_get(12)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        active_execution_id: row
            .try_get(13)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        created_at: row
            .try_get(14)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        updated_at: row
            .try_get(15)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        application_id: row
            .try_get(16)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        compose_service: row
            .try_get(17)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
    })
}

pub async fn get(
    state: &SharedState,
    organization_id: &str,
    task_id: &str,
) -> Result<ScheduledTaskDto, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(&format!(
        "SELECT {TASK_COLUMNS} FROM scheduled_tasks WHERE id = ?1 AND organization_id = ?2"
    ))
    .bind(task_id)
    .bind(organization_id)
    .fetch_optional(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    row.map(|row| read_task(&row))
        .transpose()?
        .ok_or(AppError::ResourceNotFound)
}

pub async fn list(
    state: &SharedState,
    organization_id: &str,
) -> Result<Vec<ScheduledTaskDto>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(&format!(
        "SELECT {TASK_COLUMNS} FROM scheduled_tasks WHERE organization_id = ?1 ORDER BY created_at DESC"
    ))
    .bind(organization_id)
    .fetch_all(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let mut tasks = Vec::new();
    for row in &rows {
        tasks.push(read_task(row)?);
    }
    Ok(tasks)
}

pub async fn create(
    state: &SharedState,
    organization_id: &str,
    user: &AuthUser,
    req: &CreateScheduledTaskRequest,
) -> Result<ScheduledTaskDto, AppError> {
    // Authorization is the caller's `RequireOrg<perm::ScheduledTaskManage>` extractor; `user` is only read
    // here for attribution and the sysadmin-only runner options below.
    let user_id = user.user_id.as_str();
    let is_sysadmin = user.is_sysadmin;
    validate_request(req, is_sysadmin)?;
    validate_runner_for_cloud(state, &req.runner)?;
    if req.runner == "application_exec" {
        let application_id = req.application_id.as_deref().unwrap_or_default();
        if !application_belongs_to_organization(state, organization_id, application_id).await? {
            return Err(AppError::ApplicationNotFound);
        }
    }
    let server_id = resolve_server_id(
        state,
        organization_id,
        req.server_id.as_deref(),
        Some(&req.runner),
    )
    .await?;
    let id = Uuid::now_v7().to_string();
    let now = now_ms();
    let next_run = next_run_at(&req.cron_expression)?;
    let image =
        (req.runner == "ephemeral").then(|| req.image.as_deref().unwrap_or("").trim().to_string());
    let application_id = (req.runner == "application_exec")
        .then(|| req.application_id.clone())
        .flatten();
    let compose_service = (req.runner == "application_exec")
        .then(|| req.compose_service.clone())
        .flatten()
        .filter(|value| !value.trim().is_empty());
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("INSERT INTO scheduled_tasks (id, organization_id, name, description, cron_expression, runner, image, shell, script, timeout_seconds, network_access, server_id, next_run_at, created_by, created_at, updated_at, application_id, compose_service) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)")
        .bind(id.clone())
        .bind(organization_id)
        .bind(req.name.trim())
        .bind(req.description.trim())
        .bind(req.cron_expression.trim())
        .bind(req.runner.clone())
        .bind(image)
        .bind(req.shell.clone())
        .bind(req.script.clone())
        .bind(req.timeout_seconds)
        .bind(i64::from(req.network_access))
        .bind(server_id)
        .bind(next_run)
        .bind(user_id)
        .bind(now)
        .bind(now)
        .bind(application_id)
        .bind(compose_service)
        .execute(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    get(state, organization_id, &id).await
}

/// Validates the (possibly unsaved) form for a test run. Validation and tenancy checks
/// match `create`, except that name and cron may still be blank.
pub async fn test(
    state: &SharedState,
    organization_id: &str,
    user: &AuthUser,
    req: &CreateScheduledTaskRequest,
) -> Result<runner::TestParams, AppError> {
    let mut checked = req.clone();
    if checked.name.trim().is_empty() {
        checked.name = "test".into();
    }
    if checked.cron_expression.trim().is_empty() {
        checked.cron_expression = "* * * * *".into();
    }
    validate_request(&checked, user.is_sysadmin)?;
    validate_runner_for_cloud(state, &req.runner)?;
    if req.runner == "application_exec" {
        let application_id = req.application_id.as_deref().unwrap_or_default();
        if !application_belongs_to_organization(state, organization_id, application_id).await? {
            return Err(AppError::ApplicationNotFound);
        }
    }
    let server_id = resolve_server_id(
        state,
        organization_id,
        req.server_id.as_deref(),
        Some(&req.runner),
    )
    .await?;
    let compose_service = req
        .compose_service
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    Ok(runner::TestParams {
        runner: req.runner.clone(),
        server_id,
        image: req.image.clone(),
        network_access: req.network_access,
        application_id: req.application_id.clone(),
        compose_service,
        shell: req.shell.clone(),
        script: req.script.clone(),
        timeout_seconds: req.timeout_seconds,
    })
}

pub async fn get_detail(
    state: &SharedState,
    organization_id: &str,
    task_id: &str,
) -> Result<ScheduledTaskDetailDto, AppError> {
    use sqlx::Row;
    let task = get(state, organization_id, task_id).await?;
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row =
        sqlx::query("SELECT script FROM scheduled_tasks WHERE id = ?1 AND organization_id = ?2")
            .bind(task_id)
            .bind(organization_id)
            .fetch_optional(&conn)
            .await
            .map_err(|error| AppError::DatabaseError(error.to_string()))?
            .ok_or(AppError::ResourceNotFound)?;
    Ok(ScheduledTaskDetailDto {
        task,
        script: row
            .try_get(0)
            .map_err(|error| AppError::DatabaseError(error.to_string()))?,
    })
}

pub async fn update(
    state: &SharedState,
    organization_id: &str,
    task_id: &str,
    is_sysadmin: bool,
    req: &UpdateScheduledTaskRequest,
) -> Result<ScheduledTaskDto, AppError> {
    get(state, organization_id, task_id).await?;
    validate_request(
        &CreateScheduledTaskRequest {
            name: req.name.clone(),
            description: req.description.clone(),
            cron_expression: req.cron_expression.clone(),
            runner: req.runner.clone(),
            image: req.image.clone(),
            shell: req.shell.clone(),
            script: req.script.clone(),
            timeout_seconds: req.timeout_seconds,
            network_access: req.network_access,
            server_id: req.server_id.clone(),
            application_id: req.application_id.clone(),
            compose_service: req.compose_service.clone(),
        },
        is_sysadmin,
    )?;
    validate_runner_for_cloud(state, &req.runner)?;
    if req.runner == "application_exec" {
        let application_id = req.application_id.as_deref().unwrap_or_default();
        if !application_belongs_to_organization(state, organization_id, application_id).await? {
            return Err(AppError::ApplicationNotFound);
        }
    }
    let existing = get(state, organization_id, task_id).await?;
    let server_id = match req.server_id.as_deref() {
        Some(value) if !value.trim().is_empty() => {
            resolve_server_id(state, organization_id, Some(value), Some(&req.runner)).await?
        }
        _ => existing.server_id,
    };
    // `resolve_server_id` is skipped when the pin is preserved, so re-check the
    // kept server when a task is switched to the remote-only runner.
    if req.runner == "remote_shell" && server_id == LOCAL_SERVER_ID {
        return Err(AppError::ValidationError(
            "The remote_shell runner requires a remote server".into(),
        ));
    }
    if state.is_cloud && server_id == LOCAL_SERVER_ID {
        return Err(AppError::ValidationError(
            "Cloud mode requires a ready remote server for scheduled tasks".into(),
        ));
    }
    let image =
        (req.runner == "ephemeral").then(|| req.image.as_deref().unwrap_or("").trim().to_string());
    let application_id = (req.runner == "application_exec")
        .then(|| req.application_id.clone())
        .flatten();
    let compose_service = (req.runner == "application_exec")
        .then(|| req.compose_service.clone())
        .flatten()
        .filter(|value| !value.trim().is_empty());
    let next_run: Option<i64> = req
        .enabled
        .then(|| next_run_at(&req.cron_expression))
        .transpose()?;
    let now = now_ms();
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("UPDATE scheduled_tasks SET name = ?1, description = ?2, cron_expression = ?3, runner = ?4, image = ?5, shell = ?6, script = ?7, timeout_seconds = ?8, network_access = ?9, server_id = ?10, enabled = ?11, next_run_at = ?12, updated_at = ?13, application_id = ?14, compose_service = ?15 WHERE id = ?16 AND organization_id = ?17")
        .bind(req.name.trim())
        .bind(req.description.trim())
        .bind(req.cron_expression.trim())
        .bind(req.runner.clone())
        .bind(image)
        .bind(req.shell.clone())
        .bind(req.script.clone())
        .bind(req.timeout_seconds)
        .bind(i64::from(req.network_access))
        .bind(server_id)
        .bind(i64::from(req.enabled))
        .bind(next_run)
        .bind(now)
        .bind(application_id)
        .bind(compose_service)
        .bind(task_id)
        .bind(organization_id)
        .execute(&conn)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    get(state, organization_id, task_id).await
}

pub async fn delete(
    state: &SharedState,
    organization_id: &str,
    task_id: &str,
) -> Result<(), AppError> {
    let task = get(state, organization_id, task_id).await?;
    if task.active_execution_id.is_some() {
        return Err(AppError::ValidationError(
            "Stop or replace the active execution before deleting this task".into(),
        ));
    }
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("DELETE FROM scheduled_tasks WHERE id = ?1 AND organization_id = ?2")
        .bind(task_id)
        .bind(organization_id)
        .execute(&conn)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    Ok(())
}

/// Retention window for run history, in days (`scheduled_task_settings`); 0 keeps it forever.
pub async fn history_retention_days(state: &SharedState) -> Result<i64, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let days: Option<i64> = sqlx::query_scalar(
        "SELECT history_retention_days FROM scheduled_task_settings WHERE id = 1",
    )
    .fetch_optional(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    Ok(days.unwrap_or(60))
}

/// Purges executions (and their logs) older than the configured retention window so the
/// log tables cannot grow without bound. Queued/running executions are never purged.
pub async fn purge_expired_history(state: &SharedState) -> Result<(), AppError> {
    let days = history_retention_days(state).await?;
    if days <= 0 {
        return Ok(());
    }
    let conn = crate::git::repositories::db::get_conn(state).await;
    let cutoff = now_ms() - days * 24 * 60 * 60 * 1000;
    let logs = sqlx::query(
        "DELETE FROM scheduled_task_execution_logs WHERE execution_id IN (SELECT id FROM scheduled_task_executions WHERE queued_at < ?1 AND status NOT IN ('queued', 'running'))",
    )
    .bind(cutoff)
    .execute(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?
    .rows_affected();
    let executions = sqlx::query("DELETE FROM scheduled_task_executions WHERE queued_at < ?1 AND status NOT IN ('queued', 'running')")
        .bind(cutoff)
        .execute(&conn)
        .await
        .map_err(|error| AppError::DatabaseError(error.to_string()))?
        .rows_affected();
    sqlx::query(
        "UPDATE scheduled_tasks SET active_execution_id = NULL, updated_at = ?1 WHERE active_execution_id NOT IN (SELECT id FROM scheduled_task_executions)",
    )
    .bind(now_ms())
    .execute(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    if executions > 0 {
        tracing::info!(%executions, %logs, "purged expired scheduled task history");
    }
    Ok(())
}

/// Removes every execution and log line for a task. Refuses while an execution
/// is active so the live run is not deleted out from under the runner.
pub async fn clear_history(
    state: &SharedState,
    organization_id: &str,
    task_id: &str,
) -> Result<(), AppError> {
    let task = get(state, organization_id, task_id).await?;
    if task.active_execution_id.is_some() {
        return Err(AppError::ValidationError(
            "Stop or replace the active execution before clearing history".into(),
        ));
    }
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "DELETE FROM scheduled_task_execution_logs WHERE execution_id IN (SELECT id FROM scheduled_task_executions WHERE task_id = ?1 AND organization_id = ?2)",
    )
    .bind(task_id)
    .bind(organization_id)
    .execute(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    sqlx::query(
        "DELETE FROM scheduled_task_executions WHERE task_id = ?1 AND organization_id = ?2",
    )
    .bind(task_id)
    .bind(organization_id)
    .execute(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    Ok(())
}

pub async fn run_now(
    state: &SharedState,
    organization_id: &str,
    task_id: &str,
) -> Result<ScheduledTaskExecutionDto, AppError> {
    queue_run(state, organization_id, task_id, "manual").await
}

/// Queues an execution of the task, replacing one still active. `trigger` is
/// `manual` for a user's run and `scheduled` for the cron scheduler's.
pub async fn queue_run(
    state: &SharedState,
    organization_id: &str,
    task_id: &str,
    trigger: &str,
) -> Result<ScheduledTaskExecutionDto, AppError> {
    let task = get(state, organization_id, task_id).await?;
    validate_runner_for_cloud(state, &task.runner)?;
    let id = Uuid::now_v7().to_string();
    let now = now_ms();
    let conn = crate::git::repositories::db::get_conn(state).await;
    if let Some(previous) = task.active_execution_id {
        sqlx::query("UPDATE scheduled_task_executions SET status = 'replaced', finished_at = ?1 WHERE id = ?2 AND status IN ('queued', 'running')")
            .bind(now)
            .bind(&previous)
            .execute(&conn)
            .await
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        super::live::finished(&previous, "replaced");
    }
    sqlx::query("INSERT INTO scheduled_task_executions (id, task_id, organization_id, trigger, runner, image, application_id, compose_service, network_access, server_id, shell, script, timeout_seconds, status, queued_at) SELECT ?1, id, organization_id, ?5, runner, image, application_id, compose_service, network_access, server_id, shell, script, timeout_seconds, 'queued', ?2 FROM scheduled_tasks WHERE id = ?3 AND organization_id = ?4")
        .bind(id.clone())
        .bind(now)
        .bind(task_id)
        .bind(organization_id)
        .bind(trigger)
        .execute(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    sqlx::query("UPDATE scheduled_tasks SET active_execution_id = ?1, updated_at = ?2 WHERE id = ?3 AND organization_id = ?4")
        .bind(id.clone())
        .bind(now)
        .bind(task_id)
        .bind(organization_id)
        .execute(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    get_execution(state, organization_id, task_id, &id).await
}

pub async fn cancel_execution(
    state: &SharedState,
    organization_id: &str,
    task_id: &str,
    execution_id: &str,
) -> Result<ScheduledTaskExecutionDto, AppError> {
    use sqlx::Row;
    let conn = crate::git::repositories::db::get_conn(state).await;
    let (status, runner, server_id, container_name, process_group_id, application_id) = {
        let row = sqlx::query("SELECT status, runner, server_id, container_name, process_group_id, application_id FROM scheduled_task_executions WHERE organization_id = ?1 AND task_id = ?2 AND id = ?3")
            .bind(organization_id)
            .bind(task_id)
            .bind(execution_id)
            .fetch_optional(&conn)
            .await
            .map_err(|e| AppError::DatabaseError(e.to_string()))?
            .ok_or(AppError::ResourceNotFound)?;
        let status: String = row
            .try_get(0)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        let runner: String = row
            .try_get(1)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        let server_id: String = row
            .try_get(2)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        let container_name: Option<String> = row
            .try_get(3)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        let process_group_id: Option<i64> = row
            .try_get(4)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        let application_id: Option<String> = row
            .try_get(5)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        (
            status,
            runner,
            server_id,
            container_name,
            process_group_id,
            application_id,
        )
    };
    if !matches!(status.as_str(), "queued" | "running") {
        return Err(AppError::ValidationError(
            "This execution is no longer running".into(),
        ));
    }
    let now = now_ms();
    // mark the terminal state first so the runner cannot overwrite it when the process dies
    crate::git::repositories::db::retry_on_stale_snapshot(|| async {
        sqlx::query("UPDATE scheduled_task_executions SET status = 'canceled', finished_at = ?1, error = ?2 WHERE id = ?3 AND status IN ('queued', 'running')")
            .bind(now)
            .bind("Execution killed by user")
            .bind(execution_id)
            .execute(&conn)
            .await
            .map_err(|e| AppError::DatabaseError(e.to_string()))
    })
    .await?;
    let _ = sqlx::query("INSERT INTO scheduled_task_execution_logs (execution_id, sequence, stream, message, created_at) SELECT ?1, COALESCE(MAX(sequence) + 1, 0), 'system', ?2, ?3 FROM scheduled_task_execution_logs WHERE execution_id = ?1")
        .bind(execution_id)
        .bind("Execution killed by user")
        .bind(now)
        .execute(&conn)
        .await;
    super::live::finished(execution_id, "canceled");
    crate::git::repositories::db::retry_on_stale_snapshot(|| async {
        sqlx::query("UPDATE scheduled_tasks SET active_execution_id = NULL, updated_at = ?1 WHERE active_execution_id = ?2")
            .bind(now)
            .bind(execution_id)
            .execute(&conn)
            .await
            .map_err(|e| AppError::DatabaseError(e.to_string()))
    })
    .await?;
    match runner.as_str() {
        "ephemeral" => {
            if let Some(name) = container_name {
                runner::kill_container(state, &server_id, &name).await?;
            }
        }
        "remote_shell" => {
            if let Some(pid) = process_group_id {
                runner::kill_remote_process(state, &server_id, pid).await?;
            }
        }
        "application_exec" => {
            if let (Some(application_id), Some(container)) = (application_id, container_name) {
                runner::kill_application_exec(state, &application_id, &container, execution_id)
                    .await;
            }
        }
        _ => {
            if let Some(pid) = process_group_id {
                runner::kill_process_group(pid).await;
            }
        }
    }
    get_execution(state, organization_id, task_id, execution_id).await
}

pub async fn list_executions(
    state: &SharedState,
    organization_id: &str,
    task_id: &str,
) -> Result<Vec<ScheduledTaskExecutionDto>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(&format!("SELECT {EXECUTION_COLUMNS} FROM scheduled_task_executions WHERE organization_id = ?1 AND task_id = ?2 ORDER BY queued_at DESC LIMIT 100"))
        .bind(organization_id)
        .bind(task_id)
        .fetch_all(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    rows.iter().map(read_execution).collect()
}

pub async fn get_execution(
    state: &SharedState,
    organization_id: &str,
    task_id: &str,
    execution_id: &str,
) -> Result<ScheduledTaskExecutionDto, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(&format!("SELECT {EXECUTION_COLUMNS} FROM scheduled_task_executions WHERE organization_id = ?1 AND task_id = ?2 AND id = ?3"))
        .bind(organization_id)
        .bind(task_id)
        .bind(execution_id)
        .fetch_optional(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?
        .map(|row| read_execution(&row))
        .transpose()?
        .ok_or(AppError::ResourceNotFound)
}

pub async fn get_execution_detail(
    state: &SharedState,
    organization_id: &str,
    task_id: &str,
    execution_id: &str,
) -> Result<ScheduledTaskExecutionDetailDto, AppError> {
    let execution = get_execution(state, organization_id, task_id, execution_id).await?;
    use sqlx::Row;
    let conn = crate::git::repositories::db::get_conn(state).await;
    let (server_name, server_user) = {
        let row = sqlx::query(
            "SELECT s.name, s.ssh_user FROM scheduled_task_executions e LEFT JOIN servers s ON s.id = e.server_id WHERE e.id = ?1",
        )
        .bind(execution_id)
        .fetch_optional(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?
        .ok_or(AppError::ResourceNotFound)?;
        (
            row.try_get::<Option<String>, _>(0)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            row.try_get::<Option<String>, _>(1)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        )
    };
    let rows = sqlx::query("SELECT sequence, stream, message, created_at FROM scheduled_task_execution_logs WHERE execution_id = ?1 ORDER BY sequence ASC")
        .bind(execution_id)
        .fetch_all(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let mut logs = Vec::new();
    for row in &rows {
        logs.push(ScheduledTaskExecutionLogDto {
            sequence: row
                .try_get(0)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            stream: row
                .try_get(1)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            message: row
                .try_get(2)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            created_at: row
                .try_get(3)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        });
    }
    Ok(ScheduledTaskExecutionDetailDto {
        execution,
        server_name,
        server_user,
        logs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dtos::scheduled_task::CreateScheduledTaskRequest;

    fn assert_validation_error(result: Result<(), AppError>, needle: &str) {
        match result {
            Err(AppError::ValidationError(message)) => assert!(
                message.contains(needle),
                "expected validation error containing '{needle}', got: {message}"
            ),
            other => panic!("expected ValidationError, got {other:?}"),
        }
    }

    fn base_request() -> CreateScheduledTaskRequest {
        CreateScheduledTaskRequest {
            name: "nightly".into(),
            description: "".into(),
            cron_expression: "0 0 * * *".into(),
            runner: "ephemeral".into(),
            image: Some("alpine:3.20".into()),
            shell: "bash".into(),
            script: "echo ok".into(),
            timeout_seconds: 60,
            network_access: false,
            server_id: None,
            application_id: None,
            compose_service: None,
        }
    }

    #[test]
    fn validate_request_accepts_a_standard_ephemeral_task() {
        assert!(validate_request(&base_request(), true).is_ok());
    }

    #[test]
    fn validate_request_rejects_an_unknown_runner() {
        let mut req = base_request();
        req.runner = "sidecar".into();
        assert_validation_error(
            validate_request(&req, true),
            "Runner must be ephemeral, remote_shell, litepod_api, or application_exec",
        );
    }

    #[test]
    fn validate_request_requires_an_image_for_ephemeral() {
        let mut req = base_request();
        req.image = Some("   ".into());
        assert_validation_error(
            validate_request(&req, true),
            "image is required for ephemeral",
        );
    }

    #[test]
    fn validate_request_accepts_a_remote_shell_task_without_image() {
        let mut req = base_request();
        req.runner = "remote_shell".into();
        req.image = None;
        assert!(validate_request(&req, true).is_ok());
    }

    #[test]
    fn validate_request_rejects_an_image_for_remote_shell() {
        let mut req = base_request();
        req.runner = "remote_shell".into();
        req.image = Some("alpine:3.20".into());
        assert_validation_error(
            validate_request(&req, true),
            "remote_shell jobs cannot specify an image",
        );
    }

    #[test]
    fn validate_request_accepts_application_exec_with_an_application() {
        let mut req = base_request();
        req.runner = "application_exec".into();
        req.image = None;
        req.application_id = Some("application-1".into());
        assert!(validate_request(&req, true).is_ok());
    }

    #[test]
    fn validate_request_rejects_application_exec_without_an_application() {
        let mut req = base_request();
        req.runner = "application_exec".into();
        req.image = None;
        assert_validation_error(
            validate_request(&req, true),
            "An application is required for application_exec jobs",
        );
    }

    #[tokio::test]
    async fn validate_runner_for_cloud_rejects_litepod_api_only_in_cloud() {
        let (cloud, _dir) = crate::testing::test_state_for(true).await;
        assert_validation_error(
            validate_runner_for_cloud(&cloud, "litepod_api"),
            "not available in cloud mode",
        );
        assert!(validate_runner_for_cloud(&cloud, "ephemeral").is_ok());
        let (self_hosted, _dir) = crate::testing::test_state_for(false).await;
        assert!(validate_runner_for_cloud(&self_hosted, "litepod_api").is_ok());
    }

    #[tokio::test]
    async fn resolve_server_id_rejects_the_local_server_in_cloud() {
        let (cloud, _dir) = crate::testing::test_state_for(true).await;
        for requested in [None, Some("local")] {
            match resolve_server_id(&cloud, "organization-a", requested, Some("ephemeral")).await {
                Err(AppError::ValidationError(message)) => {
                    assert!(message.contains("Cloud mode requires a ready remote server"))
                }
                other => panic!("expected ValidationError, got {other:?}"),
            }
        }
        let (self_hosted, _dir) = crate::testing::test_state_for(false).await;
        assert_eq!(
            resolve_server_id(&self_hosted, "organization-a", None, Some("ephemeral"))
                .await
                .unwrap(),
            LOCAL_SERVER_ID
        );
    }

    #[test]
    fn validate_request_accepts_every_supported_shell_only() {
        for shell in ["sh", "bash", "zsh", "fish", "nu"] {
            let mut req = base_request();
            req.shell = shell.into();
            assert!(validate_request(&req, true).is_ok(), "{shell}");
        }
        let mut req = base_request();
        req.shell = "pwsh".into();
        assert_validation_error(validate_request(&req, true), "Shell must be");
    }

    #[tokio::test]
    async fn database_accepts_the_new_shells_on_tasks_and_executions() {
        let (state, _dir) = crate::testing::test_state_for(false).await;
        for shell in ["zsh", "fish", "nu"] {
            sqlx::query("INSERT INTO scheduled_tasks (id, organization_id, server_id, name, cron_expression, runner, shell, script, timeout_seconds, created_by, created_at, updated_at) VALUES (?1, 'organization-a', 'local', 'task', '* * * * *', 'litepod_api', ?2, 'echo ran', 10, 'editor', 1, 1)")
                .bind(format!("task-{shell}"))
                .bind(shell)
                .execute(&state.db)
                .await
                .unwrap();
            sqlx::query("INSERT INTO scheduled_task_executions (id, task_id, organization_id, server_id, trigger, runner, shell, script, timeout_seconds, status, queued_at) VALUES (?1, ?1, 'organization-a', 'local', 'manual', 'litepod_api', ?2, 'echo ran', 10, 'queued', 1)")
                .bind(format!("task-{shell}"))
                .bind(shell)
                .execute(&state.db)
                .await
                .unwrap();
        }
        assert!(sqlx::query("INSERT INTO scheduled_tasks (id, organization_id, server_id, name, cron_expression, runner, shell, script, timeout_seconds, created_by, created_at, updated_at) VALUES ('task-x', 'organization-a', 'local', 'task', '* * * * *', 'litepod_api', 'pwsh', 'echo ran', 10, 'editor', 1, 1)")
            .execute(&state.db)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn boot_guard_refuses_cloud_with_a_task_on_the_local_server() {
        let (cloud, _dir) = crate::testing::test_state_for(true).await;
        assert!(assert_no_local_scheduled_tasks(&cloud).await.is_ok());
        sqlx::query("INSERT INTO scheduled_tasks (id, organization_id, server_id, name, cron_expression, runner, shell, script, timeout_seconds, created_by, created_at, updated_at) VALUES ('task-1', 'organization-a', 'local', 'task', '* * * * *', 'litepod_api', 'sh', 'echo ran', 10, 'editor', 1, 1)")
            .execute(&cloud.db)
            .await
            .unwrap();
        assert_validation_error(
            assert_no_local_scheduled_tasks(&cloud).await,
            "IS_CLOUD=true cannot start",
        );
    }

    #[test]
    fn validate_request_rejects_an_image_for_application_exec() {
        let mut req = base_request();
        req.runner = "application_exec".into();
        req.application_id = Some("application-1".into());
        assert_validation_error(
            validate_request(&req, true),
            "application_exec jobs cannot specify an image",
        );
    }
}
