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

	function applyTheme() {
		monacoApi?.editor.setTheme(
			document.documentElement.classList.contains('dark') ? 'vs-dark' : 'vs',
		);
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
			theme: document.documentElement.classList.contains('dark') ? 'vs-dark' : 'vs',
		});

		// Size to the current content (could already overflow when the cap kicks
		// in immediately on mount).
		fitToContent();
		editor.onDidContentSizeChange(() => fitToContent());

		themeObserver = new MutationObserver(applyTheme);
		themeObserver.observe(document.documentElement, {
			attributes: true,
			attributeFilter: ['class'],
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
