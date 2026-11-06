<script lang="ts">
	import OsIcon from '#lib/components/os-icon.svelte';
	import { cn } from '#lib/utils.js';

	let {
		server = null,
		detail = null,
		placeholder = '',
		class: className = '',
	}: {
		/** The picked server, or `null` to render the placeholder. */
		server?: {
			name: string;
			os?: string | null;
			distribution?: string | null;
		} | null;
		/** Secondary text after the name, e.g. the ingress mode. */
		detail?: string | null;
		/** Shown when no server is picked. */
		placeholder?: string;
		class?: string;
	} = $props();
</script>

<!-- Every server picker names its host the same way: the platform mark first, then
     the name, with optional secondary text such as the ingress mode. -->
<span class={cn('flex min-w-0 items-center gap-2', className)}>
	{#if server}
		<OsIcon os={server.os} distribution={server.distribution} class="size-4 shrink-0" />
		<span class="truncate">{server.name}</span>
		{#if detail}
			<span class="shrink-0 whitespace-nowrap text-muted-foreground">· {detail}</span>
		{/if}
	{:else}
		<span class="truncate text-muted-foreground">{placeholder}</span>
	{/if}
</span>
