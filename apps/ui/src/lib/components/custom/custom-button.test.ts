import { createRawSnippet } from 'svelte';
import { render, screen } from '@testing-library/svelte';
import { expect, it } from 'vitest';
import CustomButton from './custom-button.svelte';

it('renders a semantic button with an optional trailing inset', () => {
	const text = createRawSnippet(() => ({ render: () => '<span>Deploy</span>' }));
	const icon = createRawSnippet(() => ({ render: () => '<span aria-hidden="true">→</span>' }));
	render(CustomButton, { children: text, icon });
	expect(screen.getByRole('button', { name: 'Deploy' })).toBeTruthy();
	expect(document.querySelector('[data-slot="custom-button-icon"]')).toBeTruthy();
});
