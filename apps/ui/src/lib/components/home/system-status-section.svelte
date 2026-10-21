<script lang="ts">
	import { onMount, type Snippet } from 'svelte';
	import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';
	import NetworkIcon from '@lucide/svelte/icons/network';
	import ContainerIcon from '@lucide/svelte/icons/container';
	import CpuIcon from '@lucide/svelte/icons/cpu';
	import MemoryStickIcon from '@lucide/svelte/icons/memory-stick';
	import HardDriveIcon from '@lucide/svelte/icons/hard-drive';
	import DatabaseIcon from '@lucide/svelte/icons/database';
	import ZapIcon from '@lucide/svelte/icons/zap';
	import ChartLineIcon from '@lucide/svelte/icons/chart-line';
	import ActivityIcon from '@lucide/svelte/icons/activity';
	import CircleCheckIcon from '@lucide/svelte/icons/circle-check';
	import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import KpiCard from '#lib/components/home/kpi-card.svelte';
	import {
		toneClass,
		toneDot,
		toneFill,
		utilizationTone,
		type StatusTone,
	} from '#lib/utils/status-tone.js';
	import { Progress } from '#lib/components/ui/progress/index.js';
	import {
		getDatabaseStorage,
		getDebugOverview,
		type DatabaseStorage,
		type DebugOverview,
	} from '#lib/clients/debug-client.js';
	import { relativeTime } from '#lib/utils/relative-time.js';
	import { formatDuration } from '#lib/utils/duration.js';
	import { toast } from '#lib/utils/toast.js';
	import { i18n, t } from '#lib/i18n/index.svelte.js';

	type PodmanInfo = {
		info?: {
			host?: {
				cpus?: number;
				cpuUtilization?: { idlePercent?: number };
				memTotal?: number;
				memFree?: number;
				memAvailable?: number;
				hostname?: string;
				kernel?: string;
				uptime?: string;
				distribution?: { distribution?: string; version?: string };
			};
			store?: {
				graphRootAllocated?: number;
				graphRootUsed?: number;
			};
		};
		version?: { Version?: string; ApiVersion?: string };
	};

	type CaddyInfo = {
		config?: { apps?: Record<string, unknown> & { http?: { servers?: Record<string, unknown> } } };
	};

	// The debug page passes `showDatabase` and gets the full status board (summary, KPIs, service
	// details, SQLite usage); the home page keeps the compact two-card view.
	let { showDatabase = false }: { showDatabase?: boolean } = $props();

	let overview = $state<DebugOverview | null>(null);
	let podman = $state<PodmanInfo | null>(null);
	let loading = $state(true);
	let latencyMs = $state<number | null>(null);
	let checkedAt = $state<number | null>(null);
	let now = $state(Date.now());

	const services = [
		{ key: 'caddy' as const, title: 'Caddy', icon: NetworkIcon },
		{ key: 'podman' as const, title: 'Podman', icon: ContainerIcon },
	];

	const resourceTiles = $derived.by(() => {
		const host = podman?.info?.host;
		const store = podman?.info?.store;
		const tiles: {
			key: string;
			title: string;
			icon: typeof CpuIcon;
			percent: number;
			detail: string;
		}[] = [];

		if (host?.cpuUtilization?.idlePercent !== undefined) {
			const percent = Math.max(0, Math.min(100, 100 - host.cpuUtilization.idlePercent));
			tiles.push({
				key: 'cpu',
				title: t('common.home.cpu'),
				icon: CpuIcon,
				percent,
				detail: host.cpus ? `${host.cpus} vCPUs` : `${percent.toFixed(0)}%`,
			});
		}

		if (host?.memTotal && (host.memAvailable ?? host.memFree) !== undefined) {
			const memAvailable = host.memAvailable ?? host.memTotal - (host.memFree ?? 0);
			const used = host.memTotal - memAvailable;
			tiles.push({
				key: 'memory',
				title: t('common.home.memory'),
				icon: MemoryStickIcon,
				percent: Math.max(0, Math.min(100, (used / host.memTotal) * 100)),
				detail: `${formatBytes(used)} / ${formatBytes(host.memTotal)}`,
			});
		}

		if (store?.graphRootAllocated) {
			tiles.push({
				key: 'storage',
				title: t('common.home.storage'),
				icon: HardDriveIcon,
				percent: Math.max(
					0,
					Math.min(100, ((store.graphRootUsed ?? 0) / store.graphRootAllocated) * 100),
				),
				detail: `${formatBytes(store.graphRootUsed)} / ${formatBytes(store.graphRootAllocated)}`,
			});
		}

		return tiles;
	});

	let database = $state<DatabaseStorage | null>(null);
	const databaseFacts = $derived.by((): [string, string][] => {
		if (!database) return [];
		const total =
			(database.file_bytes ?? 0) + (database.wal_bytes ?? 0) + (database.shm_bytes ?? 0);
		const facts: [string, string][] = [
			[t('common.home.database_total'), formatBytes(total)],
			[t('common.home.database_file'), formatBytes(database.file_bytes)],
			[t('common.home.database_wal'), formatBytes(database.wal_bytes)],
			[t('common.home.database_shm'), formatBytes(database.shm_bytes)],
			[t('common.home.database_free'), formatBytes(database.free_bytes)],
		];
		if (database.page_count !== null && database.page_size !== null)
			facts.push([
				t('common.home.database_pages'),
				`${database.page_count.toLocaleString(i18n.locale)} × ${formatBytes(database.page_size)}`,
			]);
		return facts;
	});

	const metricsStats = $derived(database?.metrics?.stats ?? null);
	const metricsFacts = $derived.by((): [string, string][] => {
		if (!metricsStats) return [];
		const when = (ms: number | null) => (ms === null ? '—' : relativeTime(ms, now, i18n.locale));
		const facts: [string, string][] = [
			[t('common.home.database_file'), formatBytes(metricsStats.file_bytes)],
		];
		if (metricsStats.wal_bytes !== null)
			facts.push([t('common.home.database_wal'), formatBytes(metricsStats.wal_bytes)]);
		facts.push(
			[t('common.home.metrics_samples'), metricsStats.sample_count.toLocaleString(i18n.locale)],
			[
				t('common.home.metrics_containers'),
				metricsStats.container_count.toLocaleString(i18n.locale),
			],
			[
				t('common.home.metrics_request_rows'),
				(metricsStats.request_rows ?? 0).toLocaleString(i18n.locale),
			],
			[t('common.home.metrics_oldest'), when(metricsStats.oldest_sample_at)],
			[t('common.home.metrics_newest'), when(metricsStats.newest_sample_at)],
			[t('common.home.metrics_interval'), formatDuration(metricsStats.sample_interval_secs)],
			[
				t('common.home.metrics_retention'),
				t('common.home.metrics_retention_days', { count: metricsStats.default_retention_days }),
			],
		);
		return facts;
	});

	const dragonflyStats = $derived(database?.dragonfly?.stats ?? null);
	const dragonflyFacts = $derived.by((): [string, string][] => {
		const stats = dragonflyStats;
		if (!stats) return [];
		const count = (value: number | null) =>
			value === null ? '—' : value.toLocaleString(i18n.locale);
		const lookups = (stats.keyspace_hits ?? 0) + (stats.keyspace_misses ?? 0);
		const facts: [string, string | null][] = [
			[t('operations.debug.version'), stats.version],
			[
				t('operations.debug.uptime'),
				stats.uptime_secs === null ? null : formatDuration(stats.uptime_secs),
			],
			[t('common.home.dragonfly_keys'), count(stats.key_count)],
			[t('common.home.dragonfly_memory'), formatBytes(stats.used_memory_bytes)],
			[t('common.home.dragonfly_peak_memory'), formatBytes(stats.peak_memory_bytes)],
			[
				t('common.home.dragonfly_max_memory'),
				stats.max_memory_bytes === 0
					? t('common.home.dragonfly_no_limit')
					: formatBytes(stats.max_memory_bytes),
			],
			[t('common.home.dragonfly_clients'), count(stats.connected_clients)],
			[t('common.home.dragonfly_commands'), count(stats.total_commands)],
			[
				t('common.home.dragonfly_hit_rate'),
				lookups === 0 ? '—' : `${(((stats.keyspace_hits ?? 0) / lookups) * 100).toFixed(1)}%`,
			],
			[t('common.home.dragonfly_expired'), count(stats.expired_keys)],
			[t('common.home.dragonfly_evicted'), count(stats.evicted_keys)],
		];
		return facts.filter((fact): fact is [string, string] => Boolean(fact[1]));
	});

	/** Key counts per `litepod:<group>` prefix, relative to the largest group. */
	const dragonflyPrefixBars = $derived.by(() => {
		const prefixes = dragonflyStats?.prefixes ?? [];
		const max = Math.max(1, ...prefixes.map((prefix) => prefix.keys));
		return prefixes.map((prefix) => ({
			name: prefix.name,
			percent: (prefix.keys / max) * 100,
			label: prefix.keys.toLocaleString(i18n.locale),
		}));
	});

	/** Bytes when dbstat exists, otherwise row counts; bars are relative to the largest table. */
	const tableBars = $derived.by(() => {
		if (!database) return [];
		const amount = (table: DatabaseStorage['tables'][number]) =>
			(database!.per_table_mode === 'bytes' ? table.bytes : table.rows) ?? 0;
		const max = Math.max(1, ...database.tables.map(amount));
		return database.tables.map((table) => ({
			name: table.name,
			percent: (amount(table) / max) * 100,
			label:
				database!.per_table_mode === 'bytes'
					? formatBytes(table.bytes)
					: t('common.home.database_rows', {
							count: (table.rows ?? 0).toLocaleString(i18n.locale),
						}),
		}));
	});

	/** DuckDB tables: bytes on disk, rows beside them since fresh rows sit in the WAL. */
	const metricsTableBars = $derived.by(() => {
		const tables = metricsStats?.tables ?? [];
		const max = Math.max(1, ...tables.map((table) => table.bytes));
		return tables.map((table) => ({
			name: table.name,
			percent: (table.bytes / max) * 100,
			label: `${formatBytes(table.bytes)} · ${t('common.home.database_rows', {
				count: table.rows.toLocaleString(i18n.locale),
			})}`,
		}));
	});

	const downCount = $derived(
		overview ? services.filter((service) => overview![service.key].status !== 'up').length : 0,
	);
	const overallTone = $derived<StatusTone>(
		downCount === 0 ? 'ok' : downCount === services.length ? 'danger' : 'warn',
	);

	/** What each service reported about itself, as label/value rows; unknown facts are left out. */
	function serviceFacts(key: 'caddy' | 'podman'): [string, string][] {
		const current = overview?.[key];
		if (!current || current.status !== 'up') return [];
		let facts: [string, string | undefined][];
		if (key === 'podman') {
			const detail = current.detail as PodmanInfo;
			const host = detail.info?.host;
			const distro = host?.distribution;
			facts = [
				[t('operations.debug.version'), detail.version?.Version],
				[t('operations.debug.api_version'), detail.version?.ApiVersion],
				// Podman appends "(Approximately N days)"; the first part is enough.
				[t('operations.debug.uptime'), host?.uptime?.replace(/\s*\(.*\)$/, '')],
				[t('operations.debug.hostname'), host?.hostname],
				[
					t('operations.debug.os'),
					[distro?.distribution, distro?.version].filter(Boolean).join(' ') || undefined,
				],
				[t('operations.debug.kernel'), host?.kernel],
			];
		} else {
			const apps = (current.detail as CaddyInfo).config?.apps;
			facts = [
				[t('operations.debug.caddy_apps'), apps ? Object.keys(apps).join(', ') : undefined],
				[
					t('operations.debug.http_servers'),
					apps?.http?.servers ? String(Object.keys(apps.http.servers).length) : undefined,
				],
			];
		}
		return facts.filter((fact): fact is [string, string] => Boolean(fact[1]));
	}

	function downMessage(key: 'caddy' | 'podman'): string | null {
		const current = overview?.[key];
		if (!current || current.status === 'up') return null;
		return typeof current.detail?.message === 'string' ? current.detail.message : null;
	}

	function formatBytes(bytes: number | null | undefined): string {
		if (bytes === undefined || bytes === null || Number.isNaN(bytes)) return '—';
		const units = ['B', 'KB', 'MB', 'GB', 'TB'];
		let value = bytes;
		let unitIndex = 0;
		while (value >= 1024 && unitIndex < units.length - 1) {
			value /= 1024;
			unitIndex++;
		}
		return `${value.toFixed(unitIndex === 0 || value >= 10 ? 0 : 1)} ${units[unitIndex]}`;
	}

	async function refreshDatabase() {
		try {
			database = await getDatabaseStorage();
		} catch (error) {
			toast.error(error instanceof Error ? error.message : t('common.home.database_load_failed'));
		}
	}

	async function refresh() {
		loading = true;
		if (showDatabase) void refreshDatabase();
		const started = performance.now();
		try {
			overview = await getDebugOverview();
			latencyMs = Math.round(performance.now() - started);
			checkedAt = Date.now();
			podman = overview.podman.status === 'up' ? (overview.podman.detail as PodmanInfo) : null;
		} catch (error) {
			toast.error(
				error instanceof Error ? error.message : t('common.home.system_status_load_failed'),
			);
		} finally {
			loading = false;
		}
	}

	onMount(() => {
		void refresh();
		// Only the board shows "checked N ago", so only it needs a ticking clock.
		if (!showDatabase) return;
		const timer = setInterval(() => (now = Date.now()), 15_000);
		return () => clearInterval(timer);
	});
</script>

<!-- `relative` bars (a table's share of the largest one) are not utilisation, so stay neutral. -->
{#snippet meter(percent: number, relative = false)}
	{@const tone = relative ? 'ok' : utilizationTone(percent)}
	<span class="lp-well block h-1.5 overflow-hidden rounded-full bg-muted">
		<i
			class="block h-full rounded-full motion-safe:transition-[width] motion-safe:duration-300"
			style="width: {percent}%; background: {tone === 'ok'
				? 'var(--muted-foreground)'
				: toneFill(tone)}"></i>
	</span>
{/snippet}

{#snippet statusPill(up: boolean)}
	<span
		class="inline-flex shrink-0 items-center gap-1.5 rounded-full px-2 py-0.5 text-xs font-medium {toneClass(
			up ? 'ok' : 'danger',
		)}">
		<i class="size-1.5 rounded-full {toneDot(up ? 'ok' : 'danger')}"></i>
		{up ? t('common.home.online') : t('common.home.unavailable')}
	</span>
{/snippet}

<!-- One card look for every service and store on the board. -->
{#snippet statusCard(
	Icon: typeof DatabaseIcon,
	title: string,
	up: boolean,
	facts: [string, string][],
	message: string | null,
	extra?: Snippet,
)}
	<article class="lp-raised min-w-0 overflow-hidden rounded-2xl border bg-card">
		<header class="lp-groove-b flex items-center justify-between gap-3 px-4 py-3">
			<span class="flex min-w-0 items-center gap-2.5">
				<span
					class="lp-inset grid size-[30px] shrink-0 place-items-center rounded-lg bg-muted text-muted-foreground">
					<Icon class="size-4" />
				</span>
				<h3 class="truncate text-[15px] font-semibold tracking-tight">{title}</h3>
			</span>
			{@render statusPill(up)}
		</header>
		<div class="flex flex-col gap-4 p-4">
			{#if message}
				<p
					class="rounded-lg border border-destructive/30 bg-destructive/5 p-3 font-mono text-xs break-words text-destructive">
					{message}
				</p>
			{:else if facts.length}
				<dl class="grid grid-cols-2 gap-2.5">
					{#each facts as [label, value] (label)}
						<div class="lp-inset flex min-w-0 flex-col gap-0.5 rounded-lg bg-muted/60 px-3 py-2.5">
							<dt class="font-mono text-[11.5px] text-muted-foreground">{label}</dt>
							<dd class="truncate text-[13px] font-medium tabular-nums" title={value}>{value}</dd>
						</div>
					{/each}
				</dl>
			{:else}
				<p class="text-sm text-muted-foreground">
					{up ? t('common.home.online') : t('common.home.unavailable')}
				</p>
			{/if}
			{@render extra?.()}
		</div>
	</article>
{/snippet}

{#snippet sqliteTables()}
	{#if database?.per_table_mode === 'rows'}
		<p class="text-xs text-muted-foreground">{t('common.home.database_rows_mode')}</p>
	{/if}
	{@render tableList(tableBars, t('common.home.database_tables'))}
{/snippet}

{#snippet metricsTables()}
	{@render tableList(metricsTableBars, t('common.home.database_tables'))}
{/snippet}

{#snippet dragonflyPrefixes()}
	{@render tableList(dragonflyPrefixBars, t('common.home.dragonfly_prefixes'))}
	{#if dragonflyStats?.prefixes_truncated}
		<p class="text-xs text-muted-foreground">
			{t('common.home.dragonfly_prefixes_truncated', {
				count: (10_000).toLocaleString(i18n.locale),
			})}
		</p>
	{/if}
{/snippet}

{#snippet tableList(bars: { name: string; percent: number; label: string }[], heading: string)}
	{#if bars.length > 0}
		<div class="flex flex-col gap-2">
			<p class="font-mono text-[11px] tracking-[.04em] text-muted-foreground uppercase">
				{heading}
			</p>
			<div class="flex max-h-80 flex-col gap-2 overflow-y-auto">
				{#each bars as table (table.name)}
					<div class="flex items-center gap-3 text-sm">
						<span class="min-w-0 flex-1 truncate font-mono text-xs" title={table.name}
							>{table.name}</span>
						<span class="w-20 shrink-0 sm:w-32">{@render meter(table.percent, true)}</span>
						<span class="min-w-20 shrink-0 text-right text-xs tabular-nums text-muted-foreground"
							>{table.label}</span>
					</div>
				{/each}
			</div>
		</div>
	{/if}
{/snippet}

{#if showDatabase}
	<!-- Debug: full status board -->
	<section class="flex flex-col gap-5" aria-label={t('common.home.system_status')}>
		{#if !overview && loading}
			<Skeleton class="h-[74px] rounded-2xl" />
			<div class="kpis">
				{#each { length: 4 } as _, index (index)}
					<Skeleton class="h-32 rounded-2xl" />
				{/each}
			</div>
			<div class="grid gap-4 md:grid-cols-2">
				<Skeleton class="h-48 rounded-2xl" />
				<Skeleton class="h-48 rounded-2xl" />
			</div>
		{:else if overview}
			<div class="lp-raised flex flex-wrap items-center gap-3.5 rounded-2xl border bg-card p-4">
				<span
					class="lp-inset grid size-10 shrink-0 place-items-center rounded-xl {toneClass(
						overallTone,
					)}">
					{#if overallTone === 'ok'}
						<CircleCheckIcon class="size-5" />
					{:else}
						<CircleAlertIcon class="size-5" />
					{/if}
				</span>
				<div class="min-w-0 flex-1">
					<p class="text-[15px] font-semibold tracking-tight" role="status">
						{downCount === 0
							? t('operations.debug.all_operational')
							: t('operations.debug.degraded', { count: downCount, total: services.length })}
					</p>
					<p class="flex flex-wrap gap-x-2 text-xs text-muted-foreground">
						{#if checkedAt}
							<span
								>{t('operations.debug.checked', {
									time: relativeTime(checkedAt, now, i18n.locale),
								})}</span>
						{/if}
						{#if latencyMs !== null}
							<span class="font-mono">{t('operations.debug.latency', { ms: latencyMs })}</span>
						{/if}
					</p>
				</div>
				<Button variant="outline" size="sm" onclick={refresh} disabled={loading}>
					<RefreshCwIcon class={loading ? 'size-4 motion-safe:animate-spin' : 'size-4'} />
					{t('common.common.refresh')}
				</Button>
			</div>

			<div class="kpis">
				<KpiCard
					title={t('operations.debug.services_up')}
					value="{services.length - downCount}/{services.length}"
					icon={ActivityIcon}
					ticks={services.map((service) =>
						overview![service.key].status === 'up' ? 'ok' : 'danger',
					)} />
				{#each resourceTiles as tile (tile.key)}
					<KpiCard
						title={tile.title}
						value="{tile.percent.toFixed(0)}%"
						hint={tile.detail}
						icon={tile.icon}>
						{@render meter(tile.percent)}
					</KpiCard>
				{/each}
			</div>
			{#if resourceTiles.length > 0}
				<p class="-mt-2 text-xs text-muted-foreground">{t('common.home.storage_hint')}</p>
			{/if}

			<div class="flex flex-col gap-3">
				<h2 class="text-[15px] font-semibold tracking-tight">{t('operations.debug.services')}</h2>
				<div class="grid gap-4 md:grid-cols-2">
					{#each services as service (service.key)}
						{@render statusCard(
							service.icon,
							service.title,
							overview[service.key].status === 'up',
							serviceFacts(service.key),
							downMessage(service.key),
						)}
					{/each}
				</div>
			</div>
		{:else}
			<div class="rounded-2xl border border-dashed p-6 text-center text-sm text-muted-foreground">
				{t('common.home.status_unavailable')}
			</div>
		{/if}

		<!-- Loaded on its own, so it still shows when Caddy or Podman are down. -->
		{#if database}
			<div class="flex flex-col gap-3">
				<h2 class="text-[15px] font-semibold tracking-tight">{t('operations.debug.storage')}</h2>
				<div class="grid items-start gap-4 md:grid-cols-2">
					{@render statusCard(DatabaseIcon, 'SQLite', true, databaseFacts, null, sqliteTables)}
					{@render statusCard(
						ChartLineIcon,
						t('common.home.database_metrics'),
						metricsStats !== null,
						metricsFacts,
						database.metrics?.error ?? null,
						metricsTables,
					)}
					{@render statusCard(
						ZapIcon,
						'DragonflyDB',
						dragonflyStats !== null,
						dragonflyFacts,
						database.dragonfly?.error ?? null,
						dragonflyPrefixes,
					)}
				</div>
			</div>
		{/if}
	</section>
{:else}
	<!-- Home: compact view -->
	<section class="flex flex-col gap-3">
		<div class="flex items-center justify-between">
			<h2 class="text-[15px] font-semibold tracking-tight">{t('common.home.system_status')}</h2>
			<Button variant="ghost" size="sm" onclick={refresh} disabled={loading}>
				<RefreshCwIcon class={loading ? 'size-4 motion-safe:animate-spin' : 'size-4'} />
				{t('common.common.refresh')}
			</Button>
		</div>

		{#if !overview && loading}
			<div class="grid gap-4 sm:grid-cols-2">
				<Skeleton class="h-[74px] rounded-2xl" />
				<Skeleton class="h-[74px] rounded-2xl" />
			</div>
		{:else if overview}
			<div class="grid gap-4 sm:grid-cols-2">
				{#each services as service (service.key)}
					{@const Icon = service.icon}
					<div class="lp-raised rounded-2xl border bg-card p-4 text-sm">
						<div class="flex flex-row items-center justify-between gap-3">
							<div class="flex items-center gap-3">
								<div
									class="lp-inset grid size-[30px] place-items-center rounded-lg bg-muted text-muted-foreground">
									<Icon class="size-4" />
								</div>
								<span class="font-medium">{service.title}</span>
							</div>
							{@render statusPill(overview[service.key].status === 'up')}
						</div>
					</div>
				{/each}
			</div>

			{#if resourceTiles.length > 0}
				<div class="grid gap-4 sm:grid-cols-3">
					{#each resourceTiles as tile (tile.key)}
						{@const Icon = tile.icon}
						<div class="lp-raised rounded-2xl border bg-card p-4 text-sm">
							<div class="flex flex-col gap-2">
								<div class="flex items-center gap-2 text-muted-foreground">
									<Icon class="size-4" />
									<span class="text-sm font-medium text-foreground">{tile.title}</span>
								</div>
								<Progress value={tile.percent} />
								<div class="flex items-center justify-between text-xs text-muted-foreground">
									<span>{tile.detail}</span>
									<span>{tile.percent.toFixed(0)}%</span>
								</div>
							</div>
						</div>
					{/each}
				</div>
				<p class="text-xs text-muted-foreground">
					{t('common.home.storage_hint')}
				</p>
			{/if}
		{:else}
			<div class="rounded-2xl border border-dashed p-6 text-center text-sm text-muted-foreground">
				{t('common.home.status_unavailable')}
			</div>
		{/if}
	</section>
{/if}

<style>
	.kpis {
		display: grid;
		grid-template-columns: repeat(4, minmax(0, 1fr));
		gap: 12px;
	}
	@media (max-width: 1100px) {
		.kpis {
			grid-template-columns: repeat(2, minmax(0, 1fr));
		}
	}
	@media (max-width: 420px) {
		.kpis {
			grid-template-columns: minmax(0, 1fr);
		}
	}
</style>
