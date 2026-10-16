import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, expect, it, vi } from 'vitest';
import CertificatesSettings from './certificates-settings.svelte';
const mocks = vi.hoisted(() => ({
	list: vi.fn(),
	getMaterial: vi.fn(),
	save: vi.fn(),
	remove: vi.fn(),
	test: vi.fn(),
}));
vi.mock('#lib/clients/certificate-client.js', () => ({
	listCertificates: mocks.list,
	getCertificateMaterial: mocks.getMaterial,
	saveCertificate: mocks.save,
	deleteCertificate: mocks.remove,
	testCertificate: mocks.test,
}));
afterEach(() => {
	cleanup();
	vi.resetAllMocks();
});
it('renames a saved certificate without sending a PEM pair and blocks deleting in-use certificates', async () => {
	const certificate = {
		id: 'cert1',
		name: 'GoDaddy',
		domains: ['example.com'],
		expires_at: '2027-01-01',
		in_use: true,
	};
	mocks.list.mockResolvedValue([certificate]);
	mocks.getMaterial.mockResolvedValue({
		certificate_pem: 'saved certificate',
		private_key_pem: 'saved key',
	});
	mocks.save.mockResolvedValue({ ...certificate, name: 'Empresa' });
	render(CertificatesSettings);
	await waitFor(() => expect(screen.getByText('GoDaddy')).toBeTruthy());
	expect((screen.getByRole('button', { name: 'Delete' }) as HTMLButtonElement).disabled).toBe(true);
	await fireEvent.click(screen.getByRole('button', { name: 'Edit' }));
	await waitFor(() =>
		expect(
			(screen.getByRole('textbox', { name: 'Certificate chain (PEM)' }) as HTMLTextAreaElement)
				.value,
		).toBe('saved certificate'),
	);
	expect(
		(screen.getByRole('textbox', { name: 'Private key (PEM)' }) as HTMLTextAreaElement).value,
	).toBe('saved key');
	await fireEvent.input(screen.getByRole('textbox', { name: 'Name' }), {
		target: { value: 'Empresa' },
	});
	await fireEvent.submit(screen.getByRole('textbox', { name: 'Name' }).closest('form')!);
	await waitFor(() => expect(mocks.save).toHaveBeenCalledWith({ name: 'Empresa' }, 'cert1'));
});
it('creates a named certificate and clears PEM fields after saving', async () => {
	mocks.list.mockResolvedValue([]);
	mocks.save.mockResolvedValue({
		id: 'cert1',
		name: 'Empresa',
		domains: ['example.com'],
		expires_at: '2027-01-01',
		in_use: false,
	});
	render(CertificatesSettings);
	await fireEvent.click(screen.getByRole('button', { name: 'Add certificate' }));
	await fireEvent.input(screen.getByRole('textbox', { name: 'Name' }), {
		target: { value: 'Empresa' },
	});
	await fireEvent.input(screen.getByRole('textbox', { name: 'Certificate chain (PEM)' }), {
		target: { value: 'certificate' },
	});
	await fireEvent.input(screen.getByRole('textbox', { name: 'Private key (PEM)' }), {
		target: { value: 'private key' },
	});
	await fireEvent.submit(screen.getByRole('textbox', { name: 'Name' }).closest('form')!);
	await waitFor(() =>
		expect(mocks.save).toHaveBeenCalledWith(
			{ name: 'Empresa', certificate_pem: 'certificate', private_key_pem: 'private key' },
			undefined,
		),
	);
	await waitFor(() =>
		expect(screen.queryByRole('textbox', { name: 'Private key (PEM)' })).toBeNull(),
	);
	await fireEvent.click(screen.getByRole('button', { name: 'Add certificate' }));
	expect(
		(screen.getByRole('textbox', { name: 'Private key (PEM)' }) as HTMLTextAreaElement).value,
	).toBe('');
});
it('tests an unsaved self-signed certificate without storing it', async () => {
	mocks.list.mockResolvedValue([]);
	mocks.test.mockResolvedValue({
		key_matches: true,
		dates_valid: true,
		san_present: true,
		chain_trusted: false,
		chain_error: 'self-signed certificate',
		domains: [],
	});
	render(CertificatesSettings);
	await fireEvent.click(screen.getByRole('button', { name: 'Add certificate' }));
	await fireEvent.input(screen.getByRole('textbox', { name: 'Certificate chain (PEM)' }), {
		target: { value: 'certificate' },
	});
	await fireEvent.input(screen.getByRole('textbox', { name: 'Private key (PEM)' }), {
		target: { value: 'private key' },
	});
	await fireEvent.click(screen.getByRole('button', { name: 'Test' }));
	await waitFor(() =>
		expect(mocks.test).toHaveBeenCalledWith({
			certificate_pem: 'certificate',
			private_key_pem: 'private key',
		}),
	);
	await waitFor(() => expect(screen.getByText(/Certificate and key match/)).toBeTruthy());
	expect(screen.getByText(/Chain not trusted by this server/)).toBeTruthy();
	expect(mocks.save).not.toHaveBeenCalled();
});
