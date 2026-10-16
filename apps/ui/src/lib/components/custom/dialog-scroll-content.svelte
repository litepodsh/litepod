<script lang="ts">
	import { Dialog as DialogPrimitive } from 'bits-ui';
	import DialogPortal from '#lib/components/ui/dialog/dialog-portal.svelte';
	import type { Snippet } from 'svelte';
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import { cn, type WithoutChildrenOrChild } from '#lib/utils.js';
	import type { ComponentProps } from 'svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import { t } from '#lib/i18n/index.svelte.js';
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

<!--
	Same shape as `ui/dialog/dialog-content.svelte`, `flex flex-col
	overflow-hidden` instead of `grid` so a `shrink-0` header/footer and an
	`overflow-y-auto` body in between stay put while only the body scrolls -
	use this instead of `Dialog.Content` whenever the dialog has a footer and
	body content that can grow past the viewport (see
	`application-form-dialog.svelte` for the header/body/footer split this
	expects from its children).
-->
<DialogPortal {...portalProps}>
	<Dialog.Overlay />
	<DialogPrimitive.Content
		bind:ref
		data-slot="dialog-content"
		class={cn(
			'bg-popover text-popover-foreground data-open:animate-in data-closed:animate-out data-closed:fade-out-0 data-open:fade-in-0 data-closed:zoom-out-95 data-open:zoom-in-95 ring-foreground/10 flex max-h-[90vh] flex-col gap-6 overflow-hidden rounded-[min(var(--radius-4xl),28px)] p-6 text-sm shadow-[0_1.5rem_4rem_color-mix(in_oklch,var(--lp-shadow)_calc(18%*var(--lp-shadow-strength)),transparent)] ring-1 duration-200 sm:max-w-md fixed top-1/2 left-1/2 z-50 w-full -translate-x-1/2 -translate-y-1/2 outline-none',
			className,
		)}
		{...restProps}>
		{@render children?.()}
		{#if showCloseButton}
			<DialogPrimitive.Close data-slot="dialog-close">
				{#snippet child({ props })}
					<Button
						variant="ghost"
						class="bg-secondary absolute top-4 right-4"
						size="icon-sm"
						{...props}>
						<XIcon />
						<span class="sr-only">{t('common.common.close')}</span>
					</Button>
				{/snippet}
			</DialogPrimitive.Close>
		{/if}
	</DialogPrimitive.Content>
</DialogPortal>
