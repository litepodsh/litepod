import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, expect, it } from 'vitest';
import ShellPicker from './shell-picker.svelte';

afterEach(cleanup);

it('offers five selectable shells', async () => {
	render(ShellPicker, { value: 'bash' });
	const radios = screen.getAllByRole('radio');
	expect(radios.map((radio) => radio.querySelector('b')?.textContent)).toEqual([
		'sh',
		'bash',
		'zsh',
		'fish',
		'nu',
	]);
	for (const radio of radios) {
		expect(radio.hasAttribute('aria-disabled')).toBe(false);
		expect(radio.textContent).not.toContain('Coming soon');
	}
	expect(radios[1].getAttribute('aria-checked')).toBe('true');

	for (const radio of radios.slice(2)) {
		await fireEvent.click(radio);
		expect(radio.getAttribute('aria-checked')).toBe('true');
	}
});
