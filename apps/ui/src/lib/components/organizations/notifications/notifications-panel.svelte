<script lang="ts">
	import SkeletonRows from '#lib/components/custom/skeleton-rows.svelte';
	import { onMount } from 'svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
	import DeleteConfirmControl from '#lib/components/custom/delete-confirm-control.svelte';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import BellIcon from '@lucide/svelte/icons/bell';
	import ProviderCard from './provider-card.svelte';
	import ProviderDialog from './provider-dialog.svelte';
	import DeliveryLog from './delivery-log.svelte';
	import { isInFlight } from './status';
	import {
		createProvider,
		deleteProvider,
		getCatalog,
		listProviders,
		testSaved,
		updateProvider,
		type NotificationCatalog,
		type NotificationProviderDto,
	} from '#lib/clients/notification-client.js';
	import { toast } from '#lib/utils/toast.js';
	import { invalidateOnboarding } from '#lib/state/onboarding.svelte.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let { organizationId, canManage }: { organizationId: string; canManage: boolean } = $props();

	let catalog = $state<NotificationCatalog | null>(null);
	let providers = $state<NotificationProviderDto[]>([]);
	let loading = $state(true);
	let dialogOpen = $state(false);
	let editing = $state<NotificationProviderDto | null>(null);
	let confirmDelete = $state<NotificationProviderDto | null>(null);
	let deleting = $state(false);
	let testingId = $state<string | null>(null);
	let logOpen = $state(false);
	let logProvider = $state<NotificationProviderDto | null>(null);

	const existingNames = $derived(providers.map((provider) => provider.name));

	onMount(() => {
		load();

		// Deliveries land from a background worker, so the card health only
		// changes if we refetch. Poll while something is in flight, and stop the
		// moment the queue drains — a quiet page costs nothing.
		const POLL_MS = 3000;
		let timer: ReturnType<typeof setInterval> | null = null;
		$effect(() => {
			const pending = providers.some(
				(provider) => provider.last_delivery && isInFlight(provider.last_delivery.status),
			);
			if (pending && timer === null) {
				timer = setInterval(() => {
					if (document.visibilityState === 'visible') refresh();
				}, POLL_MS);
			} else if (!pending && timer !== null) {
				clearInterval(timer);
				timer = null;
			}
		});
		return () => {
			if (timer !== null) clearInterval(timer);
		};
	});

	async function load() {
		loading = true;
		try {
			const [loadedCatalog, loadedProviders] = await Promise.all([
				getCatalog(organizationId),
				listProviders(organizationId),
			]);
			catalog = loadedCatalog;
			providers = loadedProviders;
		} catch (err) {
			toast.error(
				err instanceof Error ? err.message : t('organizations.notifications.load_failed'),
			);
		} finally {
			loading = false;
		}
	}

	/** Refetch without flashing the loading placeholder. */
	async function refresh() {
		try {
			providers = await listProviders(organizationId);
		} catch {
			// A failed poll is not worth a toast; the next tick retries.
		}
	}

	function openCreate() {
		editing = null;
		dialogOpen = true;
	}

	function openEdit(provider: NotificationProviderDto) {
		editing = provider;
		dialogOpen = true;
	}

	async function handleSubmit(input: {
		name: string;
		kind: NotificationProviderDto['kind'];
		decoration: boolean;
		config?: { webhook_url?: string; bot_token?: string; chat_id?: string };
		actions: NotificationProviderDto['actions'];
	}) {
		if (editing) {
			const saved = await updateProvider(organizationId, editing.id, {
				name: input.name,
				enabled: editing.enabled,
				decoration: input.decoration,
				config: input.config,
				actions: input.actions,
			});
			providers = providers.map((provider) => (provider.id === saved.id ? saved : provider));
			toast.success(t('organizations.notifications.updated', { name: saved.name }));
		} else {
			const created = await createProvider(organizationId, {
				name: input.name,
				kind: input.kind,
				enabled: true,
				decoration: input.decoration,
				config: input.config ?? {},
				actions: input.actions,
			});
			providers = [...providers, created];
			toast.success(t('organizations.notifications.created', { name: created.name }));
		}
		invalidateOnboarding();
	}

	async function handleDelete() {
		if (!confirmDelete) return;
		deleting = true;
		try {
			await deleteProvider(organizationId, confirmDelete.id);
			providers = providers.filter((provider) => provider.id !== confirmDelete!.id);
			toast.success(t('organizations.notifications.deleted', { name: confirmDelete.name }));
			invalidateOnboarding();
			confirmDelete = null;
		} catch (err) {
			toast.error(
				err instanceof Error ? err.message : t('organizations.notifications.delete_failed'),
			);
		} finally {
			deleting = false;
		}
	}

	async function handleTest(provider: NotificationProviderDto) {
		testingId = provider.id;
		try {
			const result = await testSaved(organizationId, provider.id);
			if (result.delivered) {
				toast.success(result.message);
				invalidateOnboarding();
			} else {
				toast.error(result.message);
			}
		} catch (err) {
			toast.error(
				err instanceof Error ? err.message : t('organizations.notifications.test_failed'),
			);
		} finally {
			testingId = null;
		}
	}

	function openLog(provider: NotificationProviderDto) {
		logProvider = provider;
		logOpen = true;
	}
</script>

<div class="flex flex-col gap-4">
	<div class="flex items-start justify-between gap-4">
		<div class="min-w-0">
			<h2 class="text-lg font-semibold tracking-tight">{t('organizations.notifications.title')}</h2>
			<p class="mt-1 text-sm text-muted-foreground">
				{t('organizations.notifications.description')}
			</p>
		</div>
		{#if canManage}
			<Button size="sm" onclick={openCreate} disabled={loading}>
				<PlusIcon class="size-4" />
				{t('organizations.notifications.add')}
			</Button>
		{/if}
	</div>

	{#if loading}
		<SkeletonRows rows={3} height="h-20" label={t('organizations.notifications.loading')} />
	{:else if providers.length === 0}
		<div
			class="flex flex-col items-center gap-3 rounded-2xl border border-dashed border-foreground/15 bg-card p-8 text-center">
			<div class="lp-raised flex size-12 items-center justify-center rounded-xl border bg-muted">
				<BellIcon class="size-5 text-muted-foreground" />
			</div>
			<div class="flex flex-col gap-1">
				<p class="text-sm font-medium">{t('organizations.notifications.empty')}</p>
				<p class="text-muted-foreground text-xs">
					{t('organizations.notifications.empty_hint')}
				</p>
			</div>
			{#if canManage}
				<Button size="sm" variant="outline" onclick={openCreate}>
					<PlusIcon class="size-4" />
					{t('organizations.notifications.add')}
				</Button>
			{/if}
		</div>
	{:else}
		<div class="flex flex-col gap-3">
			{#each providers as provider (provider.id)}
				<ProviderCard
					{provider}
					actions={catalog?.actions ?? []}
					{canManage}
					testing={testingId === provider.id}
					onEdit={() => openEdit(provider)}
					onDelete={() => (confirmDelete = provider)}
					onTest={() => handleTest(provider)}
					onViewLog={() => openLog(provider)} />
			{/each}
		</div>
	{/if}

	{#if catalog && !catalog.organization_is_root}
		<p class="text-muted-foreground text-xs">
			{t('organizations.notifications.root_only')}
		</p>
	{/if}
</div>

{#if catalog}
	<ProviderDialog
		bind:open={dialogOpen}
		{organizationId}
		provider={editing}
		providers={catalog.providers}
		actions={catalog.actions}
		organizationIsRoot={catalog.organization_is_root}
		{existingNames}
		onSubmit={handleSubmit} />
{/if}

<DeliveryLog bind:open={logOpen} {organizationId} provider={logProvider} />

<AlertDialog.Root
	open={confirmDelete !== null}
	onOpenChange={(next) => {
		if (!next) confirmDelete = null;
	}}>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title>{t('organizations.notifications.delete_confirm')}</AlertDialog.Title>
			<AlertDialog.Description>
				{confirmDelete
					? t('organizations.notifications.delete_description', { name: confirmDelete.name })
					: ''}
			</AlertDialog.Description>
		</AlertDialog.Header>
		<AlertDialog.Footer>
			<AlertDialog.Cancel>{t('organizations.common.cancel')}</AlertDialog.Cancel>
			<DeleteConfirmControl disabled={deleting} onconfirm={handleDelete} />
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>
