import { render } from '@testing-library/svelte';
import { expect, it } from 'vitest';
import OsIcon from './os-icon.svelte';

it('renders the Alpine Linux mark for the alpine distribution', () => {
	const { container } = render(OsIcon, { distribution: 'alpine' });
	expect(container.querySelector('svg path')?.getAttribute('fill')).toBe('#0d597f');
});

it('renders the FreeBSD mark for the freebsd distribution', () => {
	const { container } = render(OsIcon, { distribution: 'freebsd' });
	expect(container.querySelector('svg path')?.getAttribute('fill')).toBe('#AB2B28');
});
