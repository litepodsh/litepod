<script lang="ts">
	import { cn } from '#lib/utils.js';

	// The favicon's geometry (rounded square, pill at -24deg) in currentColor,
	// with the pill cut out and filled by a halftone that fades to the corner.
	let { class: className }: { class?: string } = $props();
	const uid = $props.id();
	const pill = { x: 34, y: 42, width: 52, height: 36, rx: 18, transform: 'rotate(-24 60 60)' };
</script>

<svg
	class={cn('litepod-mark block size-6.5', className)}
	viewBox="0 0 120 120"
	role="img"
	aria-label="Litepod">
	<defs>
		<pattern
			id="{uid}-ht"
			width="7"
			height="7"
			patternUnits="userSpaceOnUse"
			patternTransform="rotate(-24 60 60)">
			<circle cx="3.5" cy="3.5" r="2.1" fill="currentColor" />
		</pattern>
		<linearGradient id="{uid}-fade" x1="0" y1="0" x2="1" y2="1">
			<stop offset="0" stop-color="#fff" />
			<stop offset=".85" stop-color="#fff" stop-opacity=".15" />
		</linearGradient>
		<mask id="{uid}-fade-mask">
			<rect width="120" height="120" fill="url(#{uid}-fade)" />
		</mask>
		<mask id="{uid}-cut" maskUnits="userSpaceOnUse" x="0" y="0" width="120" height="120">
			<rect x="10" y="10" width="100" height="100" rx="26" fill="#fff" />
			<rect {...pill} fill="#000" />
		</mask>
		<clipPath id="{uid}-pill"><rect {...pill} /></clipPath>
	</defs>
	<rect x="10" y="10" width="100" height="100" rx="26" fill="currentColor" mask="url(#{uid}-cut)" />
	<g class="halftone" clip-path="url(#{uid}-pill)" opacity=".75">
		<g mask="url(#{uid}-fade-mask)"><rect width="120" height="120" fill="url(#{uid}-ht)" /></g>
	</g>
</svg>

<style>
	/* Warm (the default palette) tints the mark with the oxide accent; cool stays ink. */
	:global(:root:not([data-palette='cool'])) .litepod-mark {
		color: var(--primary);
	}
	.halftone {
		transition: opacity 0.4s var(--lp-ease);
	}
	:global(a:hover) > .litepod-mark .halftone,
	.litepod-mark:hover .halftone {
		opacity: 1;
	}
</style>
