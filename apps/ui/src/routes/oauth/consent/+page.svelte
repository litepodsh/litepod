<script lang="ts">
	import { page } from '$app/state';
	import { onMount } from 'svelte';
	import * as Card from '#lib/components/ui/card/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { t } from '#lib/i18n/index.svelte.js';
	import {
		getConsent,
		submitConsent,
		organizationFromResource,
	} from '#lib/clients/oauth-client.js';

	const params = page.url.searchParams;
	const clientId = params.get('client_id') ?? '';
	const redirectUri = params.get('redirect_uri') ?? '';
	const organizationId = organizationFromResource(params.get('resource') ?? '') ?? '';

	let clientName = $state('');
	let organizationName = $state('');
	let redirectHost = $state('');
	let remote = $state(false);
	let loading = $state(true);
	let submitting = $state(false);
	let error = $state('');

	onMount(async () => {
		// The page must not invent a decision from a hand-edited URL. A missing
		// piece means the authorize redirect never ran; show it, offer nothing.
		if (!organizationId || !clientId || !redirectUri) {
			error = t('organizations.oauth.error');
			loading = false;
			return;
		}
		try {
			const info = await getConsent(organizationId, clientId, redirectUri);
			clientName = info.client_name;
			organizationName = info.organization.name;
			redirectHost = info.redirect_host;
			remote = info.remote;
		} catch (err) {
			error = err instanceof Error ? err.message : t('organizations.oauth.error');
		} finally {
			loading = false;
		}
	});

	async function decide(approve: boolean) {
		if (submitting) return;
		submitting = true;
		error = '';
		try {
			const redirectTo = await submitConsent(
				organizationId,
				{
					response_type: params.get('response_type') ?? '',
					client_id: clientId,
					redirect_uri: redirectUri,
					code_challenge: params.get('code_challenge') ?? '',
					code_challenge_method: params.get('code_challenge_method') ?? '',
					state: params.get('state') ?? '',
				},
				approve,
			);
			// The answer always goes to the client's own redirect_uri.
			window.location.assign(redirectTo);
		} catch (err) {
			error = err instanceof Error ? err.message : t('organizations.oauth.error');
			submitting = false;
		}
	}
</script>

<svelte:head>
	<title>{t('organizations.oauth.title')} - litepod</title>
</svelte:head>

<div class="bg-muted flex min-h-svh flex-col items-center justify-center p-6 md:p-10">
	<Card.Root class="w-full max-w-md">
		<Card.Header class="text-center">
			<Card.Title>{t('organizations.oauth.title')}</Card.Title>
		</Card.Header>
		<Card.Content class="flex flex-col gap-4">
			{#if loading}
				<p class="text-muted-foreground text-center text-sm">{t('common.common.loading')}</p>
			{:else if error}
				<p class="text-destructive text-center text-sm font-medium">{error}</p>
			{:else}
				<p class="text-balance text-center text-sm font-medium">
					{t('organizations.oauth.wants_access', { client: clientName, organization: organizationName })}
				</p>
				<p class="text-muted-foreground border-border text-balance rounded-md border border-dashed px-3 py-2 text-center text-xs" role="note">
					{t('organizations.oauth.unverified')}
				</p>
				<p class="text-muted-foreground text-center text-sm">{t('organizations.oauth.can_do')}</p>
				<p class="text-muted-foreground text-center text-xs">
					{t('organizations.oauth.redirect_note', { host: redirectHost })}
				</p>
				{#if remote}
					<p class="text-destructive text-balance text-center text-sm font-medium" role="alert">
						{t('organizations.oauth.remote_warning', { host: redirectHost })}
					</p>
				{/if}
				<div class="mt-2 flex gap-3">
					<Button
						variant="outline"
						class="flex-1"
						disabled={submitting}
						onclick={() => decide(false)}>
						{t('organizations.oauth.deny')}
					</Button>
					<Button class="flex-1" disabled={submitting} onclick={() => decide(true)}>
						{submitting ? t('common.common.loading') : t('organizations.oauth.approve')}
					</Button>
				</div>
			{/if}
		</Card.Content>
	</Card.Root>
</div>
