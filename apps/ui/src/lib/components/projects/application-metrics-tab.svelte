<script lang="ts">
	import CpuIcon from '@lucide/svelte/icons/cpu';
	import MemoryStickIcon from '@lucide/svelte/icons/memory-stick';
	import ActivityIcon from '@lucide/svelte/icons/activity';
	import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
	import HistoryIcon from '@lucide/svelte/icons/history';
	import CellChart from '#lib/components/custom/cell-chart.svelte';
	import MetricsHistoryExplorer from '#lib/components/metrics/metrics-history-explorer.svelte';
	import { Progress } from '#lib/components/ui/progress/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import {
		formatCpu,
		formatMemory,
		getApplicationMetricsHistory,
		metricIndex,
		streamApplicationContainerMetrics,
		type ContainerMetric,
		type ContainerMetricsSnapshot,
	} from '#lib/clients/container-metrics-client.js';
	import { compactNumber, getApplicationAnalytics } from '#lib/clients/analytics-client.js';
	import { i18n, t } from '#lib/i18n/index.svelte.js';

	// containerNames: running container names, i.e. metricContainerIds(...) from the app page.
	let {
		organizationId,
		projectId,
		environmentId,
		applicationId,
		containerNames,
	}: {
		organizationId: string;
		projectId: string;
		environmentId: string;
		applicationId: string;
		containerNames: string[];
	} = $props();

	const HISTORY = 48;

	let metrics = $state<ContainerMetric[] | null>(null);
	let cpuHistory = $state<number[]>([]);
	let streamError = $state(false);

	let namesKey = $derived(containerNames.join('\n'));

	$effect(() => {
		const names = namesKey ? namesKey.split('\n') : [];
		metrics = null;
		cpuHistory = [];
		streamError = false;
		if (names.length === 0 || !organizationId) return;

		const stream = streamApplicationContainerMetrics(
			organizationId,
			projectId,
			environmentId,
			applicationId,
		);
		const onMetrics = (event: Event) => {
			try {
				const index = metricIndex(
					JSON.parse((event as MessageEvent<string>).data) as ContainerMetricsSnapshot,
				);
				const next = names.map((n) => index.get(n)).filter((m): m is ContainerMetric => !!m);
				metrics = next;
				streamError = false;
				const cpu = sum(next.map((m) => m.cpu_percent));
				if (cpu !== null) cpuHistory = [...cpuHistory, cpu].slice(-HISTORY);
			} catch {
				// Ignore malformed stream events and keep the last valid metrics.
			}
		};
		const onError = () => (streamError = true);
		stream.addEventListener('metrics', onMetrics);
		stream.addEventListener('error', onError);
		return () => {
			stream.removeEventListener('metrics', onMetrics);
			stream.removeEventListener('error', onError);
			stream.close();
		};
	});

	function sum(values: (number | null)[]): number | null {
		const known = values.filter((v): v is number => v !== null && Number.isFinite(v));
		return known.length === 0 ? null : known.reduce((a, b) => a + b, 0);
	}

	let cpu = $derived(metrics ? sum(metrics.map((m) => m.cpu_percent)) : null);
	let memUsed = $derived(metrics ? sum(metrics.map((m) => m.memory_usage_bytes)) : null);
	let memLimit = $derived(
		metrics && metrics.length > 0 && metrics.every((m) => m.memory_limit_bytes !== null)
			? sum(metrics.map((m) => m.memory_limit_bytes))
			: null,
	);
	let memPercent = $derived(
		memUsed !== null && memLimit ? Math.min(100, (memUsed / memLimit) * 100) : null,
	);
	// Real 24h requests from the Caddy access logs; hidden if the call fails.
	let requests = $state<number[] | null>(null);
	$effect(() => {
		if (!organizationId) return;
		let cancelled = false;
		requests = null;
		getApplicationAnalytics(organizationId, projectId, environmentId, applicationId, '24h').then(
			(result) => {
				if (!cancelled) requests = result.series.map((point) => point.requests);
			},
			() => {},
		);
		return () => (cancelled = true);
	});
</script>

{#snippet head(Icon: typeof CpuIcon, title: string, hint: string)}
	<div class="flex items-center gap-2.5">
		<div
			class="lp-inset grid size-[30px] flex-none place-items-center rounded-lg bg-muted text-muted-foreground">
			<Icon class="size-4" />
		</div>
		<div class="min-w-0">
			<p class="text-[13px] font-medium">{title}</p>
			<p class="truncate text-xs text-muted-foreground">{hint}</p>
		</div>
	</div>
{/snippet}

<section class="flex flex-col gap-3" data-testid="application-metrics-tab">
	{#if containerNames.length === 0}
		<div class="rounded-2xl border border-dashed p-6 text-center text-sm text-muted-foreground">
			{t('projects.application_metrics.empty')}
		</div>
	{:else}
		{#if streamError}
			<p
				role="status"
				class="flex items-center gap-2 rounded-xl border border-warning/30 bg-warning/10 px-3 py-2 text-xs text-warning">
				<CircleAlertIcon class="size-3.5 flex-none" />
				{t('projects.application_metrics.stream_error')}
			</p>
		{/if}

		<div class="grid gap-3 sm:grid-cols-2">
			<div class="lp-raised flex min-w-0 flex-col gap-3.5 rounded-2xl border bg-card p-4">
				{@render head(
					CpuIcon,
					t('projects.application_metrics.cpu'),
					t('projects.application_metrics.cpu_hint'),
				)}
				{#if metrics === null}
					<Skeleton class="h-8 w-24 rounded-md" />
					<Skeleton class="h-10 rounded-md" />
				{:else}
					<p
						class="text-[26px] font-semibold tracking-tight tabular-nums"
						data-testid="metrics-cpu">
						{formatCpu(cpu)}
					</p>
					<CellChart
						values={cpuHistory}
						cols={HISTORY}
						rows={6}
						tone="success"
						format={(v) => `${v.toFixed(1)}%`}
						ariaLabel={t('projects.application_metrics.cpu')} />
				{/if}
			</div>

			<div class="lp-raised flex min-w-0 flex-col gap-3.5 rounded-2xl border bg-card p-4">
				{@render head(
					MemoryStickIcon,
					t('projects.application_metrics.memory'),
					t('projects.application_metrics.memory_hint'),
				)}
				{#if metrics === null}
					<Skeleton class="h-8 w-24 rounded-md" />
					<Skeleton class="h-2 rounded-full" />
				{:else}
					<p
						class="text-[26px] font-semibold tracking-tight tabular-nums"
						data-testid="metrics-memory">
						{formatMemory(memUsed, memLimit)}
					</p>
					<div class="mt-auto flex flex-col gap-1.5">
						<Progress value={memPercent ?? 0} />
						<div class="flex justify-between text-xs text-muted-foreground tabular-nums">
							<span>
								{t('projects.application_metrics.containers', { count: metrics.length })}
							</span>
							<span>
								{memPercent === null
									? t('projects.application_metrics.no_limit')
									: `${memPercent.toFixed(0)}%`}
							</span>
						</div>
					</div>
				{/if}
			</div>
		</div>
	{/if}

	<div
		class="lp-raised flex min-w-0 flex-col gap-3.5 rounded-2xl border bg-card p-4"
		data-testid="metrics-history">
		{@render head(
			HistoryIcon,
			t('projects.application_metrics.history'),
			t('projects.application_metrics.history_hint'),
		)}
		<MetricsHistoryExplorer
			initialRange="1h"
			load={(window) =>
				getApplicationMetricsHistory(
					organizationId,
					projectId,
					environmentId,
					applicationId,
					window,
				)} />
	</div>

	{#if requests}
		<div class="lp-raised flex min-w-0 flex-col gap-3.5 rounded-2xl border bg-card p-4">
			<div class="flex items-center justify-between gap-3">
				{@render head(
					ActivityIcon,
					t('projects.application_metrics.requests'),
					t('projects.application_metrics.requests_hint'),
				)}
				<span class="text-sm font-semibold tabular-nums">
					{compactNumber(
						requests.reduce((a, b) => a + b, 0),
						i18n.locale,
					)}
				</span>
			</div>
			<CellChart
				values={requests}
				cols={48}
				rows={8}
				format={(v) => t('analytics.panel.requests_count', { count: Math.round(v) })}
				ariaLabel={t('projects.application_metrics.requests')} />
		</div>
	{/if}
</section>
