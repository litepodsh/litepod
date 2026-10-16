<script lang="ts">
	import { cn, type WithoutChildrenOrChild } from '#lib/utils.js';
	import DropdownMenuPortal from './dropdown-menu-portal.svelte';
	import { DropdownMenu as DropdownMenuPrimitive } from 'bits-ui';
	import type { ComponentProps } from 'svelte';

	let {
		ref = $bindable(null),
		sideOffset = 4,
		align = 'start',
		portalProps,
		class: className,
		...restProps
	}: DropdownMenuPrimitive.ContentProps & {
		portalProps?: WithoutChildrenOrChild<ComponentProps<typeof DropdownMenuPortal>>;
	} = $props();
</script>

<DropdownMenuPortal {...portalProps}>
	<DropdownMenuPrimitive.Content
		bind:ref
		data-slot="dropdown-menu-content"
		{sideOffset}
		{align}
		class={cn(
			'data-open:animate-in data-closed:animate-out data-closed:fade-out-0 data-open:fade-in-0 data-closed:zoom-out-95 data-open:zoom-in-95 data-[side=bottom]:slide-in-from-top-2 data-[side=left]:slide-in-from-right-2 data-[side=right]:slide-in-from-left-2 data-[side=top]:slide-in-from-bottom-2 bg-popover text-popover-foreground min-w-32 p-1.5 ring-foreground/12 rounded-[12px] ring-1 shadow-[0_20px_50px_-18px_color-mix(in_oklch,var(--lp-shadow)_calc(14%*var(--lp-shadow-strength)),transparent)] duration-100 data-[side=inline-start]:slide-in-from-right-2 data-[side=inline-end]:slide-in-from-left-2 z-50 w-(--bits-dropdown-menu-anchor-width) overflow-x-hidden overflow-y-auto outline-none data-closed:overflow-hidden',
			className,
		)}
		{...restProps} />
</DropdownMenuPortal>
