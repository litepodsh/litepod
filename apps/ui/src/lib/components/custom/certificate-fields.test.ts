import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, expect, it, vi } from 'vitest';
import CertificateFields from './certificate-fields.svelte';
const mocks = vi.hoisted(() => ({ list: vi.fn(), me: vi.fn(), goto: vi.fn() }));
vi.mock('#lib/clients/certificate-client.js', () => ({ listCertificates: mocks.list }));
vi.mock('#lib/clients/auth-client.js', () => ({ getMe: mocks.me }));
vi.mock('$app/navigation', () => ({ goto: mocks.goto }));
afterEach(() => {
	cleanup();
	vi.resetAllMocks();
});
it('lists named certificates for an administrator without showing PEM inputs', async () => {
	mocks.me.mockResolvedValue({ is_sysadmin: true });
	mocks.list.mockResolvedValue([{ id: 'cert1', name: 'GoDaddy', domains: ['example.com'] }]);
	render(CertificateFields, { provider: 'custom' });
	await waitFor(() =>
		expect(screen.getByRole('option', { name: 'GoDaddy — example.com' })).toBeTruthy(),
	);
	expect(screen.queryAllByRole('textbox')).toHaveLength(0);
});
it('shows the assigned certificate for nonadministrators without fetching the inventory', async () => {
	mocks.me.mockResolvedValue({ is_sysadmin: false });
	render(CertificateFields, {
		provider: 'custom',
		certificateId: 'cert1',
		certificateName: 'Empresa',
	});
	await waitFor(() => expect(mocks.me).toHaveBeenCalled());
	expect(screen.getByRole('option', { name: 'Empresa' })).toBeTruthy();
	expect(mocks.list).not.toHaveBeenCalled();
	expect((screen.getAllByRole('combobox')[1] as HTMLSelectElement).disabled).toBe(true);
});
it('opens certificate settings when custom is chosen without a saved certificate', async () => {
	mocks.me.mockResolvedValue({ is_sysadmin: true });
	mocks.list.mockResolvedValue([]);
	render(CertificateFields);
	await waitFor(() => expect(mocks.list).toHaveBeenCalled());
	await fireEvent.change(screen.getByRole('combobox'), { target: { value: 'custom' } });
	await waitFor(() => expect(mocks.goto).toHaveBeenCalledWith('/settings?tab=certificates'));
	expect(screen.getByRole('link', { name: 'Settings → Certificates' }).getAttribute('href')).toBe(
		'/settings?tab=certificates',
	);
});
