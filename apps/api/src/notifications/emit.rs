//! Outbound notifications: Slack, Discord, Telegram and Microsoft Teams.
//!
//! Emitting code only ever calls the `notify_*` helpers below. They resolve
//! the audience, write one queued row per subscriber and return; the worker
//! does the network I/O. Every helper swallows its own errors — a webhook must
//! never be able to fail a deploy, a backup or a prune.

use super::{dispatch, event, repository};

use crate::state::state::SharedState;

use event::{CleanupSummary, DeploymentContext, Event};

/// Shared tail for every emitter: queue the event, log and drop any failure.
async fn emit(state: &SharedState, event: Event) {
    let action = event.action;
    match dispatch::enqueue(state, &event).await {
        Ok(0) => {}
        Ok(count) => tracing::debug!(action = action.key(), count, "notifications queued"),
        Err(error) => tracing::warn!(
            action = action.key(),
            error = %error,
            "could not queue notifications"
        ),
    }
}

/// Deep link to the application a run belongs to, so a chat message can be
/// clicked straight into the UI.
fn application_url(state: &SharedState, context: &repository::RunContext) -> Option<String> {
    let base = state.public_url.trim_end_matches('/');
    if base.is_empty() {
        return None;
    }
    Some(format!(
        "{base}/projects/{}/environments/{}/applications/{}",
        context.project_id, context.environment_id, context.application_id
    ))
}

async fn deployment_context(
    state: &SharedState,
    run_id: &str,
    finished_at: i64,
) -> Option<DeploymentContext> {
    let context = match repository::run_context(state, run_id).await {
        Ok(Some(context)) => context,
        Ok(None) => return None,
        Err(error) => {
            tracing::warn!(run_id, error = %error, "could not resolve run for notification");
            return None;
        }
    };
    let url = application_url(state, &context);
    Some(DeploymentContext {
        organization_id: context.organization_id.clone(),
        application_name: context.application_name.clone(),
        project_name: context.project_name.clone(),
        environment_name: context.environment_name.clone(),
        commit_sha: context.commit_sha.clone(),
        commit_message: context.commit_message.clone(),
        triggered_by: context.triggered_by.clone(),
        duration_ms: Some(finished_at - context.started_at),
        url,
    })
}

/// Called from `deployment::history::finish`. `phase` is the run's status
/// *before* the terminal write (`building` or `deploying`) — it is what tells
/// a build failure apart from a deploy failure.
pub async fn notify_deployment_finished(
    state: &SharedState,
    run_id: &str,
    status: &str,
    phase: &str,
    error: Option<&str>,
) {
    // A canceled run is a user action, not an event worth paging about.
    if status != "succeeded" && status != "failed" {
        return;
    }
    let finished_at = chrono::Utc::now().timestamp_millis();
    let Some(context) = deployment_context(state, run_id, finished_at).await else {
        return;
    };
    let event = if status == "succeeded" {
        event::app_deploy(context)
    } else {
        event::app_build_error(context, phase, error)
    };
    emit(state, event).await;
}

pub async fn notify_backup_finished(
    state: &SharedState,
    succeeded: bool,
    triggered_by: &str,
    size_bytes: Option<i64>,
    s3_key: Option<&str>,
    duration_ms: Option<i64>,
    error: Option<&str>,
) {
    emit(
        state,
        event::litepod_backup(
            succeeded,
            triggered_by,
            size_bytes,
            s3_key,
            duration_ms,
            error,
        ),
    )
    .await;
}

pub async fn notify_cleanup(state: &SharedState, summary: CleanupSummary, actor: Option<&str>) {
    emit(state, event::docker_cleanup(summary, actor)).await;
}

/// Emits the per-database backup event (success or failure) for an
/// organization's managed database. Called from the background run and from
/// restore; swallows its own errors like every `notify_*` helper.
pub async fn notify_database_backup(
    state: &SharedState,
    organization_id: &str,
    application_name: &str,
    engine: &str,
    succeeded: bool,
    triggered_by: &str,
    size_bytes: Option<i64>,
    error: Option<&str>,
) {
    emit(
        state,
        event::database_backup(
            organization_id,
            application_name,
            engine,
            succeeded,
            triggered_by,
            size_bytes,
            error,
        ),
    )
    .await;
}

pub async fn notify_restart(state: &SharedState, target: &str, actor: Option<&str>) {
    emit(state, event::litepod_restart(target, actor)).await;
}

pub async fn notify_server_reboot(
    state: &SharedState,
    organization_id: &str,
    server_name: &str,
    report: &event::RebootReport,
) {
    if let Some(event) = event::server_reboot(organization_id, server_name, report) {
        emit(state, event).await;
    }
}

pub async fn notify_security_alert(
    state: &SharedState,
    flag: crate::security::login_anomaly::Flag,
    subject: &str,
    ip: &str,
    place: Option<&str>,
) {
    let base = state.public_url.trim_end_matches('/');
    let url = (!base.is_empty()).then(|| format!("{base}/admin/security?flagged=1"));
    emit(
        state,
        event::security_alert(flag.key(), subject, ip, place, url),
    )
    .await;
}
