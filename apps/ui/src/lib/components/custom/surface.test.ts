import { createRawSnippet } from 'svelte';
import { render } from '@testing-library/svelte';
import { expect, it } from 'vitest';
import Surface from './surface.svelte';

it('renders one outer bezel and one inner surface', () => {
	const content = createRawSnippet(() => ({ render: () => '<span>Content</span>' }));
	const { container } = render(Surface, { children: content });
	expect(container.querySelector('[data-slot="surface-outer"]')).toBeTruthy();
	expect(container.querySelector('[data-slot="surface-inner"]')).toBeTruthy();
});
