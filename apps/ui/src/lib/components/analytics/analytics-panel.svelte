<script lang="ts">
	import ActivityIcon from '@lucide/svelte/icons/activity';
	import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';
	import GaugeIcon from '@lucide/svelte/icons/gauge';
	import ArrowUpFromLineIcon from '@lucide/svelte/icons/arrow-up-from-line';
	import ChartColumnIcon from '@lucide/svelte/icons/chart-column';
	import LayoutGridIcon from '@lucide/svelte/icons/layout-grid';
	import UsersIcon from '@lucide/svelte/icons/users';
	import AnalyticsDashboard from './analytics-dashboard.svelte';
	import CellChart from '#lib/components/custom/cell-chart.svelte';
	import SegmentedSwitch from '#lib/components/custom/segmented-switch.svelte';
	import KpiCard from '#lib/components/home/kpi-card.svelte';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import {
		ANALYTICS_RANGES,
		compactNumber,
		formatLatency,
		type Analytics,
		type AnalyticsRange,
	} from '#lib/clients/analytics-client.js';
	import { formatBytes } from '#lib/clients/container-metrics-client.js';
	import type { OrganizationApp } from '#lib/utils/organization-apps.js';
	import { toneDot, type StatusTone } from '#lib/utils/status-tone.js';
	import { i18n, t } from '#lib/i18n/index.svelte.js';

	/** The whole Analytics view: range, four headline numbers, one chart, short ranked lists. */
	let {
		load,
		apps,
		title,
		description,
	}: {
		load: (range: AnalyticsRange) => Promise<Analytics>;
		/** Organization view: names and links for the "Busiest apps" list. Omit for one app. */
		apps?: OrganizationApp[];
		/** Page heading; omit inside an app tab. */
		title?: string;
		description: string;
	} = $props();

	const VIEW_KEY = 'litepod:analytics-view';
	type View = 'overview' | 'dashboard';
	function storedView(): View {
		try {
			return localStorage.getItem(VIEW_KEY) === 'dashboard' ? 'dashboard' : 'overview';
		} catch {
			return 'overview';
		}
	}
	function saveView(next: View) {
		try {
			localStorage.setItem(VIEW_KEY, next);
		} catch {
			// A remembered view is a convenience; the toggle still works without it.
		}
	}

	let view = $state<View>(storedView());
	let range = $state<AnalyticsRange>('24h');
	let data = $state<Analytics | null>(null);
	let error = $state('');

	$effect(() => {
		const selected = range;
		let cancelled = false;
		data = null;
		error = '';
		load(selected).then(
			(result) => {
				if (!cancelled) data = result;
			},
			(caught) => {
				if (!cancelled)
					error = caught instanceof Error ? caught.message : t('analytics.panel.load_failed');
			},
		);
		return () => (cancelled = true);
	});

	type Row = { key: string; label: string; href?: string; value: number; tone?: StatusTone };

	let totals = $derived(data?.totals);
	let empty = $derived(!!data && data.totals.requests === 0);
	let errorRate = $derived(
		totals && totals.requests ? (totals.errors_5xx / totals.requests) * 100 : 0,
	);
	let requests = $derived(data?.series.map((point) => point.requests) ?? []);
	let peak = $derived(Math.max(0, ...requests));
	let appById = $derived(new Map((apps ?? []).map((app) => [app.application.id, app])));

	let lists = $derived.by(() => {
		if (!data) return [];
		const out: { title: string; rows: Row[]; mono?: boolean }[] = [];
		if (apps) {
			out.push({
				title: t('analytics.panel.top_apps'),
				rows: data.applications.slice(0, 6).map((entry) => {
					const app = appById.get(entry.key);
					return {
						key: entry.key,
						label: app?.application.name ?? t('analytics.panel.other_app'),
						href: app?.href,
						value: entry.requests,
					};
				}),
			});
		}
		out.push({
			title: t('analytics.panel.top_domains'),
			mono: true,
			rows: data.hosts
				.slice(0, 6)
				.map((entry) => ({ key: entry.key, label: entry.key, value: entry.requests })),
		});
		out.push({
			title: t('analytics.panel.top_pages'),
			mono: true,
			rows: data.paths
				.slice(0, 6)
				.map((entry) => ({ key: entry.key, label: entry.key, value: entry.requests })),
		});
		out.push({
			title: t('analytics.panel.top_referrers'),
			mono: true,
			rows: data.referrers
				.slice(0, 6)
				.map((entry) => ({ key: entry.key, label: entry.key, value: entry.requests })),
		});
		out.push({
			title: t('analytics.panel.clients'),
			rows: data.clients.slice(0, 6).map((entry) => ({
				key: entry.key,
				label: clientLabel(entry.key),
				value: entry.requests,
				tone: entry.key === 'bot' || entry.key === 'tool' ? 'info' : undefined,
			})),
		});
		out.push({
			title: t('analytics.panel.methods'),
			mono: true,
			rows: data.methods
				.slice(0, 6)
				.map((entry) => ({ key: entry.key, label: entry.key, value: entry.requests })),
		});
		out.push({
			title: t('analytics.panel.protocols'),
			mono: true,
			rows: data.protocols
				.slice(0, 6)
				.map((entry) => ({ key: entry.key, label: entry.key, value: entry.requests })),
		});
		out.push({
			title: t('analytics.panel.status_codes'),
			rows: data.statuses.slice(0, 6).map((entry) => ({
				key: String(entry.status),
				label: `${entry.status} ${statusLabel(entry.status)}`,
				value: entry.requests,
				tone: statusTone(entry.status),
			})),
		});
		return out;
	});

	const BROWSERS: Record<string, string> = {
		chrome: 'Chrome',
		safari: 'Safari',
		firefox: 'Firefox',
		edge: 'Edge',
		opera: 'Opera',
	};
	function clientLabel(key: string): string {
		if (BROWSERS[key]) return BROWSERS[key];
		if (key === 'bot' || key === 'tool' || key === 'unknown') return t(`analytics.client.${key}`);
		return t('analytics.client.other');
	}

	function statusTone(status: number): StatusTone {
		if (status >= 500) return 'danger';
		if (status >= 400) return 'warn';
		if (status >= 300) return 'info';
		return 'ok';
	}

	function statusLabel(status: number): string {
		if (status >= 500) return t('analytics.status.server_error');
		if (status >= 400) return t('analytics.status.client_error');
		if (status >= 300) return t('analytics.status.redirect');
		return t('analytics.status.success');
	}

	// Hover readout for the chart: when, how many, and which apps they went to.
	let hovered = $state<number | null>(null);
	const TOOLTIP_APPS = 5;
	function bucketTime(index: number): string {
		const point = data?.series[index];
		if (!point || !data) return '';
		const end = point.ts + data.bucket_seconds * 1000;
		const day: Intl.DateTimeFormatOptions = { weekday: 'short', month: 'short', day: 'numeric' };
		const time: Intl.DateTimeFormatOptions = { hour: '2-digit', minute: '2-digit' };
		const at = (ms: number, options: Intl.DateTimeFormatOptions) =>
			new Date(ms).toLocaleString(i18n.locale, options);
		return `${at(point.ts, { ...day, ...time })} – ${at(end, time)}`;
	}
	let tooltip = $derived.by(() => {
		const point = hovered === null ? undefined : data?.series[hovered];
		if (!point || hovered === null) return null;
		const entries = (point.applications ?? []).map((entry) => ({
			key: entry.key,
			label: appById.get(entry.key)?.application.name ?? t('analytics.panel.other_app'),
			requests: entry.requests,
		}));
		const rest = entries.slice(TOOLTIP_APPS).reduce((sum, entry) => sum + entry.requests, 0);
		return {
			left: ((hovered + 0.5) / requests.length) * 100,
			time: bucketTime(hovered),
			requests: point.requests,
			errors: point.errors_5xx,
			apps: entries.slice(0, TOOLTIP_APPS),
			rest,
		};
	});

	const count = (value: number) => compactNumber(value, i18n.locale);
</script>

<section class="flex min-w-0 flex-col gap-6">
	<div class="flex flex-wrap items-end justify-between gap-4">
		<div class="min-w-0">
			{#if title}
				<h1 class="text-2xl leading-tight font-semibold tracking-[-.035em] sm:text-[30px]">
					{title}
				</h1>
			{/if}
			<p class="mt-1.5 text-sm text-muted-foreground">{description}</p>
		</div>
		<div class="flex w-full flex-wrap items-center gap-2 sm:w-auto">
			<SegmentedSwitch
				items={[
					{ value: 'overview', label: t('analytics.panel.view_overview'), icon: LayoutGridIcon },
					{ value: 'dashboard', label: t('analytics.panel.view_dashboard'), icon: GaugeIcon },
				]}
				bind:value={view}
				onchange={saveView}
				label={t('analytics.panel.view')}
				class="w-full sm:w-64" />
			<SegmentedSwitch
				items={ANALYTICS_RANGES.map((value) => ({ value, label: t(`analytics.range.${value}`) }))}
				bind:value={range}
				label={t('analytics.panel.range')}
				class="w-full sm:w-80" />
		</div>
	</div>

	{#if error}
		<div class="rounded-2xl border border-destructive/30 bg-card p-6 text-sm text-destructive">
			{error}
		</div>
	{:else if !data}
		<div
			class="flex flex-col gap-6"
			role="status"
			aria-busy="true"
			aria-label={t('analytics.panel.loading')}>
			<div class="kpis">
				{#each { length: 5 } as _, index (index)}
					<Skeleton class="h-32 rounded-2xl" />
				{/each}
			</div>
			<Skeleton class="h-72 rounded-2xl" />
		</div>
	{:else if empty}
		<div
			class="flex flex-col items-center gap-3 rounded-2xl border border-dashed border-foreground/15 bg-card px-6 py-14 text-center">
			<span
				class="lp-inset grid size-11 place-items-center rounded-xl bg-muted text-muted-foreground">
				<ChartColumnIcon class="size-5" />
			</span>
			<h2 class="font-medium">{t('analytics.panel.empty_title')}</h2>
			<p class="max-w-md text-sm text-muted-foreground">{t('analytics.panel.empty_body')}</p>
		</div>
	{:else if view === 'dashboard'}
		<AnalyticsDashboard {data} {apps} />
	{:else if totals}
		<div class="kpis">
			<KpiCard
				title={t('analytics.panel.requests')}
				sub={t('analytics.panel.requests_sub')}
				value={count(totals.requests)}
				icon={ActivityIcon} />
			<KpiCard
				title={t('analytics.panel.visitors')}
				sub={t('analytics.panel.visitors_sub')}
				value={count(totals.visitors)}
				icon={UsersIcon} />
			<KpiCard
				title={t('analytics.panel.error_rate')}
				sub={t('analytics.panel.error_rate_sub')}
				value={`${errorRate < 10 ? errorRate.toFixed(1) : Math.round(errorRate)}%`}
				hint={t('analytics.panel.errors_count', { count: count(totals.errors_5xx) })}
				icon={TriangleAlertIcon} />
			<KpiCard
				title={t('analytics.panel.latency')}
				sub={t('analytics.panel.latency_sub')}
				value={formatLatency(totals.p95_ms)}
				hint={t('analytics.panel.avg_hint', { value: formatLatency(totals.avg_ms) })}
				icon={GaugeIcon} />
			<KpiCard
				title={t('analytics.panel.bandwidth')}
				sub={t('analytics.panel.bandwidth_sub')}
				value={formatBytes(totals.bytes_out)}
				icon={ArrowUpFromLineIcon} />
		</div>

		<div class="lp-raised flex min-w-0 flex-col gap-4 rounded-2xl border bg-card p-5">
			<div class="flex flex-wrap items-baseline justify-between gap-2">
				<h2 class="text-[15px] font-semibold tracking-tight">
					{t('analytics.panel.requests_over_time')}
				</h2>
				<span class="text-xs text-muted-foreground tabular-nums">
					{t('analytics.panel.peak', { value: count(peak) })}
				</span>
			</div>
			<div class="relative">
				<CellChart
					values={requests}
					cols={requests.length}
					rows={8}
					onhover={(index) => (hovered = index)}
					ariaLabel={t('analytics.panel.chart_aria')} />
				{#if tooltip}
					<div
						role="tooltip"
						class="pointer-events-none absolute bottom-full z-10 mb-2 w-max max-w-64 -translate-x-1/2 rounded-xl border bg-popover px-3 py-2 text-xs text-popover-foreground shadow-md"
						style="left: clamp(8rem, {tooltip.left}%, calc(100% - 8rem))">
						<p class="text-muted-foreground tabular-nums">{tooltip.time}</p>
						<p class="mt-0.5 text-sm font-semibold tabular-nums">
							{t('analytics.panel.requests_count', { count: count(tooltip.requests) })}
						</p>
						{#if tooltip.errors}
							<p class="text-destructive tabular-nums">
								{t('analytics.panel.errors_count', { count: count(tooltip.errors) })}
							</p>
						{/if}
						{#if tooltip.apps.length}
							<ul class="mt-1.5 flex flex-col gap-1 border-t pt-1.5">
								{#each tooltip.apps as app (app.key)}
									<li class="flex items-center justify-between gap-4">
										<span class="truncate">{app.label}</span>
										<span class="text-muted-foreground tabular-nums">{count(app.requests)}</span>
									</li>
								{/each}
								{#if tooltip.rest}
									<li class="flex justify-between gap-4 text-muted-foreground">
										<span>{t('analytics.panel.tooltip_more')}</span>
										<span class="tabular-nums">{count(tooltip.rest)}</span>
									</li>
								{/if}
							</ul>
						{/if}
					</div>
				{/if}
			</div>
		</div>

		<div class="lists" class:even={lists.length % 2 === 0}>
			{#each lists as list (list.title)}
				{@const top = Math.max(1, ...list.rows.map((row) => row.value))}
				<div class="lp-raised flex min-w-0 flex-col rounded-2xl border bg-card">
					<h2 class="lp-groove-b px-5 py-3.5 text-[15px] font-semibold tracking-tight">
						{list.title}
					</h2>
					{#if list.rows.length === 0}
						<p class="px-5 py-6 text-sm text-muted-foreground">{t('analytics.panel.no_rows')}</p>
					{:else}
						<ol class="flex flex-col gap-3.5 px-5 py-4">
							{#each list.rows as row (row.key)}
								<li class="flex min-w-0 flex-col gap-1.5">
									<span class="flex min-w-0 items-center justify-between gap-3 text-sm">
										<span class="flex min-w-0 items-center gap-2">
											{#if row.tone}
												<i class="size-2 shrink-0 rounded-full {toneDot(row.tone)}"></i>
											{/if}
											{#if row.href}
												<a
													href={row.href}
													class="truncate underline-offset-4 hover:underline {list.mono
														? 'font-mono text-[13px]'
														: ''}">{row.label}</a>
											{:else}
												<span class="truncate {list.mono ? 'font-mono text-[13px]' : ''}"
													>{row.label}</span>
											{/if}
										</span>
										<span class="shrink-0 text-muted-foreground tabular-nums"
											>{count(row.value)}</span>
									</span>
									<span class="lp-well block h-1 overflow-hidden rounded-full bg-muted">
										<i
											class="block h-full rounded-full {row.tone
												? toneDot(row.tone)
												: 'bg-primary'}"
											style="width: {(row.value / top) * 100}%"></i>
									</span>
								</li>
							{/each}
						</ol>
					{/if}
				</div>
			{/each}
		</div>
	{/if}
</section>

<style>
	.kpis {
		display: grid;
		grid-template-columns: repeat(auto-fit, minmax(min(220px, 100%), 1fr));
		gap: 12px;
	} /* Three lists side by side, four as a 2x2 grid; one column on phones. */
	.lists {
		display: grid;
		grid-template-columns: minmax(0, 1fr);
		gap: 16px;
		align-items: start;
	}
	@media (min-width: 768px) {
		.lists {
			grid-template-columns: repeat(2, minmax(0, 1fr));
		}
	}
	@media (min-width: 1100px) {
		.lists:not(.even) {
			grid-template-columns: repeat(3, minmax(0, 1fr));
		}
	}
</style>
