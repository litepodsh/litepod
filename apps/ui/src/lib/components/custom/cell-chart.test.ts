import { render } from '@testing-library/svelte';
import { expect, it } from 'vitest';
import CellChart from './cell-chart.svelte';

it('draws cols x rows cells and fills by value', () => {
	const { container } = render(CellChart, {
		values: [0, 5, 10],
		cols: 3,
		rows: 4,
		max: 10,
		ariaLabel: 'cpu',
	});
	expect(container.querySelectorAll('rect.cell')).toHaveLength(12);
	const filled = [...container.querySelectorAll('g')].map(
		(g) => g.querySelectorAll('rect.cell.on').length,
	);
	expect(filled).toEqual([0, 2, 4]);
});

it('resamples to the requested column count', () => {
	const { container } = render(CellChart, {
		values: [1, 2],
		cols: 6,
		rows: 2,
		ariaLabel: 'traffic',
	});
	expect(container.querySelectorAll('g')).toHaveLength(6);
});
