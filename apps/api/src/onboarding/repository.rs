use chrono::Utc;

use crate::{dtos::response::AppError, state::state::SharedState};

use super::{OnboardingAction, OnboardingPreferences, TASK_KEYS};

pub const ONBOARDING_TTL_SECS: u64 = 2_592_000;

fn key(user_id: &str, organization_id: &str) -> String {
    format!("litepod:onboarding:{user_id}:{organization_id}")
}

fn cache_error(error: String) -> AppError {
    AppError::InternalError(format!("onboarding preferences unavailable: {error}"))
}

pub async fn get(
    state: &SharedState,
    user_id: &str,
    organization_id: &str,
) -> OnboardingPreferences {
    match state
        .ephemeral
        .get_json_refresh(&key(user_id, organization_id), ONBOARDING_TTL_SECS)
        .await
    {
        Ok(Some(value)) => value,
        Ok(None) => OnboardingPreferences::default(),
        Err(error) => {
            tracing::debug!(%error, "onboarding cache unavailable; using defaults");
            OnboardingPreferences::default()
        }
    }
}

pub async fn apply(
    state: &SharedState,
    user_id: &str,
    organization_id: &str,
    action: OnboardingAction,
) -> Result<OnboardingPreferences, AppError> {
    if matches!(action, OnboardingAction::Reset) {
        state
            .ephemeral
            .delete(&key(user_id, organization_id))
            .await
            .map_err(cache_error)?;
        return Ok(OnboardingPreferences::default());
    }

    let mut preferences = get(state, user_id, organization_id).await;
    match action {
        OnboardingAction::SelectGuide { guide } => preferences.active_guide = guide,
        OnboardingAction::SelectRoute { route } => preferences.deploy_route = Some(route),
        OnboardingAction::SetPosition { position } => preferences.panel_position = position,
        OnboardingAction::SetGuidedResource {
            project_id,
            application_id,
        } => {
            preferences.guided_project_id = project_id;
            preferences.guided_application_id = application_id;
        }
        OnboardingAction::MarkTask { task } => {
            validate_task(&task)?;
            if !preferences.manual_tasks.iter().any(|value| value == &task) {
                preferences.manual_tasks.push(task);
            }
        }
        OnboardingAction::ReopenTask { task } => {
            validate_task(&task)?;
            preferences.manual_tasks.retain(|value| value != &task);
        }
        OnboardingAction::Dismiss => preferences.dismissed_at = Some(Utc::now().timestamp_millis()),
        OnboardingAction::Reset => unreachable!(),
    }
    state
        .ephemeral
        .set_json(
            &key(user_id, organization_id),
            &preferences,
            ONBOARDING_TTL_SECS,
        )
        .await
        .map_err(cache_error)?;
    Ok(preferences)
}

pub async fn remember_notification_test(
    state: &SharedState,
    user_id: &str,
    organization_id: &str,
    provider_id: &str,
) -> Result<(), AppError> {
    let mut preferences = get(state, user_id, organization_id).await;
    preferences.tested_notification_provider_id = Some(provider_id.to_owned());
    state
        .ephemeral
        .set_json(
            &key(user_id, organization_id),
            &preferences,
            ONBOARDING_TTL_SECS,
        )
        .await
        .map_err(cache_error)
}

fn validate_task(task: &str) -> Result<(), AppError> {
    if TASK_KEYS.contains(&task) {
        Ok(())
    } else {
        Err(AppError::ValidationError("Unknown onboarding task".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_keys_are_isolated() {
        assert_ne!(key("a", "org"), key("b", "org"));
        assert_ne!(key("a", "org"), key("a", "other"));
    }

    #[test]
    fn task_keys_are_validated() {
        assert!(validate_task("deploy.domain").is_ok());
        assert!(validate_task("deploy.unknown").is_err());
    }
}
