import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type {
	BasicAuthConfigDto,
	ServerSummaryDto,
	SourceConfigDto,
} from '#lib/clients/project-client.js';
import ApplicationAdvancedTab from './application-advanced-tab.svelte';

beforeEach(() => {
	vi.stubGlobal('matchMedia', () => ({ matches: true }));
	vi.stubGlobal(
		'ResizeObserver',
		class {
			observe() {}
			unobserve() {}
			disconnect() {}
		},
	);
});
afterEach(() => {
	cleanup();
	vi.unstubAllGlobals();
});

it('keeps manual CPU and RAM inputs usable when server capacity is unknown', async () => {
	const source = {
		build_method: 'docker',
		build_cpu_cores: null,
		build_memory_mib: null,
	} as SourceConfigDto;
	const server = {
		name: 'Podman VM',
		capacity: { cpu_cores: null, memory_bytes: null, disk_free_bytes: null },
	} as ServerSummaryDto;
	const basicAuth = {
		enabled: false,
		realm: 'Restricted',
		users: [],
		protected_domain_ids: [],
	} as BasicAuthConfigDto;
	const onSaveSource = vi.fn().mockResolvedValue(undefined);
	render(ApplicationAdvancedTab, {
		server,
		source,
		domains: [],
		basicAuth,
		onSaveSource,
		onLoadResourceBaseline: vi.fn().mockResolvedValue(null),
		onSaveBasicAuth: vi.fn(),
		onAddBasicAuthUser: vi.fn(),
		onDeleteBasicAuthUser: vi.fn(),
		onSaveHealthCheck: vi.fn(),
	});
	await fireEvent.click(document.getElementById('resource-limits')!);
	const cpu = screen.getByRole('spinbutton', { name: 'CPU cores' }) as HTMLInputElement;
	const memory = screen.getByRole('spinbutton', { name: 'Memory (MiB)' }) as HTMLInputElement;
	expect(cpu.disabled).toBe(false);
	expect(memory.disabled).toBe(false);
	expect(
		screen.getAllByText('Server capacity is unavailable; enter the limit manually.'),
	).toHaveLength(2);
	await fireEvent.input(cpu, { target: { value: '2' } });
	await fireEvent.input(memory, { target: { value: '1024' } });
	await fireEvent.submit(
		screen.getByRole('button', { name: 'Save resource limits' }).closest('form')!,
	);
	await waitFor(() =>
		expect(onSaveSource).toHaveBeenCalledWith(
			expect.objectContaining({ build_cpu_cores: 2, build_memory_mib: 1024 }),
		),
	);
});

it('allows both physical maxima and suggests turning off limits', () => {
	const source = {
		build_method: 'docker',
		build_cpu_cores: 8,
		build_memory_mib: 8192,
	} as SourceConfigDto;
	const server = {
		name: 'Linux',
		capacity: { cpu_cores: 8, memory_bytes: 8 * 1024 ** 3, disk_free_bytes: null },
	} as ServerSummaryDto;
	const basicAuth = {
		enabled: false,
		realm: 'Restricted',
		users: [],
		protected_domain_ids: [],
	} as BasicAuthConfigDto;
	render(ApplicationAdvancedTab, {
		server,
		source,
		domains: [],
		basicAuth,
		onLoadResourceBaseline: vi.fn().mockResolvedValue({ cpu_cores: 1, memory_bytes: 1024 ** 3 }),
		onSaveSource: vi.fn(),
		onSaveBasicAuth: vi.fn(),
		onAddBasicAuthUser: vi.fn(),
		onDeleteBasicAuthUser: vi.fn(),
		onSaveHealthCheck: vi.fn(),
	});
	expect(
		screen.getByText('If you want to use all CPU and memory capacity, you can turn limits off.'),
	).toBeTruthy();
	// The sliders track the number inputs.
	const thumbs = screen.getAllByRole('slider');
	expect(thumbs).toHaveLength(2);
	expect(thumbs[0].getAttribute('aria-valuenow')).toBe('8');
	expect(thumbs[1].getAttribute('aria-valuenow')).toBe('8192');
});
