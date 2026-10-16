<script lang="ts">
	import { Switch as SwitchPrimitive } from 'bits-ui';
	import { cn, type WithoutChildrenOrChild } from '#lib/utils.js';

	let {
		ref = $bindable(null),
		class: className,
		checked = $bindable(false),
		size = 'default',
		...restProps
	}: WithoutChildrenOrChild<SwitchPrimitive.RootProps> & {
		size?: 'sm' | 'default';
	} = $props();

	let thumbSurface = $state<HTMLSpanElement | null>(null);
	let previousChecked: boolean | undefined;

	$effect(() => {
		const isChecked = checked === true;
		if (!thumbSurface || previousChecked === undefined) {
			previousChecked = isChecked;
			return;
		}
		if (previousChecked === isChecked) return;
		previousChecked = isChecked;
		if (matchMedia('(prefers-reduced-motion: reduce)').matches) return;

		thumbSurface.getAnimations().forEach((animation) => animation.cancel());
		// Squash and stretch: the thumb flattens as it travels, overshoots, settles.
		thumbSurface.animate(
			[
				{ transform: 'scale(1)' },
				{ transform: 'scale(1.18, 0.9)', offset: 0.4 },
				{ transform: 'scale(0.96, 1.04)', offset: 0.75 },
				{ transform: 'scale(1)' },
			],
			{
				duration: 450,
				easing: getComputedStyle(thumbSurface).getPropertyValue('--lp-ease').trim(),
			},
		);
	});
</script>

<SwitchPrimitive.Root
	bind:ref
	bind:checked
	data-slot="soft-switch"
	data-size={size}
	class={cn(
		'peer relative inline-flex shrink-0 outline-none focus-visible:ring-3 focus-visible:ring-ring/30 aria-invalid:ring-3 aria-invalid:ring-destructive/20 dark:aria-invalid:ring-destructive/40 after:absolute after:-inset-x-3 after:-inset-y-2 data-disabled:cursor-not-allowed data-disabled:opacity-50',
		className,
	)}
	{...restProps}>
	<SwitchPrimitive.Thumb
		data-slot="soft-switch-thumb"
		class="pointer-events-none absolute rounded-full">
		<span bind:this={thumbSurface} data-slot="soft-switch-thumb-surface"></span>
	</SwitchPrimitive.Thumb>
</SwitchPrimitive.Root>

<style>
	/* A recessed well; the fill grows from the thumb's side and a glossy thumb
	   rides on top. Colors come from tokens so all four themes work. */
	:global([data-slot='soft-switch']) {
		--soft-track-width: 2.875rem;
		--soft-track-height: 1.625rem;
		--soft-thumb-size: 1.25rem;
		--soft-inset: 0.1875rem;
		--soft-travel: calc(var(--soft-track-width) - var(--soft-thumb-size) - (var(--soft-inset) * 2));
		isolation: isolate;
		width: var(--soft-track-width);
		height: var(--soft-track-height);
		border-radius: 999px;
		background: linear-gradient(
			180deg,
			color-mix(in oklch, var(--lp-deep) 88%, black),
			var(--lp-deep) 70%
		);
		box-shadow:
			inset 0 2px 3px rgb(0 0 0 / 0.2),
			inset 0 1px 1px rgb(0 0 0 / 0.16),
			inset 0 -1px 1px var(--lp-bevel),
			0 1px 0 var(--lp-bevel),
			0 0 0 1px color-mix(in oklch, var(--lp-ink) 8%, transparent);
		transition: box-shadow 400ms var(--lp-ease);
	}

	:global([data-slot='soft-switch']::before) {
		position: absolute;
		inset: 0;
		z-index: 0;
		border-radius: inherit;
		background: radial-gradient(
			120% 140% at 30% 0%,
			color-mix(in oklch, var(--primary) 72%, white),
			var(--primary) 55%,
			color-mix(in oklch, var(--primary) 82%, black)
		);
		box-shadow:
			inset 0 2px 3px rgb(0 0 0 / 0.25),
			inset 0 -1px 1px rgb(255 255 255 / 0.3);
		clip-path: inset(0 calc(100% - var(--soft-track-height)) 0 0 round 999px);
		content: '';
		opacity: 0;
		pointer-events: none;
		transition:
			clip-path 450ms var(--lp-ease),
			opacity 150ms linear 320ms;
	}

	:global([data-slot='soft-switch'][data-size='sm']) {
		--soft-track-width: 2.125rem;
		--soft-track-height: 1.25rem;
		--soft-thumb-size: 0.875rem;
	}

	:global([data-slot='soft-switch'][data-state='checked']),
	:global(.dark [data-slot='soft-switch'][data-state='checked']) {
		box-shadow:
			inset 0 2px 3px rgb(0 0 0 / 0.25),
			inset 0 -1px 1px rgb(255 255 255 / 0.3),
			0 1px 0 var(--lp-bevel),
			0 0 0 1px color-mix(in oklch, var(--primary) 45%, transparent),
			0 0 14px -4px color-mix(in oklch, var(--primary) 70%, transparent);
	}

	:global([data-slot='soft-switch'][data-state='checked']::before) {
		clip-path: inset(0 round 999px);
		opacity: 1;
		transition:
			clip-path 450ms var(--lp-ease),
			opacity 300ms var(--lp-ease);
	}

	:global([data-slot='soft-switch-thumb']) {
		z-index: 1;
		top: var(--soft-inset);
		left: var(--soft-inset);
		width: var(--soft-thumb-size);
		height: var(--soft-thumb-size);
		transition: transform 450ms var(--lp-ease);
	}

	:global([data-slot='soft-switch-thumb-surface']) {
		display: block;
		width: 100%;
		height: 100%;
		border-radius: inherit;
		background:
			radial-gradient(60% 55% at 35% 28%, rgb(255 255 255 / 0.95), rgb(255 255 255 / 0) 70%),
			linear-gradient(180deg, #fdfdfe, #e9ebf0 70%, #dfe2e8);
		box-shadow:
			inset 0 -2px 3px rgb(16 22 40 / 0.14),
			inset 0 1px 1px #fff,
			0 0 0 0.5px rgb(16 22 40 / 0.12),
			0 2px 3px rgb(16 22 40 / 0.22),
			0 6px 10px -3px rgb(16 22 40 / 0.28);
		will-change: transform;
	}

	:global(.dark [data-slot='soft-switch']) {
		background: linear-gradient(180deg, #05060a, color-mix(in oklch, var(--lp-deep) 60%, black));
		box-shadow:
			inset 0 2px 4px rgb(0 0 0 / 0.8),
			inset 0 -1px 1px rgb(255 255 255 / 0.06),
			0 1px 0 rgb(255 255 255 / 0.05),
			0 0 0 1px rgb(0 0 0 / 0.6);
	}

	:global(.dark [data-slot='soft-switch-thumb-surface']) {
		background:
			radial-gradient(60% 55% at 35% 28%, rgb(255 255 255 / 0.7), rgb(255 255 255 / 0) 70%),
			linear-gradient(180deg, #e6e9ef, #b9bec8);
		box-shadow:
			inset 0 -2px 3px rgb(0 0 0 / 0.25),
			inset 0 1px 1px rgb(255 255 255 / 0.8),
			0 2px 3px rgb(0 0 0 / 0.5),
			0 6px 12px -3px rgb(0 0 0 / 0.6);
	}

	:global([data-slot='soft-switch-thumb'][data-state='checked']) {
		transform: translateX(var(--soft-travel));
	}

	:global([dir='rtl'] [data-slot='soft-switch-thumb'][data-state='checked']) {
		transform: translateX(calc(var(--soft-travel) * -1));
	}

	@media (prefers-reduced-motion: reduce) {
		:global([data-slot='soft-switch']),
		:global([data-slot='soft-switch']::before),
		:global([data-slot='soft-switch-thumb']) {
			transition: none;
		}
	}
</style>
