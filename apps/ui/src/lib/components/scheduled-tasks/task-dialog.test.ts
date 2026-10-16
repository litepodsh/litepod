import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import TaskDialog from './task-dialog.svelte';
import type { ScheduledTaskDetail } from '#lib/clients/scheduled-tasks-client.js';

const api = vi.hoisted(() => ({
	createScheduledTask: vi.fn(),
	updateScheduledTask: vi.fn(),
	streamScheduledTaskTest: vi.fn(),
}));

vi.mock('#lib/clients/scheduled-tasks-client.js', async (original) => ({
	...(await original<object>()),
	...api,
}));
vi.mock('#lib/clients/auth-client.js', () => ({ getMe: async () => ({ is_sysadmin: false }) }));
vi.mock('#lib/clients/system-client.js', () => ({
	getSystemVersion: async () => ({ is_cloud: false }),
}));
vi.mock('#lib/clients/server-client.js', () => ({
	listServers: async () => [{ id: 'local', name: 'Local', kind: 'local', state: 'ready' }],
}));
vi.mock('#lib/clients/project-client.js', () => ({
	listProjects: async () => [],
	getProject: async () => ({ project: {}, environments: [] }),
	composeStatus: async () => [],
}));
vi.mock('#lib/utils/load-monaco.js', () => ({
	loadMonaco: () => Promise.reject(new Error('jsdom')),
}));
vi.mock('#lib/utils/lazy-nerd-font.js', () => ({ loadNerdFont: () => Promise.resolve() }));

beforeEach(() => {
	api.createScheduledTask.mockReset().mockResolvedValue({ id: 'new' });
	api.updateScheduledTask.mockReset().mockResolvedValue({ id: 'task' });
});
afterEach(cleanup);

const FIELDS = [
	'application_id',
	'compose_service',
	'cron_expression',
	'description',
	'image',
	'name',
	'network_access',
	'runner',
	'script',
	'server_id',
	'shell',
	'timeout_seconds',
];

it('shows the fields of the picked runner', async () => {
	render(TaskDialog, { open: true, organizationId: 'org' });
	await fireEvent.click(await screen.findByRole('radio', { name: /Ephemeral container/ }));
	expect(screen.getByLabelText('Docker image')).toBeTruthy();
	expect(screen.getByText('Allow network connection')).toBeTruthy();
	expect(screen.queryByLabelText('Application')).toBeNull();

	await fireEvent.click(screen.getByRole('radio', { name: /Inside an app/ }));
	expect(screen.getByLabelText('Application')).toBeTruthy();
	expect(screen.getByText('Compose service (optional)')).toBeTruthy();
	expect(screen.queryByLabelText('Docker image')).toBeNull();
	expect(screen.queryByText('Allow network connection')).toBeNull();
});

it('explains the cron and lists the next three runs', async () => {
	render(TaskDialog, { open: true, organizationId: 'org' });
	const cron = await screen.findByLabelText('Cron expression');
	await fireEvent.input(cron, { target: { value: '0 3 * * *' } });
	expect(screen.getAllByText('Every day at 03:00 UTC').length).toBeGreaterThan(0);
	const runs = within(screen.getByRole('list', { name: 'Next runs' })).getAllByRole('listitem');
	expect(runs).toHaveLength(3);
	expect(runs[0].textContent).toMatch(/03:00 UTC/);
});

it('creates the task with the request field names', async () => {
	render(TaskDialog, { open: true, organizationId: 'org' });
	await fireEvent.input(await screen.findByLabelText('Task name'), {
		target: { value: 'backup' },
	});
	await fireEvent.input(screen.getByLabelText('Docker image'), {
		target: { value: 'alpine:3.22' },
	});
	await fireEvent.click(screen.getByRole('button', { name: /Create task/ }));
	await waitFor(() => expect(api.createScheduledTask).toHaveBeenCalledOnce());
	const [organizationId, body] = api.createScheduledTask.mock.calls[0];
	expect(organizationId).toBe('org');
	expect(Object.keys(body).sort()).toEqual(FIELDS);
	expect(body).toMatchObject({
		name: 'backup',
		runner: 'ephemeral',
		image: 'alpine:3.22',
		cron_expression: '0 0 * * *',
		server_id: 'local',
		shell: 'bash',
		timeout_seconds: 3600,
		network_access: false,
	});
});

it('updates an existing task and keeps its enabled flag', async () => {
	const editing = {
		id: 'task',
		name: 'cleanup',
		description: '',
		cron_expression: '*/5 * * * *',
		runner: 'remote_shell',
		image: null,
		application_id: null,
		compose_service: null,
		shell: 'sh',
		script: 'podman system df',
		timeout_seconds: 300,
		network_access: false,
		enabled: false,
		server_id: 'srv',
	} as unknown as ScheduledTaskDetail;
	render(TaskDialog, { open: true, organizationId: 'org', editing });
	expect(await screen.findByText('Edit scheduled task')).toBeTruthy();
	await fireEvent.click(screen.getByRole('button', { name: /Save changes/ }));
	await waitFor(() => expect(api.updateScheduledTask).toHaveBeenCalledOnce());
	const [, id, body] = api.updateScheduledTask.mock.calls[0];
	expect(id).toBe('task');
	expect(Object.keys(body).sort()).toEqual([...FIELDS, 'enabled'].sort());
	expect(body).toMatchObject({
		enabled: false,
		runner: 'remote_shell',
		shell: 'sh',
		server_id: 'srv',
		cron_expression: '*/5 * * * *',
	});
});

it('clones a task as a new one with a suffixed name', async () => {
	const cloneFrom = {
		id: 'task',
		name: 'cleanup',
		description: 'nightly',
		cron_expression: '*/5 * * * *',
		runner: 'remote_shell',
		image: null,
		application_id: null,
		compose_service: null,
		shell: 'sh',
		script: 'podman system df',
		timeout_seconds: 300,
		network_access: false,
		enabled: true,
		server_id: 'srv',
	} as unknown as ScheduledTaskDetail;
	render(TaskDialog, { open: true, organizationId: 'org', cloneFrom });
	expect(await screen.findByText('New scheduled task')).toBeTruthy();
	await fireEvent.click(screen.getByRole('button', { name: /Create task/ }));
	await waitFor(() => expect(api.createScheduledTask).toHaveBeenCalledOnce());
	expect(api.updateScheduledTask).not.toHaveBeenCalled();
	const [, body] = api.createScheduledTask.mock.calls[0];
	expect(body).toMatchObject({
		name: 'cleanup (copy)',
		description: 'nightly',
		cron_expression: '*/5 * * * *',
		runner: 'remote_shell',
		shell: 'sh',
		script: 'podman system df',
		timeout_seconds: 300,
		server_id: 'srv',
	});
});

it('warns that the default ephemeral image lacks zsh, fish and nu, and sends the picked shell', async () => {
	render(TaskDialog, { open: true, organizationId: 'org' });
	const hint = /default image only has sh and bash/;
	await fireEvent.input(await screen.findByLabelText('Task name'), {
		target: { value: 'fishy' },
	});
	expect(screen.queryByText(hint)).toBeNull();
	await fireEvent.click(screen.getByRole('radio', { name: /^fish/ }));
	expect(screen.getByText(hint).textContent).toContain('includes fish');
	await fireEvent.input(screen.getByLabelText('Docker image'), {
		target: { value: 'ghcr.io/acme/fish:4' },
	});
	expect(screen.queryByText(hint)).toBeNull();
	await fireEvent.click(screen.getByRole('button', { name: /Create task/ }));
	await waitFor(() => expect(api.createScheduledTask).toHaveBeenCalledOnce());
	expect(api.createScheduledTask.mock.calls[0][1]).toMatchObject({
		shell: 'fish',
		image: 'ghcr.io/acme/fish:4',
	});
});
