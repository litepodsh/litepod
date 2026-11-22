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

it('keeps the typed memory value even when it is not a slider step multiple', async () => {
	const source = {
		build_method: 'railpack',
		build_cpu_cores: 6,
		build_memory_mib: 1920,
	} as SourceConfigDto;
	const server = {
		name: 'Linux',
		capacity: { cpu_cores: 8, memory_bytes: 16 * 1024 ** 3, disk_free_bytes: null },
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
		onSaveSource: vi.fn(),
		onLoadResourceBaseline: vi.fn().mockResolvedValue(null),
		onSaveBasicAuth: vi.fn(),
		onAddBasicAuthUser: vi.fn(),
		onDeleteBasicAuthUser: vi.fn(),
		onSaveHealthCheck: vi.fn(),
	});
	const memory = screen.getByRole('spinbutton', { name: 'Memory (MiB)' }) as HTMLInputElement;
	await fireEvent.input(memory, { target: { value: '' } });
	for (const value of ['2', '20', '204']) {
		await fireEvent.input(memory, { target: { value } });
	}
	// The slider step is 64; bits-ui used to snap 204 to 192 and write it back.
	expect(memory.value).toBe('204');
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

it('updates the CPU limit while the slider is still being dragged', async () => {
	const source = {
		build_method: 'docker',
		build_cpu_cores: 8,
		build_memory_mib: 8192,
	} as SourceConfigDto;
	render(ApplicationAdvancedTab, {
		server: {
			name: 'Linux',
			capacity: { cpu_cores: 8, memory_bytes: 16 * 1024 ** 3, disk_free_bytes: null },
		} as ServerSummaryDto,
		source,
		domains: [],
		basicAuth: {
			enabled: false,
			realm: 'Restricted',
			users: [],
			protected_domain_ids: [],
		} as BasicAuthConfigDto,
		onSaveSource: vi.fn(),
		onLoadResourceBaseline: vi.fn().mockResolvedValue(null),
		onSaveBasicAuth: vi.fn(),
		onAddBasicAuthUser: vi.fn(),
		onDeleteBasicAuthUser: vi.fn(),
		onSaveHealthCheck: vi.fn(),
	});
	const cpu = screen.getByRole('spinbutton', { name: 'CPU cores' }) as HTMLInputElement;
	const cpuSlider = screen.getAllByRole('slider')[0].closest('[data-slot="slider"]') as HTMLElement;
	cpuSlider.getBoundingClientRect = () =>
		({ left: 0, right: 200, top: 0, bottom: 10, width: 200, height: 10, x: 0, y: 0 }) as DOMRect;

	await fireEvent.pointerDown(cpuSlider, { clientX: 200, clientY: 5, button: 0 });
	await fireEvent.pointerMove(cpuSlider, { clientX: 100.25, clientY: 5 });
	// Still dragging: the value must already be reflected before pointerup commits it.
	expect(cpu.value).toBe('4.1');
	await fireEvent.pointerUp(cpuSlider, { clientX: 100.25, clientY: 5 });
});

it('saves separate limits for the running app of a Git build, and clears them', async () => {
	const source = {
		deploy_type: 'git',
		build_method: 'docker',
		build_cpu_cores: 8,
		build_memory_mib: 16384,
		runtime_cpu_cores: null,
		runtime_memory_mib: null,
	} as SourceConfigDto;
	const onSaveRuntimeLimits = vi.fn().mockResolvedValue(undefined);
	render(ApplicationAdvancedTab, {
		server: {
			name: 'web',
			capacity: { cpu_cores: 2, memory_bytes: 4 * 1024 ** 3, disk_free_bytes: null },
		} as ServerSummaryDto,
		source,
		domains: [],
		basicAuth: {
			enabled: false,
			realm: 'Restricted',
			users: [],
			protected_domain_ids: [],
		} as BasicAuthConfigDto,
		onSaveSource: vi.fn(),
		onLoadResourceBaseline: vi.fn().mockResolvedValue(null),
		onSaveBasicAuth: vi.fn(),
		onAddBasicAuthUser: vi.fn(),
		onDeleteBasicAuthUser: vi.fn(),
		onSaveHealthCheck: vi.fn(),
		onSaveRuntimeLimits,
	});
	const form = screen.getByRole('button', { name: 'Save running app limits' }).closest('form')!;
	await fireEvent.click(document.getElementById('runtime-limits')!);
	await fireEvent.input(document.getElementById('runtime-cpu-cores')!, {
		target: { value: '0.5' },
	});
	await fireEvent.input(document.getElementById('runtime-memory-mib')!, {
		target: { value: '512' },
	});
	await fireEvent.submit(form);
	await waitFor(() =>
		expect(onSaveRuntimeLimits).toHaveBeenCalledWith({
			runtime_cpu_cores: 0.5,
			runtime_memory_mib: 512,
		}),
	);

	await fireEvent.click(document.getElementById('runtime-limits')!);
	await fireEvent.submit(form);
	await waitFor(() =>
		expect(onSaveRuntimeLimits).toHaveBeenLastCalledWith({
			runtime_cpu_cores: null,
			runtime_memory_mib: null,
		}),
	);
});
