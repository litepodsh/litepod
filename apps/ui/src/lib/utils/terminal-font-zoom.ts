/**
 * Shared terminal font sizing. Browser zoom shortcuts (Ctrl/Cmd + -, +, 0) are
 * remapped to these values so only the terminal text changes size while the
 * surrounding dialog keeps its expanded layout.
 */
export const TERMINAL_FONT_SIZE = {
	initial: 13,
	min: 8,
	max: 24,
	step: 1,
} as const;

export type TerminalFontZoom = 'in' | 'out' | 'reset';

/** The terminal zoom represented by this keydown, or null for anything else. */
export function terminalFontZoom(event: KeyboardEvent): TerminalFontZoom | null {
	if (!(event.metaKey || event.ctrlKey) || event.altKey) return null;
	if (event.key === '+' || event.key === '=' || event.code === 'NumpadAdd') return 'in';
	if (event.key === '-' || event.key === '_' || event.code === 'NumpadSubtract') return 'out';
	if (event.key === '0') return 'reset';
	return null;
}

export function clampTerminalFontSize(size: number): number {
	return Math.min(TERMINAL_FONT_SIZE.max, Math.max(TERMINAL_FONT_SIZE.min, size));
}

export function zoomedTerminalFontSize(current: number, zoom: TerminalFontZoom): number {
	if (zoom === 'reset') return TERMINAL_FONT_SIZE.initial;
	const delta = zoom === 'in' ? TERMINAL_FONT_SIZE.step : -TERMINAL_FONT_SIZE.step;
	return clampTerminalFontSize(current + delta);
}
