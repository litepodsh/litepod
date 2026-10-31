import { api } from './base-client';

export type ProviderKind = 'slack' | 'discord' | 'telegram' | 'msteams';

export type NotificationAction =
	| 'app_deploy'
	| 'app_build_error'
	| 'litepod_backup'
	| 'docker_cleanup'
	| 'litepod_restart'
	| 'server_reboot'
	| 'database_backup'
	| 'security_alert';

export type DeliveryStatus = 'pending' | 'sending' | 'delivered' | 'failed' | 'dead';

export type DeliveryDto = {
	id: string;
	provider_id: string;
	action: string;
	status: DeliveryStatus;
	attempts: number;
	next_attempt_at: number;
	title: string;
	last_error: string | null;
	created_at: number;
	updated_at: number;
};

export type NotificationProviderDto = {
	id: string;
	organization_id: string;
	name: string;
	kind: ProviderKind;
	enabled: boolean;
	decoration: boolean;
	/** Redacted display form of the credential — never the credential itself. */
	config_hint: string;
	actions: NotificationAction[];
	created_at: number;
	updated_at: number;
	last_delivery: DeliveryDto | null;
};

export type ProviderConfigInput = {
	webhook_url?: string;
	bot_token?: string;
	chat_id?: string;
};

export type CreateProviderInput = {
	name: string;
	kind: ProviderKind;
	enabled: boolean;
	decoration: boolean;
	config: ProviderConfigInput;
	actions: NotificationAction[];
};

export type UpdateProviderInput = {
	name: string;
	enabled: boolean;
	decoration: boolean;
	/** Omit to keep the stored credential. */
	config?: ProviderConfigInput;
	actions: NotificationAction[];
};

export type CatalogProvider = {
	kind: ProviderKind;
	label: string;
	uses_webhook_url: boolean;
	/** Same allowlist the API enforces, so the UI can warn before submitting. */
	allowed_hosts: string;
};

export type CatalogAction = {
	key: NotificationAction;
	label: string;
	description: string;
	global: boolean;
};

export type NotificationCatalog = {
	providers: CatalogProvider[];
	actions: CatalogAction[];
	organization_is_root: boolean;
};

export type TestResult = {
	delivered: boolean;
	message: string;
};

const base = (organizationId: string) => `/api/v1/organizations/${organizationId}/notifications`;

export async function getCatalog(organizationId: string): Promise<NotificationCatalog> {
	return (await api<NotificationCatalog>(`${base(organizationId)}/catalog`)).data!;
}

export async function listProviders(organizationId: string): Promise<NotificationProviderDto[]> {
	return (await api<NotificationProviderDto[]>(base(organizationId))).data!;
}

export async function createProvider(
	organizationId: string,
	input: CreateProviderInput,
): Promise<NotificationProviderDto> {
	return (
		await api<NotificationProviderDto>(base(organizationId), {
			method: 'POST',
			body: JSON.stringify(input),
		})
	).data!;
}

export async function updateProvider(
	organizationId: string,
	providerId: string,
	input: UpdateProviderInput,
): Promise<NotificationProviderDto> {
	return (
		await api<NotificationProviderDto>(`${base(organizationId)}/${providerId}`, {
			method: 'PUT',
			body: JSON.stringify(input),
		})
	).data!;
}

export async function deleteProvider(organizationId: string, providerId: string): Promise<void> {
	await api(`${base(organizationId)}/${providerId}`, { method: 'DELETE' });
}

/** Sends a real message using unsaved form values. */
export async function testUnsaved(
	organizationId: string,
	input: { kind: ProviderKind; decoration: boolean; config: ProviderConfigInput },
): Promise<TestResult> {
	return (
		await api<TestResult>(`${base(organizationId)}/test`, {
			method: 'POST',
			body: JSON.stringify(input),
		})
	).data!;
}

export async function testSaved(organizationId: string, providerId: string): Promise<TestResult> {
	return (await api<TestResult>(`${base(organizationId)}/${providerId}/test`, { method: 'POST' }))
		.data!;
}

export async function listDeliveries(
	organizationId: string,
	providerId: string,
): Promise<DeliveryDto[]> {
	return (await api<DeliveryDto[]>(`${base(organizationId)}/${providerId}/deliveries`)).data!;
}
