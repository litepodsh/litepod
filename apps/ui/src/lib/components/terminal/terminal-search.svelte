<script lang="ts">
	import { tick } from 'svelte';
	import type { SearchAddon } from '@xterm/addon-search';
	import type { Terminal } from '@xterm/xterm';
	import ChevronDownIcon from '@lucide/svelte/icons/chevron-down';
	import ChevronUpIcon from '@lucide/svelte/icons/chevron-up';
	import XIcon from '@lucide/svelte/icons/x';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let { addon, terminal }: { addon: SearchAddon | null; terminal: Terminal | null } = $props();
	let visible = $state(false);
	let query = $state('');
	let input = $state<HTMLInputElement | null>(null);
	let resultIndex = $state(-1);
	let resultCount = $state(0);

	const options = {
		decorations: {
			matchBackground: '#854d0e',
			matchOverviewRuler: '#f59e0b',
			activeMatchBackground: '#15803d',
			activeMatchColorOverviewRuler: '#22c55e',
		},
	};

	$effect(() => {
		if (!addon) return;
		// onDidChangeResults only fires when findNext/findPrevious are called with
		// the `decorations` option, which is always the case below.
		const disposable = addon.onDidChangeResults((result) => {
			resultIndex = result.resultIndex;
			resultCount = result.resultCount;
		});
		return () => disposable.dispose();
	});

	export async function show() {
		visible = true;
		await tick();
		input?.focus();
		input?.select();
	}

	function resetResults() {
		addon?.clearDecorations();
		resultIndex = -1;
		resultCount = 0;
	}

	// `incremental` keeps the viewport on the current match while the query is
	// still being typed; an explicit next/prev advances.
	function findNext(incremental = false) {
		if (!addon || !query) {
			resetResults();
			return;
		}
		addon.findNext(query, { ...options, incremental });
	}

	function findPrevious() {
		if (!addon || !query) {
			resetResults();
			return;
		}
		addon.findPrevious(query, options);
	}

	function close() {
		visible = false;
		addon?.clearDecorations();
		terminal?.focus();
	}

	function handleKeydown(event: KeyboardEvent) {
		event.stopPropagation();
		if (event.key === 'Escape') {
			event.preventDefault();
			close();
		} else if (event.key === 'Enter') {
			event.preventDefault();
			if (event.shiftKey) findPrevious();
			else findNext();
		}
	}
</script>

{#if visible}
	<div
		class="absolute top-2 right-2 z-30 flex items-center gap-1 rounded-xl border bg-background/95 p-1 shadow-lg backdrop-blur">
		<Input
			bind:ref={input}
			bind:value={query}
			class="h-8 w-52 rounded-lg font-mono text-xs"
			placeholder={`${t('common.terminal.search')}…`}
			aria-label={t('common.terminal.search')}
			oninput={() => findNext(true)}
			onkeydown={handleKeydown} />
		<span class="min-w-12 text-center text-xs tabular-nums text-muted-foreground">
			{#if resultCount > 0 && resultIndex >= 0}
				{resultIndex + 1}/{resultCount}
			{:else if resultCount < 0 && resultIndex >= 0}
				•
			{:else}
				0/0
			{/if}
		</span>
		<Button
			size="icon-sm"
			variant="ghost"
			aria-label={t('common.terminal.previous_match')}
			onclick={findPrevious}>
			<ChevronUpIcon />
		</Button>
		<Button
			size="icon-sm"
			variant="ghost"
			aria-label={t('common.terminal.next_match')}
			onclick={() => findNext()}>
			<ChevronDownIcon />
		</Button>
		<Button
			size="icon-sm"
			variant="ghost"
			aria-label={t('common.terminal.close_search')}
			onclick={close}>
			<XIcon />
		</Button>
	</div>
{/if}
