use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{apply, check, core, pin};
use crate::state::SharedState;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum UpdateChannel {
    #[default]
    Stable,
    Alpha,
}

impl UpdateChannel {
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "alpha" => Self::Alpha,
            _ => Self::Stable,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Alpha => "alpha",
        }
    }
}

#[derive(Debug, Default, Clone, Serialize, utoipa::ToSchema)]
pub struct UpdateState {
    /// Active channel whose manifest entry was last inspected. `latest_version`
    /// always belongs to this channel.
    pub channel: UpdateChannel,
    /// Version tag the running container was created from, e.g. `v0.1.16`.
    /// Empty string when unknown (e.g. native dev with no container).
    pub current_version: String,
    /// Version the manifest publishes, e.g. `v0.1.19`.
    pub latest_version: String,
    /// Immutable digest the manifest publishes for `latest_version` (`sha256:…`),
    /// or `None` when the channel does not pin one. Present means the updater
    /// pulls by digest; `None` falls back to the mutable tag.
    pub latest_digest: Option<String>,
    /// True when `latest_version` is strictly newer than `current_version`.
    pub update_available: bool,
    /// ISO-8601 timestamp of the last manifest check attempt.
    pub last_check_at: Option<DateTime<Utc>>,
    /// True only when the most recent attempt fetched a valid channel entry
    /// and both versions could be compared.
    pub last_check_succeeded: bool,
    /// Human-readable note about the last check attempt (info or error).
    pub last_check_note: String,
    /// True if the in-process updater loop is running.
    pub updater_running: bool,
    pub core_services: CoreUpdateState,
}

#[derive(Debug, Clone, Default, Serialize, utoipa::ToSchema)]
pub struct CoreServiceUpdateState {
    pub image: String,
    pub current_tag: String,
    pub target_tag: String,
    pub update_available: bool,
    pub ready: bool,
    pub note: String,
    /// Runs as a systemd unit (Quadlet), not a container the updater may recreate.
    pub systemd_managed: bool,
}

#[derive(Debug, Clone, Default, Serialize, utoipa::ToSchema)]
pub struct CoreUpdateState {
    pub supported: bool,
    pub caddy: CoreServiceUpdateState,
    pub dragonfly: CoreServiceUpdateState,
    pub last_apply_note: String,
}

fn pin_differs_from_boot(pin: &pin::Pin, image_tag: &str, update_channel: UpdateChannel) -> bool {
    pin.pinned_tag != image_tag || pin.channel != update_channel
}

fn initialize_update_state(
    update_state: &mut UpdateState,
    update_channel: UpdateChannel,
    interval: u64,
    manifest_url: &str,
    container: &str,
) {
    update_state.channel = update_channel;
    update_state.updater_running = interval > 0;
    update_state.last_check_succeeded = false;
    update_state.last_check_note = if interval > 0 {
        format!(
            "updater enabled: every {interval}s tracking {channel} at {manifest_url} \
             (container={container})",
            channel = update_channel.as_str()
        )
    } else {
        format!(
            "updater disabled on {} (LITEPOD_UPDATE_INTERVAL=0)",
            update_channel.as_str()
        )
    };
}

/// Public entry point: spawn the background updater if enabled.
/// Idempotent: safe to call once at boot.
pub fn spawn(state: SharedState) {
    let interval = state.update_interval_secs;
    let container = state.container_name.clone();
    let manifest_url = state.version_manifest_url.clone();
    let channel = state.update_channel;

    // Compose can recreate the `app` service from the tag in the host `.env`.
    // Surface a persisted pin mismatch, but keep this boot's configuration
    // authoritative for the active channel.
    if let Some(pinned) = pin::read(&state.data_dir) {
        if pin_differs_from_boot(&pinned, &state.image_tag, channel) {
            tracing::warn!(
                pinned = %pinned.pinned_tag,
                env_tag = %state.image_tag,
                pinned_channel = pinned.channel.as_str(),
                configured_channel = channel.as_str(),
                "updater: persisted pin differs from boot image/config; current configuration remains authoritative"
            );
        }
    }

    {
        // `spawn` runs synchronously inside the async boot sequence (called
        // directly from `async_main`, not off the runtime), so `blocking_write`
        // would panic here — nothing else holds this lock yet at boot, so a
        // non-blocking `try_write` is safe.
        let mut s = state
            .update_state
            .try_write()
            .expect("update_state lock is uncontended at boot");
        initialize_update_state(&mut s, channel, interval, &manifest_url, &container);
    }

    if interval == 0 {
        tracing::info!("litepod self-update disabled (LITEPOD_UPDATE_INTERVAL=0)");
        return;
    }

    let state_for_task = state.clone();
    tokio::spawn(async move {
        run_loop(state_for_task, Duration::from_secs(interval)).await;
    });
}

async fn run_loop(state: SharedState, interval: Duration) {
    // First check 30s after boot to let the API settle, then every `interval`.
    let initial = Duration::from_secs(30).min(interval);
    tokio::time::sleep(initial).await;

    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // First tick fires immediately after the sleep above.
    ticker.tick().await;

    loop {
        // Detection only. Installing is always an explicit user action: this
        // is a control plane, and recreating it under someone mid-deploy is a
        // bad time. Dokploy and Coolify both notify and wait for a click.
        detect_once(&state).await;
        ticker.tick().await;
    }
}

/// The version tag this container is running, from the tag it was created
/// with. Falls back to the `LITEPOD_VERSION` build arg when there is no
/// container to inspect.
async fn running_version(state: &SharedState) -> String {
    match apply::current_image_tag(&state.container_name, &state.podman_socket).await {
        Ok(tag) if !tag.is_empty() => tag,
        Ok(_) => state.version.clone(),
        Err(error) => {
            tracing::warn!(%error, "updater: could not read the running image tag");
            state.version.clone()
        }
    }
}

/// One detection cycle. Fetches the manifest, compares the channel's published
/// version against the running one, and persists the result in `UpdateState`.
/// Never applies. Errors are logged and persisted in `last_check_note`; they
/// never panic or break the API. Returns whether an update is available.
pub async fn detect_once(state: &SharedState) -> bool {
    let channel = state.update_channel;
    tracing::info!(
        url = %state.version_manifest_url,
        channel = channel.as_str(),
        "updater: fetching release manifest"
    );

    let manifest = match check::fetch_manifest(&state.version_manifest_url).await {
        Ok(m) => m,
        Err(error) => {
            tracing::warn!(%error, "updater: manifest lookup failed");
            let mut s = state.update_state.write().await;
            s.channel = channel;
            s.update_available = false;
            s.last_check_at = Some(Utc::now());
            s.last_check_succeeded = false;
            s.latest_digest = None;
            s.last_check_note = format!("manifest lookup failed for {}: {error}", channel.as_str());
            return false;
        }
    };

    let entry = match manifest.channel(channel) {
        Ok(entry) => entry,
        Err(error) => {
            let mut snapshot = state.update_state.write().await;
            snapshot.channel = channel;
            snapshot.last_check_at = Some(Utc::now());
            snapshot.last_check_succeeded = false;
            snapshot.last_check_note = error.to_string();
            snapshot.update_available = false;
            snapshot.latest_digest = None;
            return false;
        }
    };
    // A pinned digest is part of the same integrity decision as the version:
    // an unreadable digest is refused, never silently downgraded to the tag.
    let app_digest = match entry.app_digest() {
        Ok(digest) => digest,
        Err(error) => {
            let mut snapshot = state.update_state.write().await;
            snapshot.channel = channel;
            snapshot.last_check_at = Some(Utc::now());
            snapshot.last_check_succeeded = false;
            snapshot.last_check_note = error.to_string();
            snapshot.update_available = false;
            snapshot.latest_digest = None;
            return false;
        }
    };
    let core_targets = match entry.core_targets() {
        Ok(targets) => targets,
        Err(error) => {
            let mut snapshot = state.update_state.write().await;
            snapshot.channel = channel;
            snapshot.last_check_at = Some(Utc::now());
            snapshot.last_check_succeeded = false;
            snapshot.last_check_note = error.to_string();
            snapshot.update_available = false;
            snapshot.latest_digest = None;
            return false;
        }
    };

    let current = running_version(state).await;
    let latest = entry.version;

    let (Some(current_parsed), Some(latest_parsed)) = (
        check::parse_version(&current),
        check::parse_version(&latest),
    ) else {
        let note = format!(
            "unparseable version on {} (current={current}, latest={latest})",
            channel.as_str()
        );
        tracing::warn!(note, "updater: cannot compare versions");
        let mut s = state.update_state.write().await;
        s.channel = channel;
        s.current_version = current;
        s.latest_version = latest;
        s.update_available = false;
        s.last_check_at = Some(Utc::now());
        s.last_check_succeeded = false;
        s.latest_digest = None;
        s.last_check_note = note;
        return false;
    };

    let app_needs_update = latest_parsed > current_parsed;
    let core_services = core::detect_core_services(state, &core_targets).await;
    let needs_update = app_needs_update
        || core_services.caddy.update_available
        || core_services.dragonfly.update_available;

    {
        let mut s = state.update_state.write().await;
        s.channel = channel;
        s.current_version = current.clone();
        s.latest_version = latest.clone();
        s.latest_digest = app_digest.clone();
        s.core_services = core_services;
        s.update_available = needs_update;
        s.last_check_at = Some(Utc::now());
        s.last_check_succeeded = true;
        s.last_check_note = if needs_update {
            format!(
                "update available on {}: {current} -> {latest}",
                channel.as_str()
            )
        } else {
            format!("up to date on {} ({current})", channel.as_str())
        };
    }

    needs_update
}

/// Pulls the selected version and persists its boot configuration before
/// retiring this process. The caller checks `update_available` first.
pub async fn apply_now(
    state: &SharedState,
    target: String,
    channel: UpdateChannel,
    digest: Option<String>,
) -> Result<(), apply::ApplyError> {
    if target.is_empty() {
        return Err(apply::ApplyError::Recreate(
            "no target version recorded — run a check first".into(),
        ));
    }

    // Pin by immutable digest when the manifest publishes one; otherwise fall
    // back to the (mutable) tag. A digest reference cannot be re-pointed, so a
    // compromised registry account cannot substitute a different image.
    let image = match digest {
        Some(digest) => format!("{}@{}", state.image_repo, digest),
        None => format!("{}:{}", state.image_repo, target),
    };
    tracing::warn!(
        container = %state.container_name,
        image = %image,
        "updater: pulling and recreating"
    );

    let record = pin::Pin {
        pinned_tag: target.clone(),
        channel,
        applied_at: Utc::now(),
    };
    apply::pull_and_recreate(&state.container_name, &image, &state.podman_socket, || {
        let env = state
            .stack_env_file
            .clone()
            .map(super::stack_env::StackEnv::open)
            .transpose()
            .map_err(|error| apply::ApplyError::Persist(error.to_string()))?;
        let previous_pin = pin::read(&state.data_dir);
        pin::write(&state.data_dir, &record)
            .map_err(|error| apply::ApplyError::Persist(error.to_string()))?;
        // This must run inside the callback: retiring our own container kills
        // this process. A configured .env is required to persist successfully.
        if let Some(env) = env {
            if let Err(error) = env.set_app_tag(&target) {
                let rollback = match previous_pin {
                    Some(previous) => pin::write(&state.data_dir, &previous),
                    None => std::fs::remove_file(pin::path(&state.data_dir)),
                };
                if let Err(rollback_error) = rollback {
                    return Err(apply::ApplyError::Persist(format!(
                        "{error}; could not restore previous update pin: {rollback_error}"
                    )));
                }
                return Err(apply::ApplyError::Persist(error.to_string()));
            }
        }
        Ok(())
    })
    .await
}

pub async fn apply_release(
    state: &SharedState,
    target: String,
    channel: UpdateChannel,
    core_targets: check::CoreServiceTargets,
    app_digest: Option<String>,
) -> Result<(), core::CoreUpdateError> {
    core::apply_core_services(state, core_targets).await?;
    if running_version(state).await == target {
        return Ok(());
    }
    apply_now(state, target, channel, app_digest)
        .await
        .map_err(|error| core::CoreUpdateError::Message(error.to_string()))
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::{UpdateChannel, UpdateState, initialize_update_state, pin, pin_differs_from_boot};

    #[tokio::test]
    async fn app_update_persists_before_retirement_and_rolls_back_on_failure() {
        use axum::{
            Json, Router,
            http::{Method, StatusCode, Uri},
            response::IntoResponse,
        };
        use serde_json::json;
        use std::{
            fs,
            sync::{Arc, Mutex},
        };

        for failure in [
            "none",
            "env",
            "missing_env",
            "pin",
            "start",
            "env_without_pin",
            "no_env",
        ] {
            let (mut state, db_path) = crate::testing::test_state().await;
            let dir = std::env::temp_dir().join(format!("litepod-update-{}", uuid::Uuid::now_v7()));
            fs::create_dir(&dir).unwrap();
            let env_path = dir.join(".env");
            let old_env = "LITEPOD_IMAGE_TAG=v0.1.56\nCADDY_IMAGE_TAG=2.11.4\nHASH_SECRET=keep\n";
            fs::write(&env_path, old_env).unwrap();
            let old_pin = pin::Pin {
                pinned_tag: "v0.1.56".into(),
                channel: UpdateChannel::Stable,
                applied_at: Utc::now(),
            };
            if failure != "env_without_pin" {
                pin::write(&dir, &old_pin).unwrap();
            }
            match failure {
                "env" | "env_without_pin" => {
                    fs::create_dir(env_path.with_extension("env.tmp")).unwrap()
                }
                "pin" => fs::create_dir(pin::path(&dir).with_extension("json.tmp")).unwrap(),
                "missing_env" => fs::remove_file(&env_path).unwrap(),
                _ => (),
            }
            let calls = Arc::new(Mutex::new(Vec::new()));
            let requests = calls.clone();
            let saved_dir = dir.clone();
            let router =
                Router::new().fallback(move |method: Method, uri: Uri| {
                    let requests = requests.clone();
                    let dir = saved_dir.clone();
                    async move {
                        let path = uri.path();
                        requests.lock().unwrap().push(format!("{method} {uri}"));
                        if path.ends_with("/containers/litepod-api/json") {
                            return Json(json!({"Config": {}, "HostConfig": {}})).into_response();
                        }
                        if path.ends_with("/images/pull") {
                            return "{}\n".into_response();
                        }
                        if path.ends_with("/containers/create") {
                            return Json(json!({"Id":"new", "Warnings":[]})).into_response();
                        }
                        if path.ends_with("/start") && failure == "start" {
                            return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({
                            "cause":"start failed", "message":"start failed", "response":500,
                        }))).into_response();
                        }
                        if path.ends_with("/containers/json") {
                            return Json(json!([])).into_response();
                        }
                        if method == Method::DELETE && path.contains("/litepod-api-old-") {
                            // A self-update dies here in production: nothing after this
                            // request may be needed to preserve the selected version.
                            assert!(matches!(failure, "none" | "no_env"));
                            let tag = if failure == "no_env" {
                                "v0.1.56"
                            } else {
                                "v0.1.72"
                            };
                            assert!(
                                fs::read_to_string(dir.join(".env"))
                                    .unwrap()
                                    .contains(&format!("LITEPOD_IMAGE_TAG={tag}\n"))
                            );
                            assert_eq!(pin::read(&dir).unwrap().pinned_tag, "v0.1.72");
                        }
                        StatusCode::NO_CONTENT.into_response()
                    }
                });
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
            let config = Arc::get_mut(&mut state).unwrap();
            config.podman_socket = format!("http://{address}");
            config.container_name = "litepod-api".into();
            config.image_repo = "docker.io/litepod/litepod".into();
            config.data_dir = dir.clone();
            config.stack_env_file = (failure != "no_env").then_some(env_path.clone());
            let result = super::apply_now(&state, "v0.1.72".into(), UpdateChannel::Stable, None).await;
            let calls = calls.lock().unwrap();
            if matches!(failure, "none" | "no_env") {
                assert!(result.is_ok(), "{failure}: {result:?}; {calls:?}");
                assert!(
                    calls
                        .iter()
                        .any(|call| call.starts_with("DELETE ")
                            && call.contains("/litepod-api-old-"))
                );
            } else {
                assert!(result.is_err(), "{failure}");
                if failure != "start" {
                    assert!(
                        matches!(result, Err(super::apply::ApplyError::Persist(_))),
                        "{failure}: {result:?}"
                    );
                }
                if failure != "missing_env" {
                    assert_eq!(fs::read_to_string(&env_path).unwrap(), old_env);
                }
                assert_eq!(
                    pin::read(&dir).map(|p| p.pinned_tag),
                    (failure != "env_without_pin").then(|| "v0.1.56".into())
                );
                assert!(
                    !calls
                        .iter()
                        .any(|call| call.starts_with("DELETE ")
                            && call.contains("/litepod-api-old-"))
                );
                assert!(
                    calls
                        .iter()
                        .any(|call| call.starts_with("DELETE ") && call.contains("/litepod-api?"))
                );
                assert!(calls.last().unwrap().contains("/rename?name=litepod-api"));
            }
            server.abort();
            state.db.close().await;
            crate::testing::cleanup_database(&db_path);
            fs::remove_dir_all(&dir).unwrap();
        }
    }

    #[test]
    fn spawn_initialization_uses_the_configured_app_state_channel() {
        let mut update_state = UpdateState::default();

        initialize_update_state(
            &mut update_state,
            UpdateChannel::Alpha,
            60,
            "https://example.test/version.json",
            "litepod-api",
        );

        assert_eq!(update_state.channel, UpdateChannel::Alpha);
        assert!(update_state.updater_running);
        assert!(update_state.last_check_note.contains("tracking alpha"));
    }

    #[test]
    fn pin_channel_mismatch_does_not_replace_the_configured_active_channel() {
        let configured_channel = UpdateChannel::Alpha;
        let pin = pin::Pin {
            pinned_tag: "v0.1.22".into(),
            channel: UpdateChannel::Stable,
            applied_at: Utc::now(),
        };
        let mut update_state = UpdateState::default();

        assert!(pin_differs_from_boot(&pin, "v0.1.22", configured_channel));
        initialize_update_state(
            &mut update_state,
            configured_channel,
            0,
            "https://example.test/version.json",
            "litepod-api",
        );

        assert_eq!(update_state.channel, UpdateChannel::Alpha);
        assert!(!update_state.updater_running);
    }
}
