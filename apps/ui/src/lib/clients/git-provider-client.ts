import { api } from './base-client';

export type GitProviderDto = {
	id: string;
	organization_id: string;
	name: string;
	provider_type: 'github' | 'gitea' | 'forgejo' | 'gitlab' | 'bitbucket' | 'generic';
	host_url: string;
	username: string;
	has_password: boolean;
	auth_method: 'oauth2' | 'token' | 'gitlab_oauth' | 'bitbucket_oauth' | 'gitea_oauth';
	kind: 'credentials' | 'github_app';
	created_at: number;
	updated_at: number;
};

export type GitProviderInput = {
	name: string;
	provider_type: string;
	host_url?: string;
	username: string;
	password?: string;
	auth_method?: string;
	gitlab_oauth_client_id?: string;
	gitlab_oauth_client_secret?: string;
	bitbucket_oauth_client_id?: string;
	bitbucket_oauth_client_secret?: string;
	gitea_oauth_client_id?: string;
	gitea_oauth_client_secret?: string;
};

export type GitProviderTestInput = Omit<GitProviderInput, 'name' | 'password'> & {
	password: string;
	repo_url?: string;
};

function providerPath(organizationId: string, providerId?: string): string {
	const base = `/api/v1/organizations/${encodeURIComponent(organizationId)}/git-providers`;
	return providerId ? `${base}/${encodeURIComponent(providerId)}` : base;
}

export async function listGitProviders(organizationId: string): Promise<GitProviderDto[]> {
	const response = await api<GitProviderDto[]>(providerPath(organizationId));
	return response.data ?? [];
}

export async function createGitProvider(
	organizationId: string,
	input: GitProviderInput,
): Promise<GitProviderDto> {
	const response = await api<GitProviderDto>(providerPath(organizationId), {
		method: 'POST',
		body: JSON.stringify(input),
	});
	return response.data!;
}

export async function updateGitProvider(
	organizationId: string,
	providerId: string,
	input: Partial<GitProviderInput>,
): Promise<GitProviderDto> {
	const response = await api<GitProviderDto>(providerPath(organizationId, providerId), {
		method: 'PATCH',
		body: JSON.stringify(input),
	});
	return response.data!;
}

export async function deleteGitProvider(organizationId: string, providerId: string): Promise<void> {
	await api<void>(providerPath(organizationId, providerId), { method: 'DELETE' });
}

export type GitRepositoryDto = {
	full_name: string;
	clone_url: string;
	default_branch: string | null;
	private: boolean;
};

export async function testGitProviderRepositories(
	organizationId: string,
	input: GitProviderTestInput,
): Promise<GitRepositoryDto[]> {
	const response = await api<GitRepositoryDto[]>(
		`${providerPath(organizationId)}/test-repositories`,
		{
			method: 'POST',
			body: JSON.stringify(input),
		},
	);
	return response.data ?? [];
}

export async function listGitRepositories(
	organizationId: string,
	providerId: string,
): Promise<GitRepositoryDto[]> {
	const response = await api<GitRepositoryDto[]>(
		`${providerPath(organizationId, providerId)}/repositories`,
	);
	return response.data ?? [];
}

export async function listGitBranches(
	organizationId: string,
	providerId: string,
	repoUrl: string,
): Promise<string[]> {
	const response = await api<string[]>(`${providerPath(organizationId, providerId)}/branches`, {
		method: 'POST',
		body: JSON.stringify({ repo_url: repoUrl }),
	});
	return response.data ?? [];
}

/** Branch listing with no saved provider — a public repo URL, or one carrying
 * `user:token@host` credentials. */
export async function listPublicGitBranches(
	organizationId: string,
	repoUrl: string,
): Promise<string[]> {
	const response = await api<string[]>(`${providerPath(organizationId)}/branches`, {
		method: 'POST',
		body: JSON.stringify({ repo_url: repoUrl }),
	});
	return response.data ?? [];
}

// ── GitLab OAuth (self-hosted + cloud) ──────────────────────────────────────────
//
// The user pastes client_id + client_secret once, then clicks "Connect with GitLab" which
// redirects to GitLab. GitLab bounces back to the API, which exchanges the code for tokens
// and 302s the browser back to /git-providers?gitlab_connected=<id> with the connected
// status available via getGitlabOauthStatus.

export type GitlabOauthStatusDto = {
	connected: boolean;
	user_login: string | null;
	user_id: string | null;
	scopes: string | null;
	connected_at: number | null;
	expires_at: number | null;
	has_client_credentials: boolean;
	has_webhook_secret: boolean;
	webhook_url: string;
	redirect_uri: string;
};

export type GitlabOauthBeginResponse = {
	authorize_url: string;
	redirect_uri: string;
};

export async function getGitlabOauthStatus(
	organizationId: string,
	providerId: string,
): Promise<GitlabOauthStatusDto> {
	const response = await api<GitlabOauthStatusDto>(
		`${providerPath(organizationId, providerId)}/gitlab-oauth`,
	);
	return response.data!;
}

export async function beginGitlabOauth(
	organizationId: string,
	providerId: string,
): Promise<GitlabOauthBeginResponse> {
	const response = await api<GitlabOauthBeginResponse>(
		`${providerPath(organizationId, providerId)}/gitlab-oauth/begin`,
		{ method: 'POST' },
	);
	return response.data!;
}

export async function disconnectGitlabOauth(
	organizationId: string,
	providerId: string,
): Promise<void> {
	await api<void>(`${providerPath(organizationId, providerId)}/gitlab-oauth`, { method: 'DELETE' });
}

// ── Gitea/Forgejo OAuth (self-hosted) ─────────────────────────────────────────
//
// Same authorize-code flow as GitLab, against the instance's /login/oauth
// endpoints. Scopes cover user identity + organization/repository reads the
// repos API needs.

export type GiteaOauthStatusDto = {
	connected: boolean;
	user_login: string | null;
	user_id: string | null;
	scopes: string | null;
	connected_at: number | null;
	expires_at: number | null;
	has_client_credentials: boolean;
	has_webhook_secret: boolean;
	webhook_url: string;
	redirect_uri: string;
};

export type GiteaOauthBeginResponse = {
	authorize_url: string;
	redirect_uri: string;
};

export async function getGiteaOauthStatus(
	organizationId: string,
	providerId: string,
): Promise<GiteaOauthStatusDto> {
	const response = await api<GiteaOauthStatusDto>(
		`${providerPath(organizationId, providerId)}/gitea-oauth`,
	);
	return response.data!;
}

export async function beginGiteaOauth(
	organizationId: string,
	providerId: string,
): Promise<GiteaOauthBeginResponse> {
	const response = await api<GiteaOauthBeginResponse>(
		`${providerPath(organizationId, providerId)}/gitea-oauth/begin`,
		{ method: 'POST' },
	);
	return response.data!;
}

export async function disconnectGiteaOauth(
	organizationId: string,
	providerId: string,
): Promise<void> {
	await api<void>(`${providerPath(organizationId, providerId)}/gitea-oauth`, { method: 'DELETE' });
}

// ── Bitbucket OAuth (Cloud only) ─────────────────────────────────────────────
//
// Bitbucket has no token auth left (App Passwords are gone); OAuth is the only
// connection path. Same flow as GitLab, except Bitbucket's OAuth consumer has a
// fixed callback URL set once in Bitbucket's own settings - `redirect_uri` here
// is shown to the user to paste there, never sent to Bitbucket by litepod.

export type BitbucketOauthStatusDto = {
	connected: boolean;
	user_login: string | null;
	user_id: string | null;
	scopes: string | null;
	connected_at: number | null;
	expires_at: number | null;
	has_client_credentials: boolean;
	has_webhook_secret: boolean;
	webhook_url: string;
	redirect_uri: string;
};

export type BitbucketOauthBeginResponse = {
	authorize_url: string;
	redirect_uri: string;
};

export async function getBitbucketOauthStatus(
	organizationId: string,
	providerId: string,
): Promise<BitbucketOauthStatusDto> {
	const response = await api<BitbucketOauthStatusDto>(
		`${providerPath(organizationId, providerId)}/bitbucket-oauth`,
	);
	return response.data!;
}

export async function beginBitbucketOauth(
	organizationId: string,
	providerId: string,
): Promise<BitbucketOauthBeginResponse> {
	const response = await api<BitbucketOauthBeginResponse>(
		`${providerPath(organizationId, providerId)}/bitbucket-oauth/begin`,
		{ method: 'POST' },
	);
	return response.data!;
}

export async function disconnectBitbucketOauth(
	organizationId: string,
	providerId: string,
): Promise<void> {
	await api<void>(`${providerPath(organizationId, providerId)}/bitbucket-oauth`, {
		method: 'DELETE',
	});
}

export type GitWebhookStatusDto = {
	webhook_url: string;
	webhook_secret: string;
};

export async function getGitWebhookStatus(
	organizationId: string,
	providerId: string,
): Promise<GitWebhookStatusDto> {
	const response = await api<GitWebhookStatusDto>(
		`${providerPath(organizationId, providerId)}/git-webhook`,
	);
	return response.data!;
}
