import { fireEvent, render, screen } from '@testing-library/svelte';
import { expect, it, vi } from 'vitest';
import ResizeGrip from './resize-grip.svelte';

it('resets when double-clicked', async () => {
	const onreset = vi.fn();
	render(ResizeGrip, {
		value: 20,
		min: 16,
		max: 34,
		label: 'Resize sidebar',
		onresize: vi.fn(),
		onreset,
	});
	await fireEvent.dblClick(screen.getByRole('separator', { name: 'Resize sidebar' }));
	expect(onreset).toHaveBeenCalledOnce();
});
