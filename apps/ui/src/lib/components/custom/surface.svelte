<script lang="ts">
	import type { Snippet } from 'svelte';
	import type { HTMLAttributes } from 'svelte/elements';
	import { cn, type WithElementRef } from '#lib/utils.js';

	let {
		ref = $bindable(null),
		class: className,
		children,
		...restProps
	}: WithElementRef<HTMLAttributes<HTMLDivElement>> & { children?: Snippet } = $props();
</script>

<div
	bind:this={ref}
	data-slot="surface-outer"
	class={cn(
		'lp-inset rounded-[1.8125rem] border border-foreground/10 bg-[var(--lp-deep)] p-2.5',
		className,
	)}
	{...restProps}>
	<div data-slot="surface-inner" class="lp-raised rounded-[1.25rem] bg-card p-7">
		{@render children?.()}
	</div>
</div>
