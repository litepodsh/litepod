<script lang="ts">
	import { onMount } from 'svelte';
	import { getRegistrationStatus } from '#lib/clients/auth-client.js';
	import { t } from '#lib/i18n/index.svelte.js';
	const lastUpdated = 'September 18, 2026';
	let isCloud = $state(true);

	onMount(async () => {
		try {
			isCloud = (await getRegistrationStatus()).is_cloud;
		} catch {
			// Keep the cloud document as the safe default.
		}
	});
</script>

<div class="min-h-screen bg-background">
	<div class="mx-auto max-w-3xl px-6 py-16">
		<a href="/login" class="text-sm text-muted-foreground hover:text-foreground"
			>{t('legal.common.back_to_login')}</a>
		<h1 class="mt-6 text-3xl font-semibold tracking-tight text-foreground">
			{t('legal.common.refund_policy')}
		</h1>
		<p class="mt-2 text-sm text-muted-foreground">
			{t('legal.common.last_updated', { date: lastUpdated })}
		</p>
		{#if isCloud}
			<div class="mt-10 space-y-10 text-sm leading-relaxed text-foreground">
				<section>
					<h2 class="text-lg font-semibold">{t('legal.terms.refund_policy')}</h2>
					<p class="mt-3 text-muted-foreground">{t('legal.terms.refund_policy_description')}</p>
				</section>
				<section>
					<h2 class="text-lg font-semibold">{t('legal.terms.refund_how')}</h2>
					<p class="mt-3 text-muted-foreground">{t('legal.terms.refund_policy_how')}</p>
				</section>
				<section>
					<h2 class="text-lg font-semibold">{t('legal.terms.refund_cancellations')}</h2>
					<p class="mt-3 text-muted-foreground">{t('legal.terms.refund_policy_renewals')}</p>
				</section>
				<section>
					<h2 class="text-lg font-semibold">{t('legal.terms.merchant_of_record')}</h2>
					<p class="mt-3 text-muted-foreground">
						{t('legal.terms.merchant_of_record_description')}
					</p>
				</section>
				<section>
					<h2 class="text-lg font-semibold">{t('legal.terms.complaints')}</h2>
					<p class="mt-3 text-muted-foreground">{t('legal.terms.complaints_description')}</p>
				</section>
				<section>
					<h2 class="text-lg font-semibold">{t('legal.terms.company_information')}</h2>
					<p class="mt-3 text-muted-foreground">
						{t('legal.terms.company_information_description')}
					</p>
				</section>
				<section>
					<h2 class="text-lg font-semibold">{t('legal.terms.contact')}</h2>
					<p class="mt-3 text-muted-foreground">
						{t('legal.terms.contact_description')}
						<a href="mailto:info@litepod.sh" class="underline">info@litepod.sh</a>.
					</p>
				</section>
				<p class="text-muted-foreground">
					<a href="/terms" class="underline">{t('legal.common.terms_of_service')}</a> ·
					<a href="/privacy" class="underline">{t('legal.common.privacy_policy')}</a>
				</p>
			</div>
		{:else}
			<div class="mt-10 space-y-4 text-sm leading-relaxed text-foreground">
				<h2 class="text-lg font-semibold">{t('legal.self_hosted.title')}</h2>
				<p class="text-muted-foreground">{t('legal.self_hosted.refund_description')}</p>
			</div>
		{/if}
	</div>
</div>
