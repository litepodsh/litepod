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

it('renders the OpenBSD mark for an openbsd host or distribution', () => {
	for (const props of [{ os: 'openbsd' }, { distribution: 'openbsd' }]) {
		const { container } = render(OsIcon, props);
		expect(container.querySelector('svg path')?.getAttribute('fill')).toBe('#F2CA30');
	}
});
