import { render } from '@testing-library/svelte';
import { expect, it } from 'vitest';
import LitepodMark from './litepod-mark.svelte';

it('is an accessible monochrome mark', () => {
	const { getByRole, container } = render(LitepodMark);
	expect(getByRole('img').getAttribute('aria-label')).toBe('Litepod');
	expect(container.querySelector('rect[fill="currentColor"]')).toBeTruthy();
});

it('keeps ids unique across instances', () => {
	const { container } = render(LitepodMark);
	const { container: c2 } = render(LitepodMark);
	expect(container.querySelector('pattern')!.id).not.toBe(c2.querySelector('pattern')!.id);
});
