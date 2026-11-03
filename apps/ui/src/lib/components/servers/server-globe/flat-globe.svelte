<script lang="ts" module>
	import { feature } from 'topojson-client';
	import type { Feature, FeatureCollection, Geometry } from 'geojson';
	import type { GeometryCollection, Topology } from 'topojson-specification';
	import landUrl from 'world-atlas/land-110m.json?url';

	type Land = Feature<Geometry> | FeatureCollection<Geometry>;

	// Fetched once per page load: switching views should not download the coastline again.
	let land: Promise<Land> | null = null;
	function loadLand(): Promise<Land> {
		land ??= fetch(landUrl)
			.then((response) => response.json() as Promise<Topology>)
			.then((topology) => feature(topology, topology.objects.land as GeometryCollection));
		return land;
	}
</script>

<script lang="ts">
	/**
	 * The light alternative to the WebGL globe: one SVG, no three.js, no textures.
	 * It reads the theme from CSS tokens, so a theme change needs no rebuild.
	 */
	import { geoGraticule10, geoOrthographic, geoPath } from 'd3-geo';
	import type { ServerGeo } from '#lib/clients/server-client.js';
	import { t } from '#lib/i18n/index.svelte.js';
	import { fleetCenter, isOnNearSide, type LngLat } from './flat-globe';
	import { markerColor, type GlobePalette } from './palette';

	const SIZE = 600;
	const LABEL_LIMIT = 6;
	const DRAG_SPEED = 0.35;
	const KEY_STEP = 10;
	const TURN_MS = 420;

	let {
		servers,
		main,
		palette,
		focus,
		onSelect,
	}: {
		servers: ServerGeo[];
		main: ServerGeo | null;
		palette: GlobePalette;
		focus: ServerGeo | null;
		onSelect: (server: ServerGeo) => void;
	} = $props();

	const coords = (server: ServerGeo): LngLat => [server.longitude!, server.latitude!];
	const clampLat = (lat: number) => Math.max(-80, Math.min(80, lat));

	let shape = $state.raw<Land | null>(null);
	let center = $state<LngLat>([-30, 20]);

	$effect(() => {
		let stale = false;
		void loadLand().then((loaded) => {
			if (!stale) shape = loaded;
		});
		return () => {
			stale = true;
		};
	});

	// Re-aim only when the set of servers changes, not when a refresh returns the same fleet.
	let fleetKey = $derived(servers.map((server) => server.server_id).join());
	$effect(() => {
		void fleetKey;
		const [lng, lat] = fleetCenter(servers.map(coords));
		center = [lng, clampLat(lat)];
	});

	let projection = $derived(
		geoOrthographic()
			.scale(SIZE / 2 - 10)
			.translate([SIZE / 2, SIZE / 2])
			.rotate([-center[0], -center[1]])
			.clipAngle(90),
	);
	let path = $derived(geoPath(projection));

	let markers = $derived(
		servers
			.filter((server) => isOnNearSide(coords(server), center))
			.map((server) => ({ server, xy: projection(coords(server)) }))
			.filter((marker): marker is { server: ServerGeo; xy: [number, number] } => !!marker.xy),
	);
	let hidden = $derived(servers.length - markers.length);
	let arcs = $derived(
		main
			? servers
					.filter((server) => server.server_id !== main.server_id)
					.map((server) =>
						path({ type: 'LineString', coordinates: [coords(main), coords(server)] }),
					)
			: [],
	);

	let turnFrame = 0;
	$effect(() => () => cancelAnimationFrame(turnFrame));

	// Choosing a hidden server in the list turns the globe to it; a visible one just lights up.
	$effect(() => {
		if (!focus || focus.latitude === null || focus.longitude === null) return;
		const target = coords(focus);
		if (isOnNearSide(target, center)) return;
		turnTo(target);
	});

	function turnTo(target: LngLat) {
		cancelAnimationFrame(turnFrame);
		const goal: LngLat = [target[0], clampLat(target[1])];
		if (window.matchMedia?.('(prefers-reduced-motion: reduce)').matches) {
			center = goal;
			return;
		}
		const from = [...center];
		// The short way round, not always through the same meridian.
		const dLng = ((goal[0] - from[0] + 540) % 360) - 180;
		const started = performance.now();
		const step = (now: number) => {
			const progress = Math.min(1, (now - started) / TURN_MS);
			const eased = 1 - (1 - progress) ** 3;
			center = [from[0] + dLng * eased, from[1] + (goal[1] - from[1]) * eased];
			if (progress < 1) turnFrame = requestAnimationFrame(step);
		};
		turnFrame = requestAnimationFrame(step);
	}

	let drag: { x: number; y: number; from: LngLat } | null = null;

	function startDrag(event: PointerEvent) {
		if ((event.target as Element).closest('button')) return;
		cancelAnimationFrame(turnFrame);
		drag = { x: event.clientX, y: event.clientY, from: [...center] };
		(event.currentTarget as Element).setPointerCapture(event.pointerId);
	}

	function moveDrag(event: PointerEvent) {
		if (!drag) return;
		center = [
			drag.from[0] - (event.clientX - drag.x) * DRAG_SPEED,
			clampLat(drag.from[1] + (event.clientY - drag.y) * DRAG_SPEED),
		];
	}

	function turnWithKeys(event: KeyboardEvent) {
		const delta: Record<string, LngLat> = {
			ArrowLeft: [-KEY_STEP, 0],
			ArrowRight: [KEY_STEP, 0],
			ArrowUp: [0, KEY_STEP],
			ArrowDown: [0, -KEY_STEP],
		};
		const move = delta[event.key];
		if (!move) return;
		event.preventDefault();
		center = [center[0] + move[0], clampLat(center[1] + move[1])];
	}
</script>

<div class="flex size-full flex-col items-center justify-center p-3">
	<!-- A real control: drag or arrow keys turn the globe. -->
	<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
	<div
		class="relative aspect-square h-full max-h-full max-w-full cursor-grab touch-pan-y rounded-full outline-none select-none focus-visible:ring-3 focus-visible:ring-ring/30 active:cursor-grabbing"
		role="application"
		tabindex="0"
		aria-label={t('organizations.topology.globe_drag_hint')}
		onpointerdown={startDrag}
		onpointermove={moveDrag}
		onpointerup={() => (drag = null)}
		onpointercancel={() => (drag = null)}
		onkeydown={turnWithKeys}>
		<svg viewBox="0 0 {SIZE} {SIZE}" class="size-full" aria-hidden="true">
			<path d={path({ type: 'Sphere' })} class="sphere" />
			<path d={path(geoGraticule10())} class="graticule" />
			{#if shape}<path d={path(shape)} class="land" />{/if}
			{#each arcs as arc, index (index)}
				{#if arc}<path d={arc} class="arc" />{/if}
			{/each}
		</svg>
		{#each markers as { server, xy } (server.server_id)}
			{@const color = markerColor(server, palette)}
			{@const lit = focus?.server_id === server.server_id}
			<button
				type="button"
				class="absolute flex -translate-x-[5px] -translate-y-1/2 items-center gap-2 whitespace-nowrap outline-none"
				style="left: {(xy[0] / SIZE) * 100}%; top: {(xy[1] / SIZE) * 100}%"
				aria-label={server.name}
				onclick={() => onSelect(server)}>
				<span
					class="size-2.5 shrink-0 rounded-full"
					style="background: {color}; box-shadow: 0 0 0 {lit
						? 5
						: 3}px color-mix(in srgb, {color} 25%, transparent)"></span>
				{#if lit || servers.length <= LABEL_LIMIT}
					<span
						class="rounded-md border bg-card px-1.5 py-0.5 text-left text-xs leading-4 text-muted-foreground max-sm:hidden">
						<b class="block font-medium text-foreground">{server.name}</b>
						{#if server.city}{server.city.split(',')[0]}{/if}
					</span>
				{/if}
			</button>
		{/each}
	</div>
	{#if hidden > 0}
		<p class="absolute bottom-10 rounded-md bg-card px-2 py-0.5 text-xs text-muted-foreground">
			{t('organizations.topology.globe_far_side', { count: hidden })}
		</p>
	{/if}
</div>

<style>
	.sphere {
		fill: var(--lp-deep);
		stroke: color-mix(in srgb, var(--lp-ink) 12%, transparent);
	}
	.graticule {
		fill: none;
		stroke: color-mix(in srgb, var(--lp-ink) 6%, transparent);
	}
	.land {
		fill: color-mix(in srgb, var(--lp-ink) 16%, var(--lp-deep));
	}
	.arc {
		fill: none;
		stroke: var(--lp-accent);
		stroke-width: 1.5;
		stroke-dasharray: 4 4;
	}
</style>
