<script lang="ts">
	import { cn } from '#lib/utils.js';

	// Columns of small squares over a faint full grid: the console's only chart style.
	let {
		values,
		cols = 48,
		rows = 10,
		max,
		tone = 'primary',
		format = (v: number) => String(Math.round(v)),
		label = () => '',
		ariaLabel,
		onhover,
		caps,
		selection = null,
		class: className,
	}: {
		values: number[];
		cols?: number;
		rows?: number;
		max?: number;
		tone?: 'primary' | 'success' | 'warning' | 'muted';
		format?: (v: number) => string;
		label?: (i: number) => string;
		ariaLabel: string;
		/** Column under the pointer, null once it leaves the chart. */
		onhover?: (index: number | null) => void;
		/**
		 * Per-column capacity on the same scale as `values` (e.g. the RAM the host had
		 * then). Cells above it are drawn as unavailable. Only read when `values` already
		 * has `cols` entries; null leaves a column uncapped.
		 */
		caps?: (number | null)[];
		/** Inclusive column range drawn as selected, e.g. while dragging to zoom. */
		selection?: [number, number] | null;
		class?: string;
	} = $props();

	const S = 4;
	const G = 1.5;
	const P = S + G;
	const toneClass = {
		primary: 'text-primary',
		success: 'text-success',
		warning: 'text-warning',
		muted: 'text-muted-foreground',
	};

	function resample(src: number[], n: number) {
		if (src.length === 0) return Array<number>(n).fill(0);
		if (src.length === 1 || n === 1) return Array<number>(n).fill(src.at(-1)!);
		return Array.from({ length: n }, (_, i) => {
			const x = (i * (src.length - 1)) / (n - 1);
			const a = Math.floor(x);
			const b = Math.min(a + 1, src.length - 1);
			return src[a] + (src[b] - src[a]) * (x - a);
		});
	}

	let data = $derived(resample(values, cols));
	let capRows = $derived(
		caps && caps.length === cols
			? caps.map((cap) => (cap === null ? rows : Math.round((cap / top) * rows)))
			: null,
	);
	let top = $derived(max ?? Math.max(1, ...data) * 1.08);
	let w = $derived(cols * P - G);
	let h = $derived(rows * P - G);
</script>

<svg
	class={cn('cell-chart block h-auto w-full', toneClass[tone], className)}
	viewBox="0 0 {w} {h}"
	role="img"
	aria-label={ariaLabel}
	onpointerleave={() => onhover?.(null)}>
	{#each data as v, i (i)}
		{@const n = v > 0 ? Math.min(rows, Math.max(1, Math.round((v / top) * rows))) : 0}
		{@const cap = capRows?.[i] ?? rows}
		{@const picked = selection && i >= selection[0] && i <= selection[1]}
		<!-- svelte-ignore a11y_no_static_element_interactions (pointer readout only; the svg carries the label) -->
		<g onpointerenter={() => onhover?.(i)}>
			{#if !onhover}<title>{label(i)}{format(v)}</title>{/if}
			{#each { length: rows }, k (k)}
				<rect
					class={cn('cell', k < n && 'on', k >= cap && k >= n && 'off', picked && 'picked')}
					x={i * P}
					y={h - S - k * P}
					width={S}
					height={S}
					rx="0.8" />
			{/each}
		</g>
	{/each}
</svg>

<style>
	.cell {
		fill: currentColor;
		opacity: 0.09;
		transition: opacity 0.2s var(--lp-ease);
	}
	.cell.on {
		opacity: 0.78;
	}
	/* Beyond the capacity of that moment: there was nothing to fill. */
	.cell.off {
		fill: none;
		stroke: currentColor;
		stroke-width: 0.6;
		stroke-dasharray: 1 1;
		opacity: 0.18;
	}
	.cell.picked {
		opacity: 0.35;
	}
	.cell.picked.on {
		opacity: 1;
	}
	g:last-of-type .cell.on {
		opacity: 1;
	}
	svg:hover .cell.on {
		opacity: 0.32;
	}
	g:hover .cell.on {
		opacity: 1;
	}
	g:hover .cell {
		opacity: 0.2;
	}
	@media (prefers-reduced-motion: reduce) {
		.cell {
			transition: none;
		}
	}
</style>
