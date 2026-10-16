import { api } from './base-client';

export type Certificate = {
	id: string;
	name: string;
	domains: string[];
	expires_at: string;
	in_use: boolean;
};
export type CertificateUpload = {
	name: string;
	certificate_pem?: string;
	private_key_pem?: string;
};
export type CertificateMaterial = { certificate_pem: string; private_key_pem: string };
export type CertificateTestResult = {
	key_matches: boolean;
	dates_valid: boolean;
	san_present: boolean;
	chain_trusted: boolean;
	chain_error: string | null;
	domains: {
		host: string;
		covered: boolean;
		served: boolean | null;
		connection_error: string | null;
	}[];
};
const path = '/api/v1/settings/certificates';
export async function listCertificates(): Promise<Certificate[]> {
	return (await api<Certificate[]>(path)).data!;
}
export async function getCertificateMaterial(id: string): Promise<CertificateMaterial> {
	return (
		await api<CertificateMaterial>(`${path}/${encodeURIComponent(id)}`, { cache: 'no-store' })
	).data!;
}
export async function saveCertificate(input: CertificateUpload, id?: string): Promise<Certificate> {
	return (
		await api<Certificate>(id ? `${path}/${encodeURIComponent(id)}` : path, {
			method: id ? 'PUT' : 'POST',
			body: JSON.stringify(input),
		})
	).data!;
}
export async function deleteCertificate(id: string): Promise<void> {
	await api(`${path}/${encodeURIComponent(id)}`, { method: 'DELETE' });
}
export async function testCertificate(input: {
	id?: string;
	certificate_pem?: string;
	private_key_pem?: string;
}): Promise<CertificateTestResult> {
	return (
		await api<CertificateTestResult>(`${path}/test`, {
			method: 'POST',
			body: JSON.stringify(input),
		})
	).data!;
}
