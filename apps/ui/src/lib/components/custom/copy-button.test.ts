import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import CopyButton from './copy-button.svelte';

describe('CopyButton', () => {
	beforeEach(() => {
		Object.defineProperty(navigator, 'clipboard', { value: undefined, configurable: true });
	});

	afterEach(() => {
		cleanup();
		vi.restoreAllMocks();
		Reflect.deleteProperty(navigator, 'clipboard');
		Reflect.deleteProperty(document, 'execCommand');
	});

	it('copies through execCommand when the Clipboard API is missing', async () => {
		const exec = vi.fn(() => true);
		Object.defineProperty(document, 'execCommand', { value: exec, configurable: true });

		render(CopyButton, { props: { value: 'deployment logs' } });
		await fireEvent.click(screen.getByRole('button', { name: 'Copy deployment logs' }));

		expect(exec).toHaveBeenCalledWith('copy');
		expect(document.querySelector('textarea')).toBeNull();
		expect(screen.getByRole('button', { name: 'Copied deployment logs' })).toBeTruthy();
	});
});
