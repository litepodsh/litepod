<script lang="ts">
	import ActivityIcon from '@lucide/svelte/icons/activity';
	import GaugeIcon from '@lucide/svelte/icons/gauge';
	import NetworkIcon from '@lucide/svelte/icons/network';
	import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';
	import RecentVisitors from './recent-visitors.svelte';
	import { clientLabel } from './client-label';
	import CellChart from '#lib/components/custom/cell-chart.svelte';
	import KpiCard from '#lib/components/home/kpi-card.svelte';
	import * as Sheet from '#lib/components/ui/sheet/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import {
		compactNumber,
		formatLatency,
		type AnalyticsRange,
		type PathAnalytics,
	} from '#lib/clients/analytics-client.js';
	import { countryName, flag } from '#lib/utils/countries.js';
	import { i18n, t } from '#lib/i18n/index.svelte.js';

	/** One endpoint: how often it is called, by how many IPs, and from where. */
	let {
		path = $bindable(null),
		range,
		load,
	}: {
		/** The endpoint shown; null keeps the sheet closed. */
		path?: string | null;
		range: AnalyticsRange;
		load: (path: string, range: AnalyticsRange) => Promise<PathAnalytics>;
	} = $props();

	let data = $state<PathAnalytics | null>(null);
	let error = $state('');
	let hovered = $state<number | null>(null);

	$effect(() => {
		const selected = path;
		const span = range;
		data = null;
		error = '';
		hovered = null;
		if (!selected) return;
		let cancelled = false;
		load(selected, span).then(
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

	const count = (value: number) => compactNumber(value, i18n.locale);
	let requests = $derived(data?.series.map((point) => point.requests) ?? []);
	let point = $derived(hovered === null ? undefined : data?.series[hovered]);
	const when = (ms: number) =>
		new Date(ms).toLocaleString(i18n.locale, {
			month: 'short',
			day: 'numeric',
			hour: '2-digit',
			minute: '2-digit',
		});

	let groups = $derived(
		data
			? [
					{
						title: t('analytics.endpoint.countries'),
						rows: data.countries.map((entry) => ({
							...entry,
							label: `${flag(entry.key)} ${countryName(entry.key, i18n.locale)}`,
						})),
					},
					{
						title: t('analytics.endpoint.regions'),
						rows: data.regions.map((entry) => ({ ...entry, label: entry.key })),
					},
					{
						title: t('analytics.endpoint.cities'),
						rows: data.cities.map((entry) => ({ ...entry, label: entry.key })),
					},
					{
						title: t('analytics.endpoint.clients'),
						rows: data.clients.map((entry) => ({ ...entry, label: clientLabel(entry.key) })),
					},
					{
						title: t('analytics.endpoint.agents'),
						rows: data.agents.map((entry) => ({ ...entry, label: entry.key })),
					},
				].filter((group) => group.rows.length > 0)
			: [],
	);
</script>

<Sheet.Root
	open={path !== null}
	onOpenChange={(open) => {
		if (!open) path = null;
	}}>
	<Sheet.Content
		side="right"
		class="overflow-y-auto data-[side=right]:w-full data-[side=right]:sm:max-w-4xl">
		<Sheet.Header>
			<Sheet.Title class="font-mono break-all">{path}</Sheet.Title>
			<Sheet.Description>{t('analytics.endpoint.description')}</Sheet.Description>
		</Sheet.Header>

		<div class="flex min-w-0 flex-col gap-4 px-4 pb-6">
			{#if error}
				<p class="rounded-xl border border-destructive/30 p-4 text-sm text-destructive">{error}</p>
			{:else if !data}
				<div role="status" aria-busy="true" aria-label={t('analytics.panel.loading')}>
					<Skeleton class="h-72 rounded-2xl" />
				</div>
			{:else if data.requests === 0}
				<p class="py-10 text-center text-sm text-muted-foreground">
					{t('analytics.endpoint.no_requests')}
				</p>
			{:else}
				<div class="kpis">
					<KpiCard
						title={t('analytics.panel.requests')}
						value={count(data.requests)}
						icon={ActivityIcon} />
					<KpiCard
						title={t('analytics.endpoint.unique_ips')}
						sub={t('analytics.endpoint.unique_ips_sub')}
						value={count(data.ips)}
						icon={NetworkIcon} />
					<KpiCard
						title={t('analytics.dashboard.server_errors')}
						value={count(data.errors)}
						icon={TriangleAlertIcon} />
					<KpiCard
						title={t('analytics.panel.latency')}
						sub={t('analytics.panel.latency_sub')}
						value={formatLatency(data.p95_ms)}
						icon={GaugeIcon} />
				</div>

				<div class="lp-raised flex min-w-0 flex-col gap-3 rounded-2xl border bg-card p-4">
					<div class="flex flex-wrap items-baseline justify-between gap-2">
						<h3 class="text-[15px] font-semibold tracking-tight">
							{t('analytics.panel.requests_over_time')}
						</h3>
						{#if point}
							<span class="text-xs text-muted-foreground tabular-nums">
								{when(point.ts)} · {count(point.requests)} · {count(point.ips)} IP
							</span>
						{/if}
					</div>
					<CellChart
						values={requests}
						cols={requests.length}
						rows={6}
						onhover={(index) => (hovered = index)}
						ariaLabel={t('analytics.panel.chart_aria')} />
				</div>

				{#if data.ips === 0}
					<p class="rounded-xl border p-4 text-sm text-muted-foreground">
						{t('analytics.endpoint.no_ips')}
					</p>
				{:else}
					<div class="groups">
						{#each groups as group (group.title)}
							{@const top = Math.max(1, ...group.rows.map((row) => row.requests))}
							<section class="lp-raised flex min-w-0 flex-col rounded-2xl border bg-card">
								<header class="lp-groove-b flex items-baseline justify-between gap-3 px-4 py-3">
									<h3 class="text-[15px] font-semibold tracking-tight">{group.title}</h3>
									<span class="shrink-0 text-xs text-muted-foreground">
										{t('analytics.endpoint.row_legend')}
									</span>
								</header>
								<ol class="flex max-h-72 flex-col gap-3 overflow-y-auto px-4 py-3">
									{#each group.rows as row (row.key)}
										<li class="flex min-w-0 flex-col gap-1.5">
											<span class="flex min-w-0 items-center justify-between gap-3 text-sm">
												<span class="truncate" title={row.label}>{row.label}</span>
												<span class="shrink-0 text-muted-foreground tabular-nums">
													{count(row.requests)} · {count(row.ips)} IP
												</span>
											</span>
											<span class="lp-well block h-1 overflow-hidden rounded-full bg-muted">
												<i
													class="block h-full rounded-full bg-primary"
													style="width: {(row.requests / top) * 100}%"></i>
											</span>
										</li>
									{/each}
								</ol>
							</section>
						{/each}
					</div>

					<RecentVisitors
						visitors={data.visitors}
						title={t('analytics.endpoint.top_ips')}
						subtitle={t('analytics.endpoint.top_ips_sub', {
							shown: data.visitors.length,
							total: data.ips,
						})} />
				{/if}
			{/if}
		</div>
	</Sheet.Content>
</Sheet.Root>

<style>
	.kpis {
		display: grid;
		grid-template-columns: repeat(auto-fit, minmax(min(180px, 100%), 1fr));
		gap: 12px;
	}
	.groups {
		display: grid;
		grid-template-columns: repeat(auto-fit, minmax(min(300px, 100%), 1fr));
		gap: 12px;
		align-items: start;
	}
</style>
