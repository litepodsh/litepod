<script lang="ts">
	import { Command as CommandPrimitive } from 'bits-ui';
	import { cn } from '#lib/utils.js';

	let {
		ref = $bindable(null),
		class: className,
		...restProps
	}: CommandPrimitive.SeparatorProps = $props();
</script>

<CommandPrimitive.Separator
	bind:ref
	data-slot="command-separator"
	class={cn('bg-(--lp-emboss) shadow-[0_1px_0_var(--lp-bevel)] -mx-1.5 my-1.5 h-px', className)}
	{...restProps} />
