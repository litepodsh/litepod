<script lang="ts">
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import GithubIcon from '#lib/components/icons/github-icon.svelte';
	import LoaderCircleIcon from '@lucide/svelte/icons/loader-circle';
	import { beginGitHubAppManifest } from '#lib/clients/github-app-client.js';
	import { t } from '#lib/i18n/index.svelte.js';
	import { toast } from '#lib/utils/toast.js';

	let { open = $bindable(false), organizationId }: { open?: boolean; organizationId: string } =
		$props();

	let name = $state('');
	let target = $state<'user' | 'organization'>('user');
	let organization = $state('');
	let busy = $state(false);

	// GitHub's manifest endpoint only accepts a real browser form POST with a `manifest` field —
	// a fetch() cannot create the app, so the form below is submitted as a top-level navigation.
	let formElement = $state<HTMLFormElement | null>(null);
	let postUrl = $state('');
	let manifest = $state('');

	$effect(() => {
		if (open) {
			name = name || 'GitHub';
			busy = false;
		}
	});

	async function connect() {
		if (!name.trim()) {
			toast.error(t('common.connections.connection_name_required'));
			return;
		}
		if (target === 'organization' && !organization.trim()) {
			toast.error(t('common.connections.organization_login_required'));
			return;
		}
		busy = true;
		try {
			const form = await beginGitHubAppManifest(
				organizationId,
				name.trim(),
				target === 'organization' ? organization.trim() : undefined,
			);
			postUrl = form.post_url;
			manifest = form.manifest;
			// Wait for the bound values to land in the DOM before submitting.
			await Promise.resolve();
			formElement?.submit();
		} catch (error) {
			busy = false;
			toast.error(
				error instanceof Error ? error.message : t('git-providers.github-app.flow_failed'),
			);
		}
	}
</script>

<Dialog.Root bind:open>
	<Dialog.Content class="sm:max-w-lg">
		<Dialog.Header>
			<Dialog.Title class="flex items-center gap-2"
				><GithubIcon class="size-5" /> {t('git-providers.github-app.title')}</Dialog.Title>
			<Dialog.Description>{t('git-providers.github-app.description')}</Dialog.Description>
		</Dialog.Header>

		<div class="grid gap-4">
			<div class="grid gap-2">
				<Label for="github-app-name">{t('git-providers.github-app.name')}</Label>
				<Input
					id="github-app-name"
					bind:value={name}
					maxlength={100}
					placeholder="GitHub"
					disabled={busy} />
			</div>
			<div class="grid gap-2">
				<Label>{t('git-providers.github-app.install_on')}</Label>
				<div class="flex gap-2">
					<Button
						variant={target === 'user' ? 'default' : 'outline'}
						disabled={busy}
						onclick={() => (target = 'user')}>{t('git-providers.github-app.my_account')}</Button>
					<Button
						variant={target === 'organization' ? 'default' : 'outline'}
						disabled={busy}
						onclick={() => (target = 'organization')}
						>{t('git-providers.github-app.organization')}</Button>
				</div>
			</div>
			{#if target === 'organization'}
				<div class="grid gap-2">
					<Label for="github-app-org">{t('git-providers.github-app.org_login')}</Label>
					<Input
						id="github-app-org"
						bind:value={organization}
						placeholder="acme-inc"
						disabled={busy} />
					<p class="text-xs text-muted-foreground">
						{t('git-providers.github-app.org_hint_before')}
						<code>github.com/acme-inc</code>{t('git-providers.github-app.org_hint_after')}
					</p>
				</div>
			{/if}
			<p class="rounded-md bg-muted/50 p-3 text-xs text-muted-foreground">
				{t('git-providers.github-app.flow_description')}
			</p>
		</div>

		<form method="POST" action={postUrl} bind:this={formElement} class="hidden">
			<input type="hidden" name="manifest" value={manifest} />
		</form>

		<Dialog.Footer>
			<Button variant="outline" onclick={() => (open = false)} disabled={busy}
				>{t('common.common.cancel')}</Button>
			<Button onclick={() => void connect()} disabled={busy}>
				{#if busy}<LoaderCircleIcon class="size-4 animate-spin" />
					{t('git-providers.github-app.redirecting')}{:else}{t(
						'git-providers.github-app.continue',
					)}{/if}
			</Button>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>
