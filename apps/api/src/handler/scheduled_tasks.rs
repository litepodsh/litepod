use axum::{
    Json,
    extract::{Path, State},
};
use futures_util::StreamExt;

use crate::{
    dtos::{
        response::{AppError, created, ok},
        scheduled_task::{CreateScheduledTaskRequest, UpdateScheduledTaskRequest},
    },
    scheduled_tasks,
    security::authz::extract::{RequireOrg, perm},
    state::state::SharedState,
};

// Scheduled tasks run code on a host, so every route here is admin-level inside
// the organization — hence `RequireOrg<perm::ScheduledTaskManage>` throughout, which replaces the
// `scheduled_tasks::ensure_manage_access` call each handler used to make.
//
// Handlers below take `Path` only for the ids the extractor doesn't cover
// (`task_id`, `execution_id`); the leading `_organization_id` is already proven.

pub async fn list_scheduled_tasks_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ScheduledTaskManage>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    Ok(ok(
        scheduled_tasks::list(&state, &org.organization_id).await?
    ))
}

pub async fn create_scheduled_task_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ScheduledTaskManage>,
    Json(request): Json<CreateScheduledTaskRequest>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let task = scheduled_tasks::create(&state, &org.organization_id, &org.user, &request).await?;
    crate::git::repositories::audit_log::record(
        &state,
        &org.organization_id,
        &org.user,
        "scheduled_task.created",
        Some("scheduled_task"),
        Some(&task.id),
        Some(&task.name),
        None,
        None,
        None,
        None,
    )
    .await
    .ok();
    Ok(created(task))
}

pub async fn test_scheduled_task_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ScheduledTaskManage>,
    Json(request): Json<CreateScheduledTaskRequest>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    // validation errors still come back as a normal JSON error before the stream opens
    let params = scheduled_tasks::test(&state, &org.organization_id, &org.user, &request).await?;
    crate::git::repositories::audit_log::record(
        &state,
        &org.organization_id,
        &org.user,
        "scheduled_task.tested",
        Some("scheduled_task"),
        None,
        Some(&request.name),
        None,
        None,
        None,
        None,
    )
    .await
    .ok();
    let events = scheduled_tasks::runner::test_stream(state, params).map(|event| {
        Ok::<_, std::convert::Infallible>(
            axum::response::sse::Event::default()
                .data(serde_json::to_string(&event).unwrap_or_default()),
        )
    });
    Ok(axum::response::sse::Sse::new(events).keep_alive(axum::response::sse::KeepAlive::default()))
}

pub async fn get_scheduled_task_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ScheduledTaskManage>,
    Path((_organization_id, task_id)): Path<(String, String)>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    Ok(ok(scheduled_tasks::get_detail(
        &state,
        &org.organization_id,
        &task_id,
    )
    .await?))
}

pub async fn update_scheduled_task_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ScheduledTaskManage>,
    Path((_organization_id, task_id)): Path<(String, String)>,
    Json(request): Json<UpdateScheduledTaskRequest>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let task = scheduled_tasks::update(
        &state,
        &org.organization_id,
        &task_id,
        org.user.is_sysadmin,
        &request,
    )
    .await?;
    crate::git::repositories::audit_log::record(
        &state,
        &org.organization_id,
        &org.user,
        "scheduled_task.updated",
        Some("scheduled_task"),
        Some(&task_id),
        Some(&task.name),
        None,
        None,
        None,
        None,
    )
    .await
    .ok();
    Ok(ok(task))
}

pub async fn delete_scheduled_task_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ScheduledTaskManage>,
    Path((_organization_id, task_id)): Path<(String, String)>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    // Fetched before delete — the name would otherwise be gone by the time
    // the audit entry needs it, and a delete entry with no name is the least
    // useful kind.
    let name = scheduled_tasks::get(&state, &org.organization_id, &task_id)
        .await
        .ok()
        .map(|task| task.name);
    scheduled_tasks::delete(&state, &org.organization_id, &task_id).await?;
    crate::git::repositories::audit_log::record(
        &state,
        &org.organization_id,
        &org.user,
        "scheduled_task.deleted",
        Some("scheduled_task"),
        Some(&task_id),
        name.as_deref(),
        None,
        None,
        None,
        None,
    )
    .await
    .ok();
    Ok(ok(()))
}

pub async fn clear_scheduled_task_history_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ScheduledTaskManage>,
    Path((_organization_id, task_id)): Path<(String, String)>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    scheduled_tasks::clear_history(&state, &org.organization_id, &task_id).await?;
    Ok(ok(()))
}

pub async fn run_scheduled_task_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ScheduledTaskManage>,
    Path((_organization_id, task_id)): Path<(String, String)>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let execution = scheduled_tasks::run_now(&state, &org.organization_id, &task_id).await?;
    let worker_state = state.clone();
    let execution_id = execution.id.clone();
    tokio::spawn(async move {
        if let Err(error) = scheduled_tasks::runner::execute(worker_state, execution_id).await {
            tracing::error!(%error, "scheduled task execution failed");
        }
    });
    Ok(created(execution))
}

pub async fn list_scheduled_task_executions_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ScheduledTaskManage>,
    Path((_organization_id, task_id)): Path<(String, String)>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    Ok(ok(scheduled_tasks::list_executions(
        &state,
        &org.organization_id,
        &task_id,
    )
    .await?))
}

pub async fn cancel_scheduled_task_execution_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ScheduledTaskManage>,
    Path((_organization_id, task_id, execution_id)): Path<(String, String, String)>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    Ok(ok(scheduled_tasks::cancel_execution(
        &state,
        &org.organization_id,
        &task_id,
        &execution_id,
    )
    .await?))
}

/// Live view of one execution as server-sent events: saved rows first, then each
/// change as the runner writes it, then a final `status` event.
pub async fn stream_scheduled_task_execution_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ScheduledTaskManage>,
    Path((_organization_id, task_id, execution_id)): Path<(String, String, String)>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    // tenancy check: the execution must belong to this organization's task
    scheduled_tasks::get_execution(&state, &org.organization_id, &task_id, &execution_id).await?;
    let conn = crate::git::repositories::db::get_conn(&state).await;
    let events = scheduled_tasks::live::watch(conn, execution_id).map(|event| {
        Ok::<_, std::convert::Infallible>(
            axum::response::sse::Event::default()
                .data(serde_json::to_string(&event).unwrap_or_default()),
        )
    });
    Ok(axum::response::sse::Sse::new(events).keep_alive(axum::response::sse::KeepAlive::default()))
}

pub async fn get_scheduled_task_execution_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ScheduledTaskManage>,
    Path((_organization_id, task_id, execution_id)): Path<(String, String, String)>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    Ok(ok(scheduled_tasks::get_execution_detail(
        &state,
        &org.organization_id,
        &task_id,
        &execution_id,
    )
    .await?))
}
