//! Organization-scoped CRUD for notification providers, plus the "Test Notification"
//! endpoint.
//!
//! Authorization mirrors `handler::organizations`: membership to read, an admin role to
//! mutate. Platform-wide actions add two more requirements — sysadmin, and the
//! organization must be the root organization — so a tenant organization cannot subscribe itself to the
//! install's backup and restart traffic.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::{
    Extension, Json,
    extract::{Path, State},
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};

use crate::dtos::response::{AppError, created, ok};
use crate::notifications::{
    dispatch::{self, SendOutcome},
    event::{self, Action},
    provider::{ProviderConfig, ProviderKind},
    repository::{self, DeliveryDto, NewProvider, ProviderDto},
};
use crate::security::authz::extract::{RequireOrg, perm};
use crate::security::middleware::auth::AuthUser;
use crate::security::middleware::org_scope::OrgScope;
use crate::state::state::SharedState;

const DELIVERY_PAGE: usize = 25;
const MAX_NAME_LEN: usize = 60;
/// The test endpoints make the server issue an outbound request on demand.
/// The host allowlist already bounds *where* that can go; this bounds how often,
/// so the endpoint cannot be turned into a traffic amplifier.
const TEST_QUOTA: usize = 5;
const TEST_WINDOW: Duration = Duration::from_secs(60);

fn test_throttle() -> &'static Mutex<HashMap<String, Vec<Instant>>> {
    static THROTTLE: OnceLock<Mutex<HashMap<String, Vec<Instant>>>> = OnceLock::new();
    THROTTLE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn check_test_quota_local(user_id: &str) -> Result<(), AppError> {
    let now = Instant::now();
    let mut throttle = test_throttle()
        .lock()
        .map_err(|_| AppError::InternalError("throttle poisoned".into()))?;
    // Opportunistic sweep: without it a long-lived process accumulates one
    // entry per user who ever pressed the button.
    throttle.retain(|_, hits| {
        hits.retain(|hit| now.duration_since(*hit) < TEST_WINDOW);
        !hits.is_empty()
    });
    let hits = throttle.entry(user_id.to_string()).or_default();
    if hits.len() >= TEST_QUOTA {
        return Err(AppError::ResourceInUse(format!(
            "Too many test notifications — wait a minute and try again (limit {TEST_QUOTA} per minute)"
        )));
    }
    hits.push(now);
    Ok(())
}

async fn check_test_quota(state: &SharedState, user_id: &str) -> Result<(), AppError> {
    let key = format!("litepod:notification-test:{user_id}");
    match state
        .ephemeral
        .fixed_window_increment(&key, TEST_WINDOW.as_secs())
        .await
    {
        Ok(count) if count <= TEST_QUOTA as u64 => Ok(()),
        Ok(_) => Err(AppError::ResourceInUse(format!(
            "Too many test notifications — wait a minute and try again (limit {TEST_QUOTA} per minute)"
        ))),
        Err(error) => {
            tracing::debug!(%error, "Dragonfly notification quota unavailable; using local limit");
            check_test_quota_local(user_id)
        }
    }
}

// ── Wire types ────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ProviderConfigInput {
    #[serde(default)]
    pub webhook_url: Option<String>,
    #[serde(default)]
    pub bot_token: Option<String>,
    #[serde(default)]
    pub chat_id: Option<String>,
}

impl ProviderConfigInput {
    fn is_empty(&self) -> bool {
        let blank = |value: &Option<String>| {
            value
                .as_deref()
                .map(|value| value.trim().is_empty())
                .unwrap_or(true)
        };
        blank(&self.webhook_url) && blank(&self.bot_token) && blank(&self.chat_id)
    }

    fn into_config(self) -> ProviderConfig {
        let trim = |value: Option<String>| {
            value
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        };
        ProviderConfig {
            webhook_url: trim(self.webhook_url),
            bot_token: trim(self.bot_token),
            chat_id: trim(self.chat_id),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateProviderRequest {
    pub name: String,
    pub kind: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub decoration: bool,
    pub config: ProviderConfigInput,
    #[serde(default)]
    pub actions: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateProviderRequest {
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub decoration: bool,
    /// Omitted (or all-blank) keeps the stored credential, the same contract as
    /// `UpdateS3ConfigRequest::secret_access_key`.
    #[serde(default)]
    pub config: Option<ProviderConfigInput>,
    #[serde(default)]
    pub actions: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct TestProviderRequest {
    pub kind: String,
    #[serde(default = "default_true")]
    pub decoration: bool,
    pub config: ProviderConfigInput,
}

#[derive(Debug, Serialize)]
pub struct TestResult {
    pub delivered: bool,
    pub message: String,
}

/// Static description of what the UI should render, so the modal's provider
/// tiles and action toggles come from one source of truth.
#[derive(Debug, Serialize)]
pub struct NotificationCatalog {
    pub providers: Vec<CatalogProvider>,
    pub actions: Vec<CatalogAction>,
    pub organization_is_root: bool,
}

#[derive(Debug, Serialize)]
pub struct CatalogProvider {
    pub kind: String,
    pub label: String,
    pub uses_webhook_url: bool,
    pub allowed_hosts: String,
}

#[derive(Debug, Serialize)]
pub struct CatalogAction {
    pub key: String,
    pub label: String,
    pub description: String,
    pub global: bool,
}

fn default_true() -> bool {
    true
}

fn action_label(action: Action) -> (&'static str, &'static str) {
    match action {
        Action::AppDeploy => ("App Deploy", "Trigger the action when an app is deployed."),
        Action::AppBuildError => (
            "App Build Error",
            "Trigger the action when the build fails.",
        ),
        Action::LitepodBackup => (
            "litepod Backup",
            "Trigger the action when a litepod backup is created.",
        ),
        Action::DockerCleanup => (
            "Cleanup",
            "Trigger the action when a maintenance cleanup is performed.",
        ),
        Action::LitepodRestart => (
            "litepod Restart",
            "Trigger the action when litepod is restarted.",
        ),
        Action::ServerReboot => (
            "Server Reboot",
            "Trigger the action when a server restarts and litepod restores its applications.",
        ),
        Action::DatabaseBackup => (
            "Database Backup",
            "Trigger the action when a database backup or restore completes.",
        ),
        Action::SecurityAlert => (
            "Security Alert",
            "Trigger the action when a suspicious sign-in or a credential-stuffing pattern is detected.",
        ),
    }
}

// ── Validation ────────────────────────────────────────────────────────────

fn parse_kind(raw: &str) -> Result<ProviderKind, AppError> {
    ProviderKind::parse(raw)
        .ok_or_else(|| AppError::ValidationError(format!("Unknown notification provider '{raw}'")))
}

fn parse_name(raw: &str) -> Result<String, AppError> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(AppError::ValidationError("Name is required".into()));
    }
    if name.chars().count() > MAX_NAME_LEN {
        return Err(AppError::ValidationError(format!(
            "Name must be {MAX_NAME_LEN} characters or fewer"
        )));
    }
    Ok(name.to_string())
}

/// Parses the requested actions and enforces the root-organization rule for the
/// platform-wide ones.
async fn parse_actions(
    state: &SharedState,
    organization_id: &str,
    user: &AuthUser,
    raw: &[String],
) -> Result<Vec<Action>, AppError> {
    let mut actions = Vec::with_capacity(raw.len());
    for value in raw {
        let action = Action::parse(value)
            .ok_or_else(|| AppError::ValidationError(format!("Unknown action '{value}'")))?;
        if !actions.contains(&action) {
            actions.push(action);
        }
    }

    if dispatch::requires_root_organization(&actions) {
        if !user.is_sysadmin {
            return Err(AppError::InsufficientPermissions);
        }
        if !repository::organization_is_root(state, organization_id).await? {
            return Err(AppError::ValidationError(
                "Platform events can only be sent from the root organization".into(),
            ));
        }
    }
    Ok(actions)
}

async fn to_dto(
    state: &SharedState,
    row: repository::ProviderRow,
) -> Result<ProviderDto, AppError> {
    let actions = repository::list_actions(state, &row.id).await?;
    let last_delivery = repository::latest_delivery(state, &row.id).await?;
    Ok(ProviderDto {
        id: row.id,
        organization_id: row.organization_id,
        name: row.name,
        kind: row.kind.key().to_string(),
        enabled: row.enabled,
        decoration: row.decoration,
        config_hint: row.config_hint,
        actions,
        created_at: row.created_at,
        updated_at: row.updated_at,
        last_delivery,
    })
}

// ── Handlers ──────────────────────────────────────────────────────────────

pub async fn catalog_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgRead>,
) -> Result<impl IntoResponse, AppError> {
    let organization_id = org.organization_id;
    let organization_is_root = repository::organization_is_root(&state, &organization_id).await?;
    Ok(ok(NotificationCatalog {
        providers: ProviderKind::ALL
            .into_iter()
            .map(|kind| CatalogProvider {
                kind: kind.key().to_string(),
                label: kind.label().to_string(),
                uses_webhook_url: kind.uses_webhook_url(),
                allowed_hosts: crate::notifications::url_guard::allowed_hosts_label(kind),
            })
            .collect(),
        actions: Action::ALL
            .into_iter()
            .map(|action| {
                let (label, description) = action_label(action);
                CatalogAction {
                    key: action.key().to_string(),
                    label: label.to_string(),
                    description: description.to_string(),
                    global: action.is_global(),
                }
            })
            .collect(),
        organization_is_root,
    }))
}

pub async fn list_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgRead>,
) -> Result<impl IntoResponse, AppError> {
    let organization_id = org.organization_id;
    let rows = repository::list_providers(&state, &organization_id).await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(to_dto(&state, row).await?);
    }
    Ok(ok(out))
}

pub async fn create_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::NotificationManage>,
    Json(request): Json<CreateProviderRequest>,
) -> Result<impl IntoResponse, AppError> {
    let OrgScope {
        organization_id,
        user,
        ..
    } = org;
    let kind = parse_kind(&request.kind)?;
    let name = parse_name(&request.name)?;
    let actions = parse_actions(&state, &organization_id, &user, &request.actions).await?;
    let config = request.config.into_config();
    dispatch::validate_config(kind, &config)?;

    let provider_id = repository::insert_provider(
        &state,
        NewProvider {
            organization_id: &organization_id,
            name: &name,
            kind,
            enabled: request.enabled,
            decoration: request.decoration,
            config_encrypted: config.encrypt()?,
            config_hint: config.hint(kind),
            created_by: Some(&user.user_id),
        },
    )
    .await?;
    repository::replace_actions(&state, &provider_id, &actions).await?;

    let _ = crate::git::repositories::audit_log::record(
        &state,
        &organization_id,
        &user,
        "notification.provider_created",
        Some("notification_provider"),
        Some(&provider_id),
        Some(&name),
        None,
        None,
        None,
        None,
    )
    .await;

    let row = repository::get_provider(&state, &organization_id, &provider_id)
        .await?
        .ok_or(AppError::ResourceNotFound)?;
    Ok(created(to_dto(&state, row).await?))
}

pub async fn update_handler(
    State(state): State<SharedState>,
    Extension(_user): Extension<AuthUser>,
    RequireOrg(org, _): RequireOrg<perm::NotificationManage>,
    // The extractor covers `organization_id`.
    Path((_, provider_id)): Path<(String, String)>,
    Json(request): Json<UpdateProviderRequest>,
) -> Result<impl IntoResponse, AppError> {
    let OrgScope {
        organization_id,
        user,
        ..
    } = org;
    let existing = repository::get_provider(&state, &organization_id, &provider_id)
        .await?
        .ok_or(AppError::ResourceNotFound)?;
    let name = parse_name(&request.name)?;
    let actions = parse_actions(&state, &organization_id, &user, &request.actions).await?;

    // A blank config means "keep what is stored" — the client never receives
    // the credential back, so it cannot echo it.
    let (config_encrypted, config_hint) = match request.config {
        Some(input) if !input.is_empty() => {
            let config = input.into_config();
            dispatch::validate_config(existing.kind, &config)?;
            (config.encrypt()?, config.hint(existing.kind))
        }
        _ => (
            existing.config_encrypted.clone(),
            existing.config_hint.clone(),
        ),
    };

    repository::update_provider(
        &state,
        &provider_id,
        &name,
        request.enabled,
        request.decoration,
        &config_encrypted,
        &config_hint,
    )
    .await?;
    repository::replace_actions(&state, &provider_id, &actions).await?;

    let _ = crate::git::repositories::audit_log::record(
        &state,
        &organization_id,
        &user,
        "notification.provider_updated",
        Some("notification_provider"),
        Some(&provider_id),
        Some(&name),
        None,
        None,
        None,
        None,
    )
    .await;

    let row = repository::get_provider(&state, &organization_id, &provider_id)
        .await?
        .ok_or(AppError::ResourceNotFound)?;
    Ok(ok(to_dto(&state, row).await?))
}

pub async fn delete_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::NotificationManage>,
    // The extractor covers `organization_id`.
    Path((_, provider_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let organization_id = org.organization_id;
    let user = &org.user;
    let existing = repository::get_provider(&state, &organization_id, &provider_id)
        .await?
        .ok_or(AppError::ResourceNotFound)?;
    let deleted = repository::delete_provider(&state, &organization_id, &provider_id).await?;
    if deleted == 0 {
        return Err(AppError::ResourceNotFound);
    }
    let _ = crate::git::repositories::audit_log::record(
        &state,
        &organization_id,
        user,
        "notification.provider_deleted",
        Some("notification_provider"),
        Some(&provider_id),
        Some(&existing.name),
        None,
        None,
        None,
        None,
    )
    .await;
    Ok(ok(serde_json::json!({ "deleted": true })))
}

/// Sends a real message using the form the user is currently filling in,
/// without persisting anything. Reports the provider's own error text so a
/// revoked webhook reads as "Invalid Webhook Token" rather than "failed".
pub async fn test_unsaved_handler(
    State(state): State<SharedState>,
    Extension(_user): Extension<AuthUser>,
    RequireOrg(org, _): RequireOrg<perm::NotificationManage>,
    Json(request): Json<TestProviderRequest>,
) -> Result<impl IntoResponse, AppError> {
    let user = org.user;
    let kind = parse_kind(&request.kind)?;
    let config = request.config.into_config();
    // Validate before spending quota: a typo should not cost the user a slot.
    dispatch::validate_config(kind, &config)?;
    check_test_quota(&state, &user.user_id).await?;
    Ok(ok(run_test(kind, &config, request.decoration).await))
}

pub async fn test_saved_handler(
    State(state): State<SharedState>,
    Extension(_user): Extension<AuthUser>,
    RequireOrg(org, _): RequireOrg<perm::NotificationManage>,
    // The extractor covers `organization_id`.
    Path((_, provider_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let OrgScope {
        organization_id,
        user,
        ..
    } = org;
    let row = repository::get_provider(&state, &organization_id, &provider_id)
        .await?
        .ok_or(AppError::ResourceNotFound)?;
    let _ = crate::git::repositories::audit_log::record(
        &state,
        &organization_id,
        &user,
        "notification.provider_tested",
        Some("notification_provider"),
        Some(&provider_id),
        Some(&row.name),
        None,
        None,
        None,
        None,
    )
    .await;
    check_test_quota(&state, &user.user_id).await?;
    let config = row.config()?;
    let result = run_test(row.kind, &config, row.decoration).await;
    if result.delivered
        && let Err(error) = crate::onboarding::repository::remember_notification_test(
            &state,
            &user.user_id,
            &organization_id,
            &provider_id,
        )
        .await
    {
        tracing::warn!(%error, "notification test succeeded but onboarding cache was unavailable");
    }
    Ok(ok(result))
}

async fn run_test(kind: ProviderKind, config: &ProviderConfig, decoration: bool) -> TestResult {
    let payload = event::test_event();
    match dispatch::send_once(kind, config, decoration, &payload).await {
        SendOutcome::Delivered => TestResult {
            delivered: true,
            message: format!("Test message delivered to {}", kind.label()),
        },
        SendOutcome::RateLimited { retry_after } => TestResult {
            delivered: false,
            message: format!(
                "{} is rate limiting us — try again in {}s",
                kind.label(),
                retry_after.as_secs()
            ),
        },
        SendOutcome::Transient(error) | SendOutcome::Permanent(error) => TestResult {
            delivered: false,
            message: error,
        },
    }
}

pub async fn deliveries_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgRead>,
    // The extractor covers `organization_id`.
    Path((_, provider_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let organization_id = org.organization_id;
    // Scope check: the provider must belong to this organization.
    repository::get_provider(&state, &organization_id, &provider_id)
        .await?
        .ok_or(AppError::ResourceNotFound)?;
    let deliveries: Vec<DeliveryDto> =
        repository::list_deliveries(&state, &provider_id, DELIVERY_PAGE).await?;
    Ok(ok(deliveries))
}
