use sqlx::Row;

use crate::{dtos::response::AppError, state::state::SharedState};

use super::{
    DeployRoute, GuideKey, OnboardingGuide, OnboardingPreferences, OnboardingSummary,
    OnboardingTask, TaskStatus,
};

#[derive(Default)]
struct ResourceState {
    has_git_provider: bool,
    has_project: bool,
    project_id: Option<String>,
    application_id: Option<String>,
    deploy_type: Option<String>,
    git_ready: bool,
    has_artifact: bool,
    successful_runs: Vec<Option<String>>,
    has_domain: bool,
    notification_provider_id: Option<String>,
    notification_destination: bool,
    notification_events: bool,
    notification_enabled: bool,
}

pub async fn evaluate(
    state: &SharedState,
    organization_id: &str,
    can_edit: bool,
    preferences: OnboardingPreferences,
) -> Result<OnboardingSummary, AppError> {
    let resources = load_resources(state, organization_id, &preferences).await?;
    let inferred_route = match resources.deploy_type.as_deref() {
        Some("git") | Some("compose") => Some(DeployRoute::Git),
        Some("static") => Some(DeployRoute::Static),
        _ => None,
    };
    let route = preferences.deploy_route.or(inferred_route);
    let dismissed = preferences.dismissed_at.is_some();

    let deploy = deploy_guide(route, &resources, &preferences, can_edit, dismissed);
    let notifications = notifications_guide(&resources, &preferences, can_edit, dismissed);

    Ok(OnboardingSummary {
        active_guide: preferences.active_guide,
        deploy_route: route,
        panel_position: preferences.panel_position,
        dismissed,
        guides: vec![deploy, notifications],
    })
}

async fn load_resources(
    state: &SharedState,
    organization_id: &str,
    preferences: &OnboardingPreferences,
) -> Result<ResourceState, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let has_git_provider: i64 =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM git_providers WHERE organization_id = ?1)")
            .bind(organization_id)
            .fetch_one(&conn)
            .await?;
    let project = if let Some(project_id) = &preferences.guided_project_id {
        sqlx::query("SELECT id FROM projects WHERE id = ?1 AND organization_id = ?2")
            .bind(project_id)
            .bind(organization_id)
            .fetch_optional(&conn)
            .await?
    } else {
        sqlx::query(
            "SELECT id FROM projects WHERE organization_id = ?1 ORDER BY created_at LIMIT 1",
        )
        .bind(organization_id)
        .fetch_optional(&conn)
        .await?
    };
    let project_id: Option<String> = project
        .as_ref()
        .map(|row| row.try_get(0))
        .transpose()
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;

    let application = if let Some(application_id) = &preferences.guided_application_id {
        sqlx::query(
            "SELECT a.id, a.deploy_type, CASE WHEN a.source_upload = 1 THEN 'upload' ELSE a.git_url END FROM applications a JOIN environments e ON e.id = a.environment_id JOIN projects p ON p.id = e.project_id WHERE a.id = ?1 AND p.organization_id = ?2",
        )
        .bind(application_id)
        .bind(organization_id)
        .fetch_optional(&conn)
        .await?
    } else {
        sqlx::query(
            "SELECT a.id, a.deploy_type, CASE WHEN a.source_upload = 1 THEN 'upload' ELSE a.git_url END FROM applications a JOIN environments e ON e.id = a.environment_id JOIN projects p ON p.id = e.project_id WHERE p.organization_id = ?1 AND a.maintenance_for IS NULL ORDER BY a.created_at LIMIT 1",
        )
        .bind(organization_id)
        .fetch_optional(&conn)
        .await?
    };
    let (application_id, deploy_type, git_ready) = if let Some(row) = application {
        let id: String = row.try_get(0)?;
        let kind: String = row.try_get(1)?;
        let git_url: Option<String> = row.try_get(2)?;
        let ready = matches!(kind.as_str(), "git" | "compose")
            && git_url
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty());
        (Some(id), Some(kind), ready)
    } else {
        (None, None, false)
    };

    let mut has_artifact = false;
    let mut successful_runs = Vec::new();
    let mut has_domain = false;
    if let Some(application_id) = &application_id {
        let artifact: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM application_artifacts WHERE application_id = ?1)",
        )
        .bind(application_id)
        .fetch_one(&conn)
        .await?;
        has_artifact = artifact != 0;
        let rows = sqlx::query(
            "SELECT artifact_id FROM application_deployment_runs WHERE application_id = ?1 AND status = 'succeeded' ORDER BY started_at, id",
        )
        .bind(application_id)
        .fetch_all(&conn)
        .await?;
        successful_runs = rows
            .iter()
            .map(|row| row.try_get(0))
            .collect::<Result<Vec<Option<String>>, _>>()?;
        let domain: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM application_domains WHERE application_id = ?1 AND is_generated = 0)",
        )
        .bind(application_id)
        .fetch_one(&conn)
        .await?;
        has_domain = domain != 0;
    }

    let provider = sqlx::query(
        "SELECT id, enabled, config_hint FROM notification_providers WHERE organization_id = ?1 ORDER BY created_at LIMIT 1",
    )
    .bind(organization_id)
    .fetch_optional(&conn)
    .await?;
    let (notification_provider_id, notification_enabled, notification_destination) =
        if let Some(row) = provider {
            let id: String = row.try_get(0)?;
            let enabled: i64 = row.try_get(1)?;
            let hint: String = row.try_get(2)?;
            (Some(id), enabled != 0, !hint.trim().is_empty())
        } else {
            (None, false, false)
        };
    let notification_events = if let Some(provider_id) = &notification_provider_id {
        let exists: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM notification_subscriptions WHERE provider_id = ?1)",
        )
        .bind(provider_id)
        .fetch_one(&conn)
        .await?;
        exists != 0
    } else {
        false
    };

    Ok(ResourceState {
        has_git_provider: has_git_provider != 0,
        has_project: project_id.is_some(),
        project_id,
        application_id,
        deploy_type,
        git_ready,
        has_artifact,
        successful_runs,
        has_domain,
        notification_provider_id,
        notification_destination,
        notification_events,
        notification_enabled,
    })
}

fn deploy_guide(
    route: Option<DeployRoute>,
    resources: &ResourceState,
    preferences: &OnboardingPreferences,
    can_edit: bool,
    dismissed: bool,
) -> OnboardingGuide {
    let Some(route) = route else {
        return finish_guide(GuideKey::Deploy, "Deploy your first application", vec![]);
    };
    let project_href = Some("/projects".to_string());
    let application_href = resources
        .project_id
        .as_ref()
        .map(|id| format!("/projects/{id}"));
    let detail_href = resources
        .application_id
        .as_ref()
        .and_then(|application_id| {
            resources
                .project_id
                .as_ref()
                .map(|project_id| format!("/projects/{project_id}?application={application_id}"))
        });
    let mut tasks = Vec::new();
    if route == DeployRoute::Git {
        tasks.push(task(
            "deploy.git_provider",
            "Connect a Git provider",
            "Authorize the repository provider you want to deploy from.",
            resources.has_git_provider,
            "/git-providers",
            preferences,
            can_edit,
            dismissed,
        ));
    }
    tasks.push(task(
        "deploy.project",
        "Create a project",
        "Projects group environments and applications.",
        resources.has_project,
        project_href.as_deref().unwrap(),
        preferences,
        can_edit,
        dismissed,
    ));
    let application_done = match route {
        DeployRoute::Git => resources.git_ready,
        DeployRoute::Static => resources.deploy_type.as_deref() == Some("static"),
    };
    tasks.push(task(
        "deploy.application",
        if route == DeployRoute::Git {
            "Create a Git application"
        } else {
            "Create a static application"
        },
        "Choose the application that this guide will follow.",
        application_done,
        application_href.as_deref().unwrap_or("/projects"),
        preferences,
        can_edit,
        dismissed,
    ));
    if route == DeployRoute::Static {
        tasks.push(task(
            "deploy.artifact",
            "Upload the static files",
            "Upload the HTML, CSS, and assets that Litepod should serve.",
            resources.has_artifact,
            detail_href.as_deref().unwrap_or("/projects"),
            preferences,
            can_edit,
            dismissed,
        ));
    }
    tasks.push(task(
        "deploy.first_run",
        "Complete the first deployment",
        "Wait for one successful deployment of the guided application.",
        !resources.successful_runs.is_empty(),
        detail_href.as_deref().unwrap_or("/projects"),
        preferences,
        can_edit,
        dismissed,
    ));
    tasks.push(task(
        "deploy.domain",
        "Configure a domain",
        "Connect a custom domain; Litepod-generated addresses do not complete this step.",
        resources.has_domain,
        detail_href.as_deref().unwrap_or("/projects"),
        preferences,
        can_edit,
        dismissed,
    ));
    let redeployed = if route == DeployRoute::Static {
        resources
            .successful_runs
            .iter()
            .flatten()
            .collect::<std::collections::HashSet<_>>()
            .len()
            >= 2
    } else {
        resources.successful_runs.len() >= 2
    };
    tasks.push(task(
        "deploy.redeploy",
        "Redeploy the application",
        if route == DeployRoute::Static {
            "Upload a new artifact and deploy it successfully."
        } else {
            "Run a later successful deployment."
        },
        redeployed,
        detail_href.as_deref().unwrap_or("/projects"),
        preferences,
        can_edit,
        dismissed,
    ));
    finish_guide(GuideKey::Deploy, "Deploy your first application", tasks)
}

fn notifications_guide(
    resources: &ResourceState,
    preferences: &OnboardingPreferences,
    can_edit: bool,
    dismissed: bool,
) -> OnboardingGuide {
    let tested = resources.notification_provider_id.as_ref()
        == preferences.tested_notification_provider_id.as_ref();
    let href = "/notifications";
    finish_guide(
        GuideKey::Notifications,
        "Configure notifications",
        vec![
            task(
                "notifications.provider",
                "Choose a notification provider",
                "Add Slack, Discord, Telegram, or Microsoft Teams.",
                resources.notification_provider_id.is_some(),
                href,
                preferences,
                can_edit,
                dismissed,
            ),
            task(
                "notifications.destination",
                "Configure the destination",
                "Save the webhook or chat destination.",
                resources.notification_destination,
                href,
                preferences,
                can_edit,
                dismissed,
            ),
            task(
                "notifications.events",
                "Select deployment events",
                "Choose at least one event for this channel.",
                resources.notification_events,
                href,
                preferences,
                can_edit,
                dismissed,
            ),
            task(
                "notifications.test",
                "Send a test notification",
                "Verify that Litepod can reach the saved channel.",
                tested,
                href,
                preferences,
                can_edit,
                dismissed,
            ),
            task(
                "notifications.active",
                "Confirm the channel is active",
                "Keep the tested notification channel enabled.",
                tested && resources.notification_enabled,
                href,
                preferences,
                can_edit,
                dismissed,
            ),
        ],
    )
}

#[allow(clippy::too_many_arguments)]
fn task(
    key: &'static str,
    title: &'static str,
    description: &str,
    detected: bool,
    href: &str,
    preferences: &OnboardingPreferences,
    can_edit: bool,
    dismissed: bool,
) -> OnboardingTask {
    let manually_done = preferences.manual_tasks.iter().any(|value| value == key);
    let status = if dismissed {
        TaskStatus::Skipped
    } else if detected {
        TaskStatus::Detected
    } else if manually_done {
        TaskStatus::Manual
    } else if !can_edit {
        TaskStatus::Unavailable
    } else {
        TaskStatus::Pending
    };
    OnboardingTask {
        key,
        title,
        description: description.to_string(),
        status,
        completion_source: match status {
            TaskStatus::Detected => Some("auto"),
            TaskStatus::Manual => Some("manual"),
            _ => None,
        },
        href: (can_edit && status == TaskStatus::Pending).then(|| href.to_string()),
        can_mark_done: can_edit && status == TaskStatus::Pending,
        blocked_reason: (status == TaskStatus::Unavailable)
            .then(|| "An organization editor or admin can complete this task.".to_string()),
    }
}

fn finish_guide(key: GuideKey, title: &'static str, tasks: Vec<OnboardingTask>) -> OnboardingGuide {
    let completed = tasks
        .iter()
        .filter(|task| matches!(task.status, TaskStatus::Detected | TaskStatus::Manual))
        .count();
    let next_task_key = tasks
        .iter()
        .find(|task| task.status == TaskStatus::Pending)
        .map(|task| task.key);
    OnboardingGuide {
        key,
        title,
        completed,
        total: tasks.len(),
        is_complete: !tasks.is_empty() && completed == tasks.len(),
        next_task_key,
        tasks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detected_state_overrides_manual_state() {
        let preferences = OnboardingPreferences {
            manual_tasks: vec!["deploy.domain".into()],
            ..Default::default()
        };
        let item = task(
            "deploy.domain",
            "Domain",
            "Domain",
            true,
            "/",
            &preferences,
            true,
            false,
        );
        assert_eq!(item.status, TaskStatus::Detected);
        assert_eq!(item.completion_source, Some("auto"));
    }

    #[test]
    fn manual_state_can_complete_an_unverified_task() {
        let preferences = OnboardingPreferences {
            manual_tasks: vec!["deploy.domain".into()],
            ..Default::default()
        };
        let item = task(
            "deploy.domain",
            "Domain",
            "Domain",
            false,
            "/",
            &preferences,
            true,
            false,
        );
        assert_eq!(item.status, TaskStatus::Manual);
    }
}
