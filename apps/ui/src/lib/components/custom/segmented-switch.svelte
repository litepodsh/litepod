<script lang="ts" generics="T extends string">
	/**
	 * A small set of mutually exclusive options where the choice changes what a
	 * whole region shows, rather than submitting a value. The selected pill is a
	 * single element that slides between the options, so the movement says the
	 * region was swapped, not replaced.
	 *
	 * Options share the track equally and sit flush, so the indicator is one
	 * width and one `translate` per index: no measuring, and only `transform`
	 * animates. A gap between options would break that arithmetic.
	 */
	import { cn } from '#lib/utils.js';

	type Item = {
		value: T;
		label: string;
		// this should be `Component` after @lucide/svelte updates types
		// eslint-disable-next-line @typescript-eslint/no-explicit-any
		icon?: any;
		/** Draws an attention dot while the option is not the selected one. */
		badge?: boolean;
	};

	let {
		items,
		value = $bindable(),
		orientation = 'horizontal',
		label,
		class: className,
		onchange,
	}: {
		items: Item[];
		value: T;
		orientation?: 'horizontal' | 'vertical';
		/** Names the group for screen readers, e.g. "Sidebar contents". */
		label: string;
		class?: string;
		onchange?: (value: T) => void;
	} = $props();

	const vertical = $derived(orientation === 'vertical');
	const index = $derived(
		Math.max(
			0,
			items.findIndex((item) => item.value === value),
		),
	);

	function select(next: T) {
		value = next;
		onchange?.(next);
	}
</script>

<div
	role="group"
	aria-label={label}
	class={cn(
		'bg-secondary lp-inset relative isolate p-0.5',
		// Equal fr columns size to the widest label when the track shrink-wraps, so
		// no label truncates beside a short one; minmax(0, …) still lets a narrow
		// container squeeze them.
		vertical
			? 'flex flex-col rounded-[1.125rem]'
			: 'grid auto-cols-[minmax(0,1fr)] grid-flow-col rounded-full',
		className,
	)}>
	<span
		aria-hidden="true"
		class={cn(
			'bg-sidebar lp-raised absolute -z-10 transition-transform duration-300 ease-[var(--lp-ease)]',
			vertical ? 'inset-x-0.5 top-0.5 rounded-[.875rem]' : 'inset-y-0.5 start-0.5 rounded-full',
		)}
		style={vertical
			? `height: calc((100% - 0.25rem) / ${items.length}); transform: translateY(${index * 100}%)`
			: `width: calc((100% - 0.25rem) / ${items.length}); transform: translateX(${index * 100}%)`}
	></span>

	{#each items as item (item.value)}
		{@const selected = item.value === value}
		<button
			type="button"
			aria-pressed={selected}
			title={item.label}
			class={cn(
				'relative flex min-w-0 items-center justify-center gap-1.5 text-xs font-medium transition-[color,transform] duration-300 ease-[var(--lp-ease)] active:scale-[.98]',
				vertical ? 'h-8 rounded-[.875rem]' : 'h-7 rounded-full px-2.5',
				selected
					? 'text-sidebar-foreground'
					: 'text-muted-foreground hover:text-sidebar-foreground',
			)}
			onclick={() => select(item.value)}>
			{#if item.icon}
				<item.icon class="size-3.5 shrink-0" />
			{/if}
			{#if !vertical}
				<span class="truncate">{item.label}</span>
			{/if}
			{#if item.badge && !selected}
				<span class="bg-primary size-1.5 shrink-0 rounded-full" aria-label="New activity"></span>
			{/if}
		</button>
	{/each}
</div>
