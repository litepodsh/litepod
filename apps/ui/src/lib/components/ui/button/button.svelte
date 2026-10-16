<script lang="ts" module>
	import { cn, type WithElementRef } from '#lib/utils.js';
	import type { HTMLAnchorAttributes, HTMLButtonAttributes } from 'svelte/elements';
	import { type VariantProps, tv } from 'tailwind-variants';

	export const buttonVariants = tv({
		base: "focus-visible:border-ring focus-visible:ring-ring/30 aria-invalid:ring-destructive/20 dark:aria-invalid:ring-destructive/40 aria-invalid:border-destructive dark:aria-invalid:border-destructive/50 rounded-full border border-transparent bg-clip-padding text-sm font-medium focus-visible:ring-3 active:not-aria-[haspopup]:scale-[.98] aria-invalid:ring-3 [&_svg:not([class*='size-'])]:size-4 group/button inline-flex shrink-0 items-center justify-center whitespace-nowrap transition-all duration-300 ease-[var(--lp-ease)] outline-none select-none disabled:pointer-events-none disabled:opacity-50 [&_svg]:pointer-events-none [&_svg]:shrink-0",
		variants: {
			variant: {
				default:
					'bg-primary bg-[linear-gradient(180deg,color-mix(in_oklch,var(--primary)_88%,white),var(--primary))] text-primary-foreground shadow-[inset_1px_1px_0_rgb(255_255_255_/_0.22),inset_-1px_-1px_0_rgb(0_0_0_/_0.14),0_1px_2px_color-mix(in_oklch,var(--lp-shadow)_calc(14%*var(--lp-shadow-strength)),transparent)] active:shadow-[inset_0_1px_2px_rgb(0_0_0_/_0.18)] hover:bg-primary/90',
				outline:
					'lp-raised border-foreground/12 bg-card hover:bg-secondary hover:text-foreground aria-expanded:bg-secondary aria-expanded:text-foreground',
				secondary:
					'bg-foreground text-background shadow-[inset_1px_1px_0_rgb(255_255_255_/_0.14),inset_-1px_-1px_0_rgb(0_0_0_/_0.2),0_1px_2px_color-mix(in_oklch,var(--lp-shadow)_calc(14%*var(--lp-shadow-strength)),transparent)] active:shadow-[inset_0_1px_2px_rgb(0_0_0_/_0.2)] aria-expanded:bg-foreground aria-expanded:text-background hover:bg-foreground/90',
				ghost:
					'hover:bg-muted hover:text-foreground dark:hover:bg-muted/50 aria-expanded:bg-muted aria-expanded:text-foreground',
				destructive:
					'bg-destructive/10 hover:bg-destructive/20 focus-visible:ring-destructive/20 dark:focus-visible:ring-destructive/40 dark:bg-destructive/20 text-destructive focus-visible:border-destructive/40 dark:hover:bg-destructive/30',
				link: 'text-primary underline-offset-4 hover:underline',
			},
			size: {
				default:
					'h-8 gap-1.5 px-3 has-data-[icon=inline-end]:pr-2.5 has-data-[icon=inline-start]:pl-2.5',
				xs: "h-6 gap-1 px-2.5 text-xs has-data-[icon=inline-end]:pr-2 has-data-[icon=inline-start]:pl-2 [&_svg:not([class*='size-'])]:size-3",
				sm: 'h-7 gap-1 px-3 has-data-[icon=inline-end]:pr-2 has-data-[icon=inline-start]:pl-2',
				lg: 'h-9 gap-1.5 px-4 has-data-[icon=inline-end]:pr-3 has-data-[icon=inline-start]:pl-3',
				icon: 'size-8 rounded-[.875rem]',
				'icon-xs': "size-6 rounded-[.625rem] [&_svg:not([class*='size-'])]:size-3",
				'icon-sm': 'size-7 rounded-[.75rem]',
				'icon-lg': 'size-9 rounded-[1rem]',
			},
		},
		defaultVariants: {
			variant: 'default',
			size: 'default',
		},
	});

	export type ButtonVariant = VariantProps<typeof buttonVariants>['variant'];
	export type ButtonSize = VariantProps<typeof buttonVariants>['size'];

	export type ButtonProps = WithElementRef<HTMLButtonAttributes> &
		WithElementRef<HTMLAnchorAttributes> & {
			variant?: ButtonVariant;
			size?: ButtonSize;
		};
</script>

<script lang="ts">
	let {
		class: className,
		variant = 'default',
		size = 'default',
		ref = $bindable(null),
		href = undefined,
		type = 'button',
		disabled,
		children,
		...restProps
	}: ButtonProps = $props();
</script>

{#if href}
	<a
		bind:this={ref}
		data-slot="button"
		class={cn(buttonVariants({ variant, size }), className)}
		href={disabled ? undefined : href}
		aria-disabled={disabled}
		role={disabled ? 'link' : undefined}
		tabindex={disabled ? -1 : undefined}
		{...restProps}>
		{@render children?.()}
	</a>
{:else}
	<button
		bind:this={ref}
		data-slot="button"
		class={cn(buttonVariants({ variant, size }), className)}
		{type}
		{disabled}
		{...restProps}>
		{@render children?.()}
	</button>
{/if}
