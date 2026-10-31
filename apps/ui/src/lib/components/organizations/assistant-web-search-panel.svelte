<script lang="ts">
	/**
	 * The organization's web search for the assistant and MCP agents.
	 *
	 * There is no search built in: an owner brings a SearXNG instance or a Brave
	 * Search API key. The key is write-only, so a saved setting only offers an
	 * on/off switch and delete. Replacing a key means deleting and adding again.
	 */
	import { onMount } from 'svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
	import DeleteConfirmControl from '#lib/components/custom/delete-confirm-control.svelte';
	import SoftSwitch from '#lib/components/custom/soft-switch.svelte';
	import LoaderCircleIcon from '@lucide/svelte/icons/loader-circle';
	import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';
	import Trash2Icon from '@lucide/svelte/icons/trash-2';
	import { toast } from '#lib/utils/toast.js';
	import {
		deleteWebSearch,
		getWebSearch,
		saveWebSearch,
		setWebSearchEnabled,
		type WebSearchDto,
		type WebSearchProvider,
	} from '#lib/clients/assistant-web-search-client.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let { organizationId }: { organizationId: string } = $props();

	let setting = $state<WebSearchDto | null>(null);
	let loading = $state(true);
	let provider = $state<WebSearchProvider>('searxng');
	let baseUrl = $state('');
	let apiKey = $state('');
	let saving = $state(false);
	let toggling = $state(false);
	let failure = $state('');
	let confirmingDelete = $state(false);
	let deleting = $state(false);

	const ready = $derived(provider === 'searxng' ? baseUrl.trim() !== '' : apiKey.trim() !== '');

	onMount(async () => {
		try {
			setting = await getWebSearch(organizationId);
		} catch (error) {
			toast.error(
				error instanceof Error ? error.message : t('organizations.assistant.search_load_failed'),
			);
		} finally {
			loading = false;
		}
	});

	async function save() {
		saving = true;
		failure = '';
		try {
			setting = await saveWebSearch(organizationId, {
				provider,
				base_url: provider === 'searxng' ? baseUrl.trim() : undefined,
				api_key: provider === 'brave' ? apiKey.trim() : undefined,
			});
			// The key has been sent; nothing in the page should keep holding it.
			apiKey = '';
			baseUrl = '';
		} catch (error) {
			failure =
				error instanceof Error ? error.message : t('organizations.assistant.search_save_failed');
		} finally {
			saving = false;
		}
	}

	async function toggle(enabled: boolean) {
		toggling = true;
		try {
			setting = await setWebSearchEnabled(organizationId, enabled);
		} catch (error) {
			toast.error(
				error instanceof Error ? error.message : t('organizations.assistant.search_update_failed'),
			);
		} finally {
			toggling = false;
		}
	}

	async function remove() {
		deleting = true;
		try {
			await deleteWebSearch(organizationId);
			setting = null;
			confirmingDelete = false;
		} catch (error) {
			toast.error(
				error instanceof Error ? error.message : t('organizations.assistant.search_delete_failed'),
			);
		} finally {
			deleting = false;
		}
	}
</script>

<div class="lp-raised bg-card flex flex-col gap-4 rounded-2xl border p-4">
	<div class="max-w-[70ch]">
		<p class="text-sm font-medium">{t('organizations.assistant.search_title')}</p>
		<p class="text-muted-foreground mt-1 text-sm">
			{t('organizations.assistant.search_description')}
		</p>
	</div>

	{#if loading}
		<p class="text-muted-foreground text-sm">{t('common.connections.loading')}</p>
	{:else if setting}
		<div class="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
			<div class="min-w-0">
				<div class="flex flex-wrap items-center gap-2">
					<strong class="text-sm font-semibold">
						{setting.provider === 'brave'
							? t('organizations.assistant.search_provider_brave')
							: t('organizations.assistant.search_provider_searxng')}
					</strong>
					<span
						class="rounded-full border border-current/30 bg-current/10 px-2 py-0.5 font-mono text-[10px] tracking-wide uppercase {setting.enabled
							? 'text-success'
							: 'text-muted-foreground'}">
						{setting.enabled
							? t('organizations.assistant.search_active')
							: t('organizations.assistant.search_disabled')}
					</span>
				</div>
				<p class="text-muted-foreground mt-1 text-xs">
					{#if setting.provider === 'searxng'}
						<span class="font-mono break-all">{setting.base_url}</span>
					{:else}
						{t('organizations.assistant.search_key_stored')}
					{/if}
				</p>
			</div>
			<div class="flex shrink-0 items-center gap-3">
				<SoftSwitch
					id="web-search-enabled"
					checked={setting.enabled}
					disabled={toggling}
					onCheckedChange={toggle} />
				<Button
					variant="ghost"
					size="sm"
					class="text-destructive"
					onclick={() => (confirmingDelete = true)}>
					<Trash2Icon class="size-3.5" />
				</Button>
			</div>
		</div>
	{:else}
		<div class="flex flex-col gap-4">
			<div class="grid gap-2 sm:grid-cols-2">
				{#each [['searxng', t('organizations.assistant.search_provider_searxng')], ['brave', t('organizations.assistant.search_provider_brave')]] as [id, label] (id)}
					<button
						type="button"
						aria-pressed={provider === id}
						onclick={() => (provider = id as WebSearchProvider)}
						class="hover:border-foreground/25 aria-pressed:border-primary aria-pressed:bg-primary/5 rounded-lg border p-2.5 text-left text-sm font-medium transition-colors">
						{label}
					</button>
				{/each}
			</div>

			{#if provider === 'searxng'}
				<div class="flex flex-col gap-2">
					<Label for="web-search-url">{t('organizations.assistant.search_url_label')}</Label>
					<Input
						id="web-search-url"
						bind:value={baseUrl}
						placeholder="https://search.example.com"
						class="font-mono text-xs"
						autocomplete="off" />
					<p class="text-muted-foreground text-xs">
						{t('organizations.assistant.search_url_hint')}
					</p>
				</div>
			{:else}
				<div class="flex flex-col gap-2">
					<Label for="web-search-key">{t('organizations.assistant.search_key_label')}</Label>
					<Input
						id="web-search-key"
						type="password"
						bind:value={apiKey}
						class="font-mono text-xs"
						autocomplete="new-password" />
					<p class="text-muted-foreground text-xs">
						{t('organizations.assistant.search_key_hint')}
					</p>
				</div>
			{/if}

			{#if failure}
				<div
					class="text-destructive flex items-start gap-2 rounded-lg border border-current/25 bg-current/5 p-3 text-sm">
					<TriangleAlertIcon class="mt-0.5 size-4 shrink-0" />
					<p class="min-w-0 break-words">{failure}</p>
				</div>
			{/if}

			<div>
				<Button onclick={save} disabled={!ready || saving}>
					{#if saving}<LoaderCircleIcon class="size-4 animate-spin" />{/if}
					{saving
						? t('organizations.assistant.search_testing')
						: t('organizations.assistant.search_save')}
				</Button>
			</div>
		</div>
	{/if}
</div>

<AlertDialog.Root open={confirmingDelete} onOpenChange={(open) => (confirmingDelete = open)}>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title>{t('organizations.assistant.search_delete_title')}</AlertDialog.Title>
			<AlertDialog.Description>
				{t('organizations.assistant.search_delete_description')}
			</AlertDialog.Description>
		</AlertDialog.Header>
		<AlertDialog.Footer>
			<AlertDialog.Cancel>{t('common.connections.cancel')}</AlertDialog.Cancel>
			<DeleteConfirmControl disabled={deleting} onconfirm={remove} />
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>
