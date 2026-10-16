<script lang="ts">
	/**
	 * The mark for one model provider, by its `config::PROVIDERS` id.
	 *
	 * Every icon draws in `currentColor`, so one file covers both themes — the
	 * brand SVGs ship a fixed white that disappears on the light ground.
	 *
	 * The bring-your-own-endpoint kinds deliberately get braces rather than the
	 * mark of the API they imitate: an OpenAI-compatible gateway is not OpenAI,
	 * and a row that claims otherwise is the wrong thing to look at when a key
	 * stops working.
	 */
	import type { Component } from 'svelte';
	import BracesIcon from '@lucide/svelte/icons/braces';
	import SparklesIcon from '@lucide/svelte/icons/sparkles';
	import AnthropicIcon from '#lib/components/icons/anthropic-icon.svelte';
	import LmstudioIcon from '#lib/components/icons/lmstudio-icon.svelte';
	import OllamaIcon from '#lib/components/icons/ollama-icon.svelte';
	import OpenaiIcon from '#lib/components/icons/openai-icon.svelte';
	import OpencodeIcon from '#lib/components/icons/opencode-icon.svelte';
	import OpenrouterIcon from '#lib/components/icons/openrouter-icon.svelte';

	let { provider, class: className = 'size-4' }: { provider: string; class?: string } = $props();

	const ICONS: Record<string, Component<{ class?: string }>> = {
		opencode_zen: OpencodeIcon,
		opencode_go: OpencodeIcon,
		openrouter: OpenrouterIcon,
		openai: OpenaiIcon,
		anthropic: AnthropicIcon,
		ollama: OllamaIcon,
		lmstudio: LmstudioIcon,
		openai_compatible: BracesIcon,
		anthropic_compatible: BracesIcon,
	};

	let Icon = $derived(ICONS[provider] ?? SparklesIcon);
</script>

<Icon class={className} />
