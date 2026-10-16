<script lang="ts">
	import { onMount } from 'svelte';
	import { getRegistrationStatus } from '#lib/clients/auth-client.js';
	import { t } from '#lib/i18n/index.svelte.js';
	const lastUpdated = 'October 7, 2026';
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
			{t('legal.terms.terms_of_service')}
		</h1>
		<p class="mt-2 text-sm text-muted-foreground">
			{t('legal.common.last_updated', { date: lastUpdated })}
		</p>
		{#if isCloud}
		<div class="mt-10 space-y-10 text-sm leading-relaxed text-foreground">
			<section>
				<h2 class="text-lg font-semibold">{t('legal.terms.acceptance')}</h2>
				<p class="mt-3 text-muted-foreground">{t('legal.terms.acceptance_description')}</p>
			</section>
			<section>
				<h2 class="text-lg font-semibold">{t('legal.terms.what_is')}</h2>
				<p class="mt-3 text-muted-foreground">{t('legal.terms.what_is_description')}</p>
			</section>
			<section>
				<h2 class="text-lg font-semibold">{t('legal.terms.accounts')}</h2>
				<p class="mt-3 text-muted-foreground">{t('legal.terms.accounts_description')}</p>
			</section>
			<section>
				<h2 class="text-lg font-semibold">{t('legal.terms.subscriptions')}</h2>
				<p class="mt-3 text-muted-foreground">{t('legal.terms.subscriptions_description')}</p>
				<p class="mt-3 text-muted-foreground">{t('legal.terms.subscriptions_cancellation')}</p>
			</section>
			<section>
				<h2 class="text-lg font-semibold">{t('legal.terms.refund_policy')}</h2>
				<p class="mt-3 text-muted-foreground">{t('legal.terms.refund_policy_description')}</p>
				<p class="mt-3 text-muted-foreground">{t('legal.terms.refund_policy_how')}</p>
				<p class="mt-3 text-muted-foreground">{t('legal.terms.refund_policy_renewals')}</p>
				<p class="mt-3 text-muted-foreground">
					<a href="/refund" class="underline">{t('legal.common.refund_policy')}</a>.
				</p>
			</section>
			<section>
				<h2 class="text-lg font-semibold">{t('legal.terms.merchant_of_record')}</h2>
				<p class="mt-3 text-muted-foreground">{t('legal.terms.merchant_of_record_description')}</p>
			</section>
			<section>
				<h2 class="text-lg font-semibold">{t('legal.terms.use')}</h2>
				<p class="mt-3 text-muted-foreground">{t('legal.terms.use_description')}</p>
				<p class="mt-3 text-muted-foreground">
					<a href="/acceptable-use" class="underline">{t('legal.common.acceptable_use_policy')}</a>.
				</p>
			</section>
			<section>
				<h2 class="text-lg font-semibold">{t('legal.terms.content')}</h2>
				<p class="mt-3 text-muted-foreground">{t('legal.terms.content_description')}</p>
			</section>
			<section>
				<h2 class="text-lg font-semibold">{t('legal.terms.servers')}</h2>
				<p class="mt-3 text-muted-foreground">{t('legal.terms.servers_description')}</p>
			</section>
			<section>
				<h2 class="text-lg font-semibold">{t('legal.terms.security')}</h2>
				<p class="mt-3 text-muted-foreground">{t('legal.terms.security_description')}</p>
			</section>
			<section>
				<h2 class="text-lg font-semibold">{t('legal.terms.data_processing')}</h2>
				<p class="mt-3 text-muted-foreground">{t('legal.terms.data_processing_description')}</p>
			</section>
			<section>
				<h2 class="text-lg font-semibold">{t('legal.terms.backups')}</h2>
				<p class="mt-3 text-muted-foreground">{t('legal.terms.backups_description')}</p>
			</section>
			<section>
				<h2 class="text-lg font-semibold">{t('legal.terms.suspension')}</h2>
				<p class="mt-3 text-muted-foreground">{t('legal.terms.suspension_description')}</p>
			</section>
			<section>
				<h2 class="text-lg font-semibold">{t('legal.terms.availability')}</h2>
				<p class="mt-3 text-muted-foreground">{t('legal.terms.availability_description')}</p>
			</section>
			<section>
				<h2 class="text-lg font-semibold">{t('legal.terms.liability')}</h2>
				<p class="mt-3 text-muted-foreground">{t('legal.terms.liability_description')}</p>
			</section>
			<section>
				<h2 class="text-lg font-semibold">{t('legal.terms.indemnity')}</h2>
				<p class="mt-3 text-muted-foreground">{t('legal.terms.indemnity_description')}</p>
			</section>
			<section>
				<h2 class="text-lg font-semibold">{t('legal.terms.termination')}</h2>
				<p class="mt-3 text-muted-foreground">{t('legal.terms.termination_description')}</p>
			</section>
			<section>
				<h2 class="text-lg font-semibold">{t('legal.terms.complaints')}</h2>
				<p class="mt-3 text-muted-foreground">{t('legal.terms.complaints_description')}</p>
			</section>
			<section>
				<h2 class="text-lg font-semibold">{t('legal.terms.company_information')}</h2>
				<p class="mt-3 text-muted-foreground">{t('legal.terms.company_information_description')}</p>
			</section>
			<section>
				<h2 class="text-lg font-semibold">{t('legal.terms.changes')}</h2>
				<p class="mt-3 text-muted-foreground">{t('legal.terms.changes_description')}</p>
			</section>
			<section>
				<h2 class="text-lg font-semibold">{t('legal.terms.contact')}</h2>
				<p class="mt-3 text-muted-foreground">
					{t('legal.terms.contact_description')}
					<a href="mailto:info@litepod.sh" class="underline">info@litepod.sh</a>.
				</p>
			</section>
		</div>
		{:else}
		<div class="mt-10 space-y-4 text-sm leading-relaxed text-foreground">
			<h2 class="text-lg font-semibold">{t('legal.self_hosted.title')}</h2>
			<p class="text-muted-foreground">{t('legal.self_hosted.terms_description')}</p>
		</div>
		{/if}
	</div>
</div>
