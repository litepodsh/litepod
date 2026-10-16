<script lang="ts">
	import { cn } from '#lib/utils.js';

	/** Grafana-style stat: a coloured block, one big number, the trend as a soft area behind it. */
	let {
		title,
		value,
		values,
		tone,
	}: {
		title: string;
		value: string;
		values: number[];
		tone: 'info' | 'destructive' | 'warning' | 'success' | 'primary';
	} = $props();

	const bg = {
		info: 'bg-info',
		destructive: 'bg-destructive',
		warning: 'bg-warning',
		success: 'bg-success',
		primary: 'bg-primary',
	};

	// Cumulative, like Grafana's counters: the area always rises to the total.
	let area = $derived.by(() => {
		if (values.length < 2) return '';
		let sum = 0;
		const running = values.map((v) => (sum += v));
		const top = Math.max(1, sum);
		const points = running.map(
			(v, i) =>
				`${((i / (running.length - 1)) * 100).toFixed(2)},${(40 - (v / top) * 26).toFixed(2)}`,
		);
		return `M0,40L${points.join('L')}L100,40Z`;
	});
</script>

<section class="flex min-w-0 flex-col gap-2 rounded-xl border bg-card p-3">
	<h3 class="truncate text-[13px] font-medium">{title}</h3>
	<div class={cn('relative isolate h-[72px] overflow-hidden rounded-lg text-white', bg[tone])}>
		<svg
			viewBox="0 0 100 40"
			preserveAspectRatio="none"
			class="absolute inset-0 -z-10 size-full"
			aria-hidden="true">
			<path d={area} fill="white" fill-opacity="0.28" />
		</svg>
		<b
			class="grid h-full place-items-center text-[clamp(1.6rem,3.4vw,2.25rem)] font-semibold tracking-tight tabular-nums drop-shadow-sm"
			>{value}</b>
	</div>
</section>
