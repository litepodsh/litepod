import { api } from './base-client';

export type CertificateProvider = 'automatic' | 'letsencrypt' | 'custom';
export type CertificateInput = {
	certificate_provider?: CertificateProvider;
	certificate_id?: string;
};

export type ServerDomainSettings = {
	domain: string | null;
	email: string | null;
	https: boolean;
	certificate_provider: CertificateProvider;
	has_custom_certificate: boolean;
	certificate_id: string | null;
	certificate_name: string | null;
};

export async function getServerDomainSettings(): Promise<ServerDomainSettings> {
	return (await api<ServerDomainSettings>('/api/v1/settings/server-domain')).data!;
}

export async function updateServerDomainSettings(
	input: CertificateInput & {
		domain: string;
		email: string;
		https: boolean;
	},
): Promise<ServerDomainSettings> {
	return (
		await api<ServerDomainSettings>('/api/v1/settings/server-domain', {
			method: 'PUT',
			body: JSON.stringify(input),
		})
	).data!;
}

export type InfrastructureTarget = 'caddy' | 'api';

export async function restartInfrastructure(
	target: InfrastructureTarget,
): Promise<{ target: string; restarting: boolean }> {
	return (
		await api<{ target: string; restarting: boolean }>(
			`/api/v1/settings/infrastructure/${target}/restart`,
			{ method: 'POST' },
		)
	).data!;
}
