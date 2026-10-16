<script lang="ts">
	import SkeletonRows from '#lib/components/custom/skeleton-rows.svelte';
	import { onMount } from 'svelte';
	import EyeIcon from '@lucide/svelte/icons/eye';
	import EyeOffIcon from '@lucide/svelte/icons/eye-off';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import CopyButton from '#lib/components/custom/copy-button.svelte';
	import { ApiError } from '#lib/clients/base-client.js';
	import {
		getGitWebhookStatus,
		type GitProviderDto,
		type GitWebhookStatusDto,
	} from '#lib/clients/git-provider-client.js';
	import { t } from '#lib/i18n/index.svelte.js';
	import { toast } from '#lib/utils/toast.js';

	let { provider, organizationId }: { provider: GitProviderDto; organizationId: string } = $props();

	let status = $state<GitWebhookStatusDto | null>(null);
	let loading = $state(true);
	let showSecret = $state(false);

	async function load() {
		loading = true;
		try {
			status = await getGitWebhookStatus(organizationId, provider.id);
		} catch (error) {
			// The response carries the webhook secret, so the API only serves it to
			// Editors and above. A Viewer gets a 403 — that's the expected answer for
			// them, not a failure, so the card just stays empty.
			if (!(error instanceof ApiError && error.status === 403)) {
				toast.error(
					error instanceof Error ? error.message : t('git-providers.webhook.load_failed'),
				);
			}
		} finally {
			loading = false;
		}
	}

	onMount(() => {
		void load();
	});
</script>

<div class="rounded-lg border bg-muted/30 p-4 text-sm">
	<p class="font-medium">{t('git-providers.webhook.title')}</p>
	<p class="mt-1 text-xs text-muted-foreground">
		{t('git-providers.webhook.description', {
			provider: provider.provider_type === 'forgejo' ? 'Forgejo' : 'Gitea',
		})}
	</p>
	{#if loading}
		<SkeletonRows rows={2} height="h-10" label={t('git-providers.webhook.loading')} class="mt-3" />
	{:else if status}
		<div class="mt-3 grid gap-3">
			<div class="grid gap-1.5">
				<label class="text-xs font-medium text-muted-foreground" for="webhook-url"
					>{t('git-providers.webhook.payload_url')}</label>
				<div class="flex gap-2">
					<Input id="webhook-url" readonly value={status.webhook_url} class="font-mono text-xs" />
					<CopyButton
						value={status.webhook_url}
						label={null}
						revealOnHover={false}
						errorMessage={t('git-providers.webhook.copy_url_failed')}
						class="inline-flex size-9 shrink-0 items-center justify-center rounded-md border bg-background hover:bg-accent hover:text-accent-foreground"
						iconClass="size-4" />
				</div>
			</div>
			<div class="grid gap-1.5">
				<label class="text-xs font-medium text-muted-foreground" for="webhook-secret"
					>{t('git-providers.webhook.secret')}</label>
				<div class="relative">
					<Input
						id="webhook-secret"
						readonly
						type={showSecret ? 'text' : 'password'}
						value={status.webhook_secret}
						class="pr-20 font-mono text-xs" />
					<Button
						type="button"
						variant="ghost"
						size="icon"
						class="absolute right-9 top-0 size-9"
						onclick={() => (showSecret = !showSecret)}
						aria-label={showSecret
							? t('git-providers.webhook.hide_secret')
							: t('git-providers.webhook.reveal_secret')}>
						{#if showSecret}
							<EyeOffIcon class="size-4" />
						{:else}
							<EyeIcon class="size-4" />
						{/if}
					</Button>
					<CopyButton
						value={status.webhook_secret}
						label={null}
						revealOnHover={false}
						errorMessage={t('git-providers.webhook.copy_secret_failed')}
						class="absolute right-2 top-2" />
				</div>
			</div>
			<p class="text-xs text-muted-foreground">
				{t('git-providers.webhook.hint_before')}
				<span class="font-mono">Content type</span>
				{t('git-providers.webhook.hint_to')}
				<span class="font-mono">application/json</span>
				{t('git-providers.webhook.hint_and_check')}
				<span class="font-mono">Push events</span>
				{t('git-providers.webhook.hint_only')}
			</p>
		</div>
	{/if}
</div>
