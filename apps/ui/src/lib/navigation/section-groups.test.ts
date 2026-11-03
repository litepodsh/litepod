import { expect, it } from 'vitest';
import { groupSections } from './section-groups';

const groups = [{ value: 'a' }, { value: 'b' }, { value: 'empty' }];
const sections = [
	{ value: 'one', group: 'a' },
	{ value: 'two', group: 'b' },
	{ value: 'three', group: 'b' },
];

it('finds the open group and its sections from the current section', () => {
	const result = groupSections(groups, sections, 'three');
	expect(result.group).toBe('b');
	expect(result.items.map((item) => item.value)).toEqual(['two', 'three']);
});

it('drops groups with no visible section', () => {
	expect(groupSections(groups, sections, 'one').groups.map((item) => item.value)).toEqual([
		'a',
		'b',
	]);
});
