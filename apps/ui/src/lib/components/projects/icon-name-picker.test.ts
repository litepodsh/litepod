import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';

import IconNamePicker from './icon-name-picker.svelte';

// jsdom has no layout; the command list scrolls its highlighted item into view.
Element.prototype.scrollIntoView = vi.fn();

afterEach(cleanup);

async function openPicker(value = '') {
	const onpick = vi.fn();
	render(IconNamePicker, { value, onpick });
	await fireEvent.click(screen.getByRole('button'));
	await waitFor(() => expect(screen.getByRole('option', { name: 'Rocket' })).toBeTruthy());
	return onpick;
}

describe('IconNamePicker', () => {
	it('keeps the icons out of the form until it is opened', () => {
		render(IconNamePicker, { value: '', onpick: vi.fn() });
		expect(screen.queryByRole('option')).toBeNull();
	});

	it('picks an icon by its name', async () => {
		const onpick = await openPicker();
		await fireEvent.click(screen.getByRole('option', { name: 'Rocket' }));
		expect(onpick).toHaveBeenCalledWith('rocket');
	});

	it('offers to remove only an icon that is there', async () => {
		const onpick = await openPicker('rocket');
		await fireEvent.click(screen.getByRole('button', { name: /remove/i }));
		expect(onpick).toHaveBeenCalledWith('');
		cleanup();
		await openPicker();
		expect(screen.queryByRole('button', { name: /remove/i })).toBeNull();
	});
});
