<script lang="ts">
	import { cn, type WithElementRef, type WithoutChildren } from '#lib/utils.js';
	import type { HTMLAttributes } from 'svelte/elements';

	let {
		ref = $bindable(null),
		class: className,
		...restProps
	}: WithoutChildren<WithElementRef<HTMLAttributes<HTMLDivElement>>> = $props();
</script>

<div
	bind:this={ref}
	data-slot="skeleton"
	class={cn(
		'bg-muted rounded-[10px] bg-[linear-gradient(90deg,transparent_0%,color-mix(in_srgb,var(--card)_55%,transparent)_50%,transparent_100%)] bg-size-[200%_100%] bg-no-repeat motion-safe:animate-shimmer',
		className,
	)}
	{...restProps}>
</div>
