<script lang="ts">
	import { Button } from '#lib/components/ui/button/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import * as Tooltip from '#lib/components/ui/tooltip/index.js';
	import LoaderCircleIcon from '@lucide/svelte/icons/loader-circle';
	import PencilIcon from '@lucide/svelte/icons/pencil';
	import Trash2Icon from '@lucide/svelte/icons/trash-2';
	import SendIcon from '@lucide/svelte/icons/send';
	import HistoryIcon from '@lucide/svelte/icons/history';
	import ProviderIcon from './provider-icon.svelte';
	import { statusDot, statusLabel, relativeTime } from './status';
	import type { CatalogAction, NotificationProviderDto } from '#lib/clients/notification-client.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let {
		provider,
		actions,
		canManage,
		testing = false,
		onEdit,
		onDelete,
		onTest,
		onViewLog,
	}: {
		provider: NotificationProviderDto;
		actions: CatalogAction[];
		canManage: boolean;
		testing?: boolean;
		onEdit: () => void;
		onDelete: () => void;
		onTest: () => void;
		onViewLog: () => void;
	} = $props();

	const subscribed = $derived(actions.filter((action) => provider.actions.includes(action.key)));
	const delivery = $derived(provider.last_delivery);
</script>

<div class="lp-raised flex flex-col gap-3 rounded-2xl border bg-card p-4">
	<div class="flex items-start gap-3">
		<div class="bg-muted flex size-10 shrink-0 items-center justify-center rounded-lg">
			<ProviderIcon kind={provider.kind} class="size-5" />
		</div>

		<div class="flex min-w-0 flex-1 flex-col gap-0.5">
			<div class="flex items-center gap-2">
				<span class="truncate font-medium">{provider.name}</span>
				{#if !provider.enabled}
					<Badge variant="secondary" class="shrink-0">
						{t('organizations.notifications.paused')}
					</Badge>
				{/if}
			</div>
			<span class="text-muted-foreground truncate font-mono text-xs">{provider.config_hint}</span>
		</div>

		{#if canManage}
			<div class="flex shrink-0 items-center gap-1">
				<Button variant="ghost" size="sm" disabled={testing} onclick={onTest}>
					{#if testing}
						<LoaderCircleIcon class="size-4 animate-spin" />
					{:else}
						<SendIcon class="size-4" />
					{/if}
					<span class="sr-only sm:not-sr-only">{t('organizations.notifications.test_short')}</span>
				</Button>
				<Button
					variant="ghost"
					size="sm"
					onclick={onEdit}
					aria-label={t('organizations.notifications.edit_aria', { name: provider.name })}>
					<PencilIcon class="size-4" />
				</Button>
				<Button
					variant="ghost"
					size="sm"
					class="text-destructive hover:text-destructive"
					onclick={onDelete}
					aria-label={t('organizations.notifications.delete_aria', { name: provider.name })}>
					<Trash2Icon class="size-4" />
				</Button>
			</div>
		{/if}
	</div>

	<div class="flex flex-wrap items-center gap-1.5">
		{#if subscribed.length === 0}
			<span class="text-muted-foreground text-xs">
				{t('organizations.notifications.no_actions')}
			</span>
		{:else}
			{#each subscribed as action (action.key)}
				<Badge variant="outline" class="font-normal">{action.label}</Badge>
			{/each}
		{/if}
	</div>

	{#if delivery}
		<button
			type="button"
			onclick={onViewLog}
			class="text-muted-foreground hover:text-foreground -mx-1 flex items-center gap-2 rounded px-1 py-0.5 text-left text-xs">
			<span class="size-1.5 shrink-0 rounded-full {statusDot(delivery.status)}"></span>
			<span>
				{statusLabel(delivery.status, delivery.attempts)} · {relativeTime(delivery.updated_at)}
			</span>
			{#if delivery.last_error}
				<Tooltip.Provider>
					<Tooltip.Root>
						<Tooltip.Trigger class="truncate underline decoration-dotted">
							{delivery.last_error}
						</Tooltip.Trigger>
						<Tooltip.Content class="max-w-sm break-words">{delivery.last_error}</Tooltip.Content>
					</Tooltip.Root>
				</Tooltip.Provider>
			{/if}
			<HistoryIcon class="ml-auto size-3.5 shrink-0" />
		</button>
	{/if}
</div>
