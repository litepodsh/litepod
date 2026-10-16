<script lang="ts">
	import LinePanel from './line-panel.svelte';
	import StatTile from './stat-tile.svelte';
	import {
		compactNumber,
		formatLatency,
		type Analytics,
		type TopEntry,
	} from '#lib/clients/analytics-client.js';
	import { formatBytes } from '#lib/clients/container-metrics-client.js';
	import type { OrganizationApp } from '#lib/utils/organization-apps.js';
	import { i18n, t } from '#lib/i18n/index.svelte.js';

	/** The dense, Grafana-style view of the same data as the overview. */
	let { data, apps }: { data: Analytics; apps?: OrganizationApp[] } = $props();

	const count = (value: number) => compactNumber(value, i18n.locale);
	const percent = (value: number) => `${value < 10 ? value.toFixed(1) : Math.round(value)}%`;
	const labels = $derived({
		name: t('analytics.dashboard.name'),
		last: t('analytics.dashboard.last'),
		min: t('analytics.dashboard.min'),
		max: t('analytics.dashboard.max'),
	});

	let s = $derived(data.series);
	let ts = $derived(s.map((p) => p.ts));
	const share = (part: (p: (typeof s)[number]) => number) =>
		s.map((p) => (p.requests ? (part(p) / p.requests) * 100 : null));
	let appById = $derived(new Map((apps ?? []).map((app) => [app.application.id, app])));

	type Gauge = {
		title: string;
		head: string;
		rows: (TopEntry & { label: string; href?: string })[];
	};
	let gauges = $derived.by(() => {
		const out: Gauge[] = [
			{
				title: t('analytics.dashboard.top_pages'),
				head: t('analytics.dashboard.page'),
				rows: data.paths.map((row) => ({ ...row, label: row.key })),
			},
			{
				title: t('analytics.dashboard.top_domains'),
				head: t('analytics.dashboard.domain'),
				rows: data.hosts.map((row) => ({ ...row, label: row.key })),
			},
		];
		if (apps)
			out.push({
				title: t('analytics.dashboard.top_apps'),
				head: t('analytics.dashboard.app'),
				rows: data.applications.slice(0, 10).map((row) => {
					const app = appById.get(row.key);
					return {
						...row,
						label: app?.application.name ?? t('analytics.panel.other_app'),
						href: app?.href,
					};
				}),
			});
		out.push({
			title: t('analytics.dashboard.status_codes'),
			head: t('analytics.dashboard.status'),
			rows: data.statuses.map((row) => ({
				key: String(row.status),
				label: String(row.status),
				requests: row.requests,
				errors: row.status >= 500 ? row.requests : 0,
				bytes_out: null,
				avg_ms: null,
			})),
		});
		return out;
	});
</script>

<div class="dash">
	<LinePanel
		class="half"
		title={t('analytics.dashboard.duration')}
		{ts}
		legend
		{labels}
		format={formatLatency}
		series={[
			{ label: 'p95', color: 'text-destructive', values: s.map((p) => p.p95_ms) },
			{
				label: t('analytics.dashboard.average'),
				color: 'text-warning',
				values: s.map((p) => p.avg_ms),
			},
		]} />
	<LinePanel
		class="half"
		title={t('analytics.dashboard.error_rate')}
		{ts}
		legend
		{labels}
		format={percent}
		series={[
			{
				label: t('analytics.dashboard.all'),
				color: 'text-primary',
				values: share((p) => p.errors_4xx + p.errors_5xx),
			},
			{ label: '4xx', color: 'text-warning', values: share((p) => p.errors_4xx) },
			{ label: '5xx', color: 'text-destructive', values: share((p) => p.errors_5xx) },
		]} />

	<LinePanel
		class="quarter"
		title={t('analytics.dashboard.requests')}
		{ts}
		{labels}
		format={count}
		series={[
			{
				label: t('analytics.dashboard.requests'),
				color: 'text-success',
				values: s.map((p) => p.requests),
			},
		]} />
	<LinePanel
		class="quarter"
		title={t('analytics.dashboard.bandwidth')}
		{ts}
		{labels}
		format={formatBytes}
		series={[
			{
				label: t('analytics.dashboard.bandwidth'),
				color: 'text-info',
				values: s.map((p) => p.bytes_out),
			},
		]} />
	<div class="quarter stats">
		<StatTile
			title={t('analytics.dashboard.total_requests')}
			value={count(data.totals.requests)}
			values={s.map((p) => p.requests)}
			tone="info" />
		<StatTile
			title={t('analytics.dashboard.server_errors')}
			value={count(data.totals.errors_5xx)}
			values={s.map((p) => p.errors_5xx)}
			tone="destructive" />
	</div>
	<div class="quarter stats">
		<StatTile
			title={t('analytics.dashboard.client_errors')}
			value={count(data.totals.errors_4xx)}
			values={s.map((p) => p.errors_4xx)}
			tone="warning" />
		<StatTile
			title={t('analytics.dashboard.data_sent')}
			value={formatBytes(data.totals.bytes_out)}
			values={s.map((p) => p.bytes_out)}
			tone="success" />
	</div>

	{#each gauges as gauge (gauge.title)}
		{@const top = Math.max(1, ...gauge.rows.map((row) => row.requests))}
		<section class="half flex min-w-0 flex-col gap-2 rounded-xl border bg-card p-3">
			<h3 class="text-[13px] font-medium">{gauge.title}</h3>
			<table class="w-full table-fixed text-[13px] tabular-nums">
				<thead>
					<tr class="text-muted-foreground">
						<th class="w-[45%] pb-1.5 text-left font-medium">{gauge.head}</th>
						<th class="pb-1.5 text-right font-medium" colspan="2"
							>{t('analytics.dashboard.requests')}</th>
					</tr>
				</thead>
				<tbody>
					{#each gauge.rows as row (row.key)}
						<tr class="border-t">
							<td class="truncate py-2 pr-3 font-mono text-[12.5px] text-primary">
								{#if row.href}<a href={row.href} class="hover:underline">{row.label}</a
									>{:else}{row.label}{/if}
							</td>
							<td class="py-2">
								<span class="lp-well block h-4 overflow-hidden rounded-[3px] bg-muted">
									<i
										class="block h-full rounded-[3px] bg-gradient-to-r from-info to-primary"
										style="width: {(row.requests / top) * 100}%"></i>
								</span>
							</td>
							<td class="w-16 py-2 pl-3 text-right">{count(row.requests)}</td>
						</tr>
					{:else}
						<tr class="border-t">
							<td colspan="3" class="py-3 text-muted-foreground">{t('analytics.panel.no_rows')}</td>
						</tr>
					{/each}
				</tbody>
			</table>
		</section>
	{/each}
</div>

<style>
	/* A 4-column panel grid like a Grafana row; halves stack on tablets, all on phones. */
	.dash {
		display: grid;
		grid-template-columns: repeat(4, minmax(0, 1fr));
		gap: 8px;
		align-items: start;
	}
	.dash > :global(.half) {
		grid-column: span 2;
	}
	.stats {
		display: grid;
		gap: 8px;
	}
	@media (max-width: 1100px) {
		.dash {
			grid-template-columns: repeat(2, minmax(0, 1fr));
		}
	}
	@media (max-width: 640px) {
		.dash {
			grid-template-columns: minmax(0, 1fr);
		}
		.dash > :global(.half) {
			grid-column: auto;
		}
		.stats {
			grid-template-columns: repeat(2, minmax(0, 1fr));
		}
	}
</style>
