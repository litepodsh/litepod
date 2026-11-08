import { expect, it, vi } from 'vitest';

vi.mock('$app/state', () => ({ page: { url: new URL('http://x/projects/p1') } }));

import { clearShellTabs, setShellTabs, shellTabs } from './shell.svelte.js';

it('ties pushed tabs to the page path', () => {
	setShellTabs([{ value: 'logs', label: 'Logs' }], 'logs', () => {});
	expect(shellTabs.path).toBe('/projects/p1');
	clearShellTabs();
	expect(shellTabs.items).toEqual([]);
});
