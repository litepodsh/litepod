<script lang="ts">
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import LoaderCircleIcon from '@lucide/svelte/icons/loader-circle';
	import {
		listDeliveries,
		type DeliveryDto,
		type NotificationProviderDto,
	} from '#lib/clients/notification-client.js';
	import { toast } from '#lib/utils/toast.js';
	import { statusTone, statusLabel } from './status';
	import { t } from '#lib/i18n/index.svelte.js';

	let {
		open = $bindable(false),
		organizationId,
		provider,
	}: {
		open?: boolean;
		organizationId: string;
		provider: NotificationProviderDto | null;
	} = $props();

	let deliveries = $state<DeliveryDto[]>([]);
	let loading = $state(false);
	let wasOpen = $state(false);

	$effect(() => {
		if (open && !wasOpen && provider) load(provider.id);
		wasOpen = open;
	});

	async function load(providerId: string) {
		loading = true;
		try {
			deliveries = await listDeliveries(organizationId, providerId);
		} catch (err) {
			toast.error(
				err instanceof Error ? err.message : t('organizations.notifications.load_history_failed'),
			);
		} finally {
			loading = false;
		}
	}

	function when(timestamp: number) {
		return new Date(timestamp).toLocaleString();
	}
</script>

<Dialog.Root bind:open>
	<Dialog.Content class="max-h-[80dvh] overflow-y-auto sm:max-w-2xl">
		<Dialog.Header>
			<Dialog.Title>{t('organizations.notifications.delivery_history')}</Dialog.Title>
			<Dialog.Description>
				{t('organizations.notifications.delivery_description', {
					name: provider?.name ?? t('organizations.notifications.this_provider'),
				})}
			</Dialog.Description>
		</Dialog.Header>

		{#if loading}
			<p class="text-muted-foreground flex items-center gap-2 py-6 text-sm">
				<LoaderCircleIcon class="size-4 animate-spin" />
				{t('organizations.common.loading')}
			</p>
		{:else if deliveries.length === 0}
			<p class="text-muted-foreground py-6 text-sm">
				{t('organizations.notifications.empty_log')}
			</p>
		{:else}
			<ul class="lp-inset divide-y rounded-xl bg-muted/40">
				{#each deliveries as delivery (delivery.id)}
					<li class="flex flex-col gap-1 p-3">
						<div class="flex items-center justify-between gap-3">
							<span class="truncate text-sm font-medium">{delivery.title || delivery.action}</span>
							<Badge variant="outline" class={statusTone(delivery.status)}>
								{statusLabel(delivery.status, delivery.attempts)}
							</Badge>
						</div>
						<span class="text-muted-foreground text-xs">{when(delivery.created_at)}</span>
						{#if delivery.last_error}
							<!-- The provider's verbatim error: "Invalid Webhook Token" is
							     actionable in a way that "delivery failed" is not. -->
							<p
								class="bg-muted text-muted-foreground mt-1 rounded p-2 font-mono text-xs break-words">
								{delivery.last_error}
							</p>
						{/if}
					</li>
				{/each}
			</ul>
		{/if}
	</Dialog.Content>
</Dialog.Root>
