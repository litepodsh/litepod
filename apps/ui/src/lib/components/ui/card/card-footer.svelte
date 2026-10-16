<script lang="ts">
	import { cn, type WithElementRef } from '#lib/utils.js';
	import type { HTMLAttributes } from 'svelte/elements';

	let {
		ref = $bindable(null),
		class: className,
		children,
		...restProps
	}: WithElementRef<HTMLAttributes<HTMLDivElement>> = $props();
</script>

<div
	bind:this={ref}
	data-slot="card-footer"
	class={cn(
		'px-(--card-spacing) not-first:lp-groove-t not-first:pt-3.5 [.border-t]:lp-groove-t flex items-center',
		className,
	)}
	{...restProps}>
	{@render children?.()}
</div>
