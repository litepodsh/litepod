<script lang="ts">
	import { onDestroy, onMount } from 'svelte';
	import { loadMonaco } from '#lib/utils/load-monaco.js';

	/**
	 * A read-only Monaco editor for an assistant chat code block. Reuses the
	 * same `monaco-editor` dynamic import pattern as `script-editor` so we get
	 * the same syntax highlighters without paying for them on pages that never
	 * render code. The block fits its content (one line of YAML looks like one
	 * line), capped at `maxLines` so a 200-line paste is still scannable.
	 */
	let {
		value,
		language = 'plaintext',
		maxLines = 20,
		class: className = '',
	}: {
		value: string;
		language?: string;
		/** Maximum lines visible before scrolling kicks in. The block always
		 *  grows to show whatever it has up to this cap. */
		maxLines?: number;
		class?: string;
	} = $props();

	let host = $state<HTMLDivElement | null>(null);
	let wrapper = $state<HTMLDivElement | null>(null);
	let editor = $state<import('monaco-editor').editor.IStandaloneCodeEditor | null>(null);
	let monacoApi = $state<typeof import('monaco-editor') | null>(null);
	let themeObserver: MutationObserver | null = null;

	const LINE_HEIGHT = 18;
	const PADDING = 16; // top + bottom

	function readCssVar(name: string): string {
		return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
	}

	/** Hex (`#rrggbb`) to `rgba()`, so token colors can be faded with an alpha. */
	function withAlpha(hex: string, alpha: number): string {
		const match = /^#?([0-9a-f]{6})$/i.exec(hex);
		if (!match) return hex;
		const value = Number.parseInt(match[1], 16);
		const r = (value >> 16) & 0xff;
		const g = (value >> 8) & 0xff;
		const b = value & 0xff;
		return `rgba(${r}, ${g}, ${b}, ${alpha})`;
	}

	function currentThemeName(): string {
		return document.documentElement.classList.contains('dark') ? 'lp-dark' : 'lp-light';
	}

	// Monaco themes are static hex, so they are rebuilt from the live `--lp-*`
	// tokens: the editor then follows light/dark and the warm/cool palette alike.
	function applyTheme() {
		const monaco = monacoApi;
		if (!monaco) return;
		const name = currentThemeName();
		const dark = name === 'lp-dark';
		const background = readCssVar('--lp-bg');
		const surface = readCssVar('--lp-surface');
		const ink = readCssVar('--lp-ink');
		const muted = readCssVar('--lp-muted');
		const accent = readCssVar('--lp-accent');
		const ok = readCssVar('--lp-ok');
		const warn = readCssVar('--lp-warn');
		const info = readCssVar('--lp-info');

		monaco.editor.defineTheme(name, {
			base: dark ? 'vs-dark' : 'vs',
			inherit: true,
			rules: [
				{ token: 'comment', foreground: muted },
				{ token: 'string', foreground: ok },
				{ token: 'string.escape', foreground: accent },
				{ token: 'number', foreground: warn },
				{ token: 'constant', foreground: warn },
				{ token: 'keyword', foreground: info },
				{ token: 'type', foreground: info },
				{ token: 'identifier', foreground: ink },
				{ token: 'variable', foreground: accent },
				{ token: 'variable.predefined', foreground: accent },
				{ token: 'delimiter', foreground: muted },
				{ token: 'tag', foreground: accent },
				{ token: 'attribute.name', foreground: ink },
				{ token: 'attribute.value', foreground: ok },
				{ token: 'regexp', foreground: ok },
			],
			colors: {
				'editor.background': background,
				'editor.foreground': ink,
				'editorLineNumber.foreground': withAlpha(muted, 0.6),
				'editorLineNumber.activeForeground': ink,
				'editorCursor.foreground': accent,
				'editor.selectionBackground': withAlpha(accent, 0.16),
				'editor.inactiveSelectionBackground': withAlpha(accent, 0.08),
				'editor.lineHighlightBackground': '#00000000',
				'editor.lineHighlightBorder': '#00000000',
				'editorGutter.background': background,
				'editorWidget.background': surface,
				'editorWidget.border': withAlpha(ink, 0.12),
				'editorHoverWidget.background': surface,
				'editorHoverWidget.border': withAlpha(ink, 0.12),
				'scrollbarSlider.background': withAlpha(muted, 0.3),
				'scrollbarSlider.hoverBackground': withAlpha(muted, 0.5),
				'scrollbarSlider.activeBackground': withAlpha(muted, 0.6),
			},
		});
		monaco.editor.setTheme(name);
	}

	function fitToContent() {
		if (!wrapper || !editor) return;
		// Monaco includes its vertical padding and every visual row created by
		// word wrap in this measurement.
		const height = Math.min(editor.getContentHeight(), maxLines * LINE_HEIGHT + PADDING);
		const nextHeight = `${height}px`;
		if (wrapper.style.height === nextHeight) return;
		wrapper.style.height = nextHeight;
		editor.layout();
	}

	onMount(async () => {
		const monaco = await loadMonaco();
		monacoApi = monaco;
		if (!host) return;
		// Define the themed editor theme before create: an unknown theme name
		// would silently fall back to `vs`.
		applyTheme();
		editor = monaco.editor.create(host, {
			value,
			language,
			readOnly: true,
			minimap: { enabled: false },
			lineNumbers: 'on',
			glyphMargin: false,
			folding: false,
			// Reserve enough horizontal space for line numbers (up to 999 lines);
			// padded so the gutter does not shift as content grows.
			lineDecorationsWidth: 8,
			lineNumbersMinChars: 3,
			wordWrap: 'on',
			scrollBeyondLastLine: false,
			automaticLayout: true,
			fontFamily: "'Commit Mono', monospace",
			fontSize: 11,
			lineHeight: LINE_HEIGHT,
			padding: { top: 8, bottom: 8 },
			renderLineHighlight: 'none',
			overviewRulerLanes: 0,
			overviewRulerBorder: false,
			hideCursorInOverviewRuler: true,
			// `auto` keeps the bar hidden until content actually overflows the
			// fit-to-content height; `visible` would always reserve space.
			scrollbar: {
				vertical: 'hidden',
				horizontal: 'auto',
				alwaysConsumeMouseWheel: false,
			},
			ariaLabel: `${language} code block`,
			theme: currentThemeName(),
		});

		// Size to the current content (could already overflow when the cap kicks
		// in immediately on mount).
		fitToContent();
		editor.onDidContentSizeChange(() => fitToContent());

		themeObserver = new MutationObserver(applyTheme);
		themeObserver.observe(document.documentElement, {
			attributes: true,
			attributeFilter: ['class', 'data-palette'],
		});
	});

	// Keep the editor in sync if the bubble re-renders the same key with new
	// content (the model streams, so the first emit often finishes mid-chunk).
	$effect(() => {
		if (!editor) return;
		if (editor.getValue() !== value) editor.setValue(value);
		const model = editor.getModel();
		if (model) {
			monacoApi?.editor.setModelLanguage(model, language);
		}
		fitToContent();
	});

	onDestroy(() => {
		themeObserver?.disconnect();
		editor?.dispose();
	});
</script>

<div
	bind:this={wrapper}
	class="relative overflow-hidden rounded-md border bg-background {className}">
	<div bind:this={host} class="h-full"></div>
</div>
