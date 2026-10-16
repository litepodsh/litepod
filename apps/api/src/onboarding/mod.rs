pub mod evaluator;
pub mod repository;

use serde::{Deserialize, Serialize};

pub const TASK_KEYS: &[&str] = &[
    "deploy.git_provider",
    "deploy.project",
    "deploy.application",
    "deploy.artifact",
    "deploy.first_run",
    "deploy.domain",
    "deploy.redeploy",
    "notifications.provider",
    "notifications.destination",
    "notifications.events",
    "notifications.test",
    "notifications.active",
];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GuideKey {
    #[default]
    Deploy,
    Notifications,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeployRoute {
    Git,
    Static,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PanelPosition {
    #[default]
    Right,
    Sidebar,
    Hidden,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OnboardingPreferences {
    pub active_guide: GuideKey,
    pub deploy_route: Option<DeployRoute>,
    pub guided_project_id: Option<String>,
    pub guided_application_id: Option<String>,
    pub tested_notification_provider_id: Option<String>,
    pub panel_position: PanelPosition,
    #[serde(default)]
    pub manual_tasks: Vec<String>,
    pub dismissed_at: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum OnboardingAction {
    SelectGuide {
        guide: GuideKey,
    },
    SelectRoute {
        route: DeployRoute,
    },
    SetPosition {
        position: PanelPosition,
    },
    SetGuidedResource {
        project_id: Option<String>,
        application_id: Option<String>,
    },
    MarkTask {
        task: String,
    },
    ReopenTask {
        task: String,
    },
    Dismiss,
    Reset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Pending,
    Detected,
    Manual,
    Unavailable,
    Skipped,
}

#[derive(Debug, Clone, Serialize)]
pub struct OnboardingTask {
    pub key: &'static str,
    pub title: &'static str,
    pub description: String,
    pub status: TaskStatus,
    pub completion_source: Option<&'static str>,
    pub href: Option<String>,
    pub can_mark_done: bool,
    pub blocked_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OnboardingGuide {
    pub key: GuideKey,
    pub title: &'static str,
    pub completed: usize,
    pub total: usize,
    pub is_complete: bool,
    pub next_task_key: Option<&'static str>,
    pub tasks: Vec<OnboardingTask>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OnboardingSummary {
    pub active_guide: GuideKey,
    pub deploy_route: Option<DeployRoute>,
    pub panel_position: PanelPosition,
    pub dismissed: bool,
    pub guides: Vec<OnboardingGuide>,
}
