<script lang="ts">
	import { page } from '$app/state';
	import ArrowLeftIcon from '@lucide/svelte/icons/arrow-left';
	import MetricsHistoryExplorer from '#lib/components/metrics/metrics-history-explorer.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import { getServerMetricsHistory } from '#lib/clients/container-metrics-client.js';
	import { getServer, type Server } from '#lib/clients/server-client.js';
	import { setSelectedOrganizationId } from '#lib/utils/selected-organization.js';
	import { t } from '#lib/i18n/index.svelte.js';

	const organizationId = $derived(page.params.id!);
	const serverId = $derived(page.params.server_id!);

	let server = $state<Server | null>(null);
	$effect(() => {
		setSelectedOrganizationId(organizationId);
		getServer(organizationId, serverId).then(
			(result) => (server = result),
			() => (server = null),
		);
	});
</script>

<div class="flex w-full flex-col gap-6 px-4 py-6 sm:px-6">
	<div class="flex items-start gap-3">
		<Button
			variant="ghost"
			size="icon-sm"
			href={`/organizations/${encodeURIComponent(organizationId)}/servers`}
			aria-label={t('organizations.servers.title')}>
			<ArrowLeftIcon class="size-4" />
		</Button>
		<div class="min-w-0">
			<h1 class="truncate text-xl font-semibold tracking-tight">
				{server?.name ?? t('common.metrics_history.title')}
			</h1>
			<p class="mt-1 text-sm text-muted-foreground">{t('common.metrics_history.title')}</p>
		</div>
	</div>

	<div class="lp-raised rounded-2xl border bg-card p-4 sm:p-5">
		{#key serverId}
			<MetricsHistoryExplorer
				host
				memoryFallback={server?.facts?.capacity?.memory_bytes ?? null}
				load={(window) => getServerMetricsHistory(organizationId, serverId, window)} />
		{/key}
	</div>
</div>
