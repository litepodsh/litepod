//! Git provider webhook receivers.
//!
//! Unauthenticated but signature-verified: the endpoint lives outside the session middleware
//! because providers call it server-to-server. The URL is per-provider so the secret to verify
//! against is known before any verification work happens. GitHub, GitLab, Gitea, and Forgejo
//! share the route `/api/v1/webhooks/git/{provider_id}` — the handler looks up
//! `provider_type` to pick the right signature scheme.

use axum::{
    body::Bytes,
    extract::{Path, State},
    http::HeaderMap,
    response::IntoResponse,
};
use chrono::Utc;
use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    dtos::response::AppError,
    git::git_providers::{bitbucket_oauth, github_app, gitlab_oauth},
    state::state::SharedState,
};

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

/// GitHub caps payloads at 25 MB; anything past this is not a delivery we should be parsing.
const MAX_BODY_BYTES: usize = 5 * 1024 * 1024;

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

/// Extracts the file paths touched by a push, from the payload's `commits[]` (and GitHub's
/// `head_commit`, unioned in) — the shape (`added`/`removed`/`modified` string arrays per
/// commit) is identical across GitHub, GitLab, and Gitea/Forgejo. Returns `None` if the
/// payload has no `commits` field at all, so callers can tell "unknown" apart from "no files".
fn changed_files(payload: &serde_json::Value) -> Option<Vec<String>> {
    let commits = payload.get("commits")?.as_array()?;
    let mut files = Vec::new();
    let collect = |files: &mut Vec<String>, commit: &serde_json::Value| {
        for key in ["added", "removed", "modified"] {
            if let Some(list) = commit.get(key).and_then(|v| v.as_array()) {
                files.extend(list.iter().filter_map(|v| v.as_str()).map(String::from));
            }
        }
    };
    for commit in commits {
        collect(&mut files, commit);
    }
    if let Some(head) = payload.get("head_commit") {
        collect(&mut files, head);
    }
    Some(files)
}

/// True if any changed file falls under one of the comma-separated `watch_paths` prefixes.
fn matches_watch_paths(changed_files: &[String], watch_paths: &str) -> bool {
    watch_paths
        .split(',')
        .map(str::trim)
        .filter(|prefix| !prefix.is_empty())
        .any(|prefix| changed_files.iter().any(|file| file.starts_with(prefix)))
}

/// Constant-time signature check over the exact bytes GitHub sent — never over re-serialized JSON.
fn verify_signature(secret: &str, body: &[u8], signature: &str) -> Result<(), AppError> {
    let hex = signature
        .strip_prefix("sha256=")
        .ok_or(AppError::WebhookVerificationFailed)?;
    let expected = (0..hex.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(hex.get(index..index + 2).unwrap_or("zz"), 16))
        .collect::<Result<Vec<u8>, _>>()
        .map_err(|_| AppError::WebhookVerificationFailed)?;
    let mut mac = <Hmac<Sha256>>::new_from_slice(secret.as_bytes())
        .map_err(|_| AppError::WebhookVerificationFailed)?;
    mac.update(body);
    mac.verify_slice(&expected)
        .map_err(|_| AppError::WebhookVerificationFailed)
}

/// The identity a delivery is deduplicated on.
///
/// Providers that send a delivery id (`X-GitHub-Delivery`, `X-GitLab-Event-UUID`,
/// `X-Gitea-Delivery`) supply it directly. When the header is absent the id used
/// to fall back to a fresh `Uuid::now_v7()` — unique by construction, so the
/// `UNIQUE(git_provider_id, delivery_id)` index never fired and the whole
/// deduplication silently did nothing. Anyone who captured one correctly-signed
/// delivery could replay it as many times as they liked, each replay triggering a
/// deploy.
///
/// The fallback is now the digest of the signed body. It is stable for a given
/// delivery, and it is exactly the bytes the HMAC covers, so an attacker cannot
/// alter it to dodge the check without invalidating the signature they are
/// replaying.
///
/// Consequence worth stating: two byte-identical deliveries are treated as one.
/// For a push payload — which carries commit SHAs and timestamps — that means a
/// genuine repeat of the very same event, which is what deduplication is for.
fn delivery_identity(header_value: Option<&str>, body: &[u8]) -> String {
    match header_value
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(id) => id.to_string(),
        None => format!("body-sha256:{}", hex::encode(Sha256::digest(body))),
    }
}

/// Records the delivery, returning false when this delivery id was already handled.
async fn record_delivery(
    state: &SharedState,
    provider_id: &str,
    delivery_id: &str,
    event: &str,
    repo: Option<&str>,
    git_ref: Option<&str>,
    commit: Option<&str>,
    result: &str,
    detail: Option<&str>,
) -> bool {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "INSERT INTO webhook_deliveries (id, git_provider_id, delivery_id, event, repo_full_name, git_ref, commit_sha, result, detail, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
    )
    .bind(Uuid::now_v7().to_string())
    .bind(provider_id)
    .bind(delivery_id)
    .bind(event)
    .bind(repo)
    .bind(git_ref)
    .bind(commit)
    .bind(result)
    .bind(detail)
    .bind(now_ms())
    .execute(&conn)
    .await
    .is_ok()
}

/// Resolves the provider type from `git_providers`. An unknown id is reported as a
/// signature failure so the endpoint never confirms which provider ids exist.
async fn load_provider_type(state: &SharedState, provider_id: &str) -> Result<String, AppError> {
    use sqlx::Row;
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query("SELECT provider_type FROM git_providers WHERE id = ?1")
        .bind(provider_id)
        .fetch_optional(&conn)
        .await
        .map_err(|_| AppError::WebhookVerificationFailed)?
        .ok_or_else(|| {
            tracing::warn!(%provider_id, "webhook: unknown provider_id");
            AppError::WebhookVerificationFailed
        })?;
    row.try_get::<String, _>(0)
        .map_err(|_| AppError::WebhookVerificationFailed)
}

/// Reads the per-provider webhook secret stored in `git_providers.webhook_secret`. Gitea and
/// Forgejo share the same column; the verifier differs from GitLab's plain compare.
async fn load_git_provider_webhook_secret(
    state: &SharedState,
    provider_id: &str,
) -> Result<String, AppError> {
    use sqlx::Row;
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query("SELECT webhook_secret FROM git_providers WHERE id = ?1")
        .bind(provider_id)
        .fetch_optional(&conn)
        .await
        .map_err(|_| AppError::WebhookVerificationFailed)?
        .ok_or(AppError::WebhookVerificationFailed)?;
    let secret: Option<String> = row
        .try_get(0)
        .map_err(|_| AppError::WebhookVerificationFailed)?;
    secret
        .filter(|s| !s.is_empty())
        .ok_or(AppError::WebhookVerificationFailed)
}

/// Shared push-trigger logic: find every application subscribed to `branch`+`repo_full_name`
/// and queue a deploy. The queue's per-app serial claim handles ordering and overlap, so no
/// in-flight skip check is needed here. Returns the trigger count so each handler can record
/// the result.
async fn trigger_push_deploys(
    state: &SharedState,
    provider_id: &str,
    branch: &str,
    repo_full_name: &str,
    changed_files: Option<&[String]>,
) -> Result<usize, AppError> {
    use sqlx::Row;
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(
        "SELECT a.id, a.watch_paths FROM applications a \
         WHERE a.git_provider_id = ?1 AND a.auto_deploy = 1 AND a.deploy_type = 'git' AND a.git_branch = ?2 \
           AND (a.git_repo_full_name = ?3 OR lower(a.git_url) LIKE '%/' || lower(?3) || '.git')",
    )
    .bind(provider_id)
    .bind(branch)
    .bind(repo_full_name)
    .fetch_all(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;

    let mut application_ids = Vec::new();
    for row in &rows {
        let watch_paths: Option<String> = row
            .try_get(1)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        let watch_paths = watch_paths
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty());
        // No watch_paths configured -> unscoped, always redeploy (unchanged behavior).
        // watch_paths set but changed_files unknown (payload had no `commits[]`) -> fail open;
        // ponytail: GitHub truncates commits[] past 20 on huge pushes too, so this can also
        // under-report on a real push — switch to GitHub's compare API for an exact diff if
        // that ever causes noisy over-deploys.
        let eligible = match (watch_paths, changed_files) {
            (None, _) => true,
            (Some(_), None) => true,
            (Some(paths), Some(files)) => matches_watch_paths(files, paths),
        };
        if !eligible {
            continue;
        }
        application_ids.push(
            row.try_get::<String, _>(0)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        );
    }

    let mut triggered = 0usize;
    for application_id in application_ids {
        // Never awaited: providers time deliveries out (GitHub 10s, GitLab 10s) and a build
        // takes minutes. The queue serializes per-app so concurrent pushes cannot race.
        let task_state = state.clone();
        let task_application = application_id.clone();
        tokio::spawn(async move {
            if let Err(error) = crate::deployment::enqueue_for_application(
                &task_state,
                &task_application,
                "webhook",
                None,
            )
            .await
            {
                tracing::warn!("auto-deploy for {task_application} failed to enqueue: {error}");
            }
        });
        triggered += 1;
    }
    Ok(triggered)
}

/// Unified git webhook entry point. Dispatches to the right signature scheme based on the
/// provider's `provider_type`, then parses the push event and enqueues every matching app.
pub async fn git_webhook_handler(
    State(state): State<SharedState>,
    Path(provider_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<impl IntoResponse, AppError> {
    if body.len() > MAX_BODY_BYTES {
        tracing::warn!(%provider_id, body_len = body.len(), "webhook: payload too large");
        return Err(AppError::ValidationError(
            "Webhook payload too large".into(),
        ));
    }
    let provider_type = load_provider_type(&state, &provider_id).await?;
    tracing::info!(%provider_id, %provider_type, "webhook: received delivery");
    match provider_type.as_str() {
        "github" => handle_github(&state, &provider_id, &headers, &body).await,
        "gitlab" => handle_gitlab(&state, &provider_id, &headers, &body).await,
        "gitea" | "forgejo" => handle_gitea(&state, &provider_id, &headers, &body).await,
        "bitbucket" => handle_bitbucket(&state, &provider_id, &headers, &body).await,
        other => {
            tracing::warn!(%provider_id, %other, "webhook: unsupported provider type");
            Err(AppError::ValidationError(format!(
                "Webhook deliveries for provider type `{other}` are not supported"
            )))
        }
    }
}

async fn handle_github(
    state: &SharedState,
    provider_id: &str,
    headers: &HeaderMap,
    body: &Bytes,
) -> Result<axum::Json<serde_json::Value>, AppError> {
    let secret = github_app::webhook_secret(state, provider_id)
        .await
        .map_err(|_| {
            tracing::warn!(%provider_id, "webhook: github secret unavailable (app not installed/ready?)");
            AppError::WebhookVerificationFailed
        })?;
    let Some(signature) = header(headers, "x-hub-signature-256") else {
        tracing::warn!(%provider_id, "webhook: github delivery missing x-hub-signature-256 header");
        return Err(AppError::WebhookVerificationFailed);
    };
    verify_signature(&secret, body, signature).inspect_err(
        |_| tracing::warn!(%provider_id, "webhook: github signature verification failed"),
    )?;

    let event = header(headers, "x-github-event")
        .unwrap_or_default()
        .to_string();
    let delivery_id = delivery_identity(header(headers, "x-github-delivery"), body);
    tracing::info!(%provider_id, %event, %delivery_id, "webhook: github event received");
    if event == "ping" {
        return Ok(axum::Json(serde_json::json!({ "pong": true })));
    }
    if event != "push" {
        tracing::info!(%provider_id, %event, %delivery_id, "webhook: ignoring non-push event");
        return Ok(axum::Json(serde_json::json!({ "ignored": event })));
    }
    let payload: serde_json::Value = serde_json::from_slice(body)
        .map_err(|_| AppError::ValidationError("Malformed webhook payload".into()))?;
    let git_ref = payload
        .get("ref")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let repo_full_name = payload
        .get("repository")
        .and_then(|r| r.get("full_name"))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let commit = payload
        .get("after")
        .and_then(|v| v.as_str())
        .map(String::from);
    let deleted = payload
        .get("deleted")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !delivery_id.is_empty()
        && !record_delivery(
            state,
            provider_id,
            &delivery_id,
            &event,
            Some(&repo_full_name),
            Some(&git_ref),
            commit.as_deref(),
            "received",
            None,
        )
        .await
    {
        tracing::info!(%provider_id, %delivery_id, "webhook: duplicate delivery ignored");
        return Ok(axum::Json(serde_json::json!({ "duplicate": true })));
    }
    let Some(branch) = git_ref.strip_prefix("refs/heads/") else {
        tracing::info!(%provider_id, %git_ref, "webhook: non-branch ref ignored");
        return Ok(axum::Json(serde_json::json!({ "ignored": git_ref })));
    };
    if deleted {
        tracing::info!(%provider_id, %branch, "webhook: ignoring branch deletion");
        return Ok(axum::Json(
            serde_json::json!({ "ignored": "branch deleted" }),
        ));
    }
    let triggered = trigger_push_deploys(
        state,
        provider_id,
        branch,
        &repo_full_name,
        changed_files(&payload).as_deref(),
    )
    .await?;
    tracing::info!(%provider_id, %repo_full_name, %branch, %triggered, "webhook: deploy triggered");
    if !delivery_id.is_empty() {
        let conn = crate::git::repositories::db::get_conn(state).await;
        let _ = sqlx::query(
            "UPDATE webhook_deliveries SET result = ?1, detail = ?2 WHERE git_provider_id = ?3 AND delivery_id = ?4",
        )
        .bind(if triggered > 0 { "triggered" } else { "no_match" })
        .bind(format!("triggered={triggered}"))
        .bind(provider_id)
        .bind(delivery_id)
        .execute(&conn)
        .await;
    }
    Ok(axum::Json(serde_json::json!({ "triggered": triggered })))
}

/// Bitbucket Cloud HMAC-signs the body the same way GitHub does
/// (`X-Hub-Signature: sha256=<hex>`, no `-256` suffix on the header name), so
/// this reuses `verify_signature` rather than the plain-token compare
/// `gitlab_oauth::verify_webhook` does for GitLab.
///
/// Bitbucket's `push` payload carries an array of ref changes
/// (`push.changes[]`) instead of one `ref` per delivery, so unlike the other
/// handlers this can trigger more than one deploy per delivery - one push can
/// legitimately update several branches at once (a force-push fan-out, a
/// multi-branch CI push).
async fn handle_bitbucket(
    state: &SharedState,
    provider_id: &str,
    headers: &HeaderMap,
    body: &Bytes,
) -> Result<axum::Json<serde_json::Value>, AppError> {
    let secret = bitbucket_oauth::webhook_secret(state, provider_id)
        .await
        .inspect_err(|_| tracing::warn!(%provider_id, "webhook: bitbucket secret unavailable"))?;
    let Some(signature) = header(headers, "x-hub-signature") else {
        tracing::warn!(%provider_id, "webhook: bitbucket delivery missing x-hub-signature header");
        return Err(AppError::WebhookVerificationFailed);
    };
    verify_signature(&secret, body, signature).inspect_err(
        |_| tracing::warn!(%provider_id, "webhook: bitbucket signature verification failed"),
    )?;

    let event = header(headers, "x-event-key")
        .unwrap_or_default()
        .to_string();
    let delivery_id = delivery_identity(header(headers, "x-request-uuid"), body);
    tracing::info!(%provider_id, %event, %delivery_id, "webhook: bitbucket event received");
    if event != "repo:push" {
        tracing::info!(%provider_id, %event, %delivery_id, "webhook: ignoring non-push event");
        return Ok(axum::Json(serde_json::json!({ "ignored": event })));
    }
    let payload: serde_json::Value = serde_json::from_slice(body)
        .map_err(|_| AppError::ValidationError("Malformed webhook payload".into()))?;
    let repo_full_name = payload
        .get("repository")
        .and_then(|r| r.get("full_name"))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let changes = payload
        .get("push")
        .and_then(|p| p.get("changes"))
        .and_then(|c| c.as_array())
        .cloned()
        .unwrap_or_default();

    if !delivery_id.is_empty()
        && !record_delivery(
            state,
            provider_id,
            &delivery_id,
            &event,
            Some(&repo_full_name),
            None,
            None,
            "received",
            None,
        )
        .await
    {
        tracing::info!(%provider_id, %delivery_id, "webhook: duplicate delivery ignored");
        return Ok(axum::Json(serde_json::json!({ "duplicate": true })));
    }

    let mut triggered = 0usize;
    for change in &changes {
        // A branch deletion has no `new` state - `old` still describes what was removed.
        let Some(branch) = change
            .get("new")
            .and_then(|n| n.get("name"))
            .and_then(|v| v.as_str())
        else {
            continue;
        };
        // Bitbucket's push payload has no top-level file list the way GitHub's does, so
        // `changed_files` is unknown here - `trigger_push_deploys` already fails open
        // (redeploys) for a `watch_paths`-scoped app when that happens.
        triggered +=
            trigger_push_deploys(state, provider_id, branch, &repo_full_name, None).await?;
    }
    tracing::info!(%provider_id, %repo_full_name, %triggered, "webhook: deploy triggered");
    if !delivery_id.is_empty() {
        let conn = crate::git::repositories::db::get_conn(state).await;
        let _ = sqlx::query(
            "UPDATE webhook_deliveries SET result = ?1, detail = ?2 WHERE git_provider_id = ?3 AND delivery_id = ?4",
        )
        .bind(if triggered > 0 { "triggered" } else { "no_match" })
        .bind(format!("triggered={triggered}"))
        .bind(provider_id)
        .bind(delivery_id)
        .execute(&conn)
        .await;
    }
    Ok(axum::Json(serde_json::json!({ "triggered": triggered })))
}

async fn handle_gitlab(
    state: &SharedState,
    provider_id: &str,
    headers: &HeaderMap,
    body: &Bytes,
) -> Result<axum::Json<serde_json::Value>, AppError> {
    let supplied = header(headers, "x-gitlab-token");
    gitlab_oauth::verify_webhook(state, provider_id, supplied)
        .await
        .inspect_err(
            |_| tracing::warn!(%provider_id, "webhook: gitlab token verification failed"),
        )?;

    let event = header(headers, "x-gitlab-event")
        .unwrap_or_default()
        .to_string();
    let delivery_id = delivery_identity(
        header(headers, "x-gitlab-event-uuid").or_else(|| header(headers, "x-gitlab-webhook-uuid")),
        body,
    );
    tracing::info!(%provider_id, %event, %delivery_id, "webhook: gitlab event received");
    if event == "System Hook" || event.is_empty() {
        tracing::info!(%provider_id, %event, "webhook: ignoring system hook / missing event");
        return Ok(axum::Json(serde_json::json!({ "ignored": event })));
    }
    if event != "Push Hook" {
        tracing::info!(%provider_id, %event, %delivery_id, "webhook: ignoring non-push event");
        return Ok(axum::Json(serde_json::json!({ "ignored": event })));
    }
    let payload: serde_json::Value = serde_json::from_slice(body)
        .map_err(|_| AppError::ValidationError("Malformed webhook payload".into()))?;
    let git_ref = payload
        .get("ref")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let repo_full_name = payload
        .get("project")
        .and_then(|p| p.get("path_with_namespace"))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let commit = payload
        .get("after")
        .and_then(|v| v.as_str())
        .map(String::from);
    if !record_delivery(
        state,
        provider_id,
        &delivery_id,
        &event,
        Some(&repo_full_name),
        Some(&git_ref),
        commit.as_deref(),
        "received",
        None,
    )
    .await
    {
        tracing::info!(%provider_id, %delivery_id, "webhook: duplicate delivery ignored");
        return Ok(axum::Json(serde_json::json!({ "duplicate": true })));
    }
    let Some(branch) = git_ref.strip_prefix("refs/heads/") else {
        tracing::info!(%provider_id, %git_ref, "webhook: non-branch ref ignored");
        return Ok(axum::Json(serde_json::json!({ "ignored": git_ref })));
    };
    let triggered = trigger_push_deploys(
        state,
        provider_id,
        branch,
        &repo_full_name,
        changed_files(&payload).as_deref(),
    )
    .await?;
    tracing::info!(%provider_id, %repo_full_name, %branch, %triggered, "webhook: deploy triggered");
    let conn = crate::git::repositories::db::get_conn(state).await;
    let _ = sqlx::query(
        "UPDATE webhook_deliveries SET result = ?1, detail = ?2 WHERE git_provider_id = ?3 AND delivery_id = ?4",
    )
    .bind(if triggered > 0 { "triggered" } else { "no_match" })
    .bind(format!("triggered={triggered}"))
    .bind(provider_id)
    .bind(delivery_id)
    .execute(&conn)
    .await;
    Ok(axum::Json(serde_json::json!({ "triggered": triggered })))
}

async fn handle_gitea(
    state: &SharedState,
    provider_id: &str,
    headers: &HeaderMap,
    body: &Bytes,
) -> Result<axum::Json<serde_json::Value>, AppError> {
    let secret = load_git_provider_webhook_secret(state, provider_id).await?;
    let hex = header(headers, "x-gitea-signature").ok_or(AppError::WebhookVerificationFailed)?;
    let signature = format!("sha256={hex}");
    verify_signature(&secret, body, &signature).inspect_err(
        |_| tracing::warn!(%provider_id, "webhook: gitea/forgejo signature verification failed"),
    )?;

    let event = header(headers, "x-gitea-event")
        .unwrap_or_default()
        .to_string();
    let delivery_id = delivery_identity(header(headers, "x-gitea-delivery"), body);
    tracing::info!(%provider_id, %event, %delivery_id, "webhook: gitea/forgejo event received");
    if event.is_empty() {
        tracing::warn!(%provider_id, %delivery_id, "webhook: missing event type");
        return Ok(axum::Json(
            serde_json::json!({ "ignored": "missing event" }),
        ));
    }
    if event != "push" {
        tracing::info!(%provider_id, %event, %delivery_id, "webhook: ignoring non-push event");
        return Ok(axum::Json(serde_json::json!({ "ignored": event })));
    }
    let payload: serde_json::Value = serde_json::from_slice(body)
        .map_err(|_| AppError::ValidationError("Malformed webhook payload".into()))?;
    let git_ref = payload
        .get("ref")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let repo_full_name = payload
        .get("repository")
        .and_then(|r| r.get("full_name"))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let commit = payload
        .get("after")
        .and_then(|v| v.as_str())
        .map(String::from);
    if !record_delivery(
        state,
        provider_id,
        &delivery_id,
        &event,
        Some(&repo_full_name),
        Some(&git_ref),
        commit.as_deref(),
        "received",
        None,
    )
    .await
    {
        tracing::info!(%provider_id, %delivery_id, "webhook: duplicate delivery ignored");
        return Ok(axum::Json(serde_json::json!({ "duplicate": true })));
    }
    let Some(branch) = git_ref.strip_prefix("refs/heads/") else {
        tracing::info!(%provider_id, %git_ref, "webhook: non-branch ref ignored");
        return Ok(axum::Json(serde_json::json!({ "ignored": git_ref })));
    };
    let triggered = trigger_push_deploys(
        state,
        provider_id,
        branch,
        &repo_full_name,
        changed_files(&payload).as_deref(),
    )
    .await?;
    tracing::info!(%provider_id, %repo_full_name, %branch, %triggered, "webhook: deploy triggered");
    let conn = crate::git::repositories::db::get_conn(state).await;
    let _ = sqlx::query(
        "UPDATE webhook_deliveries SET result = ?1, detail = ?2 WHERE git_provider_id = ?3 AND delivery_id = ?4",
    )
    .bind(if triggered > 0 { "triggered" } else { "no_match" })
    .bind(format!("triggered={triggered}"))
    .bind(provider_id)
    .bind(delivery_id)
    .execute(&conn)
    .await;
    Ok(axum::Json(serde_json::json!({ "triggered": triggered })))
}

/// `POST /deploy/<token>` — no session, no API key: the random token in the path is the
/// credential. Kept deliberately terse so a leaked URL reveals nothing about the instance.
pub async fn deploy_hook_handler(
    State(state): State<SharedState>,
    Path(token): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let application_id = crate::deployment::deploy_hook::application_for_token(&state, &token)
        .await
        .map_err(|_| AppError::ApplicationNotFound)?;
    let job = crate::deployment::deploy_hook::trigger(&state, &application_id).await?;
    Ok(axum::Json(
        serde_json::json!({ "triggered": true, "job": job }),
    ))
}

/// Where an MCP agent `curl -T`s the site of a pending `deploy_static_site`.
/// The token in the path is the credential, as with the deploy hook.
pub async fn assistant_upload_handler(
    State(state): State<SharedState>,
    Path((action_id, token)): Path<(String, String)>,
    body: Result<axum::body::Bytes, axum::extract::rejection::BytesRejection>,
) -> Result<impl IntoResponse, AppError> {
    // The route's `DefaultBodyLimit` is what trips here; say so in plain words
    // instead of axum's generic "length limit exceeded".
    let body = body.map_err(|rejection| {
        if rejection.status() == axum::http::StatusCode::PAYLOAD_TOO_LARGE {
            AppError::ValidationError(
                "The upload is larger than the limit: 10 MB for a site, 100 MB for a project ZIP. Leave out large assets, node_modules and build output and upload again.".into(),
            )
        } else {
            AppError::ValidationError(format!("Could not read the upload: {rejection}"))
        }
    })?;
    if body.is_empty() {
        return Err(AppError::ValidationError("The upload is empty".into()));
    }
    crate::assistant::actions::stage_upload(&state, &action_id, &token, &body).await?;
    Ok(axum::Json(serde_json::json!({
        "uploaded": true,
        "bytes": body.len(),
        "next": "Ask the user to confirm the proposal in Litepod.",
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn an_oversized_site_is_refused_with_the_limit_in_words() {
        use axum::{body::Body, http::Request};
        use tower::ServiceExt;

        let (state, path) = crate::testing::test_state().await;
        let router = axum::Router::new()
            .route(
                "/uploads/{action_id}/{token}",
                axum::routing::put(assistant_upload_handler)
                    .layer(axum::extract::DefaultBodyLimit::max(10 * 1024 * 1024)),
            )
            .with_state(state);
        let response = router
            .oneshot(
                Request::put("/uploads/action/token")
                    .body(Body::from(vec![0u8; 10 * 1024 * 1024 + 1]))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(
            String::from_utf8_lossy(&body).contains("larger than the limit"),
            "{}",
            String::from_utf8_lossy(&body)
        );
        crate::testing::cleanup_database(&path);
    }

    #[test]
    fn watch_paths_matches_touched_prefix() {
        let files = vec!["apps/web/index.ts".to_string()];
        assert!(matches_watch_paths(&files, "apps/web,packages/shared"));
    }

    #[test]
    fn watch_paths_rejects_untouched_prefix() {
        let files = vec!["apps/api/main.rs".to_string()];
        assert!(!matches_watch_paths(&files, "apps/web,packages/shared"));
    }

    #[test]
    fn changed_files_unions_commits_and_head_commit() {
        let payload = serde_json::json!({
            "commits": [{"added": ["a.txt"], "modified": [], "removed": []}],
            "head_commit": {"added": [], "modified": ["b.txt"], "removed": []},
        });
        let mut files = changed_files(&payload).unwrap();
        files.sort();
        assert_eq!(files, vec!["a.txt".to_string(), "b.txt".to_string()]);
    }

    #[test]
    fn changed_files_none_when_commits_field_absent() {
        assert!(changed_files(&serde_json::json!({})).is_none());
    }

    #[test]
    fn signature_verification_matches_githubs_documented_vector() {
        // From GitHub's webhook docs.
        let secret = "It's a Secret to Everybody";
        let body = b"Hello, World!";
        let signature = "sha256=757107ea0eb2509fc211221cce984b8a37570b6d7586c22c46f4379c8b043e17";
        assert!(verify_signature(secret, body, signature).is_ok());
    }

    #[test]
    fn signature_verification_rejects_tampering() {
        let secret = "It's a Secret to Everybody";
        let signature = "sha256=757107ea0eb2509fc211221cce984b8a37570b6d7586c22c46f4379c8b043e17";
        assert!(
            verify_signature(secret, b"Hello, World?", signature).is_err(),
            "tampered body"
        );
        assert!(
            verify_signature("wrong secret", b"Hello, World!", signature).is_err(),
            "wrong secret"
        );
        assert!(
            verify_signature(secret, b"Hello, World!", "757107ea").is_err(),
            "missing sha256= prefix"
        );
        assert!(
            verify_signature(secret, b"Hello, World!", "sha256=zzzz").is_err(),
            "non-hex signature"
        );
    }
}

#[cfg(test)]
mod delivery_identity_tests {
    use super::*;

    #[test]
    fn the_providers_own_id_is_used_when_present() {
        assert_eq!(delivery_identity(Some("abc-123"), b"{}"), "abc-123");
    }

    #[test]
    fn a_missing_id_falls_back_to_the_body_digest_and_is_stable() {
        // The M7 bug: this used to be a fresh UUID per call, so the UNIQUE index
        // on (git_provider_id, delivery_id) never matched and a captured,
        // correctly-signed delivery could be replayed indefinitely.
        let first = delivery_identity(None, b"{\"ref\":\"refs/heads/main\"}");
        let second = delivery_identity(None, b"{\"ref\":\"refs/heads/main\"}");
        assert_eq!(
            first, second,
            "the same delivery must deduplicate against itself"
        );
        assert!(first.starts_with("body-sha256:"));
    }

    #[test]
    fn an_empty_or_blank_header_is_treated_as_missing() {
        let body = b"{}";
        let derived = delivery_identity(None, body);
        assert_eq!(delivery_identity(Some(""), body), derived);
        assert_eq!(delivery_identity(Some("   "), body), derived);
    }

    #[test]
    fn different_bodies_stay_distinct() {
        assert_ne!(
            delivery_identity(None, b"{\"commit\":\"aaa\"}"),
            delivery_identity(None, b"{\"commit\":\"bbb\"}")
        );
    }
}
