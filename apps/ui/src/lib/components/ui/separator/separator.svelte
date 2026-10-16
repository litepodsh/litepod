<script lang="ts">
	import { Separator as SeparatorPrimitive } from 'bits-ui';
	import { cn } from '#lib/utils.js';

	let {
		ref = $bindable(null),
		class: className,
		'data-slot': dataSlot = 'separator',
		...restProps
	}: SeparatorPrimitive.RootProps = $props();
</script>

<SeparatorPrimitive.Root
	bind:ref
	data-slot={dataSlot}
	class={cn(
		'shrink-0 bg-(--lp-emboss) data-[orientation=horizontal]:h-px data-[orientation=horizontal]:w-full data-[orientation=horizontal]:shadow-[0_1px_0_var(--lp-bevel)] data-[orientation=vertical]:w-px data-[orientation=vertical]:shadow-[1px_0_0_var(--lp-bevel)]',
		// this is different in shadcn/ui but self-stretch breaks things for us
		'data-[orientation=vertical]:h-full',
		className,
	)}
	{...restProps} />
