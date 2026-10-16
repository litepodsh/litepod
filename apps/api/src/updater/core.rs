use std::time::Duration;

use chrono::Utc;
use thiserror::Error;

use crate::state::SharedState;

use super::{
    CoreServiceUpdateState, CoreUpdateState, apply,
    check::CoreServiceTargets,
    core_state::{self, CoreUpdateJournal, CoreUpdatePhase},
    stack_env::{PreviousCoreTags, StackEnv},
};

pub const CADDY_IMAGE_REPO: &str = "docker.io/library/caddy";
pub const DRAGONFLY_IMAGE_REPO: &str = "docker.dragonflydb.io/dragonflydb/dragonfly";

#[derive(Debug, Error)]
pub enum CoreUpdateError {
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Apply(#[from] apply::ApplyError),
    #[error(transparent)]
    Stack(#[from] super::stack_env::StackEnvError),
}

pub async fn detect_core_services(
    state: &SharedState,
    targets: &CoreServiceTargets,
) -> CoreUpdateState {
    let supported = state.stack_env_file.is_some();
    let mut caddy = inspect_service(
        &state.caddy_container,
        CADDY_IMAGE_REPO,
        &targets.caddy,
        state,
    )
    .await;
    if apply::is_systemd_managed(&state.caddy_container, &state.podman_socket).await {
        caddy.update_available = false;
        caddy.systemd_managed = true;
    }
    let dragonfly = inspect_service(
        &std::env::var("DRAGONFLY_CONTAINER").unwrap_or_else(|_| "litepod-dragonfly".into()),
        DRAGONFLY_IMAGE_REPO,
        &targets.dragonfly,
        state,
    )
    .await;
    CoreUpdateState {
        supported,
        caddy,
        dragonfly,
        last_apply_note: if supported {
            String::new()
        } else {
            "Run the updated installer once to enable persistent core-service updates on this server.".into()
        },
    }
}

async fn inspect_service(
    container: &str,
    repo: &str,
    target: &str,
    state: &SharedState,
) -> CoreServiceUpdateState {
    match apply::current_image_tag(container, &state.podman_socket).await {
        Ok(current_tag) => CoreServiceUpdateState {
            image: format!("{repo}:{target}"),
            update_available: current_tag != target,
            ready: true,
            current_tag,
            target_tag: target.into(),
            note: String::new(),
            systemd_managed: false,
        },
        Err(error) => CoreServiceUpdateState {
            image: format!("{repo}:{target}"),
            target_tag: target.into(),
            note: error.to_string(),
            ..Default::default()
        },
    }
}

pub async fn apply_core_services(
    state: &SharedState,
    targets: CoreServiceTargets,
) -> Result<(), CoreUpdateError> {
    let path = state.stack_env_file.clone().ok_or_else(|| {
        CoreUpdateError::Message(
            "persistent core updates are unavailable; run the installer once".into(),
        )
    })?;
    let current = detect_core_services(state, &targets).await;
    if !current.caddy.ready || !current.dragonfly.ready {
        return Err(CoreUpdateError::Message(
            "could not inspect core service images".into(),
        ));
    }
    let caddy_changed = current.caddy.update_available;
    let dragonfly_changed = current.dragonfly.update_available;
    if !caddy_changed && !dragonfly_changed {
        return Ok(());
    }
    let env = StackEnv::open(path)?;
    let previous = PreviousCoreTags {
        caddy: Some(current.caddy.current_tag.clone()),
        dragonfly: Some(current.dragonfly.current_tag.clone()),
    };
    journal(
        state,
        targets.clone(),
        previous.clone(),
        CoreUpdatePhase::Preflight,
        None,
    );
    let persisted = env.replace_core_tags(&targets)?;
    journal(
        state,
        targets.clone(),
        persisted.clone(),
        CoreUpdatePhase::Persisted,
        None,
    );
    let result = async {
        if dragonfly_changed {
            let container =
                std::env::var("DRAGONFLY_CONTAINER").unwrap_or_else(|_| "litepod-dragonfly".into());
            apply::pull_and_recreate(
                &container,
                &format!("{DRAGONFLY_IMAGE_REPO}:{}", targets.dragonfly),
                &state.podman_socket,
                || Ok(()),
            )
            .await?;
            wait_for_dragonfly().await?;
            journal(
                state,
                targets.clone(),
                persisted.clone(),
                CoreUpdatePhase::DragonflyRecreated,
                None,
            );
        }
        if caddy_changed {
            apply::pull_and_recreate(
                &state.caddy_container,
                &format!("{CADDY_IMAGE_REPO}:{}", targets.caddy),
                &state.podman_socket,
                || Ok(()),
            )
            .await?;
            wait_for_caddy(state).await?;
            // Reconcile after resuming the saved config too: older installs or a
            // missing autosave may still have only the bootstrap Caddyfile.
            if let Err(error) = crate::handler::settings::restore_caddy(state).await {
                tracing::warn!(%error, "could not restore the server domain after the Caddy update");
            }
            match crate::deployment::domains::reconcile_all_routes(state).await {
                Ok((pushed, failed)) => {
                    tracing::info!(pushed, failed, "reconciled application routes after the Caddy update")
                }
                Err(error) => {
                    tracing::warn!(%error, "could not reconcile application routes after the Caddy update")
                }
            }
            journal(
                state,
                targets.clone(),
                persisted.clone(),
                CoreUpdatePhase::CaddyRecreated,
                None,
            );
        }
        Ok::<(), CoreUpdateError>(())
    }
    .await;
    if let Err(error) = result {
        let _ = env.restore(&persisted);
        journal(
            state,
            targets,
            persisted,
            CoreUpdatePhase::RollbackRequired,
            Some(error.to_string()),
        );
        return Err(error);
    }
    journal(state, targets, persisted, CoreUpdatePhase::Completed, None);
    Ok(())
}

async fn wait_for_dragonfly() -> Result<(), CoreUpdateError> {
    // Container name, not the `dragonfly` Compose service name: deployed apps join
    // the same network and a service named `dragonfly` would claim that DNS alias.
    let url =
        std::env::var("DRAGONFLY_URL").unwrap_or_else(|_| "redis://litepod-dragonfly:6379".into());
    for _ in 0..60 {
        if let Ok(client) = redis::Client::open(url.as_str()) {
            if let Ok(mut connection) = client.get_multiplexed_async_connection().await {
                if redis::cmd("PING")
                    .query_async::<redis::Value>(&mut connection)
                    .await
                    .is_ok()
                {
                    return Ok(());
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Err(CoreUpdateError::Message(
        "Dragonfly did not become ready within 30 seconds".into(),
    ))
}

fn journal(
    state: &SharedState,
    targets: CoreServiceTargets,
    previous_tags: PreviousCoreTags,
    phase: CoreUpdatePhase,
    error: Option<String>,
) {
    if let Err(error) = core_state::write(
        &state.data_dir,
        &CoreUpdateJournal {
            targets,
            previous_tags,
            phase,
            error,
            updated_at: Utc::now(),
        },
    ) {
        tracing::warn!(%error, "core update: could not persist journal");
    }
}

async fn wait_for_caddy(state: &SharedState) -> Result<(), CoreUpdateError> {
    let (client, base) = crate::deployment::caddy_admin::client(&state.caddy_admin_url, None)
        .map_err(CoreUpdateError::Message)?;
    let url = format!("{base}/config/");
    for _ in 0..60 {
        if client
            .get(&url)
            .send()
            .await
            .is_ok_and(|r| r.status().is_success())
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Err(CoreUpdateError::Message(
        "Caddy did not become ready within 30 seconds".into(),
    ))
}
