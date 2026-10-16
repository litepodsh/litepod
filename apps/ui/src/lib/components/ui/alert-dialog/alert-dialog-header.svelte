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
	data-slot="alert-dialog-header"
	class={cn(
		'-mx-(--dialog-pad) -mt-(--dialog-pad) lp-groove-b [&:has(+[data-slot=alert-dialog-footer])]:border-b-0 [&:has(+[data-slot=alert-dialog-footer])]:shadow-none grid content-start gap-1 px-5 pt-4 pb-3.5 text-left has-data-[slot=alert-dialog-media]:grid-cols-[auto_minmax(0,1fr)] has-data-[slot=alert-dialog-media]:gap-x-3.5',
		className,
	)}
	{...restProps}>
	{@render children?.()}
</div>
