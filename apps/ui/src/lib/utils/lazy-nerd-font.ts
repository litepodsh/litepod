let loading: Promise<void> | undefined;

export function loadNerdFont(): Promise<void> {
	if (typeof document === 'undefined') {
		return Promise.resolve();
	}

	if (!loading) {
		loading = (async () => {
			const font = new FontFace(
				'CommitMono Nerd Font Mono',
				'url(/fonts/CommitMonoNerdFontMono-Regular.woff2) format("woff2")',
			);
			const loadedFont = await font.load();
			document.fonts.add(loadedFont);
		})();
	}

	return loading;
}
