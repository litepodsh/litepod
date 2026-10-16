<script lang="ts">
	/**
	 * Add or edit one model connection.
	 *
	 * The provider grid comes from the API rather than a second hardcoded list
	 * here, so base URLs, protocols and key policies have exactly one home.
	 *
	 * Saving is the probe: the API only stores a connection once its key has
	 * listed the provider's models. A rejected key therefore fails in this
	 * dialog, with the provider's own words, instead of three screens later
	 * inside somebody's conversation.
	 */
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import DialogScrollContent from '#lib/components/custom/dialog-scroll-content.svelte';
	import AssistantProviderIcon from '#lib/components/custom/assistant-provider-icon.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import LoaderCircleIcon from '@lucide/svelte/icons/loader-circle';
	import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';
	import SearchIcon from '@lucide/svelte/icons/search';
	import {
		createConnection,
		updateConnection,
		type ConnectionDto,
		type ProviderDto,
	} from '#lib/clients/assistant-connection-client.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let {
		open = $bindable(false),
		organizationId,
		providers,
		/** Set to edit; null to add. */
		connection = null,
		onsaved,
	}: {
		open?: boolean;
		organizationId: string;
		providers: ProviderDto[];
		connection?: ConnectionDto | null;
		onsaved?: (connection: ConnectionDto) => void;
	} = $props();

	let providerId = $state('');
	let name = $state('');
	let baseUrl = $state('');
	let apiKey = $state('');
	let search = $state('');
	let saving = $state(false);
	let failure = $state<string | null>(null);

	const editing = $derived(connection !== null);
	// Sending a stored key to a host the user just typed is how a key walks out
	// of an organization, so the API refuses it. Ask for the key here rather
	// than let the save come back with an error the dialog could have avoided.
	const movedUrl = $derived(
		editing && Boolean(connection?.has_key) && baseUrl.trim() !== connection?.base_url,
	);
	const chosen = $derived(providers.find((provider) => provider.id === providerId) ?? null);
	const matches = $derived(
		providers.filter((provider) =>
			`${provider.name} ${provider.id}`.toLowerCase().includes(search.trim().toLowerCase()),
		),
	);

	// Reset every time the dialog opens, so a cancelled attempt never leaks its
	// half-filled key into the next one.
	$effect(() => {
		if (!open) return;
		providerId = connection?.provider ?? '';
		name = connection?.name ?? '';
		baseUrl = connection?.base_url ?? '';
		apiKey = '';
		search = '';
		failure = null;
	});

	function pick(provider: ProviderDto) {
		providerId = provider.id;
		// Only fill in what the user has not already typed: switching providers
		// by accident should not silently discard a URL they entered.
		if (!name.trim()) name = provider.name;
		if (!baseUrl.trim() || !provider.custom_url) baseUrl = provider.base_url;
	}

	async function save() {
		failure = null;
		saving = true;
		try {
			const saved = editing
				? await updateConnection(organizationId, connection!.id, {
						name: name.trim(),
						base_url: baseUrl.trim(),
						// An untouched key field keeps the stored key.
						...(apiKey.trim() ? { api_key: apiKey.trim() } : {}),
					})
				: await createConnection(organizationId, {
						name: name.trim(),
						provider: providerId,
						base_url: baseUrl.trim(),
						...(apiKey.trim() ? { api_key: apiKey.trim() } : {}),
					});
			onsaved?.(saved);
			open = false;
		} catch (error) {
			failure = error instanceof Error ? error.message : t('common.connections.save_failed');
		} finally {
			saving = false;
		}
	}

	const ready = $derived(
		Boolean(providerId) &&
			name.trim().length > 0 &&
			baseUrl.trim().length > 0 &&
			// A provider that requires a key needs one, unless one is already stored.
			(chosen?.key !== 'required' || apiKey.trim().length > 0 || Boolean(connection?.has_key)) &&
			(!movedUrl || apiKey.trim().length > 0),
	);
</script>

<Dialog.Root bind:open>
	<DialogScrollContent class="sm:max-w-xl">
		<Dialog.Header class="shrink-0">
			<Dialog.Title
				>{editing
					? t('common.connections.edit_title')
					: t('common.connections.add_title')}</Dialog.Title>
			<Dialog.Description>
				{t('organizations.assistant.key_encrypted')}
			</Dialog.Description>
		</Dialog.Header>

		<div class="flex min-h-0 flex-1 flex-col gap-5 overflow-y-auto pr-1">
			{#if !editing}
				<div class="flex flex-col gap-2">
					<Label for="provider-search">{t('common.connections.provider')}</Label>
					<div class="relative">
						<SearchIcon
							class="text-muted-foreground pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2" />
						<Input
							id="provider-search"
							type="search"
							bind:value={search}
							placeholder={t('common.connections.provider_search')}
							class="pl-9"
							autocomplete="off" />
					</div>
					<div class="grid gap-2 sm:grid-cols-3">
						{#each matches as provider (provider.id)}
							<button
								type="button"
								onclick={() => pick(provider)}
								aria-pressed={provider.id === providerId}
								class="hover:border-foreground/25 aria-pressed:border-primary aria-pressed:bg-primary/5 flex items-center gap-2.5 rounded-lg border p-2.5 text-left transition-colors">
								<AssistantProviderIcon
									provider={provider.id}
									class="text-muted-foreground size-5 shrink-0" />
								<span class="min-w-0">
									<span class="block truncate text-sm font-medium">{provider.name}</span>
									<span class="text-muted-foreground block truncate text-xs">
										{provider.custom_url && !provider.base_url
											? t('organizations.assistant.your_own_url')
											: provider.key === 'none'
												? t('organizations.assistant.local_no_key')
												: t('organizations.assistant.hosted')}
										·
										{provider.protocol === 'anthropic'
											? t('organizations.assistant.protocol_messages')
											: t('organizations.assistant.protocol_openai')}
									</span>
								</span>
							</button>
						{/each}
					</div>
					{#if matches.length === 0}
						<p class="text-muted-foreground text-xs">
							{t('organizations.assistant.no_match_before')}
							<span class="font-medium">{t('organizations.assistant.no_match_term')}</span>
							{t('organizations.assistant.no_match_after')}
						</p>
					{/if}
				</div>
			{/if}

			{#if providerId}
				<div class="flex flex-col gap-2">
					<Label for="connection-name">{t('common.connections.name')}</Label>
					<Input
						id="connection-name"
						bind:value={name}
						placeholder={t('organizations.assistant.name_placeholder')} />
					<p class="text-muted-foreground text-xs">
						{t('organizations.assistant.name_hint')}
					</p>
				</div>

				<div class="flex flex-col gap-2">
					<Label for="connection-url">{t('common.connections.base_url')}</Label>
					<Input id="connection-url" bind:value={baseUrl} class="font-mono text-xs" />
					<p class="text-muted-foreground text-xs">
						{t('organizations.assistant.base_url_hint')}
					</p>
				</div>

				{#if chosen?.key !== 'none'}
					<div class="flex flex-col gap-2">
						<Label for="connection-key">{t('common.connections.key')}</Label>
						<Input
							id="connection-key"
							type="password"
							bind:value={apiKey}
							class="font-mono text-xs"
							autocomplete="off"
							placeholder={connection?.has_key ? connection.api_key_hint : ''} />
						<p class="text-muted-foreground text-xs">
							{#if movedUrl}
								{t('organizations.assistant.key_moved_hint')}
							{:else if connection?.has_key}
								{t('organizations.assistant.key_keep_hint')}
							{:else if chosen?.key === 'optional'}
								{t('organizations.assistant.key_optional_hint')}
							{:else}
								{t('organizations.assistant.key_check_hint')}
							{/if}
						</p>
					</div>
				{/if}
			{/if}

			{#if failure}
				<div
					class="text-destructive flex items-start gap-2 rounded-lg border border-current/25 bg-current/5 p-3 text-sm">
					<TriangleAlertIcon class="mt-0.5 size-4 shrink-0" />
					<p class="min-w-0 break-words">{failure}</p>
				</div>
			{/if}
		</div>

		<Dialog.Footer class="border-border bg-popover shrink-0 border-t pt-4">
			<Button variant="ghost" onclick={() => (open = false)} disabled={saving}
				>{t('common.connections.cancel')}</Button>
			<Button onclick={save} disabled={!ready || saving}>
				{#if saving}<LoaderCircleIcon class="size-4 animate-spin" />{/if}
				{saving
					? t('common.connections.checking')
					: editing
						? t('common.connections.save_connection')
						: t('common.connections.add')}
			</Button>
		</Dialog.Footer>
	</DialogScrollContent>
</Dialog.Root>
