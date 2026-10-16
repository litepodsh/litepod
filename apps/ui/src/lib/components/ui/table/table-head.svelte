<script lang="ts">
	import { cn, type WithElementRef } from '#lib/utils.js';
	import type { HTMLThAttributes } from 'svelte/elements';

	let {
		ref = $bindable(null),
		class: className,
		children,
		...restProps
	}: WithElementRef<HTMLThAttributes> = $props();
</script>

<th
	bind:this={ref}
	data-slot="table-head"
	class={cn(
		'text-muted-foreground h-9 px-3 text-left align-middle font-mono text-[11.5px] font-medium tracking-[0.01em] whitespace-nowrap [&:has([role=checkbox])]:pr-0',
		className,
	)}
	{...restProps}>
	{@render children?.()}
</th>
