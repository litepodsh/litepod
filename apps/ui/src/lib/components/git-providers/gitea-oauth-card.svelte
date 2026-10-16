<script lang="ts">
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import CircleCheckIcon from '@lucide/svelte/icons/circle-check';
	import CopyButton from '#lib/components/custom/copy-button.svelte';
	import ExternalLinkIcon from '@lucide/svelte/icons/external-link';
	import KeyRoundIcon from '@lucide/svelte/icons/key-round';
	import LinkIcon from '@lucide/svelte/icons/link';
	import LoaderCircleIcon from '@lucide/svelte/icons/loader-circle';
	import UnlinkIcon from '@lucide/svelte/icons/unlink';
	import {
		beginGiteaOauth,
		disconnectGiteaOauth,
		getGiteaOauthStatus,
		updateGitProvider,
		type GitProviderDto,
		type GiteaOauthStatusDto,
	} from '#lib/clients/git-provider-client.js';
	import { t } from '#lib/i18n/index.svelte.js';
	import { toast } from '#lib/utils/toast.js';

	let {
		provider,
		organizationId,
		onProviderChange,
	}: {
		provider: GitProviderDto;
		organizationId: string;
		onProviderChange: (provider: GitProviderDto) => void;
	} = $props();

	let status = $state<GiteaOauthStatusDto | null>(null);
	let loading = $state(true);
	let savingCredentials = $state(false);
	let connecting = $state(false);
	let disconnecting = $state(false);
	let error = $state<string | null>(null);

	let clientId = $state('');
	let clientSecret = $state('');

	async function refresh() {
		loading = true;
		try {
			status = await getGiteaOauthStatus(organizationId, provider.id);
			error = null;
		} catch (caught) {
			error = caught instanceof Error ? caught.message : t('git-providers.gitea-oauth.load_failed');
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		void refresh();
	});

	async function saveCredentials() {
		if (!clientId.trim() || !clientSecret.trim()) {
			toast.error(t('git-providers.gitea-oauth.credentials_required'));
			return;
		}
		savingCredentials = true;
		try {
			const updated = await updateGitProvider(organizationId, provider.id, {
				gitea_oauth_client_id: clientId.trim(),
				gitea_oauth_client_secret: clientSecret.trim(),
			});
			onProviderChange(updated);
			clientId = '';
			clientSecret = '';
			await refresh();
			toast.success(t('git-providers.gitea-oauth.credentials_saved'));
		} catch (caught) {
			toast.error(
				caught instanceof Error
					? caught.message
					: t('git-providers.gitea-oauth.credentials_save_failed'),
			);
		} finally {
			savingCredentials = false;
		}
	}

	async function connect() {
		if (!status?.has_client_credentials) {
			toast.error(t('git-providers.gitea-oauth.connect_requires_credentials'));
			return;
		}
		connecting = true;
		try {
			const { authorize_url } = await beginGiteaOauth(organizationId, provider.id);
			window.location.assign(authorize_url);
		} catch (caught) {
			toast.error(
				caught instanceof Error ? caught.message : t('git-providers.gitea-oauth.connect_failed'),
			);
			connecting = false;
		}
	}

	async function disconnect() {
		disconnecting = true;
		try {
			await disconnectGiteaOauth(organizationId, provider.id);
			await refresh();
			toast.success(t('git-providers.gitea-oauth.disconnected'));
		} catch (caught) {
			toast.error(
				caught instanceof Error ? caught.message : t('git-providers.gitea-oauth.disconnect_failed'),
			);
		} finally {
			disconnecting = false;
		}
	}

	function formatExpiry(timestamp: number | null): string {
		if (!timestamp) return t('git-providers.gitea-oauth.expiry_never');
		const minutes = Math.max(0, Math.round((timestamp - Date.now()) / 60_000));
		if (minutes < 60) return t('git-providers.gitea-oauth.expiry_minutes', { count: minutes });
		const hours = Math.round(minutes / 60);
		return t('git-providers.gitea-oauth.expiry_hours', { count: hours });
	}
</script>

<section
	class="grid gap-3 rounded-lg border bg-muted/30 p-4"
	aria-label={t('git-providers.gitea-oauth.title')}>
	<header class="flex items-center justify-between gap-2">
		<div>
			<h3 class="flex items-center gap-2 text-sm font-medium">
				<KeyRoundIcon class="size-4" />
				{t('git-providers.gitea-oauth.title')}
			</h3>
			<p class="mt-1 text-xs text-muted-foreground">
				{t('git-providers.gitea-oauth.description')}
			</p>
		</div>
		{#if loading}
			<LoaderCircleIcon class="size-4 animate-spin text-muted-foreground" />
		{:else if status?.connected}
			<span
				class="inline-flex items-center gap-1 rounded-full bg-success/10 px-2 py-0.5 text-xs font-medium text-success dark:bg-success/20">
				<CircleCheckIcon class="size-3" />
				{t('git-providers.gitea-oauth.connected_as', { login: status.user_login })}
			</span>
		{:else if status?.has_client_credentials}
			<span
				class="inline-flex items-center gap-1 rounded-full bg-warning px-2 py-0.5 text-xs font-medium text-warning">
				<LinkIcon class="size-3" />
				{t('git-providers.gitea-oauth.not_connected')}
			</span>
		{/if}
	</header>

	{#if status?.connected}
		<dl class="grid gap-1 text-xs text-muted-foreground">
			<div class="flex justify-between">
				<dt>{t('git-providers.gitea-oauth.token_expires')}</dt>
				<dd class="font-mono">{formatExpiry(status.expires_at)}</dd>
			</div>
			<div class="flex justify-between">
				<dt>{t('git-providers.gitea-oauth.scopes')}</dt>
				<dd class="font-mono">{status.scopes ?? t('git-providers.gitea-oauth.scopes_default')}</dd>
			</div>
		</dl>
		<Button variant="outline" size="sm" onclick={() => void disconnect()} disabled={disconnecting}>
			<UnlinkIcon class="size-4" />
			{disconnecting
				? t('git-providers.gitea-oauth.disconnecting')
				: t('git-providers.gitea-oauth.disconnect')}
		</Button>
	{:else}
		<div class="grid gap-2 text-xs">
			<p class="font-medium">{t('git-providers.gitea-oauth.step1_title')}</p>
			<ol class="list-decimal space-y-1 pl-5 text-muted-foreground">
				<li>
					{t('git-providers.gitea-oauth.step1_open_settings', { host: provider.host_url })}
				</li>
				<li>
					{t('git-providers.gitea-oauth.step1_paste_redirect')}
					{#if status?.redirect_uri}
						<CopyButton
							value={status.redirect_uri}
							errorMessage={t('git-providers.gitea-oauth.copy_failed_redirect')}
							class="rounded bg-background px-1 py-0.5 font-mono text-[11px] text-foreground"
							iconClass="size-3" />
					{:else}
						<code class="rounded bg-background px-1 py-0.5 font-mono text-[11px]"
							>{t('common.common.loading')}</code>
					{/if}
				</li>
			</ol>
		</div>

		<div class="grid gap-2">
			<Label for="gitea-client-id">{t('git-providers.gitea-oauth.application_id')}</Label>
			<Input id="gitea-client-id" bind:value={clientId} placeholder="abc123…" autocomplete="off" />
		</div>
		<div class="grid gap-2">
			<Label for="gitea-client-secret">{t('git-providers.gitea-oauth.secret')}</Label>
			<Input
				id="gitea-client-secret"
				type="password"
				bind:value={clientSecret}
				placeholder="gto_xxxxxxxxxxxx"
				autocomplete="off" />
		</div>
		<Button
			variant="outline"
			size="sm"
			onclick={() => void saveCredentials()}
			disabled={savingCredentials || !clientId.trim() || !clientSecret.trim()}>
			{savingCredentials
				? t('common.common.saving')
				: t('git-providers.gitea-oauth.save_credentials')}
		</Button>

		<div class="grid gap-2 text-xs">
			<p class="font-medium">{t('git-providers.gitea-oauth.step2_title')}</p>
			<p class="text-muted-foreground">
				{t('git-providers.gitea-oauth.step2_description')}
			</p>
			<div class="grid gap-1 rounded-md border bg-background p-2">
				<div class="flex items-center justify-between gap-2">
					<span class="text-muted-foreground">{t('git-providers.gitea-oauth.url')}</span>
					{#if status?.webhook_url}
						<CopyButton
							value={status.webhook_url}
							errorMessage={t('git-providers.gitea-oauth.copy_failed_webhook')}
							class="min-w-0 font-mono text-[11px] text-foreground"
							iconClass="size-3" />
					{:else}
						<code class="truncate font-mono text-[11px]">—</code>
					{/if}
				</div>
				<div class="flex items-center justify-between gap-2">
					<span class="text-muted-foreground">{t('git-providers.gitea-oauth.trigger')}</span>
					<span class="font-mono text-[11px]">{t('git-providers.gitea-oauth.push_events')}</span>
				</div>
			</div>
		</div>

		<div class="grid gap-2 text-xs">
			<p class="font-medium">{t('git-providers.gitea-oauth.step3_title')}</p>
			<Button
				size="sm"
				onclick={() => void connect()}
				disabled={!status?.has_client_credentials || connecting}>
				<ExternalLinkIcon class="size-4" />
				{connecting
					? t('git-providers.gitea-oauth.opening_instance')
					: t('git-providers.gitea-oauth.connect')}
			</Button>
		</div>
	{/if}

	{#if error}
		<p
			class="rounded-md border border-destructive/30 bg-destructive/5 p-2 text-xs text-destructive">
			{error}
		</p>
	{/if}
</section>
