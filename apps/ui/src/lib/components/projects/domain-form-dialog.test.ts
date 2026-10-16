import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, expect, it, vi } from 'vitest';
import DomainFormDialog from './domain-form-dialog.svelte';
import type { DomainDto } from '#lib/clients/project-client.js';

vi.mock('#lib/clients/auth-client.js', () => ({ getMe: async () => ({ is_sysadmin: false }) }));
afterEach(cleanup);
// jsdom has no layout; bits-ui's command list scrolls the active item into view.
Element.prototype.scrollIntoView ??= () => {};

it('keeps a saved custom certificate without resending secrets when editing a domain', async () => {
	const onSubmit = vi.fn().mockResolvedValue(undefined);
	render(DomainFormDialog, {
		open: true,
		domain: {
			id: 'domain',
			host: 'example.com',
			port: 8080,
			https: true,
			certificate_provider: 'custom',
			has_custom_certificate: true,
			certificate_id: 'cert1',
			certificate_name: 'Empresa',
		} as DomainDto,
		onSubmit,
	});
	await waitFor(() =>
		expect((screen.getAllByRole('combobox')[0] as HTMLSelectElement).value).toBe('custom'),
	);
	await fireEvent.submit(screen.getByRole('textbox', { name: 'Host' }).closest('form')!);
	await waitFor(() =>
		expect(onSubmit).toHaveBeenCalledWith(
			expect.objectContaining({
				certificate_provider: 'custom',
				https: true,
				certificate_id: 'cert1',
			}),
		),
	);
	expect(onSubmit.mock.calls[0][0]).not.toHaveProperty('private_key_pem');
	expect(onSubmit.mock.calls[0][0]).not.toHaveProperty('certificate_pem');
});

it('picks the Caddy target from a searchable dropdown and fills its declared port', async () => {
	render(DomainFormDialog, {
		open: true,
		composeServices: [
			{ name: 'web', suggested_internal_ports: [3000] },
			{ name: 'postgres', suggested_internal_ports: [5432] },
		],
		onSubmit: vi.fn(),
	});
	await fireEvent.click(screen.getByRole('combobox', { name: 'Search Compose services' }));
	const search = await screen.findByPlaceholderText('Search services…');
	await fireEvent.input(search, { target: { value: 'postg' } });
	await waitFor(() => expect(screen.queryByRole('option', { name: /web/ })).toBeNull());
	await fireEvent.click(screen.getByRole('option', { name: /postgres/ }));
	await waitFor(() =>
		expect((screen.getByLabelText('Internal port') as HTMLInputElement).value).toBe('5432'),
	);
});

it('shows the kind label and the host on one compact header', async () => {
	render(DomainFormDialog, {
		open: true,
		domain: { id: 'd', host: 'shop.example.com', port: 80, https: false } as DomainDto,
		onSubmit: vi.fn(),
	});
	expect(await screen.findByText('Edit domain')).toBeTruthy();
	expect(screen.getByRole('heading', { name: 'shop.example.com' })).toBeTruthy();
});

it('fills the host from the sslip.io generator', async () => {
	const onRandomSslipHost = vi.fn().mockResolvedValue('litepod-ab12.203.0.113.10.sslip.io');
	render(DomainFormDialog, { open: true, onRandomSslipHost, onSubmit: vi.fn() });
	await fireEvent.click(await screen.findByRole('button', { name: /Random sslip.io/ }));
	expect(onRandomSslipHost).toHaveBeenCalledOnce();
	await waitFor(() =>
		expect((screen.getByRole('textbox', { name: 'Host' }) as HTMLInputElement).value).toBe(
			'litepod-ab12.203.0.113.10.sslip.io',
		),
	);
});

it('shows the real DNS result and checks again on demand', async () => {
	const onCheckDns = vi
		.fn()
		.mockResolvedValueOnce({
			status: 'invalid',
			resolved_addresses: ['198.51.100.7'],
			expected_addresses: ['203.0.113.10'],
			cname: null,
			cdn_provider: null,
			message: null,
		})
		.mockResolvedValueOnce({
			status: 'valid',
			resolved_addresses: ['203.0.113.10'],
			expected_addresses: ['203.0.113.10'],
			cname: null,
			cdn_provider: null,
			message: null,
		});
	render(DomainFormDialog, {
		open: true,
		domain: { id: 'd', host: 'shop.example.com', port: 80, https: false } as DomainDto,
		serverAddress: '203.0.113.10',
		onCheckDns,
		onSubmit: vi.fn(),
	});
	expect(await screen.findByText('Points somewhere else', {}, { timeout: 2000 })).toBeTruthy();
	expect(onCheckDns).toHaveBeenCalledWith('shop.example.com', null);
	expect(screen.getByText('Found: A 198.51.100.7')).toBeTruthy();
	await fireEvent.click(screen.getByRole('button', { name: 'Check again' }));
	expect(await screen.findByText('Resolves to 203.0.113.10')).toBeTruthy();
	expect(onCheckDns).toHaveBeenCalledTimes(2);
});

it('reports a failed DNS lookup instead of a status', async () => {
	const onCheckDns = vi.fn().mockRejectedValue(new Error('boom'));
	render(DomainFormDialog, {
		open: true,
		domain: { id: 'd', host: 'shop.example.com', port: 80, https: false } as DomainDto,
		onCheckDns,
		onSubmit: vi.fn(),
	});
	expect(await screen.findByText('Could not check DNS', {}, { timeout: 2000 })).toBeTruthy();
	expect(screen.getByText('boom')).toBeTruthy();
});
