import { api } from './base-client';

/** Fields GitHub's app-manifest endpoint expects as a browser form POST. */
export type ManifestFormDto = {
	post_url: string;
	manifest: string;
	state: string;
	provider_id: string;
};

export type GithubAppDto = {
	git_provider_id: string;
	app_id: number | null;
	slug: string | null;
	owner_login: string;
	owner_type: 'user' | 'organization';
	html_url: string;
	installation_id: number | null;
	status: 'pending_manifest' | 'pending_install' | 'ready';
	webhook_url: string;
};

function basePath(organizationId: string): string {
	return `/api/v1/organizations/${encodeURIComponent(organizationId)}/git-providers`;
}

export async function beginGitHubAppManifest(
	organizationId: string,
	name: string,
	organization?: string,
): Promise<ManifestFormDto> {
	const response = await api<ManifestFormDto>(`${basePath(organizationId)}/github-app/manifest`, {
		method: 'POST',
		body: JSON.stringify({ name, organization: organization?.trim() || null }),
	});
	return response.data!;
}

export async function getGitHubApp(
	organizationId: string,
	providerId: string,
): Promise<GithubAppDto> {
	const response = await api<GithubAppDto>(
		`${basePath(organizationId)}/${encodeURIComponent(providerId)}/github-app`,
	);
	return response.data!;
}

export async function getGitHubAppInstallUrl(
	organizationId: string,
	providerId: string,
): Promise<string> {
	const response = await api<{ install_url: string }>(
		`${basePath(organizationId)}/${encodeURIComponent(providerId)}/github-app/install-url`,
		{ method: 'POST' },
	);
	return response.data!.install_url;
}
