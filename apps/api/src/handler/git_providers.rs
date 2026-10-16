use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect},
};

use crate::{
    dtos::{
        git_provider::{
            BitbucketOauthBeginResponse, BitbucketOauthStatusDto, CreateGitProviderRequest,
            GitWebhookStatusDto, GiteaOauthBeginResponse, GiteaOauthStatusDto,
            GitlabOauthBeginResponse, GitlabOauthStatusDto, ListBranchesRequest,
            TestGitProviderRequest, UpdateGitProviderRequest,
        },
        response::{AppError, created, ok},
    },
    git::git_providers,
    git::repositories::audit_log,
    security::authz::extract::{RequireOrg, perm},
    state::state::SharedState,
};

#[utoipa::path(
    get, path = "/api/v1/organizations/{organization_id}/git-providers",
    responses((status = 200, description = "Organization git providers", body = Vec<crate::dtos::git_provider::GitProviderDto>)),
    tag = "git-providers"
)]
pub async fn list_git_providers_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgRead>,
) -> Result<impl IntoResponse, AppError> {
    Ok(ok(git_providers::list(&state, &org.organization_id).await?))
}

#[utoipa::path(
    post, path = "/api/v1/organizations/{organization_id}/git-providers", request_body = CreateGitProviderRequest,
    responses((status = 201, description = "Git provider created", body = crate::dtos::git_provider::GitProviderDto)),
    tag = "git-providers"
)]
pub async fn create_git_provider_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::GitProviderManage>,
    Json(request): Json<CreateGitProviderRequest>,
) -> Result<impl IntoResponse, AppError> {
    let provider = git_providers::create(&state, &org.organization_id, request).await?;
    audit_log::record(
        &state,
        &org.organization_id,
        &org.user,
        "git_provider.created",
        Some("git_provider"),
        Some(&provider.id),
        Some(&provider.name),
        Some(
            &serde_json::json!({
                "provider_type": provider.provider_type,
                "host_url": provider.host_url,
                "username": provider.username,
            })
            .to_string(),
        ),
        None,
        None,
        None,
    )
    .await
    .ok();
    Ok(created(provider))
}

#[utoipa::path(
    patch, path = "/api/v1/organizations/{organization_id}/git-providers/{provider_id}", request_body = UpdateGitProviderRequest,
    responses((status = 200, description = "Git provider updated", body = crate::dtos::git_provider::GitProviderDto)),
    tag = "git-providers"
)]
pub async fn update_git_provider_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::GitProviderManage>,
    // The extractor covers `organization_id`; only `provider_id` is left.
    Path((_organization_id, provider_id)): Path<(String, String)>,
    Json(request): Json<UpdateGitProviderRequest>,
) -> Result<impl IntoResponse, AppError> {
    let provider =
        git_providers::update(&state, &org.organization_id, &provider_id, request).await?;
    audit_log::record(
        &state,
        &org.organization_id,
        &org.user,
        "git_provider.updated",
        Some("git_provider"),
        Some(&provider_id),
        Some(&provider.name),
        Some(
            &serde_json::json!({
                "provider_type": provider.provider_type,
                "host_url": provider.host_url,
                "username": provider.username,
            })
            .to_string(),
        ),
        None,
        None,
        None,
    )
    .await
    .ok();
    Ok(ok(provider))
}

#[utoipa::path(
    delete, path = "/api/v1/organizations/{organization_id}/git-providers/{provider_id}",
    responses((status = 200, description = "Git provider deleted")),
    tag = "git-providers"
)]
pub async fn delete_git_provider_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::GitProviderManage>,
    // The extractor covers `organization_id`; only `provider_id` is left.
    Path((_organization_id, provider_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    // Fetched before delete so the audit entry names what's gone (and shows
    // which account/host it was) rather than just its id.
    let deleted = git_providers::list(&state, &org.organization_id)
        .await
        .ok()
        .and_then(|providers| providers.into_iter().find(|p| p.id == provider_id));
    git_providers::delete(&state, &org.organization_id, &provider_id).await?;
    audit_log::record(
        &state,
        &org.organization_id,
        &org.user,
        "git_provider.deleted",
        Some("git_provider"),
        Some(&provider_id),
        deleted.as_ref().map(|p| p.name.as_str()),
        deleted
            .as_ref()
            .map(|p| {
                serde_json::json!({
                    "provider_type": p.provider_type,
                    "host_url": p.host_url,
                    "username": p.username,
                })
                .to_string()
            })
            .as_deref(),
        None,
        None,
        None,
    )
    .await
    .ok();
    Ok(ok(()))
}

#[utoipa::path(
    post, path = "/api/v1/organizations/{organization_id}/git-providers/test-repositories", request_body = TestGitProviderRequest,
    responses((status = 200, description = "Repositories visible to draft provider credentials", body = Vec<crate::dtos::git_provider::GitRepositoryDto>)),
    tag = "git-providers"
)]
// Probes draft credentials the caller just typed; it touches no stored provider,
// and needs instance state to enforce the cloud outbound-network policy.
pub async fn test_repositories_handler(
    State(state): State<SharedState>,
    _org: RequireOrg<perm::GitProviderManage>,
    Json(request): Json<TestGitProviderRequest>,
) -> Result<impl IntoResponse, AppError> {
    Ok(ok(git_providers::test_repositories(&state, request).await?))
}

#[utoipa::path(
    get, path = "/api/v1/organizations/{organization_id}/git-providers/{provider_id}/repositories",
    responses((status = 200, description = "Repositories visible to the provider credentials", body = Vec<crate::dtos::git_provider::GitRepositoryDto>)),
    tag = "git-providers"
)]
pub async fn list_repositories_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgRead>,
    // The extractor covers `organization_id`; only `provider_id` is left.
    Path((_organization_id, provider_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    Ok(ok(git_providers::repositories(
        &state,
        &org.organization_id,
        &provider_id,
    )
    .await?))
}

#[utoipa::path(
    post, path = "/api/v1/organizations/{organization_id}/git-providers/{provider_id}/branches", request_body = ListBranchesRequest,
    responses((status = 200, description = "Remote branch names", body = Vec<String>)),
    tag = "git-providers"
)]
pub async fn list_branches_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgRead>,
    // The extractor covers `organization_id`; only `provider_id` is left.
    Path((_organization_id, provider_id)): Path<(String, String)>,
    Json(request): Json<ListBranchesRequest>,
) -> Result<impl IntoResponse, AppError> {
    Ok(ok(git_providers::branches(
        &state,
        &org.organization_id,
        &provider_id,
        &request.repo_url,
    )
    .await?))
}

#[utoipa::path(
    post, path = "/api/v1/organizations/{organization_id}/git-providers/branches", request_body = ListBranchesRequest,
    responses((status = 200, description = "Remote branch names for a repo with no saved provider (public repo, or a URL carrying embedded credentials)", body = Vec<String>)),
    tag = "git-providers"
)]
pub async fn list_public_branches_handler(
    _org: RequireOrg<perm::OrgRead>,
    Json(request): Json<ListBranchesRequest>,
) -> Result<impl IntoResponse, AppError> {
    Ok(ok(git_providers::public_branches(&request.repo_url).await?))
}

// ── GitLab OAuth ───────────────────────────────────────────────────────────────
//
// Three authenticated routes (begin / status / disconnect) and one public route
// (callback) — the public callback is wired in router.rs alongside the GitHub App
// callback so it lives outside the session middleware.

#[utoipa::path(
    get, path = "/api/v1/organizations/{organization_id}/git-providers/{provider_id}/gitlab-oauth",
    responses((status = 200, description = "GitLab OAuth connection status", body = GitlabOauthStatusDto)),
    tag = "git-providers"
)]
pub async fn gitlab_oauth_status_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgRead>,
    // The extractor covers `organization_id`; only `provider_id` is left.
    Path((_organization_id, provider_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let status =
        git_providers::gitlab_oauth::status(&state, &provider_id, &org.organization_id).await?;
    let public = git_providers::gitlab_oauth::public_url(&state).await?;
    Ok(ok(GitlabOauthStatusDto {
        connected: status.connected,
        user_login: status.user_login,
        user_id: status.user_id,
        scopes: status.scopes,
        connected_at: status.connected_at,
        expires_at: status.expires_at,
        has_client_credentials: status.has_client_credentials,
        has_webhook_secret: status.has_webhook_secret,
        webhook_url: git_providers::gitlab_oauth::git_webhook_url(&public, &provider_id),
        redirect_uri: git_providers::gitlab_oauth::redirect_uri(&public, &provider_id),
    }))
}

#[utoipa::path(
    get, path = "/api/v1/organizations/{organization_id}/git-providers/{provider_id}/git-webhook",
    responses((status = 200, description = "Webhook URL and secret for the unified git webhook route", body = GitWebhookStatusDto)),
    tag = "git-providers"
)]
/// Editor, not Viewer: the response carries the webhook secret in the clear, and
/// that secret is what lets a caller forge push deliveries for this provider. A
/// read-only member holding it would have write-equivalent power.
pub async fn git_webhook_status_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::GitProviderManage>,
    // The extractor covers `organization_id`; only `provider_id` is left.
    Path((_organization_id, provider_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let public = git_providers::gitlab_oauth::public_url(&state).await?;
    let secret = git_providers::gitlab_oauth::ensure_webhook_secret(
        &state,
        &provider_id,
        &org.organization_id,
    )
    .await?;
    Ok(ok(GitWebhookStatusDto {
        webhook_url: git_providers::gitlab_oauth::git_webhook_url(&public, &provider_id),
        webhook_secret: secret,
    }))
}

#[utoipa::path(
    post, path = "/api/v1/organizations/{organization_id}/git-providers/{provider_id}/gitlab-oauth/begin",
    responses((status = 200, description = "GitLab authorize URL the browser should be redirected to", body = GitlabOauthBeginResponse)),
    tag = "git-providers"
)]
pub async fn gitlab_oauth_begin_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::GitProviderManage>,
    // The extractor covers `organization_id`; only `provider_id` is left.
    Path((_organization_id, provider_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let authorize_url = git_providers::gitlab_oauth::begin_authorize(
        &state,
        &org.organization_id,
        &org.user.user_id,
        &provider_id,
    )
    .await?;
    let public = git_providers::gitlab_oauth::public_url(&state).await?;
    Ok(ok(GitlabOauthBeginResponse {
        authorize_url,
        redirect_uri: git_providers::gitlab_oauth::redirect_uri(&public, &provider_id),
    }))
}

#[derive(Debug, serde::Deserialize)]
pub struct GitlabOauthCallbackQuery {
    pub code: String,
    pub state: String,
}

/// Public: GitLab redirects the user's browser here with `?code=&state=`. The CSRF state
/// nonce has already been parked in `AppState::oauth_states` by the begin handler. We exchange
/// the code for tokens, persist them, and 302 the browser back into the UI with a status flag
/// so the providers page can show the connected state.
pub async fn gitlab_oauth_callback_handler(
    State(state): State<SharedState>,
    Query(query): Query<GitlabOauthCallbackQuery>,
) -> Result<impl IntoResponse, AppError> {
    let provider_id =
        git_providers::gitlab_oauth::complete_authorize(&state, &query.code, &query.state).await?;
    let public = git_providers::gitlab_oauth::public_url(&state)
        .await
        .unwrap_or_default();
    // Deep link back to the providers page with the connected provider highlighted.
    let target = format!(
        "{}/git-providers?gitlab_connected={provider_id}",
        public.trim_end_matches('/')
    );
    Ok((StatusCode::FOUND, Redirect::to(&target)))
}

#[utoipa::path(
    delete, path = "/api/v1/organizations/{organization_id}/git-providers/{provider_id}/gitlab-oauth",
    responses((status = 200, description = "GitLab OAuth tokens removed; client credentials kept")),
    tag = "git-providers"
)]
pub async fn gitlab_oauth_disconnect_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::GitProviderManage>,
    // The extractor covers `organization_id`; only `provider_id` is left.
    Path((_organization_id, provider_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    git_providers::gitlab_oauth::disconnect(&state, &provider_id, &org.organization_id).await?;
    Ok(ok(()))
}

#[utoipa::path(
    get, path = "/api/v1/organizations/{organization_id}/git-providers/{provider_id}/bitbucket-oauth",
    responses((status = 200, description = "Bitbucket OAuth connection status", body = BitbucketOauthStatusDto)),
    tag = "git-providers"
)]
pub async fn bitbucket_oauth_status_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgRead>,
    Path((_organization_id, provider_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let status =
        git_providers::bitbucket_oauth::status(&state, &provider_id, &org.organization_id).await?;
    let public = git_providers::bitbucket_oauth::public_url(&state).await?;
    Ok(ok(BitbucketOauthStatusDto {
        connected: status.connected,
        user_login: status.user_login,
        user_id: status.user_id,
        scopes: status.scopes,
        connected_at: status.connected_at,
        expires_at: status.expires_at,
        has_client_credentials: status.has_client_credentials,
        has_webhook_secret: status.has_webhook_secret,
        webhook_url: git_providers::bitbucket_oauth::git_webhook_url(&public, &provider_id),
        redirect_uri: status.redirect_uri,
    }))
}

#[utoipa::path(
    post, path = "/api/v1/organizations/{organization_id}/git-providers/{provider_id}/bitbucket-oauth/begin",
    responses((status = 200, description = "Bitbucket authorize URL", body = BitbucketOauthBeginResponse)),
    tag = "git-providers"
)]
pub async fn bitbucket_oauth_begin_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::GitProviderManage>,
    Path((_organization_id, provider_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let authorize_url = git_providers::bitbucket_oauth::begin_authorize(
        &state,
        &org.organization_id,
        &org.user.user_id,
        &provider_id,
    )
    .await?;
    let public = git_providers::bitbucket_oauth::public_url(&state).await?;
    Ok(ok(BitbucketOauthBeginResponse {
        authorize_url,
        redirect_uri: git_providers::bitbucket_oauth::redirect_uri(&public, &provider_id),
    }))
}

#[utoipa::path(
    get, path = "/api/v1/git-providers/{provider_id}/bitbucket-oauth/callback",
    responses((status = 302, description = "Redirects back to the git providers page")),
    tag = "git-providers"
)]
pub async fn bitbucket_oauth_callback_handler(
    State(state): State<SharedState>,
    Query(query): Query<GitlabOauthCallbackQuery>,
) -> Result<impl IntoResponse, AppError> {
    let provider_id =
        git_providers::bitbucket_oauth::complete_authorize(&state, &query.code, &query.state)
            .await?;
    let public = git_providers::bitbucket_oauth::public_url(&state)
        .await
        .unwrap_or_default();
    let target = format!(
        "{}/git-providers?bitbucket_connected={provider_id}",
        public.trim_end_matches('/')
    );
    Ok((StatusCode::FOUND, Redirect::to(&target)))
}

#[utoipa::path(
    delete, path = "/api/v1/organizations/{organization_id}/git-providers/{provider_id}/bitbucket-oauth",
    responses((status = 200, description = "Bitbucket OAuth tokens removed; client credentials kept")),
    tag = "git-providers"
)]
pub async fn bitbucket_oauth_disconnect_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::GitProviderManage>,
    Path((_organization_id, provider_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    git_providers::bitbucket_oauth::disconnect(&state, &provider_id, &org.organization_id).await?;
    Ok(ok(()))
}

// ── Gitea/Forgejo OAuth ───────────────────────────────────────────────────────
//
// Same shape as GitLab/Bitbucket OAuth: begin / status / disconnect live on the
// org-authenticated router, and the callback is public because the provider
// redirects the user's browser with no session attached.

#[utoipa::path(
    get, path = "/api/v1/organizations/{organization_id}/git-providers/{provider_id}/gitea-oauth",
    responses((status = 200, description = "Gitea/Forgejo OAuth connection status", body = GiteaOauthStatusDto)),
    tag = "git-providers"
)]
pub async fn gitea_oauth_status_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgRead>,
    Path((_organization_id, provider_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let status =
        git_providers::gitea_oauth::status(&state, &provider_id, &org.organization_id).await?;
    let public = git_providers::gitea_oauth::public_url(&state).await?;
    Ok(ok(GiteaOauthStatusDto {
        connected: status.connected,
        user_login: status.user_login,
        user_id: status.user_id,
        scopes: status.scopes,
        connected_at: status.connected_at,
        expires_at: status.expires_at,
        has_client_credentials: status.has_client_credentials,
        has_webhook_secret: status.has_webhook_secret,
        webhook_url: git_providers::gitea_oauth::git_webhook_url(&public, &provider_id),
        redirect_uri: git_providers::gitea_oauth::redirect_uri(&public, &provider_id),
    }))
}

#[utoipa::path(
    post, path = "/api/v1/organizations/{organization_id}/git-providers/{provider_id}/gitea-oauth/begin",
    responses((status = 200, description = "Gitea/Forgejo authorize URL", body = GiteaOauthBeginResponse)),
    tag = "git-providers"
)]
pub async fn gitea_oauth_begin_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::GitProviderManage>,
    Path((_organization_id, provider_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let authorize_url = git_providers::gitea_oauth::begin_authorize(
        &state,
        &org.organization_id,
        &org.user.user_id,
        &provider_id,
    )
    .await?;
    let public = git_providers::gitea_oauth::public_url(&state).await?;
    Ok(ok(GiteaOauthBeginResponse {
        authorize_url,
        redirect_uri: git_providers::gitea_oauth::redirect_uri(&public, &provider_id),
    }))
}

#[utoipa::path(
    get, path = "/api/v1/git-providers/{provider_id}/gitea-oauth/callback",
    responses((status = 302, description = "Redirects back to the git providers page")),
    tag = "git-providers"
)]
pub async fn gitea_oauth_callback_handler(
    State(state): State<SharedState>,
    Query(query): Query<GitlabOauthCallbackQuery>,
) -> Result<impl IntoResponse, AppError> {
    let provider_id =
        git_providers::gitea_oauth::complete_authorize(&state, &query.code, &query.state).await?;
    let public = git_providers::gitea_oauth::public_url(&state)
        .await
        .unwrap_or_default();
    let target = format!(
        "{}/git-providers?gitea_connected={provider_id}",
        public.trim_end_matches('/')
    );
    Ok((StatusCode::FOUND, Redirect::to(&target)))
}

#[utoipa::path(
    delete, path = "/api/v1/organizations/{organization_id}/git-providers/{provider_id}/gitea-oauth",
    responses((status = 200, description = "Gitea/Forgejo OAuth tokens removed; client credentials kept")),
    tag = "git-providers"
)]
pub async fn gitea_oauth_disconnect_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::GitProviderManage>,
    Path((_organization_id, provider_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    git_providers::gitea_oauth::disconnect(&state, &provider_id, &org.organization_id).await?;
    Ok(ok(()))
}
