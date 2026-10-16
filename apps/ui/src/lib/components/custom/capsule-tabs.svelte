<script lang="ts" generics="T extends string">
	/**
	 * In-page tabs drawn like the shell's main capsule (scope-tabs): pill track,
	 * raised indicator that springs to the selected option, options sized to
	 * their label. Scrolls sideways on narrow screens instead of wrapping.
	 */
	import { cn } from '#lib/utils.js';

	let {
		items,
		value = $bindable(),
		label,
		class: className,
		onchange,
	}: {
		items: { value: T; label: string }[];
		value: T;
		/** Names the group for screen readers. */
		label: string;
		class?: string;
		onchange?: (value: T) => void;
	} = $props();

	let capsule = $state<HTMLElement | null>(null);
	let indicator = $state({ x: 0, w: 0, ready: false });

	function measure() {
		const on = capsule?.querySelector<HTMLElement>('[aria-selected="true"]');
		indicator.x = on?.offsetLeft ?? 0;
		indicator.w = on?.offsetWidth ?? 0;
		on?.scrollIntoView?.({ block: 'nearest', inline: 'nearest' });
	}

	$effect(() => {
		void value;
		void items.length;
		if (!capsule) return;
		measure();
		// The first placement should not slide in from the left edge.
		requestAnimationFrame(() => (indicator.ready = true));
		if (typeof ResizeObserver === 'undefined') return;
		const observer = new ResizeObserver(measure);
		observer.observe(capsule);
		return () => observer.disconnect();
	});

	function pick(next: T) {
		value = next;
		onchange?.(next);
	}
</script>

<div bind:this={capsule} role="tablist" aria-label={label} class={cn('capsule lp-well', className)}>
	<span
		class="ind"
		class:ready={indicator.ready}
		aria-hidden="true"
		style:width="{indicator.w}px"
		style:transform="translateX({indicator.x - 4}px)"
		style:opacity={indicator.w ? 1 : 0}></span>
	{#each items as item (item.value)}
		<button
			type="button"
			role="tab"
			class="ttab"
			aria-selected={item.value === value}
			onclick={() => pick(item.value)}>
			{item.label}
		</button>
	{/each}
</div>

<style>
	.capsule {
		position: relative;
		display: inline-flex;
		gap: 2px;
		max-width: 100%;
		padding: 4px;
		overflow-x: auto;
		scrollbar-width: none;
		border: 1px solid color-mix(in srgb, var(--foreground) 9%, transparent);
		border-radius: 999px;
		background: color-mix(in srgb, var(--lp-deep) 70%, transparent);
	}
	.ind {
		position: absolute;
		top: 4px;
		bottom: 4px;
		left: 4px;
		border-radius: 999px;
		background: var(--lp-raised);
		box-shadow:
			inset 1px 1px 0 var(--lp-bevel),
			inset -1px -1px 0 var(--lp-emboss),
			0 4px 12px -4px
				color-mix(in srgb, var(--lp-shadow) calc(18% * var(--lp-shadow-strength)), transparent);
		pointer-events: none;
	}
	/* Spring: overshoots a touch, then settles on the tab. */
	.ind.ready {
		transition:
			transform 0.45s cubic-bezier(0.3, 1.25, 0.4, 1),
			width 0.45s cubic-bezier(0.3, 1.25, 0.4, 1),
			opacity 0.2s;
	}
	.ttab {
		position: relative;
		z-index: 1;
		display: inline-flex;
		align-items: center;
		height: 32px;
		padding: 0 14px;
		border-radius: 999px;
		color: var(--muted-foreground);
		font-size: 13px;
		font-weight: 500;
		white-space: nowrap;
		outline: none;
		transition:
			color 0.2s var(--lp-ease),
			transform 0.15s var(--lp-ease);
	}
	.ttab:hover,
	.ttab[aria-selected='true'] {
		color: var(--foreground);
	}
	.ttab:focus-visible {
		box-shadow: 0 0 0 3px color-mix(in srgb, var(--ring) 30%, transparent);
	}
	.ttab:active {
		transform: scale(0.96);
	}
	@media (prefers-reduced-motion: reduce) {
		.ind.ready,
		.ttab {
			transition: none;
		}
	}
</style>
