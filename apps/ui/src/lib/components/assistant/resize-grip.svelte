<script lang="ts">
	/**
	 * The hairline between a rail and the page. Drags a width in rem, which the
	 * caller writes to a custom property, so nothing but that one rail re-lays
	 * out. Pointer capture keeps the drag alive over the rest of the shell, so
	 * there are no window listeners to clean up.
	 */
	import { cn } from '#lib/utils.js';

	let {
		value,
		min,
		max,
		/** 1 when dragging away from the page grows the rail, -1 when it shrinks it. */
		sign = 1,
		label,
		class: className,
		onresize,
		onreset,
	}: {
		value: number;
		min: number;
		max: number;
		sign?: number;
		label: string;
		class?: string;
		onresize: (rem: number) => void;
		onreset?: () => void;
	} = $props();

	const REM = 16;
	const STEP = 1;

	let dragging = $state(false);
	let startX = 0;
	let startValue = 0;

	function pointerdown(event: PointerEvent) {
		event.preventDefault();
		startX = event.clientX;
		startValue = value;
		dragging = true;
		(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
	}

	function pointermove(event: PointerEvent) {
		if (!dragging) return;
		onresize(startValue + ((event.clientX - startX) * sign) / REM);
	}

	function keydown(event: KeyboardEvent) {
		const step = event.shiftKey ? STEP * 4 : STEP;
		if (event.key === 'ArrowLeft') onresize(value - step * sign);
		else if (event.key === 'ArrowRight') onresize(value + step * sign);
		else return;
		event.preventDefault();
	}
</script>

<!--
	A focusable window splitter. `separator` plus `aria-valuenow` is the ARIA
	pattern for exactly this, but Svelte's a11y pass does not count `separator`
	as interactive, so the two rules below fire on a correct element.
-->
<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div
	role="separator"
	aria-orientation="vertical"
	aria-label={label}
	aria-valuenow={Math.round(value)}
	aria-valuemin={min}
	aria-valuemax={max}
	tabindex="0"
	data-dragging={dragging || undefined}
	class={cn(
		'group absolute inset-y-5 z-20 hidden w-3 cursor-col-resize touch-none place-items-center md:grid',
		className,
	)}
	onpointerdown={pointerdown}
	onpointermove={pointermove}
	onpointerup={() => (dragging = false)}
	onpointercancel={() => (dragging = false)}
	onkeydown={keydown}
	ondblclick={onreset}>
	<span
		class="bg-foreground/14 group-hover:bg-primary group-focus-visible:bg-primary group-data-[dragging]:bg-primary h-10 w-0.5 rounded-full transition-[height,background-color] duration-300 ease-[var(--lp-ease)] group-hover:h-16 group-data-[dragging]:h-16"
	></span>
</div>
