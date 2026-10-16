<script lang="ts">
	import { onMount } from 'svelte';
	import ChevronRightIcon from '@lucide/svelte/icons/chevron-right';
	import CreditCardIcon from '@lucide/svelte/icons/credit-card';
	import InboxIcon from '@lucide/svelte/icons/inbox';
	import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';
	import WebhookIcon from '@lucide/svelte/icons/webhook';
	import { Button } from '#lib/components/ui/button/index.js';
	import { getBillingDebugStatus, type BillingDebugStatus } from '#lib/clients/debug-client.js';
	import { toneClass, toneDot, type StatusTone } from '#lib/utils/status-tone.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let status = $state<BillingDebugStatus | null>(null);
	let loading = $state(true);

	/** label, state, and the words for on/off (the overall flag reads yes/no, keys configured/missing). */
	const configRows = $derived.by((): [string, boolean, string][] => {
		if (!status) return [];
		const word = (ok: boolean) =>
			ok ? t('operations.billing.configured') : t('operations.billing.missing');
		return [
			[
				t('operations.billing.configured_label'),
				status.configured,
				status.configured ? t('operations.billing.yes') : t('operations.billing.no'),
			],
			[
				t('operations.billing.access_token'),
				status.access_token_configured,
				word(status.access_token_configured),
			],
			[
				t('operations.billing.webhook_secret'),
				status.webhook_secret_configured,
				word(status.webhook_secret_configured),
			],
		];
	});

	async function refresh(showLoading = true) {
		if (showLoading) loading = true;
		try {
			status = await getBillingDebugStatus();
		} catch (error) {
			toast.error(
				error instanceof Error ? error.message : t('operations.billing.debug_load_failed'),
			);
		} finally {
			if (showLoading) loading = false;
		}
	}

	onMount(() => {
		void refresh();
		const timer = window.setInterval(() => void refresh(false), 2_000);
		return () => window.clearInterval(timer);
	});
</script>

{#snippet pill(tone: StatusTone, label: string)}
	<span
		class="inline-flex shrink-0 items-center gap-1.5 rounded-full px-2 py-0.5 text-xs font-medium {toneClass(
			tone,
		)}">
		<i class="size-1.5 rounded-full {toneDot(tone)}"></i>{label}
	</span>
{/snippet}

{#snippet panelHeader(Icon: typeof WebhookIcon, title: string)}
	<header class="lp-groove-b flex items-center gap-2.5 px-4 py-3">
		<span
			class="lp-inset grid size-[30px] shrink-0 place-items-center rounded-lg bg-muted text-muted-foreground">
			<Icon class="size-4" />
		</span>
		<h2 class="text-[15px] font-semibold tracking-tight">{title}</h2>
	</header>
{/snippet}

<div class="flex min-w-0 flex-col gap-5 px-4 py-6 sm:px-6">
	<header class="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
		<div>
			<h1 class="text-xl font-semibold tracking-tight">{t('operations.billing.debug_title')}</h1>
			<p class="text-sm text-muted-foreground">{t('operations.billing.debug_description')}</p>
		</div>
		<div class="flex gap-2">
			<Button variant="outline" size="sm" href="/admin/billing"
				>{t('operations.billing.reconciliation')}</Button>
			<Button variant="outline" size="sm" onclick={() => refresh()} disabled={loading}>
				<RefreshCwIcon class={loading ? 'size-4 motion-safe:animate-spin' : 'size-4'} />
				{t('operations.billing.refresh')}
			</Button>
		</div>
	</header>
	{#if status}
		<section class="lp-raised min-w-0 overflow-hidden rounded-2xl border bg-card">
			{@render panelHeader(CreditCardIcon, t('operations.billing.polar_configuration'))}
			<dl class="grid grid-cols-1 gap-2.5 p-4 sm:grid-cols-2 lg:grid-cols-3">
				{#each configRows as [label, ok, word] (label)}
					<div
						class="lp-inset flex min-w-0 items-center justify-between gap-2 rounded-lg bg-muted/60 px-3 py-2.5">
						<dt class="truncate text-[13px] font-medium">{label}</dt>
						<dd>
							{@render pill(ok ? 'ok' : 'danger', word)}
						</dd>
					</div>
				{/each}
				<div
					class="lp-inset flex min-w-0 items-center justify-between gap-2 rounded-lg bg-muted/60 px-3 py-2.5">
					<dt class="truncate text-[13px] font-medium">{t('operations.billing.environment')}</dt>
					<dd class="font-mono text-[13px]">{status.environment ?? '—'}</dd>
				</div>
			</dl>
		</section>

		<section class="lp-raised min-w-0 overflow-hidden rounded-2xl border bg-card">
			{@render panelHeader(WebhookIcon, t('operations.billing_extra.recent_webhooks'))}
			{#if status.events.length}
				<ul>
					{#each status.events as event (event.event_id)}
						<li
							class="flex flex-wrap items-center justify-between gap-2 px-4 py-2.5 text-sm not-last:lp-groove-b">
							<div class="min-w-0">
								<p class="font-mono text-[13px] font-medium break-all">{event.event_type}</p>
								<p class="truncate font-mono text-xs text-muted-foreground">{event.event_id}</p>
								{#if event.last_error}
									<p class="mt-1 font-mono text-xs break-all text-destructive">{event.last_error}</p>
								{/if}
							</div>
							<div class="flex flex-col items-end gap-1">
								{@render pill(
									event.outcome === 'processed'
										? 'ok'
										: event.outcome === 'failed'
											? 'danger'
											: event.outcome === 'pending_match'
												? 'warn'
												: 'neutral',
									event.outcome,
								)}
								<p class="text-xs text-muted-foreground">
									{new Date(event.occurred_at).toLocaleString()}
								</p>
							</div>
						</li>
					{/each}
				</ul>
			{:else}
				<p class="p-6 text-sm text-muted-foreground">
					{t('operations.billing.no_verified_webhooks')}
				</p>
			{/if}
		</section>

		<section class="lp-raised min-w-0 overflow-hidden rounded-2xl border bg-card">
			{@render panelHeader(InboxIcon, t('operations.billing.delivery_log'))}
			{#if status.deliveries.length}
				<div>
					{#each status.deliveries as delivery (delivery.id)}
						<details class="group min-w-0 text-sm not-last:lp-groove-b">
							<summary
								class="flex cursor-pointer list-none flex-wrap items-center justify-between gap-2 px-4 py-2.5 transition-colors hover:bg-muted/60 [&::-webkit-details-marker]:hidden">
								<div class="flex min-w-0 items-center gap-2">
									<ChevronRightIcon
										class="size-4 shrink-0 text-muted-foreground group-open:rotate-90 motion-safe:transition-transform" />
									<div class="min-w-0">
										<p class="font-mono text-[13px] font-medium break-all">
											{delivery.event_type ?? t('operations.billing.unparsed_webhook')}
										</p>
										<p class="truncate font-mono text-xs text-muted-foreground">
											{delivery.polar_event_id ?? delivery.id}
										</p>
									</div>
								</div>
								<div class="flex flex-col items-end gap-1">
									{@render pill(
										delivery.error_message
											? 'danger'
											: delivery.response_status === null
												? 'neutral'
												: 'ok',
										String(delivery.response_status ?? t('operations.billing.pending')),
									)}
									<p class="text-xs text-muted-foreground">
										{new Date(delivery.received_at).toLocaleString()}
									</p>
								</div>
							</summary>
							<div class="flex flex-col gap-3 px-4 pt-1 pb-4">
								{#each [[t('operations.billing.request_headers'), delivery.request_headers_json], [t('operations.billing.request_body'), delivery.request_body], [t('operations.billing.response_body'), delivery.response_body ?? t('operations.billing.response_pending')]] as [label, body] (label)}
									<div class="min-w-0">
										<p
											class="mb-1 font-mono text-[11px] tracking-[.04em] text-muted-foreground uppercase">
											{label}
										</p>
										<pre
											class="lp-inset overflow-x-auto rounded-lg bg-muted/60 p-3 font-mono text-xs">{body}</pre>
									</div>
								{/each}
								{#if delivery.error_message}
									<p
										class="rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-destructive">
										{delivery.error_message}
									</p>
								{/if}
							</div>
						</details>
					{/each}
				</div>
			{:else}
				<p class="p-6 text-sm text-muted-foreground">
					{t('operations.billing.no_webhook_deliveries')}
				</p>
			{/if}
		</section>
	{:else if loading}
		<p class="text-sm text-muted-foreground">{t('operations.billing.loading_debug')}</p>
	{/if}
</div>
