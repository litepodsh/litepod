<script lang="ts">
	import { onMount } from 'svelte';
	import LayersIcon from '@lucide/svelte/icons/layers';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import CellChart from '#lib/components/custom/cell-chart.svelte';
	import ChartTooltip from '#lib/components/custom/chart-tooltip.svelte';
	import KpiCard from '#lib/components/home/kpi-card.svelte';
	import { listServers } from '#lib/clients/server-client.js';
	import { getSystemVersion } from '#lib/clients/system-client.js';
	import { listPodmanResources, type PodmanRecord } from '#lib/clients/podman-console-client.js';
	import {
		formatBytes,
		getServerMetricsHistory,
		type MetricsHistory,
	} from '#lib/clients/container-metrics-client.js';
	import { serverGroups } from '#lib/utils/server-visibility.js';
	import { isRunning } from '#lib/utils/container-status.js';
	import { i18n, t } from '#lib/i18n/index.svelte.js';

	/** Every container on the organization's ready servers, with their summed CPU over 24h. */
	let { organizationId, href }: { organizationId: string; href: string } = $props();

	let loading = $state(true);
	let failed = $state(false);
	let containers = $state<PodmanRecord[]>([]);
	let serverCount = $state(0);

	// 48 columns of 30 minutes over the last 24h, summed across every ready server.
	const DAY = 24 * 3600_000;
	const COLS = 48;
	const SLOT_MS = DAY / COLS;
	const TOP = 3;
	type Usage = { name: string; cpu: number; mem: number };
	type Slot = { cpu: number; peak: number; mem: number; containers: Usage[] } | null;
	let slots = $state<Slot[]>([]);
	let start = $state(0);
	let cpu = $derived(slots.map((slot) => slot?.cpu ?? 0));
	let lastCpu = $derived(slots.findLast((slot) => slot !== null)?.cpu ?? 0);

	/**
	 * Per slot: each container's mean CPU and memory over its buckets, summed for the
	 * total; the peak is the busiest bucket's summed `cpu_max`.
	 */
	function toSlots(histories: { server: string; history: MetricsHistory }[], from: number): Slot[] {
		const acc = Array.from({ length: COLS }, () => ({
			containers: new Map<string, { name: string; cpu: number; mem: number; n: number }>(),
			peaks: new Map<number, number>(),
		}));
		const named = histories.length > 1;
		for (const { server, history } of histories) {
			for (const container of history.containers) {
				const name = named ? `${container.container_name} · ${server}` : container.container_name;
				for (const point of container.points) {
					const i = Math.floor((point.ts - from) / SLOT_MS);
					if (i < 0 || i >= COLS) continue;
					const entry = acc[i].containers.get(name) ?? { name, cpu: 0, mem: 0, n: 0 };
					entry.cpu += point.cpu_avg ?? 0;
					entry.mem += point.mem_avg ?? 0;
					entry.n += 1;
					acc[i].containers.set(name, entry);
					acc[i].peaks.set(point.ts, (acc[i].peaks.get(point.ts) ?? 0) + (point.cpu_max ?? 0));
				}
			}
		}
		return acc.map(({ containers, peaks }) => {
			if (!containers.size) return null;
			const usage = [...containers.values()]
				.map(({ name, cpu, mem, n }) => ({ name, cpu: cpu / n, mem: mem / n }))
				.sort((a, b) => b.cpu - a.cpu || a.name.localeCompare(b.name));
			return {
				cpu: usage.reduce((sum, c) => sum + c.cpu, 0),
				mem: usage.reduce((sum, c) => sum + c.mem, 0),
				peak: Math.max(...peaks.values()),
				containers: usage,
			};
		});
	}

	let hovered = $state<number | null>(null);
	let chartHost = $state<HTMLDivElement | null>(null);
	const percent = (v: number) => `${v < 10 ? v.toFixed(1) : Math.round(v)}%`;
	let tooltip = $derived.by(() => {
		const slot = hovered === null ? null : slots[hovered];
		if (hovered === null) return null;
		const from = start + hovered * SLOT_MS;
		const at = (ms: number, options: Intl.DateTimeFormatOptions) =>
			new Date(ms).toLocaleString(i18n.locale, options);
		const time = { hour: '2-digit', minute: '2-digit' } as const;
		return {
			time: `${at(from, { weekday: 'short', day: 'numeric', month: 'short', ...time })} – ${at(from + SLOT_MS, time)}`,
			slot,
			top: slot?.containers.slice(0, TOP) ?? [],
			rest: slot ? slot.containers.slice(TOP) : [],
		};
	});

	let running = $derived(
		containers.filter((c) => isRunning(String(c.State ?? c.state ?? ''))).length,
	);
	let pods = $derived(
		new Set(containers.map((c) => String(c.PodName ?? c.Pod ?? '')).filter(Boolean)).size,
	);

	onMount(async () => {
		try {
			const [servers, version] = await Promise.all([
				listServers(organizationId),
				getSystemVersion().catch(() => ({ is_cloud: false })),
			]);
			const groups = serverGroups(servers, version.is_cloud);
			const ready = [...groups.local, ...groups.remote].filter((s) => s.state === 'ready');
			serverCount = ready.length;
			// ponytail: one request per server; fine for a handful of servers.
			const lists = await Promise.allSettled(
				ready.map((s) => listPodmanResources(organizationId, s.id, 'containers')),
			);
			if (ready.length && lists.every((l) => l.status === 'rejected')) failed = true;
			containers = lists.flatMap((l) => (l.status === 'fulfilled' ? l.value : []));
			const histories = await Promise.allSettled(
				ready.map((s) =>
					getServerMetricsHistory(organizationId, s.id, '24h').then((history) => ({
						server: s.name,
						history,
					})),
				),
			);
			start = Date.now() - DAY;
			const built = toSlots(
				histories.flatMap((h) => (h.status === 'fulfilled' ? [h.value] : [])),
				start,
			);
			slots = built.some((slot) => slot !== null) ? built : [];
		} catch {
			failed = true;
		} finally {
			loading = false;
		}
	});
</script>

{#if loading}
	<Skeleton class="h-36 rounded-2xl" />
{:else}
	<KpiCard
		title={t('common.home.containers')}
		sub={failed
			? t('common.home.containers_unavailable')
			: t('common.home.containers_sub', { pods, servers: serverCount })}
		value={failed ? '-' : String(containers.length)}
		hint={failed ? '' : t('common.org_apps.running_count', { count: running })}
		{href}
		icon={LayersIcon}>
		{#if slots.length}
			<div class="relative" bind:this={chartHost}>
				<CellChart
					values={cpu}
					cols={COLS}
					rows={5}
					onhover={(i) => (hovered = i)}
					ariaLabel={t('common.home.containers_cpu')} />
			</div>
			<ChartTooltip host={chartHost} column={hovered} cols={COLS}>
				{#if tooltip?.slot}
					<p class="text-muted-foreground tabular-nums">{tooltip.time}</p>
					<p class="mt-0.5 flex items-baseline gap-2 tabular-nums">
						<b class="text-sm font-semibold">CPU {percent(tooltip.slot.cpu)}</b>
						<span class="text-muted-foreground">
							{t('common.home.containers_tooltip_peak', { value: percent(tooltip.slot.peak) })}
						</span>
					</p>
					<p class="tabular-nums">RAM {formatBytes(tooltip.slot.mem)}</p>
					<ul class="mt-1.5 flex flex-col gap-1 border-t pt-1.5">
						{#each tooltip.top as container (container.name)}
							<li class="flex justify-between gap-4">
								<span class="truncate font-mono text-[11.5px]">{container.name}</span>
								<span class="text-muted-foreground tabular-nums">{percent(container.cpu)}</span>
							</li>
						{/each}
						{#if tooltip.rest.length}
							<li class="flex justify-between gap-4 text-muted-foreground">
								<span>
									{t('common.home.containers_tooltip_others', { count: tooltip.rest.length })}
								</span>
								<span class="tabular-nums">
									{percent(tooltip.rest.reduce((sum, c) => sum + c.cpu, 0))}
								</span>
							</li>
						{/if}
					</ul>
				{:else if tooltip}
					<p class="mt-0.5">{t('servers.card.no_data')}</p>
				{/if}
			</ChartTooltip>
			<p class="mt-2 flex justify-between gap-2 text-xs text-muted-foreground">
				<span class="truncate">{t('common.home.containers_cpu')}</span>
				<span class="font-mono tabular-nums">{Math.round(lastCpu)}%</span>
			</p>
		{/if}
	</KpiCard>
{/if}
