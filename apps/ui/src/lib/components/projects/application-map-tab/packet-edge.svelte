<script lang="ts">
	import { BaseEdge, getBezierPath } from '@xyflow/svelte';
	import type { EdgeProps } from '@xyflow/svelte';

	let {
		id,
		sourceX,
		sourceY,
		targetX,
		targetY,
		sourcePosition,
		targetPosition,
		style,
		label,
		data,
	}: EdgeProps = $props();

	const geom = $derived(
		getBezierPath({ sourceX, sourceY, sourcePosition, targetX, targetY, targetPosition }),
	);
	const path = $derived(geom[0]);
	const labelX = $derived(geom[1]);
	const labelY = $derived(geom[2]);

	const reduced =
		typeof window !== 'undefined' &&
		!!window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;

	const kind = $derived((data?.kind as string | undefined) ?? 'attach');
	const tip = $derived((data?.tip as string | undefined) ?? '');

	// A steady stream of small green packets along the attachment line. When a
	// real bytes/s rate is known (4a), the stream speeds up and thickens with it.
	const PACKETS = [0, 1, 2, 3, 4];
	const rate = $derived(typeof data?.rate === 'number' ? (data.rate as number) : null);
	// Quantise to half-decade buckets so a small rate wiggle every 3s does not
	// re-time the SMIL animation; it only changes when traffic really changes.
	const bucket = $derived(rate === null ? null : Math.round(Math.log10(Math.max(rate, 1)) * 2) / 2);
	const dur = $derived(bucket === null ? 2.6 : Math.min(2.8, Math.max(0.5, 2.8 - bucket * 0.42)));
	const radius = $derived(bucket !== null && bucket >= 5 ? 3 : 2.2);
</script>

<!-- Fat transparent path: hover target for the native tooltip. -->
<path
	d={path}
	fill="none"
	stroke="transparent"
	stroke-width="18"
	style="pointer-events: stroke; cursor: help;">
	{#if tip}<title>{tip}</title>{/if}
</path>

<BaseEdge {id} {path} {style} {label} {labelX} {labelY} />

{#if kind === 'attach' && !reduced}
	{#each PACKETS as i (i)}
		<circle r={radius} fill="var(--success)">
			<animateMotion
				dur="{dur}s"
				begin="-{(dur / PACKETS.length) * i}s"
				repeatCount="indefinite"
				{path} />
		</circle>
	{/each}
{/if}
