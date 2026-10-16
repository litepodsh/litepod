<script lang="ts">
	import { PieChart, Text } from 'layerchart';
	import * as Chart from '#lib/components/ui/chart/index.js';
	import { toneFill, utilizationTone } from '#lib/utils/status-tone.js';

	let { label, percent, detail }: { label: string; percent: number | null; detail: string } =
		$props();

	const chartConfig = {
		used: { label: 'Used', color: 'var(--primary)' },
		available: { label: 'Available', color: 'var(--muted)' },
	} satisfies Chart.ChartConfig;

	let safePercent = $derived(percent === null ? 0 : Math.max(0, Math.min(100, percent)));
	// The arc escalates with pressure, so a nearly-full resource is visible at a
	// glance instead of looking identical to an idle one. The percent below is
	// still the authority; colour only reinforces it.
	let usedColor = $derived(toneFill(utilizationTone(percent)));
	let chartData = $derived([
		{ capacity: 'used', value: safePercent, color: usedColor },
		{ capacity: 'available', value: 100 - safePercent, color: chartConfig.available.color },
	]);
	let value = $derived(percent === null ? 'Unavailable' : `${percent.toFixed(0)}%`);
</script>

<div class="min-w-0 overflow-hidden rounded-[1.5rem] bg-muted/30 p-1 ring-1 ring-foreground/5">
	<div
		class="min-w-0 rounded-[calc(1.5rem-0.25rem)] bg-background px-3 pb-5 pt-5 text-center sm:px-4 sm:pt-6"
		role="img"
		aria-label={`${label}: ${value}. ${detail}`}>
		<Chart.Container config={chartConfig} class="mx-auto aspect-square w-full max-w-72">
			<PieChart
				data={chartData}
				key="capacity"
				value="value"
				c="color"
				innerRadius={0.82}
				outerRadius={0.92}
				padding={12}
				range={[-90, 90]}
				props={{ pie: { sort: null } }}
				cornerRadius={7}>
				{#snippet aboveMarks()}
					<Text
						{value}
						textAnchor="middle"
						verticalAnchor="middle"
						class="fill-foreground text-[clamp(1.25rem,4vw,1.875rem)]! font-semibold"
						dy={-30} />
					<Text
						value={label}
						textAnchor="middle"
						verticalAnchor="middle"
						class="fill-muted-foreground! text-xs! font-medium"
						dy={14} />
				{/snippet}
			</PieChart>
		</Chart.Container>
		<p
			class="mx-auto -mt-[clamp(2.75rem,8vw,4rem)] max-w-full truncate px-1 text-xs text-muted-foreground"
			title={detail}>
			{detail}
		</p>
	</div>
</div>
