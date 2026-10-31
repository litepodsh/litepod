<script lang="ts">
	import { onMount } from 'svelte';
	import { getRegistrationStatus } from '#lib/clients/auth-client.js';
	import { t } from '#lib/i18n/index.svelte.js';
	const lastUpdated = 'October 31, 2026';
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
			{t('legal.privacy.privacy_policy')}
		</h1>
		<p class="mt-2 text-sm text-muted-foreground">
			{t('legal.common.last_updated', { date: lastUpdated })}
		</p>
		{#if isCloud}
			<div class="mt-10 space-y-10 text-sm leading-relaxed text-foreground">
				<section>
					<h2 class="text-lg font-semibold">{t('legal.privacy.controller')}</h2>
					<p class="mt-3 text-muted-foreground">{t('legal.privacy.controller_description')}</p>
				</section>
				<section>
					<h2 class="text-lg font-semibold">{t('legal.privacy.overview')}</h2>
					<p class="mt-3 text-muted-foreground">{t('legal.privacy.overview_description')}</p>
				</section>
				<section>
					<h2 class="text-lg font-semibold">{t('legal.privacy.account_data')}</h2>
					<p class="mt-3 text-muted-foreground">{t('legal.privacy.account_intro')}</p>
					<ul class="mt-3 list-disc space-y-1 pl-5 text-muted-foreground">
						<li><strong>{t('auth.register.email')}</strong> {t('legal.privacy.account_email')}</li>
						<li>
							<strong>{t('auth.register.username')}</strong>
							{t('legal.privacy.account_username')}
						</li>
						<li>
							<strong>{t('auth.register.password')}</strong>
							{t('legal.privacy.account_password')}
						</li>
						<li>{t('legal.privacy.account_two_factor')}</li>
					</ul>
				</section>
				<section>
					<h2 class="text-lg font-semibold">{t('legal.privacy.service_data')}</h2>
					<p class="mt-3 text-muted-foreground">{t('legal.privacy.service_data_description')}</p>
				</section>
				<section>
					<h2 class="text-lg font-semibold">{t('legal.privacy.payment_data')}</h2>
					<p class="mt-3 text-muted-foreground">{t('legal.privacy.payment_description')}</p>
				</section>
				<section>
					<h2 class="text-lg font-semibold">{t('legal.privacy.analytics')}</h2>
					<p class="mt-3 text-muted-foreground">{t('legal.privacy.analytics_description')}</p>
				</section>
				<section>
					<h2 class="text-lg font-semibold">{t('legal.privacy.use_data')}</h2>
					<ul class="mt-3 list-disc space-y-1 pl-5 text-muted-foreground">
						<li>{t('legal.privacy.use_data_1')}</li>
						<li>{t('legal.privacy.use_data_2')}</li>
						<li>{t('legal.privacy.use_data_3')}</li>
						<li>{t('legal.privacy.use_data_4')}</li>
					</ul>
					<p class="mt-3 text-muted-foreground">{t('legal.privacy.sell_data')}</p>
				</section>
				<section>
					<h2 class="text-lg font-semibold">{t('legal.privacy.data_sharing')}</h2>
					<p class="mt-3 text-muted-foreground">{t('legal.privacy.data_sharing_description')}</p>
				</section>
				<section>
					<h2 class="text-lg font-semibold">{t('legal.privacy.retention')}</h2>
					<p class="mt-3 text-muted-foreground">{t('legal.privacy.retention_description')}</p>
				</section>
				<section>
					<h2 class="text-lg font-semibold">{t('legal.privacy.security')}</h2>
					<p class="mt-3 text-muted-foreground">{t('legal.privacy.security_description')}</p>
				</section>
				<section>
					<h2 class="text-lg font-semibold">{t('legal.privacy.rights')}</h2>
					<p class="mt-3 text-muted-foreground">{t('legal.privacy.rights_description')}</p>
				</section>
				<section>
					<h2 class="text-lg font-semibold">{t('legal.privacy.changes')}</h2>
					<p class="mt-3 text-muted-foreground">{t('legal.privacy.changes_description')}</p>
				</section>
				<section>
					<h2 class="text-lg font-semibold">{t('legal.privacy.contact')}</h2>
					<p class="mt-3 text-muted-foreground">
						{t('legal.privacy.contact_description')}
						<a href="mailto:info@litepod.sh" class="underline">info@litepod.sh</a>.
					</p>
				</section>
			</div>
		{:else}
			<div class="mt-10 space-y-4 text-sm leading-relaxed text-foreground">
				<h2 class="text-lg font-semibold">{t('legal.self_hosted.title')}</h2>
				<p class="text-muted-foreground">{t('legal.self_hosted.privacy_description')}</p>
			</div>
		{/if}
	</div>
</div>
