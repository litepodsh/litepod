<script lang="ts">
	import { page } from '$app/state';
	import AnalyticsPanel from '#lib/components/analytics/analytics-panel.svelte';
	import { getOrganizationAnalytics } from '#lib/clients/analytics-client.js';
	import { loadOrganizationApps, type OrganizationApp } from '#lib/utils/organization-apps.js';
	import { allowed } from '#lib/state/permissions.svelte.js';
	import { t } from '#lib/i18n/index.svelte.js';

	const organizationId = $derived(page.params.id ?? '');
	let apps = $state<OrganizationApp[]>([]);

	$effect(() => {
		const id = organizationId;
		// Names for the "Busiest apps" list; ids still render if this fails.
		loadOrganizationApps(id)
			.then((next) => (apps = next))
			.catch(() => (apps = []));
	});
</script>

<svelte:head>
	<title>{t('analytics.panel.title')} · litepod</title>
</svelte:head>

<div class="px-4 py-6 sm:px-6 sm:py-8">
	{#if allowed('org.read')}
		{#key organizationId}
			<AnalyticsPanel
				title={t('analytics.panel.title')}
				description={t('analytics.panel.description')}
				{apps}
				load={(range) => getOrganizationAnalytics(organizationId, range)} />
		{/key}
	{:else}
		<p class="text-sm text-muted-foreground">{t('analytics.panel.no_access')}</p>
	{/if}
</div>
