import { describe, expect, it } from 'vitest';

import { MAX_TAB_SHORTCUTS, tabIndexFromKeydown, tabShortcutDigit } from './tab-shortcut';

function press(key: string, modifier: 'meta' | 'ctrl' | 'none' = 'meta'): KeyboardEvent {
	return {
		key,
		metaKey: modifier === 'meta',
		ctrlKey: modifier === 'ctrl',
	} as KeyboardEvent;
}

describe('tabShortcutDigit', () => {
	it('numbers the first nine tabs 1-9', () => {
		expect(tabShortcutDigit(0)).toBe(1);
		expect(tabShortcutDigit(8)).toBe(9);
	});

	it('gives the tenth tab 0, the way browsers do', () => {
		expect(tabShortcutDigit(9)).toBe(0);
	});

	it('has nothing left past the tenth', () => {
		expect(tabShortcutDigit(MAX_TAB_SHORTCUTS)).toBeUndefined();
		expect(tabShortcutDigit(-1)).toBeUndefined();
	});
});

describe('tabIndexFromKeydown', () => {
	it('maps digits to zero-based indexes, with 0 as the tenth', () => {
		expect(tabIndexFromKeydown(press('1'), 11)).toBe(0);
		expect(tabIndexFromKeydown(press('9'), 11)).toBe(8);
		expect(tabIndexFromKeydown(press('0'), 11)).toBe(9);
	});

	it('accepts either modifier', () => {
		expect(tabIndexFromKeydown(press('2', 'ctrl'), 5)).toBe(1);
		expect(tabIndexFromKeydown(press('2', 'none'), 5)).toBeNull();
	});

	it('refuses an index past the end of the row', () => {
		// The regression this guards: '0' must not select a tenth tab that is
		// not there, and must not wrap to -1.
		expect(tabIndexFromKeydown(press('0'), 4)).toBeNull();
		expect(tabIndexFromKeydown(press('5'), 4)).toBeNull();
		expect(tabIndexFromKeydown(press('4'), 4)).toBe(3);
	});

	it('ignores non-digit keys', () => {
		expect(tabIndexFromKeydown(press('a'), 5)).toBeNull();
		expect(tabIndexFromKeydown(press('Enter'), 5)).toBeNull();
	});
});
