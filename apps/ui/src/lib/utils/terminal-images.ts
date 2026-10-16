import { ImageAddon } from '@xterm/addon-image';
import type { Terminal } from '@xterm/xterm';

// NOTE: earlier this addon was implicated in a Chromium/helium renderer SIGILL
// while a server console streamed output. It is being re-enabled deliberately
// and tested on its own (no Unicode11/Ligatures). If the renderer crashes again
// on connect, this is the first thing to pull back out.
const IMAGE_OPTIONS = {
	enableSizeReports: true,
	pixelLimit: 8_388_608,
	sixelSupport: true,
	sixelScrolling: true,
	sixelPaletteLimit: 256,
	sixelSizeLimit: 10_000_000,
	storageLimit: 32,
	showPlaceholder: true,
	iipSupport: true,
	iipSizeLimit: 10_000_000,
} as const;

/** Enable bounded SIXEL and iTerm inline-image rendering for a web terminal. */
export function loadTerminalImages(terminal: Terminal): void {
	terminal.loadAddon(new ImageAddon(IMAGE_OPTIONS));
}
