import { describe, expect, it } from 'vitest';

import {
	clampTerminalFontSize,
	terminalFontZoom,
	zoomedTerminalFontSize,
	TERMINAL_FONT_SIZE,
} from './terminal-font-zoom';

const keydown = (init: KeyboardEventInit) => new KeyboardEvent('keydown', init);

describe('terminal font zoom', () => {
	it('maps the browser zoom shortcuts to terminal font zoom', () => {
		expect(terminalFontZoom(keydown({ key: '-', ctrlKey: true }))).toBe('out');
		expect(terminalFontZoom(keydown({ key: '_', metaKey: true }))).toBe('out');
		expect(terminalFontZoom(keydown({ key: '=', ctrlKey: true }))).toBe('in');
		expect(terminalFontZoom(keydown({ key: '+', metaKey: true }))).toBe('in');
		expect(terminalFontZoom(keydown({ key: '0', ctrlKey: true }))).toBe('reset');
		expect(terminalFontZoom(keydown({ key: 'Add', code: 'NumpadAdd', ctrlKey: true }))).toBe('in');
		expect(
			terminalFontZoom(keydown({ key: 'Subtract', code: 'NumpadSubtract', ctrlKey: true })),
		).toBe('out');
	});

	it('ignores keys that are not the zoom shortcut', () => {
		expect(terminalFontZoom(keydown({ key: '-' }))).toBeNull();
		expect(terminalFontZoom(keydown({ key: '-', ctrlKey: true, altKey: true }))).toBeNull();
		expect(terminalFontZoom(keydown({ key: 'a', ctrlKey: true }))).toBeNull();
	});

	it('clamps the terminal font between the supported bounds', () => {
		expect(clampTerminalFontSize(1)).toBe(TERMINAL_FONT_SIZE.min);
		expect(clampTerminalFontSize(99)).toBe(TERMINAL_FONT_SIZE.max);
		expect(zoomedTerminalFontSize(TERMINAL_FONT_SIZE.min, 'out')).toBe(TERMINAL_FONT_SIZE.min);
		expect(zoomedTerminalFontSize(TERMINAL_FONT_SIZE.max, 'in')).toBe(TERMINAL_FONT_SIZE.max);
		expect(zoomedTerminalFontSize(15, 'reset')).toBe(TERMINAL_FONT_SIZE.initial);
	});
});
