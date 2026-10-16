import { render, screen } from '@testing-library/svelte';
import { expect, it } from 'vitest';
import KpiCard from './kpi-card.svelte';

it('renders title, value and one tick per item', () => {
	render(KpiCard, { title: 'Apps', value: '3', ticks: ['ok', 'warn', 'neutral'] });
	expect(screen.getByText('Apps')).toBeTruthy();
	expect(screen.getByText('3')).toBeTruthy();
	expect(screen.getByTestId('kpi-ticks').children).toHaveLength(3);
});
