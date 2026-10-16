<script lang="ts">
	import BillingTab from '#lib/components/account/billing-tab.svelte';
	import { t } from '#lib/i18n/index.svelte.js';
</script>

<div class="flex min-w-0 flex-col gap-5 px-4 py-6 sm:px-6">
	<div class="min-w-0">
		<h1 class="text-xl font-semibold tracking-tight">{t('account.subscription.title')}</h1>
		<p class="text-sm text-muted-foreground">{t('account.subscription.description')}</p>
	</div>
	<div class="flex min-w-0 flex-col gap-4">
		<BillingTab />
	</div>
</div>
