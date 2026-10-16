import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, expect, it, vi } from 'vitest';
import DeleteResourceDialog from './delete-resource-dialog.svelte';

vi.stubGlobal(
	'matchMedia',
	vi.fn().mockReturnValue({
		matches: false,
		addEventListener: vi.fn(),
		removeEventListener: vi.fn(),
	}),
);

afterEach(cleanup);

it('autofocuses the hold control when the alert dialog opens', async () => {
	render(DeleteResourceDialog, {
		open: true,
		resourceName: 'web',
		resourceType: 'service',
		ondelete: vi.fn(),
	});
	const button = await screen.findByRole('button', { name: /Hold to delete/i });
	await waitFor(() => expect(document.activeElement).toBe(button));
});

it('runs the deletion when Enter is held on the focused control', async () => {
	vi.useFakeTimers();
	const ondelete = vi.fn().mockResolvedValue(undefined);
	render(DeleteResourceDialog, {
		open: true,
		resourceName: 'web',
		resourceType: 'service',
		ondelete,
	});
	const button = screen.getByRole('button', { name: /Hold to delete/i });
	vi.advanceTimersByTime(1); // let the dialog move focus onto the control
	expect(document.activeElement).toBe(button);

	await fireEvent.keyDown(button, { key: 'Enter' });
	vi.advanceTimersByTime(1100);
	await fireEvent.keyUp(button, { key: 'Enter' });
	expect(ondelete).toHaveBeenCalledOnce();
	vi.useRealTimers();
});
