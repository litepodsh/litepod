<script lang="ts">
	import { cn, type WithElementRef } from '#lib/utils.js';
	import type { HTMLAttributes } from 'svelte/elements';

	let {
		ref = $bindable(null),
		class: className,
		children,
		...restProps
	}: WithElementRef<HTMLAttributes<HTMLTableRowElement>> = $props();
</script>

<tr
	bind:this={ref}
	data-slot="table-row"
	class={cn(
		'hover:bg-muted data-[state=selected]:bg-muted border-b border-(--lp-emboss) shadow-[inset_0_-1px_0_var(--lp-bevel)] transition-colors',
		className,
	)}
	{...restProps}>
	{@render children?.()}
</tr>
