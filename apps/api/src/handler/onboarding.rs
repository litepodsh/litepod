use axum::{Json, extract::State, response::IntoResponse};

use crate::{
    dtos::response::{AppError, ok},
    onboarding::{OnboardingAction, OnboardingSummary, evaluator, repository},
    security::authz::extract::{RequireOrg, perm},
    security::authz::{Permission, Target},
    security::middleware::org_scope::OrgScope,
    state::state::SharedState,
};

/// The guides walk through creating a project and an app, so they only offer
/// those steps to someone who can take them org-wide.
fn can_create_projects(org: &OrgScope) -> bool {
    org.permissions
        .allows(Permission::ProjectCreate, Target::Org)
}

pub async fn get_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgRead>,
) -> Result<impl IntoResponse, AppError> {
    let preferences = repository::get(&state, &org.user.user_id, &org.organization_id).await;
    let summary = evaluator::evaluate(
        &state,
        &org.organization_id,
        can_create_projects(&org),
        preferences,
    )
    .await?;
    Ok(ok(summary))
}

pub async fn action_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgRead>,
    Json(action): Json<OnboardingAction>,
) -> Result<impl IntoResponse, AppError> {
    validate_resources(&state, &org.organization_id, &action).await?;
    let preferences =
        repository::apply(&state, &org.user.user_id, &org.organization_id, action).await?;
    let summary: OnboardingSummary = evaluator::evaluate(
        &state,
        &org.organization_id,
        can_create_projects(&org),
        preferences,
    )
    .await?;
    Ok(ok(summary))
}

async fn validate_resources(
    state: &SharedState,
    organization_id: &str,
    action: &OnboardingAction,
) -> Result<(), AppError> {
    let OnboardingAction::SetGuidedResource {
        project_id,
        application_id,
    } = action
    else {
        return Ok(());
    };
    let conn = crate::git::repositories::db::get_conn(state).await;
    if let Some(project_id) = project_id {
        let exists: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM projects WHERE id = ?1 AND organization_id = ?2)",
        )
        .bind(project_id)
        .bind(organization_id)
        .fetch_one(&conn)
        .await?;
        if exists == 0 {
            return Err(AppError::ProjectNotFound);
        }
    }
    if let Some(application_id) = application_id {
        let exists: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM applications a JOIN environments e ON e.id = a.environment_id JOIN projects p ON p.id = e.project_id WHERE a.id = ?1 AND p.organization_id = ?2)",
        )
        .bind(application_id)
        .bind(organization_id)
        .fetch_one(&conn)
        .await?;
        if exists == 0 {
            return Err(AppError::ApplicationNotFound);
        }
    }
    Ok(())
}
