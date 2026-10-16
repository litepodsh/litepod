<script lang="ts">
	import { onMount, type Snippet } from 'svelte';
	import { Textarea } from '#lib/components/ui/textarea/index.js';
	import { loadNerdFont } from '#lib/utils/lazy-nerd-font.js';
	import { loadMonaco } from '#lib/utils/load-monaco.js';
	let {
		value = $bindable('echo "Hello World"'),
		shell = 'bash',
		language = 'shell',
		label,
		height = '18rem',
		disabled = false,
		filename,
		actions,
	}: {
		value?: string;
		shell?: 'bash' | 'sh' | 'zsh' | 'fish' | 'nu';
		language?: string;
		label?: string;
		height?: string;
		disabled?: boolean;
		/** Shows a top bar with this file name (and `actions` on the right). */
		filename?: string;
		actions?: Snippet;
	} = $props();
	let host = $state<HTMLDivElement | null>(null);
	let editor = $state<import('monaco-editor').editor.IStandaloneCodeEditor | null>(null);
	let monacoApi = $state<typeof import('monaco-editor') | null>(null);
	let ready = $state(false);
	let themeObserver: MutationObserver | null = null;

	function applyTheme() {
		monacoApi?.editor.setTheme(
			document.documentElement.classList.contains('dark') ? 'vs-dark' : 'vs',
		);
	}

	onMount(() => {
		let active = true;
		void loadNerdFont().catch(() => {});
		void loadMonaco()
			.then((monaco) => {
				if (!active || !host) return;
				monacoApi = monaco;
				editor = monaco.editor.create(host, {
					value,
					language,
					readOnly: disabled,
					minimap: { enabled: false },
					automaticLayout: true,
					wordWrap: 'on',
					scrollBeyondLastLine: false,
					fontFamily: "'Commit Mono', monospace",
					ariaLabel: label ?? `${shell} script editor`,
					theme: document.documentElement.classList.contains('dark') ? 'vs-dark' : 'vs',
				});
				editor.onDidChangeModelContent(() => (value = editor?.getValue() ?? ''));
				themeObserver = new MutationObserver(applyTheme);
				themeObserver.observe(document.documentElement, {
					attributes: true,
					attributeFilter: ['class'],
				});
				ready = true;
			})
			.catch(() => {
				/* Keep the textarea usable if Monaco cannot load. */
			});
		return () => {
			active = false;
			editor?.dispose();
			themeObserver?.disconnect();
		};
	});
	$effect(() => {
		if (editor && editor.getValue() !== value) editor.setValue(value);
	});
	$effect(() => {
		editor?.updateOptions({ readOnly: disabled });
	});
</script>

<div class="lp-inset min-w-0 overflow-hidden rounded-xl border bg-background">
	{#if filename}
		<div
			class="lp-groove-b flex items-center justify-between gap-2 bg-muted/60 py-1.5 ps-3 pe-2 font-mono text-[11.5px] text-muted-foreground">
			<span class="truncate">{filename}</span>
			{@render actions?.()}
		</div>
	{/if}
	<div class="relative" style:height>
		<div bind:this={host} class="absolute inset-0" class:opacity-0={!ready}></div>
		{#if !ready}<Textarea
				bind:value
				{disabled}
				aria-label={label ?? `${shell} script editor`}
				autocomplete="off"
				spellcheck={false}
				class="absolute inset-0 h-full min-h-0 resize-none border-0 p-3 font-mono text-xs shadow-none" />{/if}
	</div>
</div>
