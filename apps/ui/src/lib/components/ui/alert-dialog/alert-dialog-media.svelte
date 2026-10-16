<script lang="ts">
	import type { HTMLAttributes } from 'svelte/elements';
	import { cn, type WithElementRef } from '#lib/utils.js';

	let {
		ref = $bindable(null),
		class: className,
		children,
		...restProps
	}: WithElementRef<HTMLAttributes<HTMLDivElement>> = $props();
</script>

<div
	bind:this={ref}
	data-slot="alert-dialog-media"
	class={cn(
		"lp-raised bg-card row-span-2 inline-flex size-10 items-center justify-center rounded-full *:[svg:not([class*='size-'])]:size-5",
		className,
	)}
	{...restProps}>
	{@render children?.()}
</div>
