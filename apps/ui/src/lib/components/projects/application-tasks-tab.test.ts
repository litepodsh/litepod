import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, expect, it, vi } from 'vitest';
import ApplicationTasksTab from './application-tasks-tab.svelte';

const task = (id: string, name: string, application_id: string | null) => ({
	id,
	organization_id: 'org',
	name,
	description: '',
	cron_expression: '0 3 * * *',
	runner: application_id ? 'application_exec' : 'ephemeral',
	image: application_id ? null : 'alpine',
	application_id,
	compose_service: 'web',
	shell: 'sh',
	timeout_seconds: 60,
	network_access: false,
	enabled: true,
	server_id: 'local',
	next_run_at: null,
	active_execution_id: null,
	created_at: 0,
	updated_at: 0,
});

vi.mock('#lib/clients/scheduled-tasks-client.js', async (original) => ({
	...(await original<object>()),
	listScheduledTasks: async () => [
		task('a', 'mine-cleanup', 'app-1'),
		task('b', 'other-app-job', 'app-2'),
		task('c', 'ephemeral-backup', null),
	],
	listScheduledTaskExecutions: async () => [],
}));
vi.mock('#lib/clients/auth-client.js', () => ({ getMe: async () => ({ is_sysadmin: true }) }));
vi.mock('#lib/clients/system-client.js', () => ({
	getSystemVersion: async () => ({ is_cloud: false }),
}));
vi.mock('#lib/clients/server-client.js', () => ({ listServers: async () => [] }));
vi.mock('#lib/clients/project-client.js', () => ({
	listProjects: vi.fn(async () => []),
	getProject: vi.fn(),
	composeStatus: async () => [],
}));
vi.mock('#lib/utils/load-monaco.js', () => ({
	loadMonaco: () => Promise.reject(new Error('jsdom')),
}));
vi.mock('#lib/utils/lazy-nerd-font.js', () => ({ loadNerdFont: () => Promise.resolve() }));

afterEach(cleanup);

const props = {
	organizationId: 'org',
	projectId: 'p',
	environmentId: 'e',
	applicationId: 'app-1',
	applicationName: 'umami',
};

it("lists only this app's tasks", async () => {
	render(ApplicationTasksTab, props);
	expect(await screen.findByText('mine-cleanup')).toBeTruthy();
	expect(screen.queryByText('other-app-job')).toBeNull();
	expect(screen.queryByText('ephemeral-backup')).toBeNull();
});

it('opens a new task already running inside this app', async () => {
	render(ApplicationTasksTab, props);
	await fireEvent.click(await screen.findByRole('button', { name: /New task in this app/ }));
	const inside = await screen.findByRole('radio', { name: /Inside an app/ });
	expect(inside.getAttribute('aria-checked')).toBe('true');
	expect(
		(screen.getByRole('radio', { name: /Ephemeral container/ }) as HTMLButtonElement).disabled,
	).toBe(true);
	const app = screen.getByLabelText('Application') as HTMLInputElement;
	expect(app.value).toBe('umami');
	expect(app.disabled).toBe(true);
});
