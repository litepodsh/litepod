<script lang="ts">
	import { AlertDialog as AlertDialogPrimitive } from 'bits-ui';
	import AlertDialogPortal from './alert-dialog-portal.svelte';
	import AlertDialogOverlay from './alert-dialog-overlay.svelte';
	import { cn, type WithoutChild, type WithoutChildrenOrChild } from '#lib/utils.js';
	import type { ComponentProps } from 'svelte';

	let {
		ref = $bindable(null),
		class: className,
		size = 'default',
		portalProps,
		...restProps
	}: WithoutChild<AlertDialogPrimitive.ContentProps> & {
		size?: 'default' | 'sm';
		portalProps?: WithoutChildrenOrChild<ComponentProps<typeof AlertDialogPortal>>;
	} = $props();
</script>

<AlertDialogPortal {...portalProps}>
	<AlertDialogOverlay />
	<AlertDialogPrimitive.Content
		bind:ref
		data-slot="alert-dialog-content"
		data-size={size}
		class={cn(
			'data-open:animate-in data-closed:animate-out data-closed:fade-out-0 data-open:fade-in-0 data-closed:zoom-out-95 data-open:zoom-in-95 bg-popover text-popover-foreground ring-foreground/14 gap-4 rounded-[18px] p-(--dialog-pad) text-sm [--dialog-pad:--spacing(5)] shadow-[0_30px_80px_-20px_color-mix(in_oklch,var(--lp-shadow)_calc(22%*var(--lp-shadow-strength)),transparent)] ring-1 duration-200 max-w-[calc(100%-2rem)] data-[size=sm]:sm:max-w-xs data-[size=default]:sm:max-w-md group/alert-dialog-content max-h-[calc(100dvh-2rem)] overflow-y-auto fixed top-1/2 left-1/2 z-50 grid w-full -translate-x-1/2 -translate-y-1/2 outline-none',
			className,
		)}
		{...restProps} />
</AlertDialogPortal>
