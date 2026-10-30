<script lang="ts">
	import { onMount } from 'svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
	import DeleteConfirmControl from '#lib/components/custom/delete-confirm-control.svelte';
	import GitBranchIcon from '@lucide/svelte/icons/git-branch';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import PencilIcon from '@lucide/svelte/icons/pencil';
	import Trash2Icon from '@lucide/svelte/icons/trash-2';
	import GitProviderIcon from '#lib/components/custom/git-provider-icon.svelte';
	import GithubAppConnectDialog from '#lib/components/git-providers/github-app-connect-dialog.svelte';
	import GitlabOauthCard from '#lib/components/git-providers/gitlab-oauth-card.svelte';
	import BitbucketOauthCard from '#lib/components/git-providers/bitbucket-oauth-card.svelte';
	import GiteaOauthCard from '#lib/components/git-providers/gitea-oauth-card.svelte';
	import GitWebhookCard from '#lib/components/git-providers/git-webhook-card.svelte';
	import {
		getGitHubApp,
		getGitHubAppInstallUrl,
		type GithubAppDto,
	} from '#lib/clients/github-app-client.js';
	import GitProviderFormDialog from '#lib/components/git-providers/git-provider-form-dialog.svelte';
	import {
		createGitProvider,
		deleteGitProvider,
		listGitProviders,
		updateGitProvider,
		type GitProviderDto,
		type GitProviderInput,
	} from '#lib/clients/git-provider-client.js';
	import { toast } from '#lib/utils/toast.js';
	import SkeletonRows from '#lib/components/custom/skeleton-rows.svelte';
	import { invalidateOnboarding } from '#lib/state/onboarding.svelte.js';
	import { t } from '#lib/i18n/index.svelte.js';

	const chip =
		'lp-raised inline-flex h-6 items-center rounded-md bg-muted px-2 text-xs font-normal';
	let avatarFailed = $state<Record<string, boolean>>({});

	/** GitHub serves a user/org's avatar at a predictable URL — real photo
	 *  instead of the generic brand icon when we can be sure it's really
	 *  github.com (not an Enterprise host that wouldn't resolve the same way). */
	function githubAvatarUrl(provider: GitProviderDto): string | null {
		if (provider.provider_type !== 'github') return null;
		if (provider.host_url && !provider.host_url.includes('github.com')) return null;
		const login =
			provider.kind === 'github_app' ? githubApps[provider.id]?.owner_login : provider.username;
		if (!login) return null;
		return `https://github.com/${encodeURIComponent(login)}.png?size=64`;
	}

	const TYPE_LABELS: Record<string, string> = {
		github: 'GitHub',
		gitlab: 'GitLab',
		bitbucket: 'Bitbucket',
		gitea: 'Gitea',
		forgejo: 'Forgejo',
		generic: 'Git',
	};

	let { organizationId }: { organizationId: string } = $props();

	let providers = $state<GitProviderDto[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);
	let dialogOpen = $state(false);
	let editing = $state<GitProviderDto | null>(null);
	let deleting = $state<GitProviderDto | null>(null);
	let busy = $state(false);
	let githubDialogOpen = $state(false);
	let githubApps = $state<Record<string, GithubAppDto>>({});
	// Tracks which providers have their GitLab OAuth card expanded. Persisted to localStorage
	// so a refresh does not collapse the section the user is editing.
	let expandedGitlabOAuth = $state<Record<string, boolean>>({});
	let expandedBitbucketOAuth = $state<Record<string, boolean>>({});
	let expandedGiteaOAuth = $state<Record<string, boolean>>({});
	let expandedGitWebhook = $state<Record<string, boolean>>({});

	onMount(async () => {
		try {
			const stored = localStorage.getItem('gitlab-oauth-expanded');
			if (stored) {
				try {
					expandedGitlabOAuth = JSON.parse(stored);
				} catch {
					expandedGitlabOAuth = {};
				}
			}
			const storedBitbucket = localStorage.getItem('bitbucket-oauth-expanded');
			if (storedBitbucket) {
				try {
					expandedBitbucketOAuth = JSON.parse(storedBitbucket);
				} catch {
					expandedBitbucketOAuth = {};
				}
			}
			const storedGitea = localStorage.getItem('gitea-oauth-expanded');
			if (storedGitea) {
				try {
					expandedGiteaOAuth = JSON.parse(storedGitea);
				} catch {
					expandedGiteaOAuth = {};
				}
			}
			providers = await listGitProviders(organizationId);
			await loadGithubApps();
			await handleCallback();
		} catch (caught) {
			error = caught instanceof Error ? caught.message : t('operations.git_providers.load_failed');
		} finally {
			loading = false;
		}
	});

	function toggleGitlabOAuth(providerId: string) {
		expandedGitlabOAuth = {
			...expandedGitlabOAuth,
			[providerId]: !expandedGitlabOAuth[providerId],
		};
		localStorage.setItem('gitlab-oauth-expanded', JSON.stringify(expandedGitlabOAuth));
	}

	function toggleGiteaOAuth(providerId: string) {
		expandedGiteaOAuth = {
			...expandedGiteaOAuth,
			[providerId]: !expandedGiteaOAuth[providerId],
		};
		localStorage.setItem('gitea-oauth-expanded', JSON.stringify(expandedGiteaOAuth));
	}

	function toggleBitbucketOAuth(providerId: string) {
		expandedBitbucketOAuth = {
			...expandedBitbucketOAuth,
			[providerId]: !expandedBitbucketOAuth[providerId],
		};
		localStorage.setItem('bitbucket-oauth-expanded', JSON.stringify(expandedBitbucketOAuth));
	}

	function toggleGitWebhook(providerId: string) {
		expandedGitWebhook = {
			...expandedGitWebhook,
			[providerId]: !expandedGitWebhook[providerId],
		};
	}

	function updateProvider(updated: GitProviderDto) {
		providers = providers.map((p) => (p.id === updated.id ? updated : p));
	}

	async function loadGithubApps() {
		const apps = await Promise.all(
			providers
				.filter((provider) => provider.kind === 'github_app')
				.map((provider) => getGitHubApp(organizationId, provider.id).catch(() => null)),
		);
		githubApps = Object.fromEntries(
			apps
				.filter((app): app is GithubAppDto => app !== null)
				.map((app) => [app.git_provider_id, app]),
		);
	}

	/// The GitHub callbacks land on the API, which bounces back here with the outcome in the query.
	async function handleCallback() {
		const params = new URLSearchParams(window.location.search);
		const githubOutcome = params.get('github_app');
		const gitlabConnected = params.get('gitlab_connected');
		const bitbucketConnected = params.get('bitbucket_connected');
		const giteaConnected = params.get('gitea_connected');
		if (!githubOutcome && !gitlabConnected && !bitbucketConnected && !giteaConnected) return;
		// Drop the callback outcome but keep the settings tab this panel lives under.
		const tab = params.get('tab');
		const path = window.location.pathname;
		history.replaceState(null, '', tab ? `${path}?tab=${encodeURIComponent(tab)}` : path);

		if (githubOutcome === 'error') {
			toast.error(params.get('message') ?? t('operations.git_providers.github_connect_failed'));
		} else if (githubOutcome) {
			const providerId = params.get('provider');
			if (providerId) {
				const app = githubApps[providerId];
				if (app?.status === 'pending_install') {
					await goToInstall(providerId);
					return;
				}
				toast.success(
					app?.status === 'ready'
						? t('operations.git_providers.github_connected')
						: t('operations.git_providers.github_app_created'),
				);
			}
		} else if (gitlabConnected) {
			toast.success(t('operations.git_providers.gitlab_connected'));
			// Make sure the OAuth card opens so the user immediately sees the new state.
			expandedGitlabOAuth = { ...expandedGitlabOAuth, [gitlabConnected]: true };
			localStorage.setItem('gitlab-oauth-expanded', JSON.stringify(expandedGitlabOAuth));
		} else if (bitbucketConnected) {
			toast.success(t('operations.git_providers.bitbucket_connected'));
			expandedBitbucketOAuth = { ...expandedBitbucketOAuth, [bitbucketConnected]: true };
			localStorage.setItem('bitbucket-oauth-expanded', JSON.stringify(expandedBitbucketOAuth));
		} else if (giteaConnected) {
			toast.success(t('operations.git_providers.gitea_connected'));
			expandedGiteaOAuth = { ...expandedGiteaOAuth, [giteaConnected]: true };
			localStorage.setItem('gitea-oauth-expanded', JSON.stringify(expandedGiteaOAuth));
		}
	}

	async function goToInstall(providerId: string) {
		try {
			window.location.assign(await getGitHubAppInstallUrl(organizationId, providerId));
		} catch (caught) {
			toast.error(
				caught instanceof Error
					? caught.message
					: t('operations.git_providers.github_installation_failed'),
			);
		}
	}

	// A fresh `gitea_oauth` provider still needs its OAuth app and Connect: open the card.
	function expandGiteaOAuth(provider: GitProviderDto) {
		if (provider.auth_method !== 'gitea_oauth') return;
		expandedGiteaOAuth = { ...expandedGiteaOAuth, [provider.id]: true };
		localStorage.setItem('gitea-oauth-expanded', JSON.stringify(expandedGiteaOAuth));
	}

	async function submit(input: GitProviderInput) {
		if (editing) {
			const updated = await updateGitProvider(organizationId, editing.id, input);
			providers = providers.map((provider) => (provider.id === updated.id ? updated : provider));
			expandGiteaOAuth(updated);
			toast.success(t('operations.git_providers.provider_saved'));
		} else {
			const created = await createGitProvider(organizationId, input);
			providers = [...providers, created];
			expandGiteaOAuth(created);
			toast.success(t('operations.git_providers.provider_added'));
		}
		invalidateOnboarding();
	}

	async function confirmDelete() {
		if (!deleting) return;
		busy = true;
		try {
			await deleteGitProvider(organizationId, deleting.id);
			providers = providers.filter((provider) => provider.id !== deleting!.id);
			toast.success(t('operations.git_providers.provider_deleted'));
			invalidateOnboarding();
			deleting = null;
		} catch (caught) {
			toast.error(
				caught instanceof Error
					? caught.message
					: t('operations.git_providers.provider_delete_failed'),
			);
		} finally {
			busy = false;
		}
	}
</script>

<div class="flex flex-col gap-4">
	<div class="flex flex-wrap items-end justify-between gap-3">
		<div class="min-w-0">
			<h2 class="text-lg font-semibold tracking-tight">{t('operations.git_providers.title')}</h2>
			<p class="mt-1 text-sm text-muted-foreground">
				{t('operations.git_providers.credentials_description')}
			</p>
		</div>
		<div class="flex flex-wrap gap-2">
			<Button size="sm" onclick={() => (githubDialogOpen = true)}
				><GitProviderIcon type="github" class="size-4" />
				{t('operations.git_providers.connect_github')}</Button>
			<Button
				size="sm"
				variant="outline"
				onclick={() => {
					editing = null;
					dialogOpen = true;
				}}><PlusIcon class="size-4" /> {t('operations.git_providers.add_credentials')}</Button>
		</div>
	</div>

	{#if loading}
		<SkeletonRows rows={3} height="h-16" label={t('operations.git_providers.loading')} />
	{:else if error}
		<div class="rounded-2xl border border-destructive/30 bg-card p-6 text-sm">
			<p class="font-medium text-destructive">{t('operations.git_providers.unable_to_load')}</p>
			<p class="mt-1 text-muted-foreground">{error}</p>
		</div>
	{:else if providers.length === 0}
		<div
			class="flex flex-col items-center justify-center rounded-2xl border border-dashed border-foreground/15 bg-card p-8 text-center">
			<div class="lp-raised flex size-12 items-center justify-center rounded-xl border bg-muted">
				<GitBranchIcon class="size-5 text-muted-foreground" />
			</div>
			<h3 class="mt-4 font-medium">{t('operations.git_providers.no_providers')}</h3>
			<p class="mt-1 max-w-sm text-sm text-muted-foreground">
				{t('operations.git_providers.no_providers_description')}
			</p>
		</div>
	{:else}
		<ul class="lp-raised divide-y overflow-hidden rounded-2xl border bg-card">
			{#each providers as provider (provider.id)}
				<li class="grid gap-3 p-4">
					<div class="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
						<div class="flex min-w-0 items-center gap-3">
							<div
								class="lp-inset flex size-9 shrink-0 items-center justify-center overflow-hidden rounded-[9px] bg-muted">
								{#if githubAvatarUrl(provider) && !avatarFailed[provider.id]}
									<img
										src={githubAvatarUrl(provider)}
										alt=""
										class="size-full object-cover"
										onerror={() => (avatarFailed[provider.id] = true)} />
								{:else}
									<GitProviderIcon type={provider.provider_type} class="size-4" />
								{/if}
							</div>
							<div class="min-w-0">
								<div class="flex flex-wrap items-center gap-1.5 font-medium">
									<span class="break-words">{provider.name}</span>
									<span class={chip}
										>{TYPE_LABELS[provider.provider_type] ?? provider.provider_type}</span>
									{#if provider.kind === 'github_app'}<span class={chip}
											>{t('operations.git_providers.app')}</span
										>{/if}
									{#if provider.auth_method === 'gitlab_oauth'}<span class={chip}>OAuth</span>{/if}
								</div>
								<div class="text-sm break-words text-muted-foreground">
									{provider.username}{provider.host_url ? ` · ${provider.host_url}` : ''}
								</div>
								{#if provider.kind === 'github_app' && githubApps[provider.id]?.status !== 'ready'}
									<button
										class="mt-1 text-sm font-medium underline-offset-4 hover:underline"
										onclick={() => void goToInstall(provider.id)}
										>{t('operations.git_providers.finish_installation')}</button>
								{/if}
							</div>
						</div>
						<div class="flex flex-wrap gap-1 sm:shrink-0">
							{#if provider.provider_type === 'gitlab'}
								<Button variant="ghost" size="sm" onclick={() => toggleGitlabOAuth(provider.id)}
									>{expandedGitlabOAuth[provider.id]
										? t('operations.git_providers.hide_oauth')
										: t('operations.git_providers.oauth')}</Button>
							{/if}
							{#if provider.provider_type === 'bitbucket'}
								<Button variant="ghost" size="sm" onclick={() => toggleBitbucketOAuth(provider.id)}
									>{expandedBitbucketOAuth[provider.id]
										? t('operations.git_providers.hide_oauth')
										: t('operations.git_providers.oauth')}</Button>
							{/if}
							{#if provider.provider_type === 'gitea' || provider.provider_type === 'forgejo'}
								<Button variant="ghost" size="sm" onclick={() => toggleGiteaOAuth(provider.id)}
									>{expandedGiteaOAuth[provider.id]
										? t('operations.git_providers.hide_oauth')
										: t('operations.git_providers.oauth')}</Button>
								<Button variant="ghost" size="sm" onclick={() => toggleGitWebhook(provider.id)}
									>{expandedGitWebhook[provider.id]
										? t('operations.git_providers.hide_webhook')
										: t('operations.git_providers.webhook')}</Button>
							{/if}
							<Button
								variant="ghost"
								size="icon"
								onclick={() => {
									editing = provider;
									dialogOpen = true;
								}}
								><PencilIcon class="size-4" /><span class="sr-only"
									>{t('operations.git_providers.edit')}</span
								></Button>
							<Button variant="ghost" size="icon" onclick={() => (deleting = provider)}
								><Trash2Icon class="size-4" /><span class="sr-only"
									>{t('operations.git_providers.delete')}</span
								></Button>
						</div>
					</div>
					{#if provider.provider_type === 'gitlab' && expandedGitlabOAuth[provider.id]}
						<GitlabOauthCard {provider} {organizationId} onProviderChange={updateProvider} />
					{/if}
					{#if provider.provider_type === 'bitbucket' && expandedBitbucketOAuth[provider.id]}
						<BitbucketOauthCard {provider} {organizationId} onProviderChange={updateProvider} />
					{/if}
					{#if (provider.provider_type === 'gitea' || provider.provider_type === 'forgejo') && expandedGiteaOAuth[provider.id]}
						<div class="mt-3">
							<GiteaOauthCard {provider} {organizationId} onProviderChange={updateProvider} />
						</div>
					{/if}
					{#if (provider.provider_type === 'gitea' || provider.provider_type === 'forgejo') && expandedGitWebhook[provider.id]}
						<div class="mt-3"><GitWebhookCard {provider} {organizationId} /></div>
					{/if}
				</li>
			{/each}
		</ul>
	{/if}
</div>

<GitProviderFormDialog
	bind:open={dialogOpen}
	provider={editing}
	{organizationId}
	onSubmit={submit} />
<GithubAppConnectDialog bind:open={githubDialogOpen} {organizationId} />

<AlertDialog.Root
	open={deleting !== null}
	onOpenChange={(value) => {
		if (!value) deleting = null;
	}}>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title
				>{t('operations.git_providers.delete_title', { name: deleting?.name })}</AlertDialog.Title>
			<AlertDialog.Description
				>{t('operations.git_providers.delete_description')}</AlertDialog.Description>
		</AlertDialog.Header>
		<AlertDialog.Footer>
			<AlertDialog.Cancel>{t('operations.maintenance.cancel')}</AlertDialog.Cancel>
			<DeleteConfirmControl disabled={busy} onconfirm={confirmDelete} />
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>
