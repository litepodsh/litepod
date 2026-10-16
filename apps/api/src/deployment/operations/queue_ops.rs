use super::super::{compose_standalone, git, history, queue, source, static_deploy};

use sqlx::Row;

use crate::{dtos::response::AppError, state::state::SharedState};

/// Resolves the queue ownership fields from an application ID. Every queue
/// insert needs `organization_id`, `project_id`, `environment_id`; the deploy hook and
/// webhooks receive only an application ID, so the server fills in the rest.
pub async fn owner_ids(
    state: &SharedState,
    application_id: &str,
) -> Result<(String, String, String), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(
        "SELECT p.organization_id, e.project_id, a.environment_id FROM applications a INNER JOIN environments e ON e.id = a.environment_id INNER JOIN projects p ON p.id = e.project_id WHERE a.id = ?1",
    )
    .bind(application_id)
    .fetch_optional(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?
    .ok_or(AppError::ApplicationNotFound)?;
    Ok((
        row.try_get(0)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        row.try_get(1)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        row.try_get(2)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
    ))
}

/// Single entry point for every deploy trigger. Dispatches by the application's
/// `deploy_type` so the deploy hook, push webhooks, and the UI Deploy button all
/// share the same queueing, serialization, and cancellation path.
pub async fn enqueue_for_application(
    state: &SharedState,
    application_id: &str,
    triggered_by: &str,
    created_by: Option<&str>,
) -> Result<queue::DeploymentJobDto, AppError> {
    let (organization_id, project_id, environment_id) = owner_ids(state, application_id).await?;
    let source_config = source::get(state, application_id).await?;
    match source_config.deploy_type.as_str() {
        "git" => {
            git::enqueue(
                state,
                &organization_id,
                &project_id,
                &environment_id,
                application_id,
                triggered_by,
                created_by,
            )
            .await
        }
        "compose" => {
            compose_standalone::enqueue(state, application_id, triggered_by, created_by).await
        }
        "static" => {
            static_deploy::enqueue(
                state,
                &organization_id,
                &project_id,
                &environment_id,
                application_id,
                triggered_by,
                created_by,
            )
            .await
        }
        "image" => {
            crate::deployment::image::enqueue(
                state,
                &organization_id,
                &project_id,
                &environment_id,
                application_id,
                triggered_by,
                created_by,
            )
            .await
        }
        other => Err(AppError::ValidationError(format!(
            "Unsupported deploy type: {other}"
        ))),
    }
}

/// Redeploys `target_run_id` instead of the application's current source -
/// same code/artifact that run deployed, everything else (env vars, resource
/// limits, domains) stays as configured right now. Dispatches on the run's
/// own `source_type` rather than the application's current `deploy_type`, so
/// this still refuses cleanly if the deploy type itself was changed since.
pub async fn enqueue_rollback_for_application(
    state: &SharedState,
    application_id: &str,
    target_run_id: &str,
    created_by: Option<&str>,
) -> Result<queue::DeploymentJobDto, AppError> {
    let target = history::rollback_source(state, application_id, target_run_id).await?;
    if target.status != "succeeded" {
        return Err(AppError::ValidationError(
            "Only a successful deploy can be rolled back to".into(),
        ));
    }
    let (organization_id, project_id, environment_id) = owner_ids(state, application_id).await?;
    match target.source_type.as_str() {
        "git" => {
            let Some(sha) = target.commit_sha else {
                return Err(AppError::ValidationError(
                    "That deploy has no recorded commit to roll back to".into(),
                ));
            };
            git::enqueue_rollback(
                state,
                &organization_id,
                &project_id,
                &environment_id,
                application_id,
                &sha,
                created_by,
            )
            .await
        }
        "compose" => {
            let Some(content) = target.compose_content else {
                return Err(AppError::ValidationError(
                    "That deploy has no recorded Compose file to roll back to".into(),
                ));
            };
            compose_standalone::enqueue_rollback(state, application_id, &content, created_by).await
        }
        "static" => {
            let Some(artifact_id) = target.artifact_id else {
                return Err(AppError::ValidationError(
                    "That deploy has no recorded artifact to roll back to".into(),
                ));
            };
            static_deploy::enqueue_rollback(
                state,
                &organization_id,
                &project_id,
                &environment_id,
                application_id,
                &artifact_id,
                created_by,
            )
            .await
        }
        "image" => {
            // Registry-image deploys redeploy the image as configured today;
            // the image reference lives in the source, not in a build artifact.
            crate::deployment::image::enqueue(
                state,
                &organization_id,
                &project_id,
                &environment_id,
                application_id,
                "manual_rollback",
                created_by,
            )
            .await
        }
        other => Err(AppError::ValidationError(format!(
            "Unsupported deploy type for rollback: {other}"
        ))),
    }
}
