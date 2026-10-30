use std::{
    ffi::OsString,
    path::{Component, Path},
    sync::mpsc::Sender,
};

use tokio::io::{AsyncBufReadExt, BufReader};

use chrono::Utc;
use futures_util::StreamExt;
use podman_api::{Podman, opts::ImageBuildOpts};

use crate::{dtos::response::AppError, git::git_providers, state::state::SharedState};

use super::{BUILD_CPU_PERIOD_US, decrypted_environment, resource_limits, resource_log_line};

pub mod compose;
pub mod railpack;

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

fn image_tag(application_id: &str) -> String {
    format!("litepod-app-{application_id}:latest")
}

pub(crate) fn repo_dir(state: &SharedState, application_id: &str) -> std::path::PathBuf {
    crate::servers::replica::scoped_path(state.data_dir.join("repos").join(application_id))
}

fn safe_repo_path(repo_root: &str, configured_path: &str) -> Result<String, String> {
    let path = Path::new(configured_path);
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| !matches!(component, Component::CurDir | Component::Normal(_)))
    {
        return Err("Dockerfile path must stay inside the repository".into());
    }
    Ok(format!("{repo_root}/{configured_path}"))
}

fn environment_file_contents(environment: &[(String, String)]) -> Result<String, String> {
    environment
        .iter()
        .map(|(name, value)| {
            if value.contains(['\n', '\r']) {
                Err(format!(
                    "Runtime variable {name} contains a newline and cannot be used in an env file"
                ))
            } else {
                Ok(format!("{name}={value}\n"))
            }
        })
        .collect()
}

fn railpack_container_context(project_path: &str) -> Result<String, String> {
    let path = Path::new(project_path);
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| !matches!(component, Component::CurDir | Component::Normal(_)))
    {
        return Err("Railpack project path must stay inside the repository".into());
    }
    let suffix = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => value.to_str(),
            Component::CurDir => None,
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/");
    Ok(if suffix.is_empty() {
        "/workspace/source".into()
    } else {
        format!("/workspace/source/{suffix}")
    })
}

/// Initializes and fetches every submodule the checked-out tree declares,
/// recursively (a submodule can itself have submodules). Reuses the parent
/// repo's own credentials - the common case where private submodules live on
/// the same host/org as the parent - rather than asking for a second set.
fn update_submodules(
    repo: &git2::Repository,
    credentials: Option<(&str, &str)>,
) -> Result<(), String> {
    let submodules = repo.submodules().map_err(|e| e.to_string())?;
    for mut submodule in submodules {
        submodule.init(true).map_err(|e| e.to_string())?;
        let mut callbacks = git2::RemoteCallbacks::new();
        if let Some((username, password)) = credentials {
            let username = username.to_string();
            let password = password.to_string();
            callbacks.credentials(move |_url, username_from_url, _allowed| {
                git2::Cred::userpass_plaintext(username_from_url.unwrap_or(&username), &password)
            });
        }
        let mut fetch = git2::FetchOptions::new();
        fetch.remote_callbacks(callbacks);
        let mut update_opts = git2::SubmoduleUpdateOptions::new();
        update_opts.fetch(fetch);
        submodule
            .update(true, Some(&mut update_opts))
            .map_err(|e| format!("Submodule '{}': {e}", submodule.name().unwrap_or("?")))?;
        // Recurse: the submodule's own working tree may declare its own
        // submodules, only discoverable once `update` has checked it out.
        if let Ok(nested) = submodule.open() {
            update_submodules(&nested, credentials)?;
        }
    }
    Ok(())
}

/// Best-effort `git lfs pull`: skipped entirely (not an error) when the tree
/// has no `.gitattributes` LFS filter, or when the `git-lfs` binary is
/// missing (older/slimmer runtime images). Real LFS content only matters to
/// the app's own build, so a missing binary degrades to pointer files
/// checked out as plain text rather than failing the whole deploy.
///
/// ponytail: shells out instead of a native LFS client because libgit2 has
/// none; runs in a scratch clone of the credentials into the remote URL for
/// this one subprocess, never persisted to `.git/config`.
fn pull_lfs_if_needed(destination: &Path, credentials: Option<(&str, &str)>) -> Result<(), String> {
    let uses_lfs = std::fs::read_to_string(destination.join(".gitattributes"))
        .map(|content| content.contains("filter=lfs"))
        .unwrap_or(false);
    if !uses_lfs {
        return Ok(());
    }
    if std::process::Command::new("git-lfs")
        .arg("version")
        .output()
        .is_err()
    {
        tracing::warn!(
            destination = %destination.display(),
            "repository declares Git LFS filters but the git-lfs binary is unavailable; pointer files were checked out instead of real content"
        );
        return Ok(());
    }
    let original_url = std::process::Command::new("git")
        .args([
            "-C",
            &destination.to_string_lossy(),
            "remote",
            "get-url",
            "origin",
        ])
        .output()
        .map_err(|e| format!("Could not read the git remote for LFS: {e}"))
        .and_then(|output| {
            if output.status.success() {
                Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
            } else {
                Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
            }
        })?;
    if let Some((username, password)) = credentials {
        if let Ok(mut url) = url::Url::parse(&original_url) {
            let _ = url.set_username(username);
            let _ = url.set_password(Some(password));
            let _ = std::process::Command::new("git")
                .args([
                    "-C",
                    &destination.to_string_lossy(),
                    "remote",
                    "set-url",
                    "origin",
                    url.as_str(),
                ])
                .status();
        }
    }
    let result = std::process::Command::new("git")
        .args(["-C", &destination.to_string_lossy(), "lfs", "pull"])
        .output();
    // Always restore the plain URL, even on failure, so no credential lingers
    // in `.git/config` past this one subprocess call.
    let _ = std::process::Command::new("git")
        .args([
            "-C",
            &destination.to_string_lossy(),
            "remote",
            "set-url",
            "origin",
            &original_url,
        ])
        .status();
    match result {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => Err(format!(
            "git lfs pull failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )),
        Err(e) => Err(format!("Could not run git lfs pull: {e}")),
    }
}

/// Blocking shallow clone; call inside `tokio::task::spawn_blocking`. Returns
/// the checked-out commit's full SHA and one-line summary - normally
/// `branch`'s tip, or `target_sha` once pinned to it below.
fn clone_repo(
    repo_url: &str,
    branch: &str,
    target_sha: Option<&str>,
    credentials: Option<(&str, &str)>,
    destination: &Path,
    progress_tx: Option<Sender<String>>,
) -> Result<(String, String, Option<String>, Option<String>), String> {
    let password = credentials.map(|(_, p)| p).unwrap_or("");
    let username = credentials.map(|(u, _)| u.to_string());
    let password_owned = credentials.map(|(_, p)| p.to_string());
    let mut last_phase = "";
    let mut last_percent = -1;

    // Retry on macOS APFS race condition where git2 tries to create lock files
    // before the pack directory exists internally.
    let mut last_err = None;
    for attempt in 0..3 {
        if attempt > 0 {
            let _ = std::fs::remove_dir_all(destination);
            std::thread::sleep(std::time::Duration::from_millis(300));
        }

        let username = username.clone();
        let password_owned = password_owned.clone();
        // The clone's own callbacks consume these below; the pin-fetch step
        // after a successful clone needs its own copy of the same credentials.
        let pin_username = username.clone();
        let pin_password = password_owned.clone();
        let checkout_tx = progress_tx.clone();
        let mut checkout_percent = -1;

        let mut callbacks = git2::RemoteCallbacks::new();
        if let (Some(username), Some(password_owned)) = (username, password_owned) {
            callbacks.credentials(move |_url, username_from_url, _allowed| {
                git2::Cred::userpass_plaintext(
                    username_from_url.unwrap_or(&username),
                    &password_owned,
                )
            });
        }

        let mut checkout = git2::build::CheckoutBuilder::new();
        checkout.progress(move |_path, current, total| {
            if let Some(tx) = &checkout_tx {
                if total > 0 {
                    let percent = (current * 100 / total) as i32;
                    if percent != checkout_percent
                        && (percent == 0 || percent == 100 || percent % 5 == 0)
                    {
                        checkout_percent = percent;
                        let _ = tx.send(format!(
                            "Checking out files: {percent}% ({current}/{total})"
                        ));
                    }
                }
            }
        });

        let progress_tx = progress_tx.clone();
        callbacks.transfer_progress(move |stats| {
            let total = stats.total_objects();
            if let Some(tx) = &progress_tx {
                if total > 0 {
                    let (phase, current, phase_total) = if stats.received_objects() < total {
                        ("Receiving objects", stats.received_objects(), total)
                    } else if stats.indexed_objects() < total {
                        ("Indexing objects", stats.indexed_objects(), total)
                    } else if stats.total_deltas() > 0
                        && stats.indexed_deltas() < stats.total_deltas()
                    {
                        (
                            "Resolving deltas",
                            stats.indexed_deltas(),
                            stats.total_deltas(),
                        )
                    } else {
                        ("Processing objects", total, total)
                    };
                    let percent = (current * 100 / phase_total) as i32;
                    if phase != last_phase {
                        last_phase = phase;
                        last_percent = -1;
                    }
                    if percent != last_percent
                        && (percent == 0 || percent == 100 || percent % 5 == 0)
                    {
                        last_percent = percent;
                        let _ = tx.send(format!("{phase}: {percent}% ({current}/{phase_total})"));
                    }
                }
            } else {
                let _ = last_phase;
                let _ = last_percent;
            }
            true
        });

        let mut fetch = git2::FetchOptions::new();
        fetch.remote_callbacks(callbacks);
        fetch.depth(1);
        let mut builder = git2::build::RepoBuilder::new();
        builder.fetch_options(fetch);
        builder.branch(branch);
        builder.with_checkout(checkout);

        match builder.clone(repo_url, destination) {
            Ok(repo) => {
                let commit = repo
                    .head()
                    .and_then(|head| head.peel_to_commit())
                    .map_err(|e| e.to_string())?;
                // Rollback: the shallow clone above only has `branch`'s tip, not
                // an arbitrary older commit. Pull that one exact commit on top
                // (most Git hosts allow fetching a reachable SHA directly, even
                // though it wasn't advertised as a branch/tag tip) and check it
                // out instead of the branch tip we just cloned.
                let commit = match target_sha {
                    Some(sha) if sha != commit.id().to_string() => {
                        let mut remote = repo.find_remote("origin").map_err(|e| e.to_string())?;
                        let mut pin_callbacks = git2::RemoteCallbacks::new();
                        if let (Some(pin_username), Some(pin_password)) =
                            (pin_username, pin_password)
                        {
                            pin_callbacks.credentials(move |_url, username_from_url, _allowed| {
                                git2::Cred::userpass_plaintext(
                                    username_from_url.unwrap_or(&pin_username),
                                    &pin_password,
                                )
                            });
                        }
                        let mut pin_fetch = git2::FetchOptions::new();
                        pin_fetch.depth(1);
                        pin_fetch.remote_callbacks(pin_callbacks);
                        remote.fetch(&[sha], Some(&mut pin_fetch), None).map_err(|e| {
                            format!(
                                "Could not fetch commit {sha}: {e} — the remote may no longer have this commit, or may not allow fetching by exact commit SHA"
                            )
                        })?;
                        let oid = git2::Oid::from_str(sha)
                            .map_err(|e| format!("Invalid commit sha {sha}: {e}"))?;
                        let target_commit = repo.find_commit(oid).map_err(|e| {
                            format!("Commit {sha} was not found in the repository after fetching it: {e}")
                        })?;
                        repo.checkout_tree(
                            target_commit.as_object(),
                            Some(git2::build::CheckoutBuilder::new().force()),
                        )
                        .map_err(|e| e.to_string())?;
                        repo.set_head_detached(oid).map_err(|e| e.to_string())?;
                        target_commit
                    }
                    _ => commit,
                };
                update_submodules(&repo, credentials)?;
                pull_lfs_if_needed(destination, credentials)?;
                let author = commit.author();
                return Ok((
                    commit.id().to_string(),
                    commit.summary().unwrap_or("").to_string(),
                    author.name().map(str::to_string),
                    author.email().map(str::to_string),
                ));
            }
            Err(e) => {
                let msg = e.to_string();
                if msg.contains(".lock") && msg.contains("No such file or directory") {
                    last_err = Some(e);
                    continue;
                }
                return Err(git_providers::sanitize_git_error(&msg, password));
            }
        }
    }
    Err(git_providers::sanitize_git_error(
        &last_err.unwrap().to_string(),
        password,
    ))
}

pub async fn enqueue(
    state: &SharedState,
    organization_id: &str,
    project_id: &str,
    environment_id: &str,
    application_id: &str,
    triggered_by: &str,
    created_by: Option<&str>,
) -> Result<super::queue::DeploymentJobDto, AppError> {
    let source = super::source::get(state, application_id).await?;
    let missing = (!source.source_upload
        && (source.git_url.as_deref().unwrap_or("").trim().is_empty()
            || source.git_branch.as_deref().unwrap_or("").trim().is_empty()))
        || source.container_port.is_none();
    let method_ready = match source.build_method.as_str() {
        "docker" => source
            .dockerfile_path
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty()),
        "compose" => {
            source
                .compose_file_path
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
                && source
                    .compose_public_service
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty())
        }
        "railpack" => true,
        _ => false,
    };
    if missing || !method_ready {
        return Err(AppError::ValidationError(
            "Configure the Git source and selected build method before deploying".into(),
        ));
    }
    super::ensure_deployment(state, application_id).await?;
    let (container_name, _) = super::identity(state, application_id).await?;
    let run_id = super::history::start(
        state,
        application_id,
        "git",
        &source.build_method,
        triggered_by,
        source.git_branch.as_deref(),
        created_by,
    )
    .await?;
    let snapshot =
        serde_json::to_string(&source).map_err(|e| AppError::InternalError(e.to_string()))?;
    let conn = crate::git::repositories::db::get_conn(state).await;
    crate::git::repositories::db::retry_on_stale_snapshot(|| async {
        sqlx::query("UPDATE application_deployments SET status = 'deploying', container_name = CASE WHEN deployed_at IS NULL THEN ?1 ELSE container_name END, last_error = NULL, updated_at = ?2 WHERE application_id = ?3")
            .bind(container_name.clone())
            .bind(now_ms())
            .bind(application_id)
            .execute(&conn)
            .await
            .map_err(|e| AppError::DatabaseError(e.to_string()))
    })
    .await?;
    super::queue::enqueue(
        state,
        super::queue::EnqueueJobInput {
            application_id: application_id.into(),
            project_id: project_id.into(),
            environment_id: environment_id.into(),
            organization_id: organization_id.into(),
            source_type: "git".into(),
            source_snapshot: snapshot.clone(),
            build_method: source.build_method.clone(),
            build_snapshot: snapshot,
            run_id,
        },
    )
    .await
}

/// Redeploys the exact commit a previous successful run built, instead of
/// the branch's current tip. Everything else (build method, resource
/// limits, env vars, domains) stays whatever is configured *now* - only the
/// source commit is pinned to the past. `pipeline` does the actual pin via
/// `history::pin_commit`, which it reads back at the top of the clone step.
pub async fn enqueue_rollback(
    state: &SharedState,
    organization_id: &str,
    project_id: &str,
    environment_id: &str,
    application_id: &str,
    target_sha: &str,
    created_by: Option<&str>,
) -> Result<super::queue::DeploymentJobDto, AppError> {
    let source = super::source::get(state, application_id).await?;
    if source.source_upload
        && super::source_archive::version_file(state, application_id, target_sha).is_none()
    {
        return Err(AppError::ValidationError(
            "That version's ZIP is no longer kept (the last 10 uploads are). Upload it again."
                .into(),
        ));
    }
    let missing = (!source.source_upload
        && (source.git_url.as_deref().unwrap_or("").trim().is_empty()
            || source.git_branch.as_deref().unwrap_or("").trim().is_empty()))
        || source.container_port.is_none();
    let method_ready = match source.build_method.as_str() {
        "docker" => source
            .dockerfile_path
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty()),
        "compose" => {
            source
                .compose_file_path
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
                && source
                    .compose_public_service
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty())
        }
        "railpack" => true,
        _ => false,
    };
    if missing || !method_ready {
        return Err(AppError::ValidationError(
            "Configure the Git source and selected build method before rolling back".into(),
        ));
    }
    super::ensure_deployment(state, application_id).await?;
    let (container_name, _) = super::identity(state, application_id).await?;
    let run_id = super::history::start(
        state,
        application_id,
        "git",
        &source.build_method,
        "manual_rollback",
        source.git_branch.as_deref(),
        created_by,
    )
    .await?;
    super::history::pin_commit(state, &run_id, target_sha).await?;
    let snapshot =
        serde_json::to_string(&source).map_err(|e| AppError::InternalError(e.to_string()))?;
    let conn = crate::git::repositories::db::get_conn(state).await;
    crate::git::repositories::db::retry_on_stale_snapshot(|| async {
        sqlx::query("UPDATE application_deployments SET status = 'deploying', container_name = CASE WHEN deployed_at IS NULL THEN ?1 ELSE container_name END, last_error = NULL, updated_at = ?2 WHERE application_id = ?3")
            .bind(container_name.clone())
            .bind(now_ms())
            .bind(application_id)
            .execute(&conn)
            .await
            .map_err(|e| AppError::DatabaseError(e.to_string()))
    })
    .await?;
    super::queue::enqueue(
        state,
        super::queue::EnqueueJobInput {
            application_id: application_id.into(),
            project_id: project_id.into(),
            environment_id: environment_id.into(),
            organization_id: organization_id.into(),
            source_type: "git".into(),
            source_snapshot: snapshot.clone(),
            build_method: source.build_method.clone(),
            build_snapshot: snapshot,
            run_id,
        },
    )
    .await
}

pub(crate) async fn run(
    state: SharedState,
    application_id: String,
    run_id: String,
) -> Result<(), String> {
    if let Err(error) = pipeline(&state, &application_id, &run_id).await {
        let _ = super::history::finish(&state, &run_id, "failed", Some(&error)).await;
        super::rollout::record_failure(&state, &application_id, &error).await;
        return Err(error);
    } else {
        let _ = super::history::finish(&state, &run_id, "succeeded", None).await;
    }
    if super::source::get(&state, &application_id)
        .await
        .map(|source| source.build_method != "compose")
        .unwrap_or(true)
    {
        let _ = std::fs::remove_dir_all(repo_dir(&state, &application_id));
    }
    Ok(())
}

/// Builds and runs a Git Docker application on its assigned SSH server. The
/// repository is cloned centrally only because its provider credentials live
/// here; the remote host receives a credential-free source archive and all
/// Podman operations below go through `ServerRuntime`.
async fn remote_docker_pipeline(
    state: &SharedState,
    application_id: &str,
    run_id: &str,
    source: &crate::dtos::deployment::SourceConfigDto,
    repo: &Path,
    runtime: &crate::servers::runtime::ServerRuntime,
) -> Result<(), String> {
    let build_environment = decrypted_environment(state, application_id, "build")
        .await
        .map_err(|error| error.to_string())?;
    if !build_environment.is_empty() {
        return Err("Remote Docker builds with build-time variables are not supported yet because build secrets must not be sent as SSH command arguments. Move these values to runtime variables or deploy on the local server.".into());
    }

    let dockerfile_path = source
        .dockerfile_path
        .as_deref()
        .ok_or_else(|| "Dockerfile path is missing".to_string())?;
    let dockerfile = repo.join(dockerfile_path);
    if !dockerfile.is_file() {
        return Err(format!(
            "No Dockerfile found at {dockerfile_path} in the repository"
        ));
    }

    // The clone's .git directory can include the source URL; never transfer it
    // to a deployment server once the commit metadata has been recorded.
    let _ = std::fs::remove_dir_all(repo.join(".git"));
    let _ = super::history::append(
        state,
        run_id,
        "info",
        "Uploading build context to the selected server",
    )
    .await;
    let remote_repo = runtime
        .upload_workspace_tree(application_id, repo, "repo")
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Remote build workspace was not created".to_string())?;
    let remote_dockerfile = safe_repo_path(&remote_repo, dockerfile_path)?;

    let _ = super::history::append(
        state,
        run_id,
        "info",
        "Building Docker image on selected server",
    )
    .await;
    let mut build = vec![
        OsString::from("build"),
        OsString::from("--file"),
        OsString::from(remote_dockerfile),
        OsString::from("--tag"),
        OsString::from(image_tag(application_id)),
    ];
    if let Some(limits) = resource_limits(source)? {
        build.extend([
            OsString::from("--cpu-period"),
            OsString::from(BUILD_CPU_PERIOD_US.to_string()),
            OsString::from("--cpu-quota"),
            OsString::from(limits.cpu_quota_us.to_string()),
            OsString::from("--memory"),
            OsString::from(limits.memory_bytes.to_string()),
            OsString::from("--memory-swap"),
            OsString::from(limits.memswap_bytes.to_string()),
        ]);
    }
    build.push(OsString::from(&remote_repo));
    // A real Dockerfile build routinely takes minutes; stream it like the
    // Railpack builds instead of the buffered `podman()` shape, which used a
    // 45s SSH-command budget meant for short commands and always timed out
    // on non-trivial images.
    stream_podman_step(
        runtime,
        &build,
        state,
        run_id,
        std::time::Duration::from_secs(30 * 60),
        "Could not build Docker image on selected server",
    )
    .await?;

    super::rollout::deploy_image(
        state,
        application_id,
        source,
        &image_tag(application_id),
        runtime,
    )
    .await
}

/// Runs the same two-stage Railpack build on the selected server. The source
/// clone and generated builder context are transferred through the existing
/// app-scoped SSH workspace; the remote builder never sees litepod's data volume
/// or another application's files.
async fn remote_railpack_pipeline(
    state: &SharedState,
    application_id: &str,
    run_id: &str,
    source: &crate::dtos::deployment::SourceConfigDto,
    repo: &Path,
    runtime: &crate::servers::runtime::ServerRuntime,
) -> Result<(), String> {
    if !super::managed_volumes::runtime_mounts(state, application_id)
        .await
        .map_err(|error| error.to_string())?
        .is_empty()
    {
        return Err("Managed-volume mounts for remote Railpack deployments are not ready yet; detach them before deploying this application".into());
    }
    let workspace = runtime
        .ensure_workspace(application_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Remote Railpack workspace was not created".to_string())?;
    let context = railpack_container_context(&source.railpack_project_path)?;
    let _ = std::fs::remove_dir_all(repo.join(".git"));
    let remote_repo = runtime
        .upload_workspace_tree(application_id, repo, "repo")
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Remote Railpack repository was not uploaded".to_string())?;
    let builder_context = railpack::stage_builder_context(run_id)?;
    let builder_directory = format!("railpack-builder-{run_id}");
    let remote_builder_context = runtime
        .upload_workspace_tree(application_id, &builder_context, &builder_directory)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Remote Railpack builder context was not uploaded".to_string())?;
    let _ = std::fs::remove_dir_all(&builder_context);
    let builder_image = format!("litepod-railpack-builder-{application_id}:{run_id}");
    let _ = super::history::append(
        state,
        run_id,
        "info",
        "Building Railpack worker on the selected server",
    )
    .await;
    stream_podman_step(
        runtime,
        &[
            OsString::from("build"),
            OsString::from("--tag"),
            OsString::from(&builder_image),
            OsString::from("--file"),
            OsString::from(format!(
                "{remote_builder_context}/Dockerfile.railpack-builder"
            )),
            OsString::from(&remote_builder_context),
        ],
        state,
        run_id,
        std::time::Duration::from_secs(30 * 60),
        "Could not build the remote Railpack worker",
    )
    .await?;

    let railpack_root = format!("{workspace}/railpack/{run_id}");
    runtime
        .remote_command(&[
            OsString::from("mkdir"),
            OsString::from("-p"),
            OsString::from(format!("{railpack_root}/out")),
            OsString::from(format!("{railpack_root}/build")),
        ])
        .await
        .map_err(|error| error.to_string())?;
    let mut build_environment = decrypted_environment(state, application_id, "build")
        .await
        .map_err(|error| error.to_string())?;
    railpack::apply_start_command(
        &mut build_environment,
        source.railpack_start_command.as_deref(),
    );
    let build_env =
        crate::servers::ssh::SecretFile::write(&environment_file_contents(&build_environment)?)
            .map_err(|error| error.to_string())?;
    let staged_build_env = runtime
        .upload_workspace_file(application_id, build_env.path(), "railpack.build.env")
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Remote Railpack build environment was not uploaded".to_string())?;
    runtime
        .remote_command(&[
            OsString::from("mv"),
            OsString::from(staged_build_env),
            OsString::from(format!("{railpack_root}/out/build-env")),
        ])
        .await
        .map_err(|error| error.to_string())?;

    let planner_name = format!("litepod-railpack-prepare-{application_id}-{run_id}");
    let _ = super::history::append(
        state,
        run_id,
        "info",
        "Preparing Railpack build plan on the selected server",
    )
    .await;
    stream_podman_step(
        runtime,
        &[
            OsString::from("run"),
            OsString::from("--rm"),
            OsString::from("--name"),
            OsString::from(&planner_name),
            OsString::from("--volume"),
            OsString::from(format!("{remote_repo}:/workspace/source:ro")),
            OsString::from("--volume"),
            OsString::from(format!("{railpack_root}:/railpack")),
            OsString::from("--env"),
            OsString::from(format!("CONTEXT_DIR={context}")),
            OsString::from("--env"),
            OsString::from("OUT_DIR=/railpack/out"),
            OsString::from(&builder_image),
        ],
        state,
        run_id,
        std::time::Duration::from_secs(5 * 60),
        "Remote Railpack plan preparation failed",
    )
    .await?;

    let builder_name = railpack::builder_name(application_id, run_id);
    let _ = super::history::append(
        state,
        run_id,
        "info",
        "Starting isolated Railpack builder on the selected server",
    )
    .await;
    let mut builder_run = vec![
        OsString::from("run"),
        OsString::from("--rm"),
        OsString::from("--privileged"),
        OsString::from("--name"),
        OsString::from(&builder_name),
        OsString::from("--entrypoint"),
        OsString::from("/usr/local/bin/railpack-build"),
        OsString::from("--volume"),
        OsString::from(format!("{remote_repo}:/workspace/source:ro")),
        OsString::from("--volume"),
        OsString::from(format!("{railpack_root}:/workspace/inputs:ro")),
        OsString::from("--volume"),
        OsString::from(format!("{railpack_root}/build:/output")),
        OsString::from("--env"),
        OsString::from(format!("IMAGE_TAG={}", image_tag(application_id))),
        OsString::from("--env"),
        OsString::from(format!("CONTEXT_DIR={context}")),
        OsString::from("--env"),
        OsString::from("PLAN_DIR=/workspace/inputs/out"),
        OsString::from("--env"),
        OsString::from("OUT_DIR=/output"),
        OsString::from("--env"),
        OsString::from(format!("RAILPACK_CLEAN_BUILD={}", source.clean_build)),
    ];
    // Unlike the plain-Docker remote build above, this previously applied no
    // resource limits at all — an unbounded remote Railpack build could
    // saturate the shared server's CPU/RAM.
    if let Some(limits) = resource_limits(source)? {
        builder_run.extend([
            OsString::from("--cpu-period"),
            OsString::from(BUILD_CPU_PERIOD_US.to_string()),
            OsString::from("--cpu-quota"),
            OsString::from(limits.cpu_quota_us.to_string()),
            OsString::from("--memory"),
            OsString::from(limits.memory_bytes.to_string()),
            OsString::from("--memory-swap"),
            OsString::from(limits.memswap_bytes.to_string()),
        ]);
    }
    builder_run.push(OsString::from(&builder_image));
    // A real `bun install && bun run build` (or equivalent) routinely takes
    // minutes; stream it so the deploy log shows progress instead of going
    // dark until it exits.
    if let Err(error) = stream_podman_step(
        runtime,
        &builder_run,
        state,
        run_id,
        std::time::Duration::from_secs(30 * 60),
        "Remote Railpack image build failed",
    )
    .await
    {
        let _ = super::history::append(state, run_id, "error", &error).await;
        return Err(error);
    }
    runtime
        .podman(&[
            OsString::from("load"),
            OsString::from("--input"),
            OsString::from(format!("{railpack_root}/build/image.tar")),
        ])
        .await
        .map_err(|error| format!("Could not load the remote Railpack image: {error}"))?;

    super::rollout::deploy_image(
        state,
        application_id,
        source,
        &image_tag(application_id),
        runtime,
    )
    .await
}

/// Runs a Podman step through `podman_stream` and appends each stdout/stderr
/// line to the run's history as it arrives, instead of the buffered
/// `podman`/`podman_with_timeout` shape (which collects the whole output and
/// only appends it once the process exits). A Railpack build routinely takes
/// minutes, and the deploy log should show progress during that time rather
/// than go dark until it's done. Mirrors `compose::pull::podman_pull`'s
/// streaming shape.
async fn stream_podman_step(
    runtime: &crate::servers::runtime::ServerRuntime,
    args: &[OsString],
    state: &SharedState,
    run_id: &str,
    budget: std::time::Duration,
    failure_label: &str,
) -> Result<(), String> {
    let mut stream = runtime
        .podman_stream(args)
        .await
        .map_err(|error| format!("{failure_label}: {error}"))?;
    let stdout = stream.take_stdout();
    let stderr = stream.take_stderr();
    let stderr_tail: Vec<String> =
        match (stdout, stderr) {
            (Some(stdout), Some(stderr)) => {
                let stdout_state = state.clone();
                let stdout_run = run_id.to_owned();
                let stderr_state = state.clone();
                let stderr_run = run_id.to_owned();
                let stdout_handle = tokio::spawn(async move {
                    stream_step_output(stdout, stdout_state, stdout_run).await
                });
                let stderr_handle = tokio::spawn(async move {
                    stream_step_output(stderr, stderr_state, stderr_run).await
                });
                let (stdout_result, stderr_result) = tokio::join!(stdout_handle, stderr_handle);
                let _ = stdout_result;
                stderr_result.unwrap_or_default()
            }
            _ => Vec::new(),
        };
    // Same budget the buffered path used, enforced here instead since
    // `podman_stream` itself doesn't time out a follow-mode child.
    let status = match tokio::time::timeout(budget, stream.wait()).await {
        Ok(result) => Some(result.map_err(|error| format!("{failure_label}: {error}"))?),
        Err(_) => {
            stream.kill().await;
            let _ = stream.wait().await;
            None
        }
    };
    let Some(status) = status else {
        return Err(format!(
            "{failure_label}: timed out after {} minutes",
            budget.as_secs() / 60
        ));
    };
    if !status.success() {
        let detail = if stderr_tail.is_empty() {
            format!("exit code {}", status.code().unwrap_or(-1))
        } else {
            stderr_tail.join("\n")
        };
        return Err(format!("{failure_label}: {detail}"));
    }
    Ok(())
}

/// Reads `reader` line by line, appending each non-empty line to the run's
/// history as it arrives, and keeps the last 40 lines to use as the error
/// detail if the step turns out to have failed.
async fn stream_step_output<R>(reader: R, state: SharedState, run_id: String) -> Vec<String>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    let mut tail = Vec::new();
    loop {
        line.clear();
        let read = match reader.read_line(&mut line).await {
            Ok(value) => value,
            Err(_) => return tail,
        };
        if read == 0 {
            return tail;
        }
        let message = line.trim_end_matches(['\r', '\n']).to_owned();
        if message.is_empty() {
            continue;
        }
        let _ = super::history::append(&state, &run_id, "info", &message).await;
        tail.push(message);
        if tail.len() > 40 {
            tail.remove(0);
        }
    }
}

/// The uploaded-ZIP counterpart of the clone: a fresh workspace with the
/// archive's files. The archive's short hash stands in for the commit, so the
/// run history still says which upload was built.
async fn unpack_upload(
    state: &SharedState,
    application_id: &str,
    run_id: &str,
    repo: &Path,
    pinned: Option<&str>,
) -> Result<(), String> {
    let _ = super::history::append(state, run_id, "info", "Unpacking the uploaded archive").await;
    // A rollback names the upload it wants; a normal deploy builds the latest.
    let archive = match pinned {
        Some(id) => super::source_archive::version_file(state, application_id, id)
            .ok_or("That version's ZIP is no longer kept. Upload it again.")?,
        None => super::source_archive::archive_file(state, application_id),
    };
    let destination = repo.to_path_buf();
    let id = tokio::task::spawn_blocking(move || {
        let _ = std::fs::remove_dir_all(&destination);
        super::source_archive::unpack(&archive, &destination)
    })
    .await
    .map_err(|error| format!("Unpack task failed: {error}"))??;
    let _ =
        super::history::set_commit(state, run_id, &id, "Uploaded ZIP archive", None, None).await;
    let _ = super::history::append(state, run_id, "info", "Archive unpacked").await;
    Ok(())
}

async fn pipeline(state: &SharedState, application_id: &str, run_id: &str) -> Result<(), String> {
    let source = super::source::get(state, application_id)
        .await
        .map_err(|e| e.to_string())?;
    // A rollback seeds `commit_sha` on this run (via `history::pin_commit`)
    // before the pipeline ever starts; a normal deploy leaves it unset until
    // the clone below discovers it. Presence here is exactly "pin to this commit".
    let target_sha = super::history::commit_sha(state, run_id)
        .await
        .map_err(|e| e.to_string())?;
    let repo = repo_dir(state, application_id);
    if source.source_upload {
        let _ = super::history::append(state, run_id, "info", "Preparing source workspace").await;
        unpack_upload(state, application_id, run_id, &repo, target_sha.as_deref()).await?;
    } else {
        let repository_url = source.git_url.as_deref().unwrap_or_default();
        let credentials: Option<(String, String)> = if let Some(provider_id) =
            source.git_provider_id.clone()
        {
            let (provider_host, allow_private) =
                git_providers::provider_host_url(state, &provider_id)
                    .await
                    .map_err(|e| e.to_string())?;
            crate::security::ssrf::validate_for_host(repository_url, &provider_host, allow_private)
                .await
                .map_err(|e| e.to_string())?;
            let _ =
                super::history::append(state, run_id, "info", "Resolving Git credentials").await;
            // Resolved per deploy: GitHub App installation tokens live one hour, so the clone must
            // use a token minted now rather than anything persisted on the provider row.
            let resolved = git_providers::resolve_credentials(state, &provider_id)
                .await
                .map_err(|e| e.to_string())?;
            Some((resolved.username, resolved.secret))
        } else {
            // No saved provider: anonymous public clone, or credentials the user
            // embedded in the pasted URL and that were saved (encrypted) at source-save time.
            crate::security::ssrf::validate_public_url(repository_url)
                .await
                .map_err(|e| e.to_string())?;
            git_providers::application_git_credentials(state, application_id)
                .await
                .map_err(|e| e.to_string())?
        };
        let _ =
            super::history::append(state, run_id, "info", "Preparing repository workspace").await;
        let _ = super::history::append(state, run_id, "info", "Cloning repository").await;
        // Clone (fresh directory every deploy).
        // Ensure complete cleanup - retry removal if files are temporarily locked (macOS).
        for attempt in 0..5 {
            if !repo.exists() {
                break;
            }
            match std::fs::remove_dir_all(&repo) {
                Ok(()) => break,
                Err(_) if attempt < 4 => std::thread::sleep(std::time::Duration::from_millis(200)),
                Err(e) => return Err(format!("Could not clean repo directory: {e}")),
            }
        }
        // Do NOT pre-create the directory - let git2's clone handle all .git structure creation.
        // Pre-creating can cause race conditions on macOS where git2 tries to write lock files
        // before its internal directory setup completes.
        {
            let url = source.git_url.clone().unwrap();
            let branch = source.git_branch.clone().unwrap();
            let destination = repo.clone();
            let clone_credentials = credentials.clone();
            let clone_target_sha = target_sha.clone();
            let (progress_tx, progress_rx) = std::sync::mpsc::channel();
            let mut clone_task = tokio::task::spawn_blocking(move || {
                clone_repo(
                    &url,
                    &branch,
                    clone_target_sha.as_deref(),
                    clone_credentials
                        .as_ref()
                        .map(|(u, p)| (u.as_str(), p.as_str())),
                    &destination,
                    Some(progress_tx),
                )
            });
            let mut progress_tick = tokio::time::interval(std::time::Duration::from_millis(250));
            let clone_result = loop {
                tokio::select! {
                    result = &mut clone_task => break result.map_err(|e| format!("Clone task failed: {e}"))?,
                    _ = progress_tick.tick() => {
                        while let Ok(message) = progress_rx.try_recv() {
                            let _ = super::history::append(state, run_id, "info", &message).await;
                        }
                    }
                }
            };
            while let Ok(message) = progress_rx.try_recv() {
                let _ = super::history::append(state, run_id, "info", &message).await;
            }
            let (commit_sha, commit_message, commit_author_name, commit_author_email) =
                clone_result.map_err(|e| {
                    format!(
                        "Could not clone {}: {e}",
                        source.git_url.as_deref().unwrap_or_default()
                    )
                })?;
            let _ = super::history::set_commit(
                state,
                run_id,
                &commit_sha,
                &commit_message,
                commit_author_name.as_deref(),
                commit_author_email.as_deref(),
            )
            .await;
            let _ = super::history::append(state, run_id, "info", "Repository cloned successfully")
                .await;
        }
    }

    if matches!(
        source.build_method.as_str(),
        "docker" | "railpack" | "compose"
    ) {
        let line = resource_log_line(&source);
        let _ = super::history::append(state, run_id, "info", &line).await;
    }

    // Compose builds inside `up --build`, so its next step is the start itself.
    super::history::step(
        state,
        if source.build_method == "compose" {
            "start"
        } else {
            "build"
        },
    )
    .await;
    let runtime = crate::servers::runtime::ServerRuntime::for_application(state, application_id)
        .await
        .map_err(|error| error.to_string())?;
    if runtime.is_remote() {
        return match source.build_method.as_str() {
            "docker" => {
                remote_docker_pipeline(state, application_id, run_id, &source, &repo, &runtime)
                    .await
            }
            "compose" => {
                let _ = super::history::append(
                    state,
                    run_id,
                    "info",
                    "Starting Podman Compose services on the selected server",
                )
                .await;
                let _ = super::history::set_status(state, run_id, "deploying").await;
                compose::run(state, application_id, &source, &repo).await
            }
            "railpack" => {
                remote_railpack_pipeline(state, application_id, run_id, &source, &repo, &runtime)
                    .await
            }
            _ => Err("Unsupported build method".into()),
        };
    }

    match source.build_method.as_str() {
        "compose" => {
            let _ =
                super::history::append(state, run_id, "info", "Starting Docker Compose services")
                    .await;
            let _ = super::history::set_status(state, run_id, "deploying").await;
            return compose::run(state, application_id, &source, &repo).await;
        }
        "railpack" => {
            let _ =
                super::history::append(state, run_id, "info", "Starting Railpack builder").await;
            let _ = super::history::append(
                state,
                run_id,
                "info",
                "Railpack will detect the application stack and generate a build plan",
            )
            .await;
            return railpack::run(state, application_id, run_id, &source).await;
        }
        "docker" => {}
        _ => return Err("Unsupported build method".into()),
    }

    let _ = super::history::append(state, run_id, "info", "Building Docker image").await;
    // Dockerfile must exist at the configured path.
    let dockerfile_path = source.dockerfile_path.clone().unwrap();
    let dockerfile = repo.join(&dockerfile_path);
    if !dockerfile.is_file() {
        return Err(format!(
            "No Dockerfile found at {dockerfile_path} in the repository"
        ));
    }

    // Build the image, keeping the tail of the log for error reporting.
    let podman =
        Podman::new(&state.podman_socket).map_err(|e| format!("Invalid Podman socket: {e}"))?;
    let build_args = decrypted_environment(state, application_id, "build")
        .await
        .map_err(|e| e.to_string())?;
    let mut options = ImageBuildOpts::builder(repo.to_string_lossy().to_string())
        .dockerfile(dockerfile_path.as_str())
        .tag(image_tag(application_id))
        .build_args(build_args);
    if let Some(limits) = resource_limits(&source)? {
        options = options
            .cpu_period(BUILD_CPU_PERIOD_US)
            .cpu_quota(limits.cpu_quota_us)
            .memory(limits.memory_bytes)
            // Podman interprets this as memory plus swap. The user can extend
            // it past RAM via `build_swap_mib`; otherwise it equals `memory`.
            .memswap(limits.memswap_bytes);
    }
    let options = options.build();
    let images = podman.images();
    let mut build = images
        .build(&options)
        .map_err(|e| format!("Could not start image build: {e}"))?;
    let mut log_tail: Vec<String> = Vec::new();
    // Remote/compose builds are already capped at 30 minutes; the local build
    // stream had no equivalent, so a runaway build could hang the job forever
    // instead of surfacing an error.
    let stream_result = tokio::time::timeout(std::time::Duration::from_secs(30 * 60), async {
        while let Some(chunk) = build.next().await {
            let chunk = chunk.map_err(|e| {
                format!(
                    "Image build failed: {}: {e}",
                    log_tail.last().cloned().unwrap_or_default()
                )
            })?;
            for line in chunk.stream.lines() {
                tracing::info!("[build {application_id}] {line}");
                let _ = super::history::append(state, run_id, "info", line).await;
                log_tail.push(line.to_string());
                if log_tail.len() > 40 {
                    log_tail.remove(0);
                }
            }
        }
        Ok::<(), String>(())
    })
    .await;
    match stream_result {
        Ok(inner) => inner?,
        Err(_) => return Err("Image build timed out after 30 minutes".to_string()),
    }

    // Verify the image exists — a silently empty stream means the build produced nothing.
    podman
        .images()
        .get(&image_tag(application_id))
        .inspect()
        .await
        .map_err(|e| {
            format!(
                "Image build failed: {}: {e}",
                log_tail.last().cloned().unwrap_or_default()
            )
        })?;

    let _ = super::history::set_status(state, run_id, "deploying").await;

    super::rollout::deploy_image(
        state,
        application_id,
        &source,
        &image_tag(application_id),
        &runtime,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::{pull_lfs_if_needed, railpack_container_context};

    #[test]
    fn lfs_pull_is_a_no_op_without_a_gitattributes_lfs_filter() {
        let dir = std::env::temp_dir().join(format!("litepod-lfs-test-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        let result = pull_lfs_if_needed(&dir, None);
        std::fs::remove_dir_all(&dir).ok();
        assert!(result.is_ok());
    }

    #[test]
    fn lfs_pull_is_skipped_when_gitattributes_has_no_lfs_filter() {
        let dir = std::env::temp_dir().join(format!("litepod-lfs-test-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".gitattributes"), "* text=auto\n").unwrap();
        let result = pull_lfs_if_needed(&dir, None);
        std::fs::remove_dir_all(&dir).ok();
        assert!(result.is_ok());
    }

    #[test]
    fn remote_railpack_context_is_confined_to_uploaded_source() {
        assert_eq!(
            railpack_container_context(".").unwrap(),
            "/workspace/source"
        );
        assert_eq!(
            railpack_container_context("services/gitea").unwrap(),
            "/workspace/source/services/gitea"
        );
        assert!(railpack_container_context("../outside").is_err());
        assert!(railpack_container_context("/etc").is_err());
    }
}
