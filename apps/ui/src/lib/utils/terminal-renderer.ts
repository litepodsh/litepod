import { CanvasAddon } from '@xterm/addon-canvas';
import { WebglAddon } from '@xterm/addon-webgl';
import type { Terminal } from '@xterm/xterm';

/**
 * Load the fastest renderer the browser will give us: WebGL, then canvas, then
 * leave the built-in DOM renderer in place. WebGL has crashed the GPU process on
 * some Linux/Chromium builds (see terminal-images.ts), so a lost context tears
 * the addon down and drops back to the DOM renderer instead of wedging the
 * terminal.
 *
 * Call after `terminal.open()`. Returns a disposer for the active renderer addon
 * (a no-op when neither accelerated renderer could load).
 */
export function loadTerminalRenderer(terminal: Terminal): () => void {
	try {
		const webgl = new WebglAddon();
		// WebglAddon throws when disposed twice (context loss, then unmount) or
		// after its canvas is gone; a throw here would break the page teardown.
		let disposed = false;
		const dispose = () => {
			if (disposed) return;
			disposed = true;
			try {
				webgl.dispose();
			} catch (error) {
				console.warn('[terminal] WebGL renderer dispose failed', error);
			}
		};
		webgl.onContextLoss(dispose);
		terminal.loadAddon(webgl);
		return dispose;
	} catch (error) {
		console.warn('[terminal] WebGL renderer unavailable, trying canvas', error);
	}
	try {
		const canvas = new CanvasAddon();
		terminal.loadAddon(canvas);
		return () => canvas.dispose();
	} catch (error) {
		console.warn('[terminal] canvas renderer unavailable, using DOM', error);
	}
	return () => {};
}
