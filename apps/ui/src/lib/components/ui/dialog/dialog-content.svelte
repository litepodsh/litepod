<script lang="ts">
	import { Dialog as DialogPrimitive } from 'bits-ui';
	import DialogPortal from './dialog-portal.svelte';
	import type { Snippet } from 'svelte';
	import * as Dialog from './index.js';
	import { cn, type WithoutChildrenOrChild } from '#lib/utils.js';
	import type { ComponentProps } from 'svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import XIcon from '@lucide/svelte/icons/x';

	let {
		ref = $bindable(null),
		class: className,
		portalProps,
		children,
		showCloseButton = true,
		...restProps
	}: WithoutChildrenOrChild<DialogPrimitive.ContentProps> & {
		portalProps?: WithoutChildrenOrChild<ComponentProps<typeof DialogPortal>>;
		children: Snippet;
		showCloseButton?: boolean;
	} = $props();
</script>

<DialogPortal {...portalProps}>
	<Dialog.Overlay />
	<DialogPrimitive.Content
		bind:ref
		data-slot="dialog-content"
		class={cn(
			'bg-popover text-popover-foreground data-open:animate-in data-closed:animate-out data-closed:fade-out-0 data-open:fade-in-0 data-closed:zoom-out-95 data-open:zoom-in-95 ring-foreground/14 grid max-w-[calc(100%-2rem)] gap-4 rounded-[18px] p-(--dialog-pad) text-sm [--dialog-pad:--spacing(5)] [&.p-0]:[--dialog-pad:0px] shadow-[0_30px_80px_-20px_color-mix(in_oklch,var(--lp-shadow)_calc(22%*var(--lp-shadow-strength)),transparent)] ring-1 duration-200 sm:max-w-md max-h-[calc(100dvh-2rem)] overflow-y-auto fixed top-1/2 left-1/2 z-50 w-full -translate-x-1/2 -translate-y-1/2 outline-none',
			className,
		)}
		{...restProps}>
		{@render children?.()}
		{#if showCloseButton}
			<DialogPrimitive.Close data-slot="dialog-close">
				{#snippet child({ props })}
					<Button variant="ghost" class="absolute top-2.5 right-3" size="icon-sm" {...props}>
						<XIcon />
						<span class="sr-only">Close</span>
					</Button>
				{/snippet}
			</DialogPrimitive.Close>
		{/if}
	</DialogPrimitive.Content>
</DialogPortal>
