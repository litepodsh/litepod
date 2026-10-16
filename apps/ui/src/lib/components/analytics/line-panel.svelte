<script lang="ts">
	import { cn } from '#lib/utils.js';
	import { i18n } from '#lib/i18n/index.svelte.js';

	/**
	 * A Grafana-style time series panel: gridlines, axis labels, one or more lines
	 * with a soft area fill, a hover crosshair and an optional Last/Min/Max legend.
	 * Plain SVG; text stays HTML so it never stretches with the plot.
	 */
	type Series = {
		label: string;
		/** Text colour class, e.g. `text-primary`; the line uses currentColor. */
		color: string;
		values: (number | null)[];
	};

	let {
		title,
		ts,
		series,
		format,
		legend = false,
		max,
		labels,
		class: className,
	}: {
		title: string;
		/** Bucket starts, epoch ms; same length as every series. */
		ts: number[];
		series: Series[];
		format: (value: number) => string;
		/** Last / Min / Max table beside the plot (stacks under it on phones). */
		legend?: boolean;
		/** Fixed top of the y axis (100 for percentages). */
		max?: number;
		/** `{ last, min, max }` column headings. */
		labels: { name: string; last: string; min: string; max: string };
		class?: string;
	} = $props();

	const uid = $props.id();
	const W = 1000;
	const H = 300;

	function nice(value: number) {
		if (value <= 0) return 1;
		const magnitude = 10 ** Math.floor(Math.log10(value));
		const step = [1, 2, 2.5, 5, 10].find((s) => s * magnitude >= value) ?? 10;
		return step * magnitude;
	}

	let top = $derived(
		max ?? nice(Math.max(0, ...series.flatMap((s) => s.values.filter((v) => v !== null)))),
	);
	let n = $derived(ts.length);
	const x = (i: number) => (n <= 1 ? 0 : (i / (n - 1)) * W);
	const y = (v: number) => H - (Math.min(v, top) / top) * H;

	function path(values: (number | null)[], area: boolean) {
		let d = '';
		let start = -1;
		let last = -1;
		values.forEach((v, i) => {
			if (v === null) return;
			d += `${start < 0 ? 'M' : 'L'}${x(i).toFixed(1)},${y(v).toFixed(1)}`;
			if (start < 0) start = i;
			last = i;
		});
		if (!area || start < 0) return d;
		return `${d}L${x(last).toFixed(1)},${H}L${x(start).toFixed(1)},${H}Z`;
	}

	let ticks = $derived([1, 0.75, 0.5, 0.25, 0].map((f) => top * f));
	let xTicks = $derived(n < 2 ? [] : [0.1, 0.3, 0.5, 0.7, 0.9].map((f) => Math.round(f * (n - 1))));
	let spanMs = $derived(n > 1 ? ts[n - 1] - ts[0] : 0);
	function timeLabel(ms: number) {
		const options: Intl.DateTimeFormatOptions =
			spanMs > 2 * 86_400_000
				? { month: 'short', day: 'numeric' }
				: { hour: '2-digit', minute: '2-digit' };
		return new Date(ms).toLocaleString(i18n.locale, options);
	}

	function stats(values: (number | null)[]) {
		const known = values.filter((v): v is number => v !== null);
		return {
			last: known.at(-1) ?? null,
			min: known.length ? Math.min(...known) : null,
			max: known.length ? Math.max(...known) : null,
		};
	}

	let hover = $state<number | null>(null);
	function onmove(event: PointerEvent) {
		const box = (event.currentTarget as HTMLElement).getBoundingClientRect();
		const f = Math.min(1, Math.max(0, (event.clientX - box.left) / box.width));
		hover = n > 1 ? Math.round(f * (n - 1)) : null;
	}
</script>

<section class={cn('panel flex min-w-0 flex-col gap-2 rounded-xl border bg-card p-3', className)}>
	<h3 class="text-[13px] font-medium">{title}</h3>
	<div class={cn('flex min-w-0 gap-4', legend && 'max-md:flex-col')}>
		<div class="grid min-w-0 flex-1 grid-cols-[auto_minmax(0,1fr)] gap-x-2">
			<div
				class="flex h-44 flex-col justify-between text-right text-[10.5px] text-muted-foreground tabular-nums">
				{#each ticks as tick, index (index)}
					<span class="-my-1.5">{format(tick)}</span>
				{/each}
			</div>
			<div
				class="relative h-44 min-w-0"
				role="img"
				aria-label={title}
				onpointermove={onmove}
				onpointerleave={() => (hover = null)}>
				<svg
					viewBox="0 0 {W} {H}"
					preserveAspectRatio="none"
					class="absolute inset-0 size-full overflow-visible">
					{#each ticks as tick, index (index)}
						<line
							x1="0"
							x2={W}
							y1={y(tick)}
							y2={y(tick)}
							class="stroke-border"
							vector-effect="non-scaling-stroke" />
					{/each}
					{#each xTicks as index (index)}
						<line
							x1={x(index)}
							x2={x(index)}
							y1="0"
							y2={H}
							class="stroke-border"
							stroke-dasharray="2 4"
							vector-effect="non-scaling-stroke" />
					{/each}
					{#each series as s, index (s.label)}
						<g class={s.color}>
							<defs>
								<linearGradient id="{uid}-{index}" x1="0" x2="0" y1="0" y2="1">
									<stop offset="0" stop-color="currentColor" stop-opacity="0.32" />
									<stop offset="1" stop-color="currentColor" stop-opacity="0.02" />
								</linearGradient>
							</defs>
							<path d={path(s.values, true)} fill="url(#{uid}-{index})" />
							<path
								d={path(s.values, false)}
								fill="none"
								stroke="currentColor"
								stroke-width="1.75"
								stroke-linejoin="round"
								vector-effect="non-scaling-stroke" />
						</g>
					{/each}
					{#if hover !== null}
						<line
							x1={x(hover)}
							x2={x(hover)}
							y1="0"
							y2={H}
							class="stroke-muted-foreground"
							vector-effect="non-scaling-stroke" />
					{/if}
				</svg>
				{#if hover !== null}
					<div
						class="pointer-events-none absolute top-1 z-10 rounded-lg border bg-popover px-2.5 py-1.5 text-[11.5px] shadow-md"
						style={hover > n / 2
							? `right: ${100 - (hover / (n - 1)) * 100 + 2}%`
							: `left: ${(hover / (n - 1)) * 100 + 2}%`}>
						<p class="mb-1 text-muted-foreground">{timeLabel(ts[hover])}</p>
						{#each series as s (s.label)}
							{@const v = s.values[hover]}
							<p class="flex items-center gap-2 whitespace-nowrap">
								<i class="h-0.5 w-3 rounded-full bg-current {s.color}"></i>
								<span class="text-muted-foreground">{s.label}</span>
								<b class="ml-auto font-medium tabular-nums">{v === null ? '-' : format(v)}</b>
							</p>
						{/each}
					</div>
				{/if}
			</div>
			<span></span>
			<div class="relative mt-1 h-4 text-[10.5px] text-muted-foreground">
				{#each xTicks as index, position (index)}
					<!-- Every other label on narrow panels so times never overlap. -->
					<span
						class={cn(
							'absolute -translate-x-1/2 whitespace-nowrap',
							position % 2 === 1 && 'narrow-hide',
						)}
						style="left: {(index / (n - 1)) * 100}%">{timeLabel(ts[index])}</span>
				{/each}
			</div>
		</div>
		{#if legend}
			<table class="h-fit shrink-0 text-[11.5px] tabular-nums md:w-64">
				<thead>
					<tr class="text-primary">
						<th class="pb-1 text-left font-medium">{labels.name}</th>
						<th class="pb-1 text-right font-medium">{labels.last}</th>
						<th class="pb-1 text-right font-medium">{labels.min}</th>
						<th class="pb-1 text-right font-medium">{labels.max}</th>
					</tr>
				</thead>
				<tbody>
					{#each series as s (s.label)}
						{@const st = stats(s.values)}
						<tr class="border-t">
							<td class="py-1 pr-2">
								<span class="flex items-center gap-1.5 whitespace-nowrap">
									<i class="h-1 w-3 rounded-full bg-current {s.color}"></i>{s.label}
								</span>
							</td>
							<td class="py-1 pl-2 text-right">{st.last === null ? '-' : format(st.last)}</td>
							<td class="py-1 pl-2 text-right">{st.min === null ? '-' : format(st.min)}</td>
							<td class="py-1 pl-2 text-right">{st.max === null ? '-' : format(st.max)}</td>
						</tr>
					{/each}
				</tbody>
			</table>
		{/if}
	</div>
</section>

<style>
	.panel {
		container-type: inline-size;
	}
	@container (max-width: 440px) {
		.narrow-hide {
			display: none;
		}
	}
</style>
