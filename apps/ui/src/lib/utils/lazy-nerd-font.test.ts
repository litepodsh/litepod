import { afterEach, describe, expect, it, vi } from 'vitest';

describe('loadNerdFont', () => {
	afterEach(() => {
		vi.unstubAllGlobals();
		vi.resetModules();
	});

	it('creates, loads, and registers the font only once', async () => {
		let resolveLoad!: (font: FontFace) => void;
		const font = { load: vi.fn() } as unknown as FontFace;
		font.load = vi.fn(() => new Promise<FontFace>((resolve) => (resolveLoad = resolve)));
		const FontFaceMock = vi.fn(function FontFaceMock() {
			return font;
		});
		const add = vi.fn();

		vi.stubGlobal('FontFace', FontFaceMock);
		vi.stubGlobal('document', { fonts: { add } });

		const { loadNerdFont } = await import('./lazy-nerd-font');
		const first = loadNerdFont();
		const second = loadNerdFont();

		expect(FontFaceMock).toHaveBeenCalledTimes(1);
		expect(font.load).toHaveBeenCalledTimes(1);
		expect(FontFaceMock).toHaveBeenCalledWith(
			'CommitMono Nerd Font Mono',
			'url(/fonts/CommitMonoNerdFontMono-Regular.woff2) format("woff2")',
		);

		resolveLoad(font);
		await Promise.all([first, second]);
		await loadNerdFont();

		expect(add).toHaveBeenCalledTimes(1);
		expect(add).toHaveBeenCalledWith(font);
		expect(FontFaceMock).toHaveBeenCalledTimes(1);
		expect(font.load).toHaveBeenCalledTimes(1);
	});
});
