<script lang="ts">
	import type { HTMLAttributes } from 'svelte/elements';
	import { cn, type WithElementRef } from '#lib/utils.js';

	let {
		ref = $bindable(null),
		class: className,
		children,
		size = 'default',
		...restProps
	}: WithElementRef<HTMLAttributes<HTMLDivElement>> & { size?: 'default' | 'sm' } = $props();
</script>

<div
	bind:this={ref}
	data-slot="card"
	data-size={size}
	class={cn(
		'bg-card text-card-foreground border-foreground/9 gap-(--card-spacing) overflow-hidden rounded-[15px] border py-(--card-spacing) text-sm lp-panel [--card-spacing:--spacing(4)] has-[>img:first-child]:pt-0 data-[size=sm]:[--card-spacing:--spacing(3)] *:[img:first-child]:rounded-t-[15px] *:[img:last-child]:rounded-b-[15px] group/card flex flex-col',
		className,
	)}
	{...restProps}>
	{@render children?.()}
</div>
