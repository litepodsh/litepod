import { api } from './base-client';

export type PlanPrice = {
	id: string;
	role: 'base' | 'per_server';
	interval: 'month' | 'year';
	amount: string;
	currency_code: string;
	trial_period: {
		interval: string;
		frequency: number;
		requires_payment_method: boolean;
	} | null;
};

export type BillingPlan = {
	product_id: string;
	name: string;
	description: string;
	prices: PlanPrice[];
};

export type BillingEstimate = {
	interval: 'month' | 'year';
	server_amount: string;
	server_quantity: number;
	currency_code: string;
	estimated_total: string;
};

export type BillingOverview = {
	polar_environment: 'sandbox' | 'production';
	plan: BillingPlan;
	subscription: {
		status: string;
		product_id: string | null;
		server_price_id: string | null;
		purchased_server_quantity: number;
		period_ends_at: number | null;
		grace_until: number | null;
		in_grace: boolean;
		cancel_at_period_end: boolean;
	};
	usage: {
		remote_servers: number;
		organizations: number;
		users: number;
		purchased_server_quantity: number;
	};
	estimate: BillingEstimate | null;
};

export type BillingInvoice = {
	order_id: string;
	invoice_number: string | null;
	status: string;
	amount: string;
	currency_code: string;
	issued_at: number | null;
	document_url: string | null;
};

export async function getBillingOverview(): Promise<BillingOverview> {
	return (await api<BillingOverview>('/api/v1/billing/overview')).data!;
}

export async function createBillingCheckout(
	interval: 'month' | 'year',
): Promise<{ checkout_url: string }> {
	return (
		await api<{ checkout_url: string }>('/api/v1/billing/checkout', {
			method: 'POST',
			body: JSON.stringify({ interval }),
		})
	).data!;
}

export async function updateServerQuantity(quantity: number): Promise<void> {
	await api<void>('/api/v1/billing/servers/quantity', {
		method: 'PATCH',
		body: JSON.stringify({ quantity }),
	});
}

export async function getBillingInvoices(): Promise<BillingInvoice[]> {
	return (await api<BillingInvoice[]>('/api/v1/billing/invoices')).data!;
}

export async function getBillingInvoicePdfUrl(orderId: string): Promise<string> {
	return (await api<{ url: string }>(`/api/v1/billing/invoices/${encodeURIComponent(orderId)}/pdf`))
		.data!.url;
}

export async function getBillingPortalUrl(): Promise<string> {
	return (await api<{ url: string }>('/api/v1/billing/portal', { method: 'POST' })).data!.url;
}
