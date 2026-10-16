<script lang="ts">
	import { Button } from '#lib/components/ui/button/index.js';
	import GlobeView, { SKY, PRELOAD } from './server-globe/globe-gl-view.svelte';
	import SkeletonRows from '#lib/components/custom/skeleton-rows.svelte';
	import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';
	import type { ServerGeo } from '#lib/clients/server-client.js';
	import { isDaylight } from '#lib/utils/solar.js';
	import { i18n, t } from '#lib/i18n/index.svelte.js';
	import { markerColor, readPalette, stateColor, type GlobePalette } from './server-globe/palette';

	let {
		servers,
		loading = false,
		error = null,
		onRefresh,
		isCloud = false,
	}: {
		servers: ServerGeo[];
		loading?: boolean;
		error?: string | null;
		onRefresh: () => Promise<void>;
		/** Cloud mode: the anchor pin is the "Litepod Cloud" hub, not a main server. */
		isCloud?: boolean;
	} = $props();

	/** Palette key paired with the legend message it explains. */
	const LEGEND = [
		['accent', 'main'],
		['ok', 'ready'],
		['warn', 'degraded'],
		['muted', 'offline'],
	] as const;

	let palette = $state<GlobePalette | null>(null);
	let clock = $state(Date.now());
	let focus = $state<ServerGeo | null>(null);

	let located = $derived(
		servers.filter((server) => server.latitude !== null && server.longitude !== null),
	);
	let unlocated = $derived(servers.filter((server) => server.latitude === null));
	let main = $derived(located.find((server) => server.is_main) ?? null);
	let remotes = $derived(located.filter((server) => !server.is_main));
	// The local host is a real server and counts; the cloud hub is the control plane, not one.
	let fleet = $derived(isCloud ? servers.filter((server) => !server.is_main) : servers);

	/** Servers per legend tone, for the summary bar. */
	let summary = $derived.by(() => {
		if (!palette) return [];
		const current = palette;
		return LEGEND.slice(1).map(([tone, key]) => ({
			key,
			color: current[tone],
			count: fleet.filter((server) => stateColor(server, current) === current[tone]).length,
		}));
	});

	// The globe is painted from theme tokens, so it has to be rebuilt on a theme change.
	$effect(() => {
		const root = document.documentElement;
		const sync = () => (palette = readPalette());
		sync();
		const observer = new MutationObserver(sync);
		observer.observe(root, { attributes: true, attributeFilter: ['class', 'data-palette'] });
		return () => observer.disconnect();
	});

	// A minute is the finest the clock is ever read at.
	$effect(() => {
		const timer = setInterval(() => {
			if (!replaying) clock = Date.now();
		}, 30_000);
		return () => clearInterval(timer);
	});

	/** Dev only: replays the next total eclipse, six hours squeezed into twenty seconds. */
	let replaying = $state<'lunar' | 'solar' | null>(null);
	let view = $state<{ lat: number; lng: number; altitude: number } | null>(null);
	let replayFrame = 0;
	/** Dev only: bumped to send the easter-egg UFO now instead of at its hourly minute. */
	let ufo = $state(0);

	async function replayEclipse(kind: 'lunar' | 'solar') {
		cancelAnimationFrame(replayFrame);
		const lunar = await import('#lib/utils/lunar.js');
		let peak: number;
		if (kind === 'lunar') {
			peak = lunar.nextTotalLunarEclipse();
			const [lng, lat] = lunar.sublunarPoint(peak);
			// Just past the scene moon (2800 out), looking back through it at the earth.
			view = { lat, lng, altitude: 31 };
		} else {
			const eclipse = lunar.nextTotalSolarEclipse();
			peak = eclipse.peak;
			view = { lat: eclipse.lat, lng: eclipse.lng, altitude: 2.3 };
		}
		replaying = kind;
		const span = 6 * 3600e3;
		const started = performance.now();
		const step = (now: number) => {
			const progress = (now - started) / 20_000;
			if (progress >= 1) {
				replaying = null;
				view = null;
				clock = Date.now();
				return;
			}
			clock = peak - span / 2 + progress * span;
			replayFrame = requestAnimationFrame(step);
		};
		replayFrame = requestAnimationFrame(step);
	}

	$effect(() => () => cancelAnimationFrame(replayFrame));

	const utcTime = (ms: number) => new Date(ms).toISOString().slice(11, 16);

	function localTime(server: ServerGeo) {
		if (!server.timezone) return null;
		try {
			return new Intl.DateTimeFormat(i18n.locale, {
				timeZone: server.timezone,
				hour: '2-digit',
				minute: '2-digit',
				hour12: false,
			}).format(clock);
		} catch {
			// ip.guide returned a zone this browser does not know.
			return null;
		}
	}

	const place = (server: ServerGeo) =>
		[server.city, server.country].filter(Boolean).join(', ') ||
		t(`organizations.topology.globe_reason_${server.unlocated_reason ?? 'lookup_failed'}`);

	/** Glass over the night sky: the same in both themes, since the sky never changes. */
	const glass =
		'border border-white/12 bg-[rgba(10,12,16,.6)] text-[#e6e9ef] backdrop-blur-md hover:bg-[rgba(10,12,16,.75)] hover:text-white';
</script>

<!-- Start the texture downloads now, in parallel with the three.js chunk, instead of after it. -->
<svelte:head>
	{#each PRELOAD as href (href)}
		<link rel="preload" as="image" {href} crossorigin="anonymous" />
	{/each}
</svelte:head>

{#if error}
	<div
		class="rounded-lg border border-destructive/40 bg-destructive/10 p-4 text-sm text-destructive">
		{error}
	</div>
{:else if loading && servers.length === 0}
	<SkeletonRows rows={1} height="h-64" label={t('organizations.topology.globe_loading')} />
{:else if servers.length === 0}
	<p class="rounded-xl border border-dashed p-6 text-sm text-muted-foreground">
		{t('organizations.topology.globe_empty')}
	</p>
{:else}
	<div class="grid gap-4 min-[1100px]:grid-cols-[minmax(0,1fr)_300px]">
		<!-- The sky is plain CSS so it paints before three.js or any texture arrives. -->
		<div
			class="relative h-[56vh] min-h-[340px] overflow-hidden rounded-xl border border-foreground/10 bg-[#05060a] bg-cover bg-center sm:h-[min(62vh,560px)] sm:min-h-[380px]"
			style="background-image: url({SKY})">
			{#if palette}
				<GlobeView
					servers={located}
					{main}
					{palette}
					{clock}
					{focus}
					{view}
					{ufo}
					{isCloud}
					timeOf={localTime}
					onSelect={(server: ServerGeo) => (focus = server)} />
			{/if}

			<ul
				class="pointer-events-none absolute top-3 left-3 flex flex-col gap-1.5 text-[11px] text-[#c9ced8] sm:text-xs">
				{#each LEGEND as [tone, key] (key)}
					{@const labelKey = key === 'main' && isCloud ? 'cloud' : key}
					<li class="flex items-center gap-2">
						<span
							class="size-2 rounded-full"
							style="background: {palette ? palette[tone] : 'transparent'}"></span>
						{t(`organizations.topology.globe_legend_${labelKey}`)}
					</li>
				{/each}
			</ul>

			<div class="absolute top-3 right-3 flex items-center gap-2">
				<!-- Dev-only tooling, so not translated. -->
				{#if import.meta.env.DEV}
					{#each ['lunar', 'solar'] as const as kind (kind)}
						<Button
							variant="outline"
							size="sm"
							class={glass}
							disabled={replaying !== null}
							onclick={() => void replayEclipse(kind)}>
							{kind === 'lunar' ? 'Lunar eclipse' : 'Solar eclipse'}
						</Button>
					{/each}
					<Button variant="outline" size="sm" class={glass} onclick={() => ufo++}>UFO</Button>
				{/if}
				<span class="rounded-full px-2.5 py-1 font-mono text-xs {glass}">
					{#if replaying}{new Date(clock).toISOString().slice(0, 10)}{/if}
					{utcTime(clock)} UTC
				</span>
				<Button
					variant="outline"
					size="sm"
					class={glass}
					disabled={loading}
					onclick={() => void onRefresh()}>
					<RefreshCwIcon class={loading ? 'motion-safe:animate-spin' : ''} />
					{t('organizations.common.refresh')}
				</Button>
			</div>

			<div
				class="absolute inset-x-3 bottom-3 flex flex-wrap items-center justify-between gap-2 rounded-[14px] border border-white/10 bg-[rgba(10,12,16,.55)] py-2 pr-2.5 pl-3.5 text-[11.5px] text-[#d5dae3] shadow-[inset_0_1px_0_rgba(255,255,255,.06)] backdrop-blur-lg sm:text-[12.5px]">
				<div class="flex flex-wrap gap-x-3.5 gap-y-1">
					<span>
						<b class="font-mono font-semibold text-white">{fleet.length}</b>
						{t('organizations.topology.globe_summary_servers')}
					</span>
					{#each summary as item (item.key)}
						<span class="inline-flex items-center gap-1.5">
							<i class="size-[7px] rounded-full" style="background: {item.color}"></i>
							{t(`organizations.topology.globe_summary_${item.key}`, { count: item.count })}
						</span>
					{/each}
				</div>
				{#if main}
					<span class="hidden sm:inline">
						{t('organizations.topology.globe_hub')}:
						<b class="font-semibold text-white">{main.name}</b>{main.city
							? `, ${main.city.split(',')[0]}`
							: ''}
					</span>
				{/if}
			</div>
		</div>

		<aside
			class="flex flex-col gap-1.5 rounded-xl border border-foreground/9 bg-card p-2 lp-panel sm:p-3">
			<div class="flex items-baseline justify-between px-1 pt-0.5">
				<h2 class="text-sm font-semibold">{t('organizations.topology.globe_fleet')}</h2>
				<small class="font-mono text-[11.5px] text-muted-foreground">
					{t('organizations.topology.globe_counts', {
						located: located.length,
						unlocated: unlocated.length,
					})}
				</small>
			</div>

			{#snippet cap(label: string)}
				<p class="px-1 pt-2 pb-0.5 font-mono text-[11.5px] text-muted-foreground">{label}</p>
			{/snippet}

			{#snippet row(server: ServerGeo)}
				{@const time = localTime(server)}
				<button
					type="button"
					class="grid w-full grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-2.5 rounded-[10px] border px-2.5 py-2 text-left transition-colors hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none {server.is_main
						? 'border-primary/25 bg-primary/10'
						: 'border-transparent'}"
					onmouseenter={() => (focus = server)}
					onmouseleave={() => (focus = null)}
					onfocus={() => (focus = server)}
					onblur={() => (focus = null)}>
					<span
						class="size-2 rounded-full"
						style="background: {palette ? markerColor(server, palette) : 'transparent'}"></span>
					<span class="min-w-0">
						<span class="block truncate text-sm font-medium">{server.name}</span>
						<small class="block truncate text-xs text-muted-foreground">{place(server)}</small>
					</span>
					<span class="text-right text-xs text-muted-foreground">
						{#if time && server.latitude !== null && server.longitude !== null}
							<b class="block font-mono font-medium text-foreground">{time}</b>
							{isDaylight(server.latitude, server.longitude, clock)
								? t('organizations.topology.globe_day')
								: t('organizations.topology.globe_night')}
						{:else}
							<span class="rounded-full border px-1.5 py-0.5">
								{t('organizations.topology.globe_no_time')}
							</span>
						{/if}
					</span>
				</button>
			{/snippet}

			{#if main}
				{@render cap(
					isCloud
						? t('organizations.topology.globe_control_plane')
						: t('organizations.servers.this_machine'),
				)}
				{@render row(main)}
			{/if}

			{#if remotes.length > 0}
				{@render cap(t('organizations.topology.globe_servers_count', { count: remotes.length }))}
				{#each remotes as server (server.server_id)}{@render row(server)}{/each}
			{/if}

			{#if unlocated.length > 0}
				{@render cap(t('organizations.topology.globe_unlocated'))}
				{#each unlocated as server (server.server_id)}{@render row(server)}{/each}
			{/if}

			<p class="mt-2 px-1 pt-3 text-xs text-muted-foreground lp-groove-t">
				{t('organizations.topology.globe_source')}
			</p>
		</aside>
	</div>
{/if}
