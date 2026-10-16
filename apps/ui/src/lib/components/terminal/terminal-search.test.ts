import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { SearchAddon } from '@xterm/addon-search';
import type { Terminal } from '@xterm/xterm';
import TerminalSearch from './terminal-search.svelte';

afterEach(cleanup);

function setup() {
	let reportResults: ((result: { resultIndex: number; resultCount: number }) => void) | undefined;
	const addon = {
		onDidChangeResults: vi.fn((listener) => {
			reportResults = listener;
			return { dispose: vi.fn() };
		}),
		findNext: vi.fn(() => true),
		findPrevious: vi.fn(() => true),
		clearDecorations: vi.fn(),
	} as unknown as SearchAddon;
	const terminal = { focus: vi.fn() } as unknown as Terminal;
	const rendered = render(TerminalSearch, { addon, terminal });
	return {
		...rendered,
		addon,
		terminal,
		report: (index: number, count: number) =>
			reportResults?.({ resultIndex: index, resultCount: count }),
	};
}

describe('TerminalSearch', () => {
	it('searches incrementally and reports the active match', async () => {
		const { component, addon, report } = setup();
		await component.show();
		const input = screen.getByRole('textbox', { name: 'Search terminal' });
		await fireEvent.input(input, { target: { value: 'error' } });
		expect(addon.findNext).toHaveBeenCalledWith(
			'error',
			expect.objectContaining({ incremental: true }),
		);
		report(1, 4);
		await waitFor(() => expect(screen.getByText('2/4')).toBeTruthy());
	});

	it('supports previous, next, and close keyboard controls', async () => {
		const { component, addon, terminal } = setup();
		await component.show();
		const input = screen.getByRole('textbox', { name: 'Search terminal' });
		await fireEvent.input(input, { target: { value: 'ready' } });
		await fireEvent.keyDown(input, { key: 'Enter', shiftKey: true });
		expect(addon.findPrevious).toHaveBeenCalled();
		await fireEvent.keyDown(input, { key: 'Enter' });
		expect(addon.findNext).toHaveBeenCalled();
		await fireEvent.keyDown(input, { key: 'Escape' });
		expect(addon.clearDecorations).toHaveBeenCalled();
		expect(terminal.focus).toHaveBeenCalled();
		expect(screen.queryByRole('textbox', { name: 'Search terminal' })).toBeNull();
	});
});
