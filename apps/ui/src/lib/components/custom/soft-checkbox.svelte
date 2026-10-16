<script lang="ts">
	import { Checkbox as CheckboxPrimitive } from 'bits-ui';
	import CheckIcon from '@lucide/svelte/icons/check';
	import MinusIcon from '@lucide/svelte/icons/minus';
	import { cn, type WithoutChildrenOrChild } from '#lib/utils.js';

	let {
		ref = $bindable(null),
		checked = $bindable(false),
		indeterminate = $bindable(false),
		class: className,
		...restProps
	}: WithoutChildrenOrChild<CheckboxPrimitive.RootProps> = $props();

	let checkboxSurface = $state<HTMLSpanElement | null>(null);
	let previousState: 'checked' | 'indeterminate' | 'unchecked' | undefined;

	$effect(() => {
		const state = indeterminate ? 'indeterminate' : checked === true ? 'checked' : 'unchecked';
		if (!checkboxSurface || previousState === undefined) {
			previousState = state;
			return;
		}
		if (previousState === state) return;
		previousState = state;
		if (matchMedia('(prefers-reduced-motion: reduce)').matches) return;

		checkboxSurface.getAnimations().forEach((animation) => animation.cancel());
		checkboxSurface.animate(
			[
				{ transform: 'scale(1)' },
				{ transform: 'scale(1.12)', offset: 0.45 },
				{ transform: 'scale(1)' },
			],
			{
				duration: 280,
				easing: getComputedStyle(checkboxSurface).getPropertyValue('--lp-ease').trim(),
			},
		);
	});
</script>

<CheckboxPrimitive.Root
	bind:ref
	bind:checked
	bind:indeterminate
	data-slot="soft-checkbox"
	class={cn(
		'peer relative flex size-4 shrink-0 items-center justify-center rounded-[5px] outline-none after:absolute after:-inset-x-3 after:-inset-y-2 focus-visible:ring-3 focus-visible:ring-ring/30 aria-invalid:ring-3 aria-invalid:ring-destructive/20 dark:aria-invalid:ring-destructive/40 group-has-disabled/field:opacity-50 disabled:cursor-not-allowed disabled:opacity-50',
		className,
	)}
	{...restProps}>
	{#snippet children({ checked, indeterminate })}
		<span bind:this={checkboxSurface} data-slot="soft-checkbox-surface"></span>
		<div data-slot="soft-checkbox-indicator">
			{#if checked}
				<CheckIcon strokeWidth={3} />
			{:else if indeterminate}
				<MinusIcon strokeWidth={3} />
			{/if}
		</div>
	{/snippet}
</CheckboxPrimitive.Root>

<style>
	:global([data-slot='soft-checkbox-surface']) {
		position: absolute;
		inset: 0;
		overflow: hidden;
		border-radius: inherit;
		background-color: var(--lp-deep);
		box-shadow:
			inset 1px 1px 2px color-mix(in srgb, var(--lp-emboss) 80%, transparent),
			inset -1px -1px 2px color-mix(in srgb, var(--lp-bevel) 80%, transparent);
		transition: box-shadow 280ms var(--lp-ease);
	}

	:global([data-slot='soft-checkbox-surface']::before) {
		position: absolute;
		inset: 0;
		background: linear-gradient(
			90deg,
			color-mix(in oklch, var(--primary) 82%, white),
			var(--primary)
		);
		content: '';
		transform: scaleX(0);
		transform-origin: right;
		transition: transform 280ms var(--lp-ease);
	}

	:global([data-slot='soft-checkbox'][data-state='checked'] [data-slot='soft-checkbox-surface']),
	:global(
		[data-slot='soft-checkbox'][data-state='indeterminate'] [data-slot='soft-checkbox-surface']
	) {
		box-shadow:
			inset 1px 1px 2px rgb(0 0 0 / 0.128),
			inset -1px -1px 2px rgb(255 255 255 / 0.16);
	}

	:global(
		[data-slot='soft-checkbox'][data-state='checked'] [data-slot='soft-checkbox-surface']::before
	),
	:global(
		[data-slot='soft-checkbox'][data-state='indeterminate']
			[data-slot='soft-checkbox-surface']::before
	) {
		transform: scaleX(1);
		transform-origin: left;
	}

	@media (prefers-reduced-motion: reduce) {
		:global([data-slot='soft-checkbox-surface']::before) {
			transform: none;
			opacity: 0;
			transition: opacity 160ms var(--lp-ease);
		}

		:global(
			[data-slot='soft-checkbox'][data-state='checked'] [data-slot='soft-checkbox-surface']::before
		),
		:global(
			[data-slot='soft-checkbox'][data-state='indeterminate']
				[data-slot='soft-checkbox-surface']::before
		) {
			opacity: 1;
		}
	}

	:global([data-slot='soft-checkbox-indicator']) {
		position: relative;
		z-index: 1;
		display: grid;
		place-items: center;
		color: var(--primary-foreground);
	}

	:global([data-slot='soft-checkbox-indicator'] > svg) {
		width: 0.8rem;
		height: 0.8rem;
	}
</style>
