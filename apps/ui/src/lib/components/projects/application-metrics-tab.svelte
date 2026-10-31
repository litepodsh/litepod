<script lang="ts">
	import HistoryIcon from '@lucide/svelte/icons/history';
	import MetricsHistoryExplorer from '#lib/components/metrics/metrics-history-explorer.svelte';
	import { getApplicationMetricsHistory } from '#lib/clients/container-metrics-client.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let {
		organizationId,
		projectId,
		environmentId,
		applicationId,
	}: {
		organizationId: string;
		projectId: string;
		environmentId: string;
		applicationId: string;
	} = $props();
</script>

<section
	class="lp-raised flex min-w-0 flex-col gap-3.5 rounded-2xl border bg-card p-4"
	data-testid="application-metrics-tab">
	<div class="flex items-center gap-2.5">
		<div
			class="lp-inset grid size-[30px] flex-none place-items-center rounded-lg bg-muted text-muted-foreground">
			<HistoryIcon class="size-4" />
		</div>
		<div class="min-w-0">
			<p class="text-[13px] font-medium">{t('projects.application_metrics.history')}</p>
			<p class="truncate text-xs text-muted-foreground">
				{t('projects.application_metrics.history_hint')}
			</p>
		</div>
	</div>
	<MetricsHistoryExplorer
		initialRange="1h"
		load={(window) =>
			getApplicationMetricsHistory(organizationId, projectId, environmentId, applicationId, window)} />
</section>
