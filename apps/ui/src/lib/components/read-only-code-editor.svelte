<script lang="ts">
	import { onDestroy, onMount } from 'svelte';
	import { loadNerdFont } from '#lib/utils/lazy-nerd-font.js';
	import { loadMonaco } from '#lib/utils/load-monaco.js';

	let {
		content,
		language = 'plaintext',
		label,
		minHeight = '32rem',
		height,
	}: {
		content: string;
		language?: string;
		label: string;
		minHeight?: string;
		/**
		 * Exact pixel height (e.g. `"99px"`). When set, overrides `minHeight`
		 * so the editor fits the content snugly instead of reserving a floor.
		 */
		height?: string;
	} = $props();

	let host = $state<HTMLDivElement | null>(null);
	let editor = $state<import('monaco-editor').editor.IStandaloneCodeEditor | null>(null);
	let monacoApi = $state<typeof import('monaco-editor') | null>(null);
	let themeObserver: MutationObserver | null = null;

	function applyTheme() {
		monacoApi?.editor.setTheme(
			document.documentElement.classList.contains('dark') ? 'vs-dark' : 'vs',
		);
	}

	onMount(async () => {
		const [, monaco] = await Promise.all([loadNerdFont(), loadMonaco()]);
		monacoApi = monaco;
		applyTheme();
		themeObserver = new MutationObserver(applyTheme);
		themeObserver.observe(document.documentElement, {
			attributes: true,
			attributeFilter: ['class'],
		});
	});

	$effect(() => {
		if (!host || !monacoApi || editor) return;
		editor = monacoApi.editor.create(host, {
			value: content,
			language,
			minimap: { enabled: false },
			automaticLayout: true,
			readOnly: true,
			domReadOnly: true,
			wordWrap: 'on',
			scrollBeyondLastLine: false,
			fontFamily: "'Commit Mono', 'CommitMono Nerd Font Mono', monospace",
			fontLigatures: false,
			ariaLabel: label,
		});
	});

	$effect(() => {
		if (!editor || !monacoApi) return;
		const model = editor.getModel();
		if (!model) return;
		if (model.getValue() !== content) editor.setValue(content);
		monacoApi.editor.setModelLanguage(model, language);
	});

	onDestroy(() => {
		editor?.dispose();
		themeObserver?.disconnect();
	});
</script>

<div class="overflow-hidden rounded-lg border bg-background">
	<div bind:this={host} style:height style:min-height={!height ? minHeight : undefined}></div>
</div>
