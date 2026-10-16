<script lang="ts">
	/**
	 * The organization's model connections.
	 *
	 * A row's status carries the provider's own error text rather than a red
	 * icon: a rejected key and a closed local port are fixed in completely
	 * different ways, and "failed" tells the user neither.
	 */
	import { onMount } from 'svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
	import DeleteConfirmControl from '#lib/components/custom/delete-confirm-control.svelte';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';
	import PencilIcon from '@lucide/svelte/icons/pencil';
	import Trash2Icon from '@lucide/svelte/icons/trash-2';
	import { toast } from '#lib/utils/toast.js';
	import AssistantConnectionDialog from './assistant-connection-dialog.svelte';
	import AssistantProviderIcon from '#lib/components/custom/assistant-provider-icon.svelte';
	import {
		deleteConnection,
		listConnections,
		listProviders,
		refreshConnection,
		updateConnection,
		type ConnectionDto,
		type ProviderDto,
	} from '#lib/clients/assistant-connection-client.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let { organizationId }: { organizationId: string } = $props();

	let connections = $state<ConnectionDto[]>([]);
	let providers = $state<ProviderDto[]>([]);
	let loading = $state(true);
	let dialogOpen = $state(false);
	let editing = $state<ConnectionDto | null>(null);
	let deleting = $state<ConnectionDto | null>(null);
	let busy = $state<string | null>(null);

	// A local endpoint is one that authenticates nothing. Grouping by that rather
	// than by hostname keeps the distinction the user actually cares about: which
	// of these rows involves a key they have to manage.
	const local = $derived(
		connections.filter((connection) => providerFor(connection)?.key === 'none'),
	);
	const hosted = $derived(
		connections.filter((connection) => providerFor(connection)?.key !== 'none'),
	);

	function providerFor(connection: ConnectionDto): ProviderDto | undefined {
		return providers.find((provider) => provider.id === connection.provider);
	}

	function ago(timestamp: number | null): string {
		if (!timestamp) return t('organizations.assistant.never_read');
		const minutes = Math.floor((Date.now() - timestamp) / 60000);
		if (minutes < 1) return t('organizations.assistant.read_just_now');
		if (minutes < 60) return t('organizations.assistant.read_minutes_ago', { count: minutes });
		const hours = Math.floor(minutes / 60);
		if (hours < 24) return t('organizations.assistant.read_hours_ago', { count: hours });
		return t('organizations.assistant.read_days_ago', { count: Math.floor(hours / 24) });
	}

	onMount(async () => {
		try {
			[providers, connections] = await Promise.all([
				listProviders(),
				listConnections(organizationId),
			]);
		} catch (error) {
			toast.error(error instanceof Error ? error.message : t('common.connections.load_failed'));
		} finally {
			loading = false;
		}
	});

	function replace(saved: ConnectionDto) {
		const index = connections.findIndex((connection) => connection.id === saved.id);
		if (index === -1) connections = [...connections, saved];
		else connections[index] = saved;
		// Only one row can be the default, and the server has just moved it.
		if (saved.is_default) {
			connections = connections.map((connection) =>
				connection.id === saved.id ? connection : { ...connection, is_default: false },
			);
		}
	}

	async function reload(connection: ConnectionDto) {
		busy = connection.id;
		try {
			replace(await refreshConnection(organizationId, connection.id));
		} catch (error) {
			toast.error(
				error instanceof Error ? error.message : t('organizations.assistant.reload_failed'),
			);
		} finally {
			busy = null;
		}
	}

	async function makeDefault(connection: ConnectionDto) {
		busy = connection.id;
		try {
			replace(await updateConnection(organizationId, connection.id, { is_default: true }));
		} catch (error) {
			toast.error(
				error instanceof Error ? error.message : t('organizations.assistant.default_failed'),
			);
		} finally {
			busy = null;
		}
	}

	async function remove() {
		if (!deleting) return;
		const target = deleting;
		busy = target.id;
		try {
			await deleteConnection(organizationId, target.id);
			// The server promotes a replacement when the default goes, so read the
			// list back rather than guessing which row took over.
			connections = await listConnections(organizationId);
			deleting = null;
		} catch (error) {
			toast.error(
				error instanceof Error ? error.message : t('organizations.assistant.delete_failed'),
			);
		} finally {
			busy = null;
		}
	}

	function add() {
		editing = null;
		dialogOpen = true;
	}

	function edit(connection: ConnectionDto) {
		editing = connection;
		dialogOpen = true;
	}
</script>

{#snippet row(connection: ConnectionDto)}
	<article
		class="lp-raised bg-card flex flex-col gap-3 rounded-2xl border p-4 sm:flex-row sm:items-center sm:gap-4"
		class:border-primary={connection.is_default}>
		<span
			class="lp-inset bg-muted text-muted-foreground grid size-10 shrink-0 place-items-center rounded-[9px]">
			<AssistantProviderIcon provider={connection.provider} class="size-5" />
		</span>

		<div class="min-w-0 flex-1">
			<div class="flex flex-wrap items-center gap-2">
				<strong class="text-sm font-semibold">{connection.name}</strong>
				{#if connection.status === 'ok'}
					<span
						class="text-success rounded-full border border-current/30 bg-current/10 px-2 py-0.5 font-mono text-[10px] tracking-wide uppercase"
						>{t('organizations.assistant.verified')}</span>
				{:else if connection.status === 'failed'}
					<span
						class="text-warning rounded-full border border-current/30 bg-current/10 px-2 py-0.5 font-mono text-[10px] tracking-wide uppercase"
						>{t('organizations.assistant.not_reachable')}</span>
				{/if}
				{#if connection.is_default}
					<span class="text-primary font-mono text-[10px] tracking-wide uppercase">
						{t('organizations.assistant.default')}
					</span>
				{/if}
			</div>

			<p class="text-muted-foreground mt-1 flex flex-wrap items-center gap-x-2 gap-y-0.5 text-xs">
				{#if connection.has_key}
					<span class="font-mono">{connection.api_key_hint}</span>
					<span class="opacity-40">·</span>
				{:else}
					<span class="font-mono">{connection.base_url}</span>
					<span class="opacity-40">·</span>
				{/if}
				<span
					>{t('organizations.assistant.models_count', { count: connection.models.length })}</span>
				<span class="opacity-40">·</span>
				<span>
					{t('organizations.assistant.catalog', { time: ago(connection.models_fetched_at) })}
				</span>
			</p>

			{#if connection.status_detail}
				<!-- The provider's own words: a 401 and a closed port need different fixes. -->
				<p class="text-warning mt-1 font-mono text-xs break-words">{connection.status_detail}</p>
			{/if}
		</div>

		<div class="flex shrink-0 flex-wrap gap-1">
			{#if !connection.is_default}
				<Button
					variant="ghost"
					size="sm"
					disabled={busy === connection.id}
					onclick={() => makeDefault(connection)}
					>{t('organizations.assistant.make_default')}</Button>
			{/if}
			<Button
				variant="ghost"
				size="sm"
				disabled={busy === connection.id}
				onclick={() => reload(connection)}>
				<RefreshCwIcon class="size-3.5" />
				{t('common.connections.reload')}
			</Button>
			<Button variant="ghost" size="sm" onclick={() => edit(connection)}>
				<PencilIcon class="size-3.5" />
			</Button>
			<Button
				variant="ghost"
				size="sm"
				class="text-destructive"
				onclick={() => (deleting = connection)}>
				<Trash2Icon class="size-3.5" />
			</Button>
		</div>
	</article>
{/snippet}

<div class="flex flex-col gap-4">
	<div class="flex flex-wrap items-start justify-between gap-4">
		<div class="max-w-[70ch]">
			<p class="text-sm font-medium">{t('common.connections.model_connections')}</p>
			<p class="text-muted-foreground mt-1 text-sm">
				{t('organizations.assistant.connections_description')}
			</p>
		</div>
		<Button onclick={add}>
			<PlusIcon class="size-4" />
			{t('common.connections.add')}
		</Button>
	</div>

	{#if loading}
		<p class="text-muted-foreground text-sm">{t('common.connections.loading')}</p>
	{:else if connections.length === 0}
		<button
			type="button"
			onclick={add}
			class="text-muted-foreground hover:text-foreground hover:border-primary w-full rounded-2xl border border-dashed border-foreground/15 p-6 text-sm transition-colors">
			{t('common.connections.none')}
		</button>
	{:else}
		<div class="flex flex-col gap-2">
			{#each hosted as connection (connection.id)}
				{@render row(connection)}
			{/each}
		</div>

		{#if local.length > 0}
			<div class="mt-2 flex flex-col gap-2">
				<p class="text-muted-foreground font-mono text-[10px] tracking-widest uppercase">
					{t('organizations.assistant.on_this_machine')}
				</p>
				{#each local as connection (connection.id)}
					{@render row(connection)}
				{/each}
			</div>
		{/if}
	{/if}
</div>

<AssistantConnectionDialog
	bind:open={dialogOpen}
	{organizationId}
	{providers}
	connection={editing}
	onsaved={replace} />

<AlertDialog.Root open={deleting !== null} onOpenChange={(open) => !open && (deleting = null)}>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title
				>{t('common.connections.delete_title', { name: deleting?.name })}</AlertDialog.Title>
			<AlertDialog.Description>
				{t('common.connections.delete_description')}
			</AlertDialog.Description>
		</AlertDialog.Header>
		<AlertDialog.Footer>
			<AlertDialog.Cancel>{t('common.connections.cancel')}</AlertDialog.Cancel>
			<DeleteConfirmControl disabled={busy !== null} onconfirm={remove} />
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>
