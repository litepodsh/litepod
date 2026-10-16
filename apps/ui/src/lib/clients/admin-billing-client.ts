import { api } from './base-client';

export type AdminBillingAccount = {
	user_id: string;
	name: string;
	email: string;
	status: string | null;
	polar_customer_id: string | null;
	polar_subscription_id: string | null;
	polar_product_id: string | null;
	period_starts_at: number | null;
	period_ends_at: number | null;
	grace_until: number | null;
};

export type ExternalPaymentRecord = {
	id: string;
	polar_subscription_id: string | null;
	polar_customer_id: string;
	customer_email: string | null;
	polar_product_id: string | null;
	polar_price_id: string | null;
	subscription_status: string;
	state: 'candidate_match' | 'unassigned_external' | 'assigned' | 'ignored';
	candidate_user_id: string | null;
	assigned_user_id: string | null;
	discovered_at: number;
};

export type ReconciliationSummary = {
	processed_count: number;
	pending_count: number;
	failed_count: number;
	candidate_count: number;
	unassigned_count: number;
	synced_count: number;
};

export async function getAdminBillingAccounts() {
	return (await api<AdminBillingAccount[]>('/api/v1/admin/billing/accounts')).data!;
}

export async function getExternalPaymentRecords() {
	return (await api<ExternalPaymentRecord[]>('/api/v1/admin/billing/external-records')).data!;
}

export async function reconcileBilling() {
	return (await api<ReconciliationSummary>('/api/v1/admin/billing/reconcile', { method: 'POST' }))
		.data!;
}

export async function assignExternalPayment(recordId: string, userId: string) {
	return (
		await api<void>(
			`/api/v1/admin/billing/external-records/${encodeURIComponent(recordId)}/assign`,
			{
				method: 'POST',
				body: JSON.stringify({ user_id: userId }),
			},
		)
	).data;
}
