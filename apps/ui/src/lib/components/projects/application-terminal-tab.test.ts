import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, expect, it, vi } from 'vitest';

vi.mock('@xterm/xterm', () => ({
	Terminal: class {
		options = {};
		onSelectionChange() {}
		attachCustomKeyEventHandler() {}
		loadAddon() {}
		open() {}
		onData() {
			return { dispose() {} };
		}
		write() {}
		dispose() {}
	},
}));
vi.mock('@xterm/addon-fit', () => ({
	FitAddon: class {
		fit() {}
		proposeDimensions() {}
	},
}));
vi.mock('@xterm/addon-web-links', () => ({ WebLinksAddon: class {} }));
vi.mock('@xterm/xterm/css/xterm.css', () => ({}));
vi.mock('#lib/utils/lazy-nerd-font.js', () => ({ loadNerdFont: async () => {} }));
vi.mock('#lib/clients/project-client.js', () => ({
	listApplicationLogTargets: async () => [
		{ container_name: 'web-1', service: 'web', status: 'running' },
		{ container_name: 'worker-1', service: 'worker', status: 'exited' },
	],
}));
const createExec = vi.fn().mockRejectedValue(new Error('stop here'));
vi.mock('#lib/clients/terminal-client.js', () => ({
	createExec: (...args: unknown[]) => createExec(...args),
	TerminalSocket: { open: vi.fn() },
}));

import ApplicationTerminalTab from './application-terminal-tab.svelte';

afterEach(cleanup);

it('lists running containers and connects with the picked shell', async () => {
	vi.stubGlobal(
		'ResizeObserver',
		class {
			observe() {}
			disconnect() {}
		},
	);
	render(ApplicationTerminalTab, {
		organizationId: 'o',
		projectId: 'p',
		environmentId: 'e',
		applicationId: 'a',
	});
	await screen.findByText('web-1 · running');
	expect(screen.queryByText(/worker-1/)).toBeNull();
	await fireEvent.click(screen.getByRole('button', { name: 'zsh' }));
	await fireEvent.click(screen.getByRole('button', { name: 'Connect' }));
	await waitFor(() =>
		expect(createExec).toHaveBeenCalledWith(
			expect.objectContaining({ containerId: 'web-1', command: ['/bin/zsh', '-l', '-i'] }),
		),
	);
	vi.unstubAllGlobals();
});
