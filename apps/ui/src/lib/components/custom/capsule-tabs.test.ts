import { cleanup, render } from '@testing-library/svelte';
import { afterEach, expect, it, vi } from 'vitest';
import CapsuleTabs from './capsule-tabs.svelte';

afterEach(cleanup);

it('never scrolls the page to reach the selected option', () => {
	const scrollIntoView = vi.fn();
	Element.prototype.scrollIntoView = scrollIntoView;
	render(CapsuleTabs, {
		items: [
			{ value: '1h', label: '1h' },
			{ value: '24h', label: '24h' },
		],
		value: '1h',
		label: 'Range',
	});
	expect(scrollIntoView).not.toHaveBeenCalled();
});

it('scrolls only the capsule to reveal an option cut off on the right', () => {
	const rect = (left: number, right: number) => ({ left, right }) as DOMRect;
	const spy = vi.spyOn(Element.prototype, 'getBoundingClientRect').mockImplementation(function (
		this: Element,
	) {
		return this.getAttribute('role') === 'tablist' ? rect(0, 100) : rect(80, 130);
	});
	const { getByRole } = render(CapsuleTabs, {
		items: [{ value: '1h', label: '1h' }],
		value: '1h',
		label: 'Range',
	});
	expect(getByRole('tablist').scrollLeft).toBe(30);
	spy.mockRestore();
});
