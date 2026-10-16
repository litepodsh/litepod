<script lang="ts">
	import { portal } from '#lib/utils/portal.js';

	/** A fixed-position tooltip that escapes the chart's card. Position is computed
	 *  from the chart wrapper's bounding rect every time `host` changes (resize,
	 *  scroll, layout shift), so it stays anchored over the hovered column. */
	let {
		host,
		column,
		cols,
		flipNearEdges = true,
		children,
	}: {
		/** Wrapper element that owns the chart. Required while visible. */
		host: HTMLElement | null;
		/** 0-based hovered column. `null` hides the tooltip. */
		column: number | null;
		/** Total columns in the chart; used to convert `column` → percent. */
		cols: number;
		/** Flip horizontally when the hovered column sits in the first or last 12
		 *  columns so the popover does not clip against the viewport edge. */
		flipNearEdges?: boolean;
		children: import('svelte').Snippet;
	} = $props();

	let rect = $state<DOMRect | null>(null);

	function refresh() {
		rect = host?.getBoundingClientRect() ?? null;
	}

	$effect(() => {
		if (column === null) return;
		refresh();
		const update = () => refresh();
		window.addEventListener('resize', update);
		window.addEventListener('scroll', update, true);
		return () => {
			window.removeEventListener('resize', update);
			window.removeEventListener('scroll', update, true);
		};
	});

	let visible = $derived(column !== null && rect !== null);
	// Percent along the host width where the hovered column sits.
	let centerPx = $derived(
		column === null || !rect ? 0 : ((column + 0.5) / cols) * rect.width,
	);
	let edge = $state<'start' | 'middle' | 'end'>('middle');
	$effect(() => {
		if (!visible || !flipNearEdges) return;
		if (column! < 12) edge = 'start';
		else if (column! > cols - 12) edge = 'end';
		else edge = 'middle';
	});

	let transform = $derived(
		edge === 'start'
			? 'translateY(-100%)'
			: edge === 'end'
				? 'translate(-100%, -100%)'
				: 'translate(-50%, -100%)',
	);
	let top = $derived(rect ? rect.top - 8 : 0);
	let left = $derived(rect ? rect.left + centerPx : 0);
</script>

<!--
  The outer div stays in the original DOM tree as a layout placeholder. The
  inner div is what the `portal` action teleports to `document.body`; that
  way the placeholder takes up no box of its own (`display: contents`) while
  the popover paints outside the card. Children render inside the portal so
  any local state survives hover transitions.
-->
<div style:display="contents">
	<div
		use:portal={'body'}
		style:position="fixed"
		style:top="{top}px"
		style:left="{left}px"
		style:transform={transform}
		style:z-index="50"
		style:visibility={visible ? 'visible' : 'hidden'}
		aria-hidden={!visible}
		class="pointer-events-none w-max max-w-64 rounded-xl border bg-popover px-3 py-2 text-xs text-popover-foreground shadow-md">
		{@render children()}
	</div>
</div>