<script lang="ts">
	import { page } from '$app/state';
	import SecurityEventsTable from '#lib/components/admin/security-events-table.svelte';
	import { t } from '#lib/i18n/index.svelte.js';

	// Alerts link here with `?flagged=1`.
	let flagged = $state(page.url.searchParams.get('flagged') === '1');
</script>

<div class="flex min-w-0 flex-col gap-5 px-4 py-6 sm:px-6">
	<header>
		<h1 class="text-xl font-semibold tracking-tight">{t('operations.security.title')}</h1>
		<p class="text-muted-foreground text-sm">{t('operations.security.description')}</p>
	</header>

	<SecurityEventsTable bind:flagged />
</div>
