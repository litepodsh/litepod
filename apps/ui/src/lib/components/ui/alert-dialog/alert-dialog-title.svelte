<script lang="ts">
	import { AlertDialog as AlertDialogPrimitive } from 'bits-ui';
	import { cn } from '#lib/utils.js';

	let {
		ref = $bindable(null),
		class: className,
		...restProps
	}: AlertDialogPrimitive.TitleProps = $props();
</script>

<AlertDialogPrimitive.Title
	bind:ref
	data-slot="alert-dialog-title"
	class={cn(
		'text-base leading-tight font-semibold tracking-[-0.015em] group-has-data-[slot=alert-dialog-media]/alert-dialog-content:col-start-2',
		className,
	)}
	{...restProps} />
