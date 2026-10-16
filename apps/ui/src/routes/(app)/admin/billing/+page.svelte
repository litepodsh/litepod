<script lang="ts">
	import { onMount } from 'svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import {
		assignExternalPayment,
		getAdminBillingAccounts,
		getExternalPaymentRecords,
		reconcileBilling,
		type AdminBillingAccount,
		type ExternalPaymentRecord,
		type ReconciliationSummary,
	} from '#lib/clients/admin-billing-client.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let accounts = $state<AdminBillingAccount[]>([]);
	let records = $state<ExternalPaymentRecord[]>([]);
	let assignment = $state<Record<string, string>>({});
	let loading = $state(true);
	let reconciling = $state(false);
	let assigning = $state<string | null>(null);
	let lastRun = $state<ReconciliationSummary | null>(null);

	function message(error: unknown) {
		return error instanceof Error ? error.message : t('operations.billing.request_failed');
	}

	function trialProgress(account: AdminBillingAccount): string | null {
		if (account.status !== 'trialing' || !account.period_ends_at) return null;
		const end = account.period_ends_at;
		const ending = new Date(end).toLocaleDateString(undefined, { day: 'numeric', month: 'short' });
		if (!account.period_starts_at || end <= account.period_starts_at)
			return t('operations.billing.trial_active_ends', { date: ending });
		const dayMs = 86_400_000;
		const total = Math.ceil((end - account.period_starts_at) / dayMs);
		const current = Math.min(
			total,
			Math.max(1, Math.floor((Date.now() - account.period_starts_at) / dayMs) + 1),
		);
		return t('operations.billing.trial_day_ends', { current, total, date: ending });
	}

	async function refresh(showLoading = true) {
		if (showLoading) loading = true;
		try {
			[accounts, records] = await Promise.all([
				getAdminBillingAccounts(),
				getExternalPaymentRecords(),
			]);
			for (const record of records) {
				if (!assignment[record.id] && record.candidate_user_id)
					assignment[record.id] = record.candidate_user_id;
			}
		} catch (error) {
			toast.error(message(error));
		} finally {
			if (showLoading) loading = false;
		}
	}

	async function reconcile() {
		reconciling = true;
		try {
			lastRun = await reconcileBilling();
			await refresh(false);
			toast.success(t('operations.billing.reconciliation_completed'));
		} catch (error) {
			toast.error(message(error));
		} finally {
			reconciling = false;
		}
	}

	async function assign(record: ExternalPaymentRecord) {
		const userId = assignment[record.id]?.trim();
		if (!userId) {
			toast.error(t('operations.billing.user_id_required'));
			return;
		}
		assigning = record.id;
		try {
			await assignExternalPayment(record.id, userId);
			await refresh(false);
			toast.success(t('operations.billing.payment_assigned'));
		} catch (error) {
			toast.error(message(error));
		} finally {
			assigning = null;
		}
	}

	onMount(() => void refresh());
</script>

<div class="flex min-w-0 flex-col gap-5 px-4 py-6 sm:px-6">
	<header class="flex flex-wrap items-end justify-between gap-3">
		<div>
			<h1 class="text-xl font-semibold tracking-tight">{t('operations.billing.title')}</h1>
			<p class="text-sm text-muted-foreground">
				{t('operations.billing.description')}
			</p>
		</div>
		<div class="flex gap-2">
			<Button variant="outline" onclick={() => refresh()} disabled={loading || reconciling}
				>{t('operations.billing.refresh')}</Button>
			<Button onclick={reconcile} disabled={reconciling}
				>{reconciling
					? t('operations.billing.reconciling')
					: t('operations.billing.reconcile')}</Button>
		</div>
	</header>

	{#if lastRun}
		<Card.Root
			><Card.Content class="grid gap-2 p-4 text-sm sm:grid-cols-6"
				><p>{t('operations.billing.synced')}: <strong>{lastRun.synced_count}</strong></p>
				<p>{t('operations.billing.processed')}: <strong>{lastRun.processed_count}</strong></p>
				<p>{t('operations.billing.pending')}: <strong>{lastRun.pending_count}</strong></p>
				<p>{t('operations.billing.failed')}: <strong>{lastRun.failed_count}</strong></p>
				<p>
					{t('operations.billing.candidate_matches')}: <strong>{lastRun.candidate_count}</strong>
				</p>
				<p>
					{t('operations.billing.unassigned')}: <strong>{lastRun.unassigned_count}</strong>
				</p></Card.Content
			></Card.Root>
	{/if}

	<Card.Root>
		<Card.Header
			><Card.Title>{t('operations.billing.accounts_title')}</Card.Title><Card.Description
				>{t('operations.billing.accounts_description')}</Card.Description
			></Card.Header>
		<Card.Content>
			<div class="overflow-x-auto">
				<table class="w-full text-left text-sm">
					<thead class="border-b text-muted-foreground"
						><tr
							><th class="p-2">{t('operations.billing.user')}</th><th class="p-2"
								>{t('operations.billing.user_id')}</th
							><th class="p-2">{t('operations.billing.status')}</th><th
								class="p-2 whitespace-nowrap">{t('operations.billing.polar_subscription')}</th
							></tr
						></thead
					><tbody>
						{#each accounts as account (account.user_id)}
							{@const progress = trialProgress(account)}
							<tr class="border-b last:border-0"
								><td class="p-2"
									><p class="font-medium">{account.name}</p>
									<p class="text-muted-foreground">{account.email}</p></td
								><td class="p-2 font-mono text-xs">{account.user_id}</td><td class="p-2"
									><p>{account.status ?? 'No linked payment'}</p>
									{#if progress}<p class="mt-1 text-xs text-info">{progress}</p>{/if}</td
								><td class="p-2 font-mono text-xs">{account.polar_subscription_id ?? '—'}</td></tr>
						{/each}
					</tbody>
				</table>
			</div>
		</Card.Content>
	</Card.Root>

	<Card.Root>
		<Card.Header
			><Card.Title>{t('operations.billing.external_payments_title')}</Card.Title><Card.Description
				>{t('operations.billing.external_payments_description')}</Card.Description
			></Card.Header>
		<Card.Content class="space-y-3">
			{#if records.length}
				{#each records as record (record.id)}
					<div class="rounded-lg border p-4">
						<div class="flex flex-wrap justify-between gap-3">
							<div>
								<p class="font-medium">
									{record.customer_email ?? t('operations.billing.polar_customer_without_email')}
								</p>
								<p class="text-muted-foreground font-mono text-xs">
									{record.polar_subscription_id ?? record.polar_customer_id}
								</p>
							</div>
							<p class="text-sm">{record.state} · {record.subscription_status}</p>
						</div>
						<p class="text-muted-foreground mt-2 text-xs">
							Price: {record.polar_price_id ?? 'unknown'} · Candidate Litepod user: {record.candidate_user_id ??
								'none'}
						</p>
						{#if record.state !== 'assigned'}
							<div class="mt-3 flex flex-wrap gap-2">
								<Input
									bind:value={assignment[record.id]}
									class="max-w-full sm:max-w-xl font-mono text-xs min-w-0"
									placeholder={t('operations.billing.user_id_placeholder')} /><Button
									size="sm"
									onclick={() => assign(record)}
									disabled={assigning === record.id}
									>{assigning === record.id
										? t('operations.billing.assigning')
										: t('operations.billing.assign')}</Button>
							</div>
						{:else}<p class="mt-3 text-sm text-success">
								{t('operations.billing.assigned', { userId: record.assigned_user_id })}
							</p>{/if}
					</div>
				{/each}
			{:else}<p class="text-muted-foreground text-sm">
					{t('operations.billing.no_payments')}
				</p>{/if}
		</Card.Content>
	</Card.Root>
</div>
