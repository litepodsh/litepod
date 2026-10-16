import { render } from '@testing-library/svelte';
import { expect, it } from 'vitest';
import SegmentedSwitch from './segmented-switch.svelte';

const items = [
	{ value: 'nav', label: 'Nav' },
	{ value: 'chat', label: 'Assistant' },
];

it('slides the indicator to the selected option', async () => {
	const { container, rerender } = render(SegmentedSwitch, {
		items,
		value: 'nav',
		label: 'Sidebar contents',
	});
	const indicator = container.querySelector('[aria-hidden="true"]') as HTMLElement;

	expect(indicator.style.transform).toBe('translateX(0%)');

	await rerender({ items, value: 'chat', label: 'Sidebar contents' });
	expect(indicator.style.transform).toBe('translateX(100%)');
});

it('reports the pressed option to assistive tech', () => {
	const { container } = render(SegmentedSwitch, {
		items,
		value: 'chat',
		label: 'Sidebar contents',
	});
	const pressed = Array.from(container.querySelectorAll('button')).map((button) =>
		button.getAttribute('aria-pressed'),
	);
	expect(pressed).toEqual(['false', 'true']);
});
