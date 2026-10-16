<script lang="ts">
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import CheckIcon from '@lucide/svelte/icons/check';
	import ExternalLinkIcon from '@lucide/svelte/icons/external-link';
	import MinusIcon from '@lucide/svelte/icons/minus';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Progress } from '#lib/components/ui/progress/index.js';
	import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
	import DeleteConfirmControl from '#lib/components/custom/delete-confirm-control.svelte';
	import SegmentedSwitch from '#lib/components/custom/segmented-switch.svelte';
	import SettingsSection from '#lib/components/projects/settings-section.svelte';
	import {
		getBillingOverview,
		createBillingCheckout,
		getBillingInvoicePdfUrl,
		getBillingInvoices,
		getBillingPortalUrl,
		updateServerQuantity,
		type BillingInvoice,
		type BillingOverview,
	} from '#lib/clients/billing-client.js';
	import { toneBadge, type StatusTone } from '#lib/utils/status-tone.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';

	type PlanPrice = BillingOverview['plan']['prices'][number];
	const LAPSED = ['past_due', 'unpaid', 'paused', 'canceled', 'cancelled'];

	let overview = $state<BillingOverview | null>(null);
	let invoices = $state<BillingInvoice[]>([]);
	let invoicesLoading = $state(false);
	let loading = $state(true);
	let checkoutLoading = $state(false);
	let portalLoading = $state(false);
	let openingInvoiceId = $state<string | null>(null);
	let interval = $state<'month' | 'year'>('month');
	let quantityInput = $state(1);
	let quantityLoading = $state(false);
	let confirmQuantityOpen = $state(false);

	const money = (amount: string, currency: string) =>
		new Intl.NumberFormat(undefined, { style: 'currency', currency }).format(Number(amount) / 100);
	const date = (value: number) => new Date(value).toLocaleDateString();
	const unit = (value: string) =>
		value === 'month'
			? t('account.subscription.month_short')
			: t('account.subscription.year_short');

	const status = $derived(overview?.subscription.status ?? '');
	const hasActiveSubscription = $derived(['active', 'trialing'].includes(status));
	// Polar rejects slot changes once a cancel is scheduled.
	const canceling = $derived(
		hasActiveSubscription && !!overview?.subscription.cancel_at_period_end,
	);
	const tone = $derived<StatusTone>(
		status === 'active'
			? 'ok'
			: status === 'trialing'
				? 'info'
				: overview?.subscription.in_grace || LAPSED.includes(status)
					? 'warn'
					: 'neutral',
	);
	const statusLabel = $derived(
		status === 'active'
			? t('account.subscription.status_active')
			: status === 'trialing'
				? t('account.subscription.status_trialing')
				: overview?.subscription.in_grace
					? t('account.subscription.status_grace')
					: LAPSED.includes(status)
						? t('account.subscription.status_other', { status: status.replace('_', ' ') })
						: t('account.subscription.status_none'),
	);

	const serverPrice = $derived<PlanPrice | null>(
		overview?.plan.prices.find(
			(price) => price.role === 'per_server' && price.interval === interval,
		) ?? null,
	);
	// The subscription's own price, independent of the interval switch: this
	// is what a quantity change actually bills.
	const activeServerPrice = $derived<PlanPrice | null>(
		overview?.plan.prices.find((price) => price.id === overview?.subscription.server_price_id) ??
			null,
	);
	const minQuantity = $derived(Math.max(1, overview?.usage.remote_servers ?? 1));
	const purchased = $derived(overview?.usage.purchased_server_quantity ?? 0);
	const quantityDelta = $derived(overview ? quantityInput - purchased : 0);
	const quantityDeltaAmount = $derived(
		activeServerPrice
			? money(
					String(Number(activeServerPrice.amount) * Math.abs(quantityDelta)),
					activeServerPrice.currency_code,
				)
			: null,
	);
	const trialLabel = (price: PlanPrice): string => {
		const trial = price.trial_period!;
		return `${trial.frequency} ${trial.interval}${trial.frequency === 1 ? '' : 's'}`;
	};
	const features = $derived([
		t('account.subscription.feature_deployments'),
		t('account.subscription.feature_databases'),
		t('account.subscription.feature_applications'),
		t('account.subscription.feature_organizations'),
		t('account.subscription.feature_users'),
		t('account.subscription.feature_rbac'),
	]);

	async function refresh() {
		try {
			overview = await getBillingOverview();
			quantityInput = overview.usage.purchased_server_quantity;
		} catch (error) {
			toast.error(error instanceof Error ? error.message : t('account.subscription.errors.load'));
		}
	}

	async function loadInvoices() {
		invoicesLoading = true;
		try {
			invoices = await getBillingInvoices();
		} catch (error) {
			toast.error(
				error instanceof Error ? error.message : t('account.subscription.errors.invoices'),
			);
		} finally {
			invoicesLoading = false;
		}
	}

	async function submitQuantity() {
		if (!overview || quantityLoading || quantityInput === purchased) return;
		quantityLoading = true;
		try {
			await updateServerQuantity(quantityInput);
			toast.success(t('account.subscription.quantity_updated'));
			await refresh();
		} catch (error) {
			toast.error(
				error instanceof Error ? error.message : t('account.subscription.errors.update_quantity'),
			);
		} finally {
			quantityLoading = false;
		}
	}

	async function checkout() {
		if (!overview || checkoutLoading) return;
		checkoutLoading = true;
		try {
			const { checkout_url } = await createBillingCheckout(interval);
			toast.message(t('account.subscription.waiting_polar'));
			window.location.assign(checkout_url);
		} catch (error) {
			toast.error(
				error instanceof Error ? error.message : t('account.subscription.errors.checkout'),
			);
			checkoutLoading = false;
		}
	}

	/** Opens the tab synchronously so popup blockers allow it, then fills it. */
	async function openExternal(load: () => Promise<string>, fallbackError: string) {
		const target = window.open('', '_blank');
		if (target) target.opener = null;
		try {
			const url = await load();
			if (target) target.location.replace(url);
			else window.location.assign(url);
		} catch (error) {
			target?.close();
			toast.error(error instanceof Error ? error.message : fallbackError);
		}
	}

	async function manageSubscription() {
		portalLoading = true;
		await openExternal(getBillingPortalUrl, t('account.subscription.errors.portal'));
		portalLoading = false;
	}

	async function openInvoicePdf(orderId: string) {
		openingInvoiceId = orderId;
		await openExternal(
			() => getBillingInvoicePdfUrl(orderId),
			t('account.subscription.errors.invoice_pdf'),
		);
		openingInvoiceId = null;
	}

	onMount(async () => {
		await Promise.all([refresh(), loadInvoices()]);
		loading = false;
		if (page.url.searchParams.get('checkout') === 'success') {
			toast.success(t('account.subscription.payment_complete'));
			// The webhook usually lands a moment after Polar redirects back.
			window.setTimeout(() => void refresh(), 2500);
		}
	});
</script>

{#snippet legalConsent()}
	<p class="text-xs text-muted-foreground">
		{t('account.subscription.legal_consent')}
		<a href="/terms" class="underline">{t('legal.common.terms_of_service')}</a>,
		<a href="/refund" class="underline">{t('legal.common.refund_policy')}</a>
		{t('legal.common.and')}
		<a href="/privacy" class="underline">{t('legal.common.privacy_policy')}</a>.
	</p>
{/snippet}

{#if loading}
	<p class="text-sm text-muted-foreground">{t('account.subscription.loading')}</p>
{:else if overview}
	<SettingsSection
		title={t('account.subscription.tab_subscription')}
		description={t('account.subscription.status_description')}>
		<div class="flex flex-wrap items-center gap-2">
			<span
				class="inline-flex items-center rounded-full border px-2.5 py-0.5 text-xs font-medium {toneBadge(
					tone,
				)}">{statusLabel}</span>
			{#if hasActiveSubscription}
				<span class="font-medium">{t('account.subscription.plan_title')}</span>
				{#if activeServerPrice}
					<span class="text-muted-foreground"
						>· {activeServerPrice.interval === 'month'
							? t('account.subscription.billed_monthly')
							: t('account.subscription.billed_annually')}</span>
				{/if}
			{/if}
		</div>

		{#if overview.subscription.period_ends_at || overview.subscription.grace_until || (status === 'trialing' && overview.estimate)}
			<dl class="grid gap-x-6 gap-y-2 sm:grid-cols-2">
				{#if overview.subscription.period_ends_at}
					<div>
						<dt class="text-xs text-muted-foreground">
							{status === 'trialing'
								? t('account.subscription.trial_ends')
								: status === 'active'
									? t('account.subscription.next_billing')
									: t('account.subscription.period_ends')}
						</dt>
						<dd class="font-medium tabular-nums">{date(overview.subscription.period_ends_at)}</dd>
					</div>
				{/if}
				{#if status === 'trialing' && overview.estimate}
					<div>
						<dt class="text-xs text-muted-foreground">{t('account.subscription.first_charge')}</dt>
						<dd class="font-medium tabular-nums">
							{money(overview.estimate.estimated_total, overview.estimate.currency_code)}
						</dd>
					</div>
				{/if}
				{#if overview.subscription.grace_until}
					<div>
						<dt class="text-xs text-muted-foreground">{t('account.subscription.grace_ends')}</dt>
						<dd class="font-medium tabular-nums">{date(overview.subscription.grace_until)}</dd>
					</div>
				{/if}
			</dl>
		{/if}

		{#if canceling}
			<p class="rounded-[10px] border px-3 py-2.5 text-[12.5px] {toneBadge('warn')}">
				{overview.subscription.period_ends_at
					? t('account.subscription.cancels_on', {
							date: date(overview.subscription.period_ends_at),
						})
					: t('account.subscription.cancels_soon')}
			</p>
		{/if}

		{#if overview.subscription.in_grace}
			<p class="rounded-[10px] border px-3 py-2.5 text-[12.5px] {toneBadge('warn')}">
				{t('account.subscription.grace_notice')}
			</p>
		{/if}

		{#if hasActiveSubscription}
			<div class="flex flex-wrap items-center gap-3">
				<Button variant="outline" size="sm" onclick={manageSubscription} disabled={portalLoading}>
					{portalLoading
						? t('account.subscription.opening_portal')
						: canceling
							? t('account.subscription.reactivate')
							: t('account.subscription.manage_subscription')}
					<ExternalLinkIcon class="size-3.5" />
				</Button>
				<p class="text-xs text-muted-foreground">{t('account.subscription.manage_payment_hint')}</p>
			</div>
		{/if}
	</SettingsSection>

	<SettingsSection
		title={t('account.subscription.plan_usage')}
		description={t('account.subscription.usage_description')}>
		<div class="grid gap-3 sm:grid-cols-3">
			<div class="grid gap-2 rounded-[10px] border border-foreground/9 p-3">
				<p class="text-xs text-muted-foreground">{t('account.subscription.usage_servers')}</p>
				<p class="text-lg font-semibold tabular-nums">
					{overview.usage.remote_servers}<span class="text-muted-foreground">/{purchased}</span>
				</p>
				<Progress
					value={overview.usage.remote_servers}
					max={Math.max(1, purchased)}
					aria-label={t('account.subscription.usage_servers')}
					class="h-1.5" />
			</div>
			<div class="grid content-start gap-2 rounded-[10px] border border-foreground/9 p-3">
				<p class="text-xs text-muted-foreground">{t('account.subscription.usage_organizations')}</p>
				<p class="text-lg font-semibold tabular-nums">{overview.usage.organizations}</p>
			</div>
			<div class="grid content-start gap-2 rounded-[10px] border border-foreground/9 p-3">
				<p class="text-xs text-muted-foreground">{t('account.subscription.usage_users')}</p>
				<p class="text-lg font-semibold tabular-nums">{overview.usage.users}</p>
			</div>
		</div>
	</SettingsSection>

	{#if canceling}
		<SettingsSection
			title={t('account.subscription.server_slots')}
			description={t('account.subscription.slots_description')}>
			<p class="text-lg font-semibold tabular-nums">{purchased}</p>
			<p class="text-xs text-muted-foreground">{t('account.subscription.slots_locked_canceled')}</p>
		</SettingsSection>
	{:else if hasActiveSubscription}
		<SettingsSection
			title={t('account.subscription.server_slots')}
			description={t('account.subscription.slots_description')}>
			<div class="flex flex-wrap items-center gap-3">
				<div class="flex items-center gap-1.5">
					<Button
						variant="outline"
						size="icon"
						aria-label="-1"
						onclick={() => (quantityInput = Math.max(minQuantity, quantityInput - 1))}
						disabled={quantityLoading || quantityInput <= minQuantity}>
						<MinusIcon class="size-4" />
					</Button>
					<Input
						type="number"
						min={minQuantity}
						aria-label={t('account.subscription.server_slots')}
						bind:value={quantityInput}
						disabled={quantityLoading}
						class="w-20 text-center tabular-nums" />
					<Button
						variant="outline"
						size="icon"
						aria-label="+1"
						onclick={() => (quantityInput = quantityInput + 1)}
						disabled={quantityLoading}>
						<PlusIcon class="size-4" />
					</Button>
				</div>
				{#if quantityInput !== purchased}
					<Button size="sm" onclick={() => (confirmQuantityOpen = true)} disabled={quantityLoading}>
						{quantityLoading
							? t('account.subscription.updating')
							: t('account.subscription.update_quantity')}
					</Button>
				{/if}
			</div>
			{#if quantityInput !== purchased}
				<p class="text-xs text-muted-foreground">
					{quantityInput > purchased
						? t('account.subscription.increase_hint')
						: t('account.subscription.decrease_hint')}
				</p>
			{:else if quantityInput === minQuantity && overview.usage.remote_servers === minQuantity}
				<p class="text-xs text-muted-foreground">
					{t('account.subscription.remove_server_hint', { min: minQuantity })}
				</p>
			{/if}
		</SettingsSection>
	{:else}
		<SettingsSection
			title={t('account.subscription.plan_title')}
			description={t('account.subscription.plan_description')}>
			<SegmentedSwitch
				class="w-full max-w-sm"
				label={t('account.subscription.plan_title')}
				items={[
					{ value: 'month', label: t('account.subscription.monthly') },
					{ value: 'year', label: t('account.subscription.annual') },
				]}
				bind:value={interval} />
			{#if serverPrice}
				<div class="flex flex-wrap items-baseline gap-x-2 gap-y-1">
					<strong class="text-2xl font-semibold tabular-nums"
						>{money(serverPrice.amount, serverPrice.currency_code)}</strong>
					<span class="text-muted-foreground"
						>{t('account.subscription.price_unit', { unit: unit(interval) })}</span>
					{#if serverPrice.trial_period}
						<span
							class="ml-1 inline-flex items-center rounded-full border px-2 py-0.5 text-xs font-medium {toneBadge(
								'info',
							)}">{t('account.subscription.free_trial', { trial: trialLabel(serverPrice) })}</span>
					{/if}
				</div>
				{#if overview.estimate}
					<p class="text-[12.5px] text-muted-foreground">
						{t('account.subscription.estimate_have')}
						<strong class="text-foreground">{overview.estimate.server_quantity}</strong>
						{overview.estimate.server_quantity === 1
							? t('account.subscription.slot_one')
							: t('account.subscription.slot_many')}
						{t('account.subscription.estimate_estimated')}
						<strong class="text-foreground"
							>{money(overview.estimate.estimated_total, overview.estimate.currency_code)}</strong
						>/{unit(overview.estimate.interval)}.
					</p>
				{/if}
				<ul class="grid gap-1.5 sm:grid-cols-2">
					{#each features as feature (feature)}
						<li class="flex items-center gap-2">
							<CheckIcon class="size-4 shrink-0 text-success" />{feature}
						</li>
					{/each}
				</ul>
				<p class="text-xs text-muted-foreground">{t('account.subscription.features_note')}</p>
				<div class="lp-groove-t grid gap-2.5 pt-4">
					{@render legalConsent()}
					<Button class="w-full font-mono sm:w-fit" onclick={checkout} disabled={checkoutLoading}>
						{checkoutLoading
							? t('account.subscription.opening_checkout')
							: serverPrice.trial_period
								? t('account.subscription.free_trial', { trial: trialLabel(serverPrice) })
								: t('account.subscription.subscribe')}
					</Button>
					{#if serverPrice.trial_period}
						<p class="text-xs text-muted-foreground">{t('account.subscription.trial_note')}</p>
					{/if}
				</div>
			{:else}
				<p class="text-muted-foreground">{t('account.subscription.pricing_unavailable')}</p>
			{/if}
		</SettingsSection>
	{/if}

	<SettingsSection
		title={t('account.subscription.tab_invoices')}
		description={t('account.subscription.invoices_description')}>
		{#snippet aside()}
			<Button
				variant="outline"
				size="sm"
				class="mt-3"
				onclick={loadInvoices}
				disabled={invoicesLoading}>
				<RefreshCwIcon
					class="size-3.5 {invoicesLoading ? 'animate-spin motion-reduce:animate-none' : ''}" />
				{t('common.common.refresh')}
			</Button>
		{/snippet}
		{#if invoices.length === 0}
			<p class="text-muted-foreground">{t('account.subscription.invoices_empty')}</p>
		{:else}
			<ul class="overflow-hidden rounded-[10px] border border-foreground/9">
				{#each invoices as invoice (invoice.order_id)}
					<li
						class="flex flex-wrap items-center justify-between gap-3 border-b border-foreground/9 px-3 py-2.5 last:border-b-0">
						<div class="min-w-0">
							<p class="truncate font-medium">{invoice.invoice_number ?? invoice.order_id}</p>
							<p class="text-xs text-muted-foreground tabular-nums">
								{invoice.issued_at ? date(invoice.issued_at) : invoice.status}
							</p>
						</div>
						<div class="flex items-center gap-3">
							<span class="font-medium tabular-nums"
								>{money(invoice.amount, invoice.currency_code)}</span>
							<Button
								variant="outline"
								size="sm"
								onclick={() => openInvoicePdf(invoice.order_id)}
								disabled={openingInvoiceId === invoice.order_id}>
								{openingInvoiceId === invoice.order_id
									? t('account.subscription.opening')
									: t('account.subscription.view_pdf')}
							</Button>
						</div>
					</li>
				{/each}
			</ul>
		{/if}
	</SettingsSection>

	<AlertDialog.Root bind:open={confirmQuantityOpen}>
		<AlertDialog.Content>
			<AlertDialog.Header>
				<AlertDialog.Title
					>{quantityDelta > 0
						? t('account.subscription.buy_slots_title')
						: t('account.subscription.release_slots_title')}</AlertDialog.Title>
				<AlertDialog.Description>
					{quantityDelta > 0
						? t('account.subscription.confirm_buy', {
								from: purchased,
								to: quantityInput,
								amount: quantityDeltaAmount,
							})
						: t('account.subscription.confirm_release', {
								from: purchased,
								to: quantityInput,
								amount: quantityDeltaAmount,
							})}
				</AlertDialog.Description>
			</AlertDialog.Header>
			<AlertDialog.Footer>
				<AlertDialog.Cancel disabled={quantityLoading}
					>{t('account.common.cancel')}</AlertDialog.Cancel>
				<DeleteConfirmControl
					label={quantityDelta > 0
						? t('account.subscription.buy_slots')
						: t('account.subscription.release_slots')}
					pendingLabel={t('account.subscription.updating')}
					variant="primary"
					disabled={quantityLoading}
					onconfirm={async () => {
						await submitQuantity();
						confirmQuantityOpen = false;
					}} />
			</AlertDialog.Footer>
		</AlertDialog.Content>
	</AlertDialog.Root>
{/if}
