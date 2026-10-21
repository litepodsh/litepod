import { render, screen } from '@testing-library/svelte';
import { expect, it } from 'vitest';
import KpiCard from './kpi-card.svelte';

it('renders title, value and one tick per item', () => {
	render(KpiCard, { title: 'Apps', value: '3', ticks: ['ok', 'warn', 'neutral'] });
	expect(screen.getByText('Apps')).toBeTruthy();
	expect(screen.getByText('3')).toBeTruthy();
	expect(screen.getByTestId('kpi-ticks').children).toHaveLength(3);
});

it('renders square ticks when asked', () => {
	const { container } = render(KpiCard, {
		title: 'Domains',
		value: '2',
		ticks: ['ok', 'neutral'],
		tickShape: 'square',
	});
	const ticks = container.querySelector('[data-testid="kpi-ticks"]')!.children;
	expect(ticks).toHaveLength(2);
	expect(ticks[0].classList.contains('size-2.5')).toBe(true);
});
