use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub const PROVIDER_TYPES: [&str; 5] = ["github", "gitea", "forgejo", "gitlab", "generic"];

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct GitProviderDto {
    pub id: String,
    pub organization_id: String,
    pub name: String,
    pub provider_type: String,
    pub host_url: String,
    pub username: String,
    pub has_password: bool,
    /// `oauth2` (token sent as basic auth `oauth2:<token>`), `token` (personal access token header),
    /// or `gitlab_oauth` (GitLab OAuth2 access token, refreshed on demand).
    pub auth_method: String,
    /// `credentials` (stored username/secret) or `github_app` (installation tokens minted per use).
    pub kind: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateGitProviderRequest {
    pub name: String,
    pub provider_type: String,
    pub host_url: Option<String>,
    pub username: String,
    pub password: String,
    /// Defaults to `token` when omitted. For GitLab providers, supplying `gitlab_oauth_client_id`
    /// + `gitlab_oauth_client_secret` automatically sets this to `gitlab_oauth`.
    pub auth_method: Option<String>,
    /// GitLab OAuth client_id (from a GitLab Application the user creates manually).
    pub gitlab_oauth_client_id: Option<String>,
    /// Plaintext GitLab OAuth client_secret; the backend encrypts it before persisting.
    pub gitlab_oauth_client_secret: Option<String>,
    /// Bitbucket OAuth consumer key (from a Bitbucket workspace's OAuth consumer). Bitbucket
    /// providers are OAuth-only, so this is the only real credential a Bitbucket create needs.
    pub bitbucket_oauth_client_id: Option<String>,
    /// Plaintext Bitbucket OAuth consumer secret; the backend encrypts it before persisting.
    pub bitbucket_oauth_client_secret: Option<String>,
    /// Gitea/Forgejo OAuth client_id (from an application the user creates in the instance).
    pub gitea_oauth_client_id: Option<String>,
    /// Plaintext Gitea/Forgejo OAuth client_secret; the backend encrypts it before persisting.
    pub gitea_oauth_client_secret: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateGitProviderRequest {
    pub name: Option<String>,
    pub provider_type: Option<String>,
    pub host_url: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub auth_method: Option<String>,
    /// Replaces the stored GitLab OAuth client_id. Only used when provider_type is `gitlab`.
    pub gitlab_oauth_client_id: Option<String>,
    /// Replaces the stored GitLab OAuth client_secret (re-encrypted on persist). Only used when
    /// provider_type is `gitlab`.
    pub gitlab_oauth_client_secret: Option<String>,
    /// Replaces the stored Bitbucket OAuth consumer key. Only used when provider_type is
    /// `bitbucket`.
    pub bitbucket_oauth_client_id: Option<String>,
    /// Replaces the stored Bitbucket OAuth consumer secret (re-encrypted on persist). Only used
    /// when provider_type is `bitbucket`.
    pub bitbucket_oauth_client_secret: Option<String>,
    /// Replaces the stored Gitea/Forgejo OAuth client_id. Only used when provider_type is
    /// `gitea` or `forgejo`.
    pub gitea_oauth_client_id: Option<String>,
    /// Replaces the stored Gitea/Forgejo OAuth client_secret (re-encrypted on persist). Only
    /// used when provider_type is `gitea` or `forgejo`.
    pub gitea_oauth_client_secret: Option<String>,
}

/// Credentials supplied by the provider form to verify repository access without saving them.
#[derive(Debug, Deserialize, ToSchema)]
pub struct TestGitProviderRequest {
    pub provider_type: String,
    pub host_url: Option<String>,
    pub username: String,
    pub password: String,
    pub auth_method: Option<String>,
    /// Required for `generic` providers, which have no repository API — verified with ls-remote.
    pub repo_url: Option<String>,
}

/// Response for `POST /organizations/{organization_id}/git-providers/{provider_id}/gitlab-oauth/begin`.
/// The UI redirects the browser to `authorize_url`; the callback arrives at the public route.
#[derive(Debug, Serialize, ToSchema)]
pub struct GitlabOauthBeginResponse {
    pub authorize_url: String,
    pub redirect_uri: String,
}

/// Mirrors `gitlab_oauth::GitlabOauthStatus` for the OpenAPI surface and UI consumption.
#[derive(Debug, Serialize, ToSchema)]
pub struct GitlabOauthStatusDto {
    pub connected: bool,
    pub user_login: Option<String>,
    pub user_id: Option<String>,
    pub scopes: Option<String>,
    pub connected_at: Option<i64>,
    pub expires_at: Option<i64>,
    pub has_client_credentials: bool,
    pub has_webhook_secret: bool,
    pub webhook_url: String,
    pub redirect_uri: String,
}

/// Response for `POST /organizations/{organization_id}/git-providers/{provider_id}/bitbucket-oauth/begin`.
#[derive(Debug, Serialize, ToSchema)]
pub struct BitbucketOauthBeginResponse {
    pub authorize_url: String,
    /// Paste this into the Bitbucket OAuth consumer's own "Callback URL" field -
    /// Bitbucket has no per-request redirect_uri, unlike GitLab.
    pub redirect_uri: String,
}

/// Mirrors `bitbucket_oauth::BitbucketOauthStatus` for the OpenAPI surface and UI consumption.
#[derive(Debug, Serialize, ToSchema)]
pub struct BitbucketOauthStatusDto {
    pub connected: bool,
    pub user_login: Option<String>,
    pub user_id: Option<String>,
    pub scopes: Option<String>,
    pub connected_at: Option<i64>,
    pub expires_at: Option<i64>,
    pub has_client_credentials: bool,
    pub has_webhook_secret: bool,
    pub webhook_url: String,
    pub redirect_uri: String,
}

/// Response for `POST /organizations/{organization_id}/git-providers/{provider_id}/gitea-oauth/begin`.
#[derive(Debug, Serialize, ToSchema)]
pub struct GiteaOauthBeginResponse {
    pub authorize_url: String,
    pub redirect_uri: String,
}

/// Mirrors `gitea_oauth::GiteaOauthStatus` for the OpenAPI surface and UI consumption.
#[derive(Debug, Serialize, ToSchema)]
pub struct GiteaOauthStatusDto {
    pub connected: bool,
    pub user_login: Option<String>,
    pub user_id: Option<String>,
    pub scopes: Option<String>,
    pub connected_at: Option<i64>,
    pub expires_at: Option<i64>,
    pub has_client_credentials: bool,
    pub has_webhook_secret: bool,
    pub webhook_url: String,
    pub redirect_uri: String,
}

/// Webhook configuration for any provider that uses the unified git webhook route. The
/// `webhook_secret` is the value the user must paste into the provider's webhook settings.
#[derive(Debug, Serialize, ToSchema)]
pub struct GitWebhookStatusDto {
    pub webhook_url: String,
    pub webhook_secret: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct GitRepositoryDto {
    /// `organization/repo` as shown by the provider.
    pub full_name: String,
    pub clone_url: String,
    pub default_branch: Option<String>,
    pub private: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ListBranchesRequest {
    pub repo_url: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct BeginManifestRequest {
    pub name: String,
    /// Organization login when the app should belong to an org rather than the user's account.
    pub organization: Option<String>,
}

/// GitHub's manifest endpoint only accepts a browser form POST, so the UI must submit these
/// fields itself rather than the backend calling out.
#[derive(Debug, Serialize, ToSchema)]
pub struct ManifestFormDto {
    pub post_url: String,
    pub manifest: String,
    pub state: String,
    pub provider_id: String,
}

/// GitHub App state for the UI. Deliberately excludes the private key, webhook secret,
/// client secret, and cached installation token.
#[derive(Debug, Serialize, ToSchema)]
pub struct GithubAppDto {
    pub git_provider_id: String,
    pub app_id: Option<i64>,
    pub slug: Option<String>,
    pub owner_login: String,
    pub owner_type: String,
    pub html_url: String,
    pub installation_id: Option<i64>,
    pub status: String,
    pub webhook_url: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct InstallUrlDto {
    pub install_url: String,
}
