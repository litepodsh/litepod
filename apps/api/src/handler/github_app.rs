use axum::{
    Json,
    extract::{Path, Query, State},
    response::{IntoResponse, Redirect},
};

use crate::{
    dtos::{
        git_provider::{BeginManifestRequest, GithubAppDto, InstallUrlDto, ManifestFormDto},
        response::{AppError, ok},
    },
    git::git_providers::github_app,
    security::authz::extract::{RequireOrg, perm},
    state::state::SharedState,
};

/// Callbacks always bounce the browser back to the UI. Nothing is rendered here and neither the
/// manifest code nor the state nonce is echoed back.
async fn back_to_ui(
    state: &SharedState,
    provider_id: Option<&str>,
    error: Option<&str>,
) -> Redirect {
    // In dev mode the tunnel only reaches the API (port 6001), not the UI (port 6002).
    // Redirect to localhost so the browser hits the SvelteKit dev server directly.
    let base = if state.dev_mode {
        "http://localhost:6002".to_string()
    } else {
        crate::handler::settings::public_base_url(state).await
    };
    let mut query = match error {
        Some(message) => format!("github_app=error&message={}", urlencoding::encode(message)),
        None => "github_app=ok".to_string(),
    };
    if let Some(provider_id) = provider_id {
        query.push_str(&format!("&provider={}", urlencoding::encode(provider_id)));
    }
    Redirect::to(&format!(
        "{}/git-providers?{query}",
        base.trim_end_matches('/')
    ))
}

#[utoipa::path(
    post, path = "/api/v1/organizations/{organization_id}/git-providers/github-app/manifest", request_body = BeginManifestRequest,
    responses((status = 200, description = "Form fields for GitHub's app manifest endpoint", body = ManifestFormDto)),
    tag = "github-app"
)]
pub async fn begin_manifest_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::GitProviderManage>,
    Json(request): Json<BeginManifestRequest>,
) -> Result<impl IntoResponse, AppError> {
    let form = github_app::begin_manifest(
        &state,
        &org.organization_id,
        &org.user.user_id,
        &request.name,
        request.organization.as_deref(),
    )
    .await?;
    let _ = crate::git::repositories::audit_log::record(
        &state,
        &org.organization_id,
        &org.user,
        "github_app.manifest_started",
        Some("git_provider"),
        Some(&form.provider_id),
        Some(&request.name),
        None,
        None,
        None,
        None,
    )
    .await;
    Ok(ok(ManifestFormDto {
        post_url: form.post_url,
        manifest: form.manifest,
        state: form.state,
        provider_id: form.provider_id,
    }))
}

#[utoipa::path(
    get, path = "/api/v1/organizations/{organization_id}/git-providers/{provider_id}/github-app",
    responses((status = 200, description = "GitHub App status", body = GithubAppDto)),
    tag = "github-app"
)]
pub async fn get_github_app_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgRead>,
    // The extractor covers `organization_id`; only `provider_id` is left.
    Path((_organization_id, provider_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let app = github_app::get(&state, &provider_id, &org.organization_id).await?;
    Ok(ok(GithubAppDto {
        webhook_url: github_app::webhook_url(
            &crate::handler::settings::public_base_url(&state).await,
            &provider_id,
        ),
        git_provider_id: app.git_provider_id,
        app_id: app.app_id,
        slug: app.slug,
        owner_login: app.owner_login,
        owner_type: app.owner_type,
        html_url: app.html_url,
        installation_id: app.installation_id,
        status: app.status,
    }))
}

#[utoipa::path(
    post, path = "/api/v1/organizations/{organization_id}/git-providers/{provider_id}/github-app/install-url",
    responses((status = 200, description = "Where to install the app on GitHub", body = InstallUrlDto)),
    tag = "github-app"
)]
pub async fn install_url_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::GitProviderManage>,
    // The extractor covers `organization_id`; only `provider_id` is left.
    Path((_organization_id, provider_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let install_url = github_app::install_url(
        &state,
        &org.organization_id,
        &org.user.user_id,
        &provider_id,
    )
    .await?;
    let _ = crate::git::repositories::audit_log::record(
        &state,
        &org.organization_id,
        &org.user,
        "github_app.install_started",
        Some("git_provider"),
        Some(&provider_id),
        None,
        None,
        None,
        None,
        None,
    )
    .await;
    Ok(ok(InstallUrlDto { install_url }))
}

#[derive(serde::Deserialize)]
pub struct ManifestCallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
}

/// Unauthenticated by design: GitHub redirects the browser cross-site, so a SameSite session
/// cookie may not ride along. The single-use state nonce is the authorization.
#[utoipa::path(
    get, path = "/api/v1/github-app/manifest/callback",
    responses((status = 302, description = "Redirects back to the git providers page")),
    tag = "github-app"
)]
pub async fn manifest_callback_handler(
    State(state): State<SharedState>,
    Query(query): Query<ManifestCallbackQuery>,
) -> impl IntoResponse {
    let (Some(code), Some(state_token)) = (query.code, query.state) else {
        return back_to_ui(&state, None, Some("GitHub did not return an app code")).await;
    };
    match github_app::complete_manifest(&state, &code, &state_token).await {
        Ok(provider_id) => back_to_ui(&state, Some(&provider_id), None).await,
        Err(error) => back_to_ui(&state, None, Some(&error.to_string())).await,
    }
}

#[derive(serde::Deserialize)]
pub struct InstallCallbackQuery {
    pub installation_id: Option<i64>,
    pub state: Option<String>,
    #[allow(dead_code)] // forwarded to GitHub analytics once the post-install survey is wired.
    pub setup_action: Option<String>,
}

#[utoipa::path(
    get, path = "/api/v1/github-app/install/callback",
    responses((status = 302, description = "Redirects back to the git providers page")),
    tag = "github-app"
)]
pub async fn install_callback_handler(
    State(state): State<SharedState>,
    Query(query): Query<InstallCallbackQuery>,
) -> impl IntoResponse {
    let (Some(installation_id), Some(state_token)) = (query.installation_id, query.state) else {
        return back_to_ui(
            &state,
            None,
            Some("GitHub did not return an installation id"),
        )
        .await;
    };
    match github_app::complete_install(&state, &state_token, installation_id).await {
        Ok(provider_id) => back_to_ui(&state, Some(&provider_id), None).await,
        Err(error) => back_to_ui(&state, None, Some(&error.to_string())).await,
    }
}
