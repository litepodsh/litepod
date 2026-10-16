// Matches CSI escape sequences (colors, cursor moves) that a plain <pre> cannot render.
const ANSI_ESCAPE = /\x1b\[[0-9;?]*[A-Za-z]/g;

/**
 * Renders raw command output the way a terminal would for the common cases:
 * `\n` starts a new line and a bare `\r` returns to the start of the current
 * line, so text written after it overwrites what was there (progress bars).
 */
export function renderTerminalText(raw: string): string {
	const lines = [''];
	let atLineStart = false;
	for (const part of raw.replace(ANSI_ESCAPE, '').split(/(\r\n|\r|\n)/)) {
		if (part === '\n' || part === '\r\n') {
			lines.push('');
			atLineStart = false;
		} else if (part === '\r') {
			atLineStart = true;
		} else if (part) {
			const current = lines[lines.length - 1];
			lines[lines.length - 1] = atLineStart ? part + current.slice(part.length) : current + part;
			atLineStart = false;
		}
	}
	return lines.join('\n');
}
