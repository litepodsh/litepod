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
	 *  Below `lg` it becomes an underlined tab strip that scrolls sideways above
	 *  the content, with the active tab kept in view. */
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

	let nav = $state<HTMLElement>();

	$effect(() => {
		nav
			?.querySelector(`[data-value="${CSS.escape(current)}"]`)
			?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
	});
</script>

<nav
	bind:this={nav}
	aria-label={label}
	class="-mx-4 flex shrink-0 gap-1 overflow-x-auto border-b border-border px-4 [scrollbar-width:none] sm:-mx-6 sm:px-6 lg:sticky lg:top-[calc(var(--app-header-height)+1.5rem)] lg:mx-0 lg:w-52 lg:flex-col lg:overflow-visible lg:border-b-0 lg:px-0">
	{#each items as item (item.value)}
		{@const active = current === item.value}
		{#if item.separated}
			<span class="lp-groove-t my-2 hidden lg:block" aria-hidden="true"></span>
		{/if}
		<button
			type="button"
			data-value={item.value}
			aria-current={active ? 'page' : undefined}
			onclick={() => onselect(item.value)}
			class="-mb-px flex h-10 shrink-0 items-center gap-2 border-b-2 px-2.5 text-sm whitespace-nowrap transition-colors lg:mb-0 lg:h-8 lg:rounded-lg lg:border-b-0 {active
				? 'border-primary font-medium text-foreground lg:lp-raised lg:bg-card'
				: 'border-transparent text-muted-foreground hover:text-foreground lg:hover:bg-muted'}">
			<item.icon class="size-4 shrink-0 {active ? 'text-primary' : ''}" />
			<span class="truncate">{item.label}</span>
		</button>
	{/each}
</nav>
