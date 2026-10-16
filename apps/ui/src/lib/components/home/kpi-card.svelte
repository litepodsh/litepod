<script lang="ts">
	import type { Component, Snippet } from 'svelte';
	import { toneDot, type StatusTone } from '#lib/utils/status-tone.js';

	/** A headline number with an optional row of ticks (one per item) and a footer. */
	let {
		title,
		sub,
		value,
		hint,
		href,
		icon: Icon,
		ticks = [],
		children,
		class: className = '',
	}: {
		title: string;
		/** Small line under the title. */
		sub?: string;
		value: string;
		hint?: string;
		href?: string;
		icon?: Component;
		ticks?: StatusTone[];
		children?: Snippet;
		class?: string;
	} = $props();
</script>

<svelte:element
	this={href ? 'a' : 'div'}
	{href}
	class="lp-raised flex min-w-0 flex-col gap-3.5 rounded-2xl border bg-card p-4 transition-colors {href
		? 'hover:border-foreground/20'
		: ''} {className}">
	<span class="flex items-center gap-2.5">
		{#if Icon}
			<span
				class="lp-inset grid size-[30px] shrink-0 place-items-center rounded-lg bg-muted text-muted-foreground">
				<Icon class="size-4" />
			</span>
		{/if}
		<span class="min-w-0">
			<span class="block truncate text-[13px] font-medium">{title}</span>
			{#if sub}<small class="block truncate text-xs text-muted-foreground">{sub}</small>{/if}
		</span>
	</span>
	<span class="flex min-w-0 items-baseline gap-2">
		<b class="text-2xl font-semibold tracking-tight tabular-nums">{value}</b>
		{#if hint}<small class="truncate text-xs text-muted-foreground">{hint}</small>{/if}
	</span>
	{#if ticks.length}
		<span class="flex h-[22px] gap-1 overflow-hidden" data-testid="kpi-ticks" aria-hidden="true">
			{#each ticks as tone, index (index)}
				<i class="h-full w-1 shrink-0 rounded-[2px] {toneDot(tone)}"></i>
			{/each}
		</span>
	{/if}
	{#if children}<div class="mt-auto min-w-0">{@render children()}</div>{/if}
</svelte:element>
