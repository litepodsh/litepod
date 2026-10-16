<script lang="ts" module>
	import type { Component } from 'svelte';

	export type SectionNavItem = {
		value: string;
		label: string;
		icon: Component;
		/** Draws a divider above this item to start a new group. */
		separated?: boolean;
	};
</script>

<script lang="ts">
	/** The vertical section list of a settings-style page (org settings, System).
	 *  Scrolls sideways above the content on small screens. */
	let {
		items,
		current,
		label,
		onselect,
	}: {
		items: SectionNavItem[];
		current: string;
		label: string;
		onselect: (value: string) => void;
	} = $props();
</script>

<nav
	aria-label={label}
	class="-mx-4 flex shrink-0 gap-1 overflow-x-auto px-4 pb-1 lg:sticky lg:top-[calc(var(--app-header-height)+1.5rem)] lg:mx-0 lg:w-52 lg:flex-col lg:overflow-visible lg:px-0 lg:pb-0">
	{#each items as item (item.value)}
		{@const active = current === item.value}
		{#if item.separated}
			<span class="lp-groove-t my-2 hidden lg:block" aria-hidden="true"></span>
		{/if}
		<button
			type="button"
			aria-current={active ? 'page' : undefined}
			onclick={() => onselect(item.value)}
			class="flex h-8 shrink-0 items-center gap-2 rounded-lg px-2.5 text-sm whitespace-nowrap transition-colors {active
				? 'lp-raised bg-card font-medium text-foreground'
				: 'text-muted-foreground hover:bg-muted hover:text-foreground'}">
			<item.icon class="size-4 shrink-0 {active ? 'text-primary' : ''}" />
			<span class="truncate">{item.label}</span>
		</button>
	{/each}
</nav>
