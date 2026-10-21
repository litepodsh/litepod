<script lang="ts" module>
	import { geoEqualEarth, geoPath } from 'd3-geo';
	import { feature } from 'topojson-client';
	import type { Feature, FeatureCollection, Geometry } from 'geojson';
	import type { GeometryCollection, Topology } from 'topojson-specification';
	import worldUrl from 'world-atlas/countries-110m.json?url';
	import { atlasAlpha2 } from '#lib/utils/countries.js';

	const WIDTH = 960;
	const HEIGHT = 470;

	type Shape = { code: string | null; d: string };

	// Fetched once per page load and shared by every map.
	let atlas: Promise<Shape[]> | null = null;
	function loadAtlas(): Promise<Shape[]> {
		atlas ??= fetch(worldUrl)
			.then((response) => response.json() as Promise<Topology>)
			.then((topology) => {
				const collection = feature(
					topology,
					topology.objects.countries as GeometryCollection<{ name: string }>,
				) as FeatureCollection<Geometry, { name: string }>;
				// Antarctica only takes room; nobody browses from it.
				const land = collection.features.filter((country) => country.id !== '010');
				const projection = geoEqualEarth().fitExtent(
					[
						[4, 4],
						[WIDTH - 4, HEIGHT - 4],
					],
					{ type: 'FeatureCollection', features: land },
				);
				const path = geoPath(projection);
				return land.map((country: Feature<Geometry, { name: string }>) => ({
					code: atlasAlpha2(country.id, country.properties?.name),
					d: path(country) ?? '',
				}));
			})
			.catch((error) => {
				atlas = null;
				throw error;
			});
		return atlas;
	}
</script>

<script lang="ts">
	import GlobeIcon from '@lucide/svelte/icons/globe';
	import XIcon from '@lucide/svelte/icons/x';
	import SegmentedSwitch from '#lib/components/custom/segmented-switch.svelte';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import { compactNumber, type CountryEntry } from '#lib/clients/analytics-client.js';
	import { countryName, flag } from '#lib/utils/countries.js';
	import { i18n, t } from '#lib/i18n/index.svelte.js';

	/** Where traffic comes from: a world choropleth beside a ranked country list. */
	let {
		countries,
		selected = $bindable(null),
	}: {
		countries: CountryEntry[];
		/** Alpha-2 of the country the visitors list is narrowed to. */
		selected?: string | null;
	} = $props();

	/** Fill strength per step, mixed from the accent into the card color. */
	const STEPS = [20, 38, 56, 76, 100];
	const LIST_LIMIT = 8;

	let shapes = $state<Shape[] | null>(null);
	let failed = $state(false);
	$effect(() => {
		loadAtlas().then(
			(next) => (shapes = next),
			() => (failed = true),
		);
	});

	type Metric = 'visitors' | 'requests';
	// Bots and scripts have no visitor id: a site that only gets API calls shows requests.
	let hasVisitors = $derived(countries.some((country) => country.visitors > 0));
	let metric = $state<Metric>('visitors');
	let shown = $derived<Metric>(hasVisitors ? metric : 'requests');

	let byCode = $derived(new Map(countries.map((country) => [country.code, country])));
	let total = $derived(countries.reduce((sum, country) => sum + country[shown], 0));
	let max = $derived(Math.max(0, ...countries.map((country) => country[shown])));
	let ranked = $derived(
		countries
			.filter((country) => country[shown] > 0)
			.toSorted((a, b) => b[shown] - a[shown] || a.code.localeCompare(b.code)),
	);

	/** Log steps: a country with a tenth of the leader still reads as "a lot". */
	function step(value: number): number {
		if (value <= 0 || max <= 0) return -1;
		const ratio = Math.log1p(value) / Math.log1p(max);
		return Math.min(STEPS.length - 1, Math.floor(ratio * STEPS.length));
	}
	function fill(code: string | null): string {
		const entry = code ? byCode.get(code) : undefined;
		const index = entry ? step(entry[shown]) : -1;
		return index < 0 ? 'var(--empty)' : `var(--step-${index})`;
	}

	const count = (value: number) => compactNumber(value, i18n.locale);
	const share = (value: number) =>
		total ? `${((value / total) * 100).toFixed(value / total < 0.1 ? 1 : 0)}%` : '0%';
	const name = (code: string) => countryName(code, i18n.locale);

	// Pointer tooltip, positioned in the map box's own percent space.
	let hover = $state<{ code: string; x: number; y: number } | null>(null);
	let box = $state<HTMLDivElement>();
	function track(event: PointerEvent, code: string | null) {
		if (!code || !box) {
			hover = null;
			return;
		}
		const rect = box.getBoundingClientRect();
		hover = {
			code,
			x: ((event.clientX - rect.left) / rect.width) * 100,
			y: ((event.clientY - rect.top) / rect.height) * 100,
		};
	}
	let hovered = $derived(hover ? byCode.get(hover.code) : undefined);
	/** Drawn last so neighbors never paint over the outline. */
	let outlined = $derived(
		(shapes ?? []).filter(
			(shape) => !!shape.code && (shape.code === selected || shape.code === hover?.code),
		),
	);

	function toggle(code: string | null) {
		if (!code || !byCode.has(code)) return;
		selected = selected === code ? null : code;
	}
</script>

<section
	class="map lp-raised flex min-w-0 flex-col rounded-2xl border bg-card"
	style:--empty="color-mix(in oklch, var(--muted-foreground) 14%, var(--card))"
	style:--step-0="color-mix(in oklch, var(--primary) {STEPS[0]}%, var(--card))"
	style:--step-1="color-mix(in oklch, var(--primary) {STEPS[1]}%, var(--card))"
	style:--step-2="color-mix(in oklch, var(--primary) {STEPS[2]}%, var(--card))"
	style:--step-3="color-mix(in oklch, var(--primary) {STEPS[3]}%, var(--card))"
	style:--step-4="color-mix(in oklch, var(--primary) {STEPS[4]}%, var(--card))">
	<header class="lp-groove-b flex flex-wrap items-center justify-between gap-3 px-5 py-3.5">
		<div class="flex min-w-0 items-center gap-2.5">
			<h2 class="text-[15px] font-semibold tracking-tight">{t('analytics.geo.title')}</h2>
			{#if selected}
				<button
					type="button"
					class="flex items-center gap-1.5 rounded-full border bg-muted px-2.5 py-0.5 text-xs hover:border-foreground/25"
					onclick={() => (selected = null)}
					aria-label={t('analytics.geo.clear_filter')}>
					<span aria-hidden="true">{flag(selected)}</span>
					{name(selected)}
					<XIcon class="size-3 text-muted-foreground" />
				</button>
			{/if}
		</div>
		{#if hasVisitors}
			<SegmentedSwitch
				items={[
					{ value: 'visitors', label: t('analytics.geo.metric_visitors') },
					{ value: 'requests', label: t('analytics.geo.metric_requests') },
				]}
				bind:value={metric}
				label={t('analytics.geo.metric')}
				class="w-full sm:w-64" />
		{/if}
	</header>

	<div class="grid min-w-0 gap-0 lg:grid-cols-[minmax(0,1fr)_300px]">
		<div class="relative min-w-0 px-3 pt-4 pb-2 sm:px-5" bind:this={box}>
			{#if failed}
				<div
					class="grid aspect-[960/470] place-items-center text-sm text-muted-foreground">
					{t('analytics.geo.map_failed')}
				</div>
			{:else if !shapes}
				<Skeleton class="aspect-[960/470] w-full rounded-xl" />
			{:else}
				<svg
					viewBox="0 0 {WIDTH} {HEIGHT}"
					class="block h-auto w-full"
					role="group"
					aria-label={t('analytics.geo.map_aria')}
					onpointerleave={() => (hover = null)}>
					{#each shapes as shape, index (index)}
						{@const entry = shape.code ? byCode.get(shape.code) : undefined}
						{#if shape.code && entry}
							{@const code = shape.code}
							<path
								d={shape.d}
								fill={fill(code)}
								class="country active"
								class:dimmed={!!selected && code !== selected}
								role="button"
								tabindex="0"
								aria-pressed={code === selected}
								aria-label="{name(code)}: {count(entry[shown])}"
								onpointermove={(event) => track(event, code)}
								onclick={() => toggle(code)}
								onkeydown={(event) => {
									if (event.key === 'Enter' || event.key === ' ') {
										event.preventDefault();
										toggle(code);
									}
								}} />
						{:else}
							<path
								d={shape.d}
								fill="var(--empty)"
								class="country"
								class:dimmed={!!selected}
								aria-hidden="true"
								onpointermove={(event) => track(event, null)} />
						{/if}
					{/each}
					{#each outlined as shape, index (index)}
						<path
							d={shape.d}
							class="outline"
							class:selected={shape.code === selected}
							aria-hidden="true" />
					{/each}
				</svg>
				{#if countries.length === 0}
					<div class="pointer-events-none absolute inset-0 grid place-items-center p-6">
						<div
							class="flex max-w-xs flex-col items-center gap-2 rounded-xl border bg-card/90 px-4 py-3 text-center backdrop-blur">
							<GlobeIcon class="size-4 text-muted-foreground" />
							<p class="text-sm font-medium">{t('analytics.geo.empty_title')}</p>
							<p class="text-xs text-muted-foreground">{t('analytics.geo.empty_body')}</p>
						</div>
					</div>
				{/if}
			{/if}

			{#if hover && hovered}
				<div
					role="tooltip"
					class="pointer-events-none absolute z-10 w-max min-w-40 -translate-x-1/2 -translate-y-[calc(100%+12px)] rounded-xl border bg-popover px-3 py-2 text-xs text-popover-foreground shadow-md"
					style="left: clamp(6rem, {hover.x}%, calc(100% - 6rem)); top: {hover.y}%">
					<p class="flex items-center gap-1.5 text-sm font-semibold">
						<span aria-hidden="true">{flag(hovered.code)}</span>{name(hovered.code)}
					</p>
					<dl class="mt-1 grid grid-cols-[auto_auto] gap-x-4 gap-y-0.5 tabular-nums">
						<dt class="text-muted-foreground">{t('analytics.geo.metric_visitors')}</dt>
						<dd class="text-right">{count(hovered.visitors)}</dd>
						<dt class="text-muted-foreground">{t('analytics.geo.metric_requests')}</dt>
						<dd class="text-right">{count(hovered.requests)}</dd>
						<dt class="text-muted-foreground">{t('analytics.geo.share')}</dt>
						<dd class="text-right">{share(hovered[shown])}</dd>
					</dl>
				</div>
			{/if}

			<div
				class="mt-2 flex flex-wrap items-center justify-between gap-x-4 gap-y-2 text-[11px] text-muted-foreground">
				<div class="flex items-center gap-2" aria-hidden="true">
					<span>{t('analytics.geo.legend_less')}</span>
					<span class="flex gap-0.5">
						{#each STEPS as _, index (index)}
							<i class="block h-2 w-5 rounded-[3px]" style="background: var(--step-{index})"></i>
						{/each}
					</span>
					<span>{t('analytics.geo.legend_more')}</span>
					<i class="ml-2 block h-2 w-5 rounded-[3px]" style="background: var(--empty)"></i>
					<span>{t('analytics.geo.legend_none')}</span>
				</div>
				<a
					href="https://db-ip.com"
					target="_blank"
					rel="noopener noreferrer"
					class="underline-offset-4 hover:text-foreground hover:underline">
					{t('analytics.geo.attribution')}
				</a>
			</div>
		</div>

		<div class="lp-groove-t flex min-w-0 flex-col lg:border-t-0 lg:border-l lg:shadow-none">
			<p
				class="flex items-baseline justify-between px-5 pt-4 pb-2 text-xs text-muted-foreground">
				<span>{t('analytics.geo.countries_count', { count: ranked.length })}</span>
				<span class="tabular-nums">
					{shown === 'visitors'
						? t('analytics.geo.metric_visitors')
						: t('analytics.geo.metric_requests')}
				</span>
			</p>
			{#if ranked.length === 0}
				<p class="px-5 pb-5 text-sm text-muted-foreground">{t('analytics.panel.no_rows')}</p>
			{:else}
				<ol class="flex flex-col px-2 pb-3">
					{#each ranked.slice(0, LIST_LIMIT) as country (country.code)}
						<li>
							<button
								type="button"
								class="group flex w-full min-w-0 flex-col gap-1.5 rounded-lg px-3 py-2 text-left transition-colors hover:bg-muted/60 {selected ===
								country.code
									? 'bg-muted'
									: ''}"
								aria-pressed={selected === country.code}
								onclick={() => toggle(country.code)}>
								<span class="flex min-w-0 items-center justify-between gap-3 text-sm">
									<span class="flex min-w-0 items-center gap-2">
										<span class="text-base leading-none" aria-hidden="true"
											>{flag(country.code)}</span>
										<span class="truncate">{name(country.code)}</span>
									</span>
									<span class="flex shrink-0 items-baseline gap-2 tabular-nums">
										<span class="text-xs text-muted-foreground">{share(country[shown])}</span>
										<span class="w-10 text-right">{count(country[shown])}</span>
									</span>
								</span>
								<span class="lp-well block h-1 overflow-hidden rounded-full bg-muted">
									<i
										class="block h-full rounded-full bg-primary"
										style="width: {(country[shown] / Math.max(1, max)) * 100}%"></i>
								</span>
							</button>
						</li>
					{/each}
				</ol>
				{#if ranked.length > LIST_LIMIT}
					<p class="px-5 pb-4 text-xs text-muted-foreground">
						{t('analytics.geo.more_countries', { count: ranked.length - LIST_LIMIT })}
					</p>
				{/if}
			{/if}
		</div>
	</div>
</section>

<style>
	.country {
		stroke: var(--card);
		stroke-width: 0.6;
		stroke-linejoin: round;
		transition:
			fill 160ms var(--lp-ease),
			opacity 160ms var(--lp-ease);
	}
	.country.active {
		cursor: pointer;
	}
	.country.active:focus-visible {
		stroke: var(--foreground);
		stroke-width: 1.4;
		outline: none;
	}
	.outline {
		fill: none;
		stroke: var(--foreground);
		stroke-width: 1;
		stroke-linejoin: round;
		pointer-events: none;
	}
	.outline.selected {
		stroke-width: 1.6;
	}
	.country.dimmed {
		opacity: 0.45;
	}
	@media (prefers-reduced-motion: reduce) {
		.country {
			transition: none;
		}
	}
</style>
