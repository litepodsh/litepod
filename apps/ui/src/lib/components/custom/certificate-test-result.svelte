<script lang="ts">
	import type { CertificateTestResult } from '#lib/clients/certificate-client.js';
	import { t } from '#lib/i18n/index.svelte.js';
	let { result }: { result: CertificateTestResult } = $props();
</script>

<div class="space-y-3 rounded-xl border border-border/70 bg-muted/30 p-4 text-sm" role="status">
	<p class="font-medium">{t('account.certificates.test_results')}</p>
	<div class="grid gap-2 sm:grid-cols-2">
		<p class={result.key_matches ? 'text-emerald-600 dark:text-emerald-400' : 'text-destructive'}>
			{result.key_matches ? '✓' : '✕'}
			{t('account.certificates.key_matches')}
		</p>
		<p class={result.dates_valid ? 'text-emerald-600 dark:text-emerald-400' : 'text-destructive'}>
			{result.dates_valid ? '✓' : '✕'}
			{t('account.certificates.dates_valid')}
		</p>
		<p class={result.san_present ? 'text-emerald-600 dark:text-emerald-400' : 'text-destructive'}>
			{result.san_present ? '✓' : '✕'}
			{t('account.certificates.san_present')}
		</p>
		<p
			class={result.chain_trusted
				? 'text-emerald-600 dark:text-emerald-400'
				: 'text-amber-600 dark:text-amber-400'}>
			{result.chain_trusted ? '✓' : '!'}
			{result.chain_trusted
				? t('account.certificates.chain_trusted')
				: t('account.certificates.chain_untrusted')}
		</p>
	</div>
	{#if result.chain_error}<p class="break-words text-xs text-muted-foreground">
			{result.chain_error}
		</p>{/if}
	{#if result.domains.length}
		<div class="space-y-2 border-t border-border/60 pt-3">
			<p class="font-medium">{t('account.certificates.assigned_domains')}</p>
			{#each result.domains as domain}
				<div class="rounded-lg bg-background/80 p-3">
					<p class="break-all font-medium">{domain.host}</p>
					<p class={domain.covered ? 'text-emerald-600 dark:text-emerald-400' : 'text-destructive'}>
						{domain.covered ? '✓' : '✕'}
						{t('account.certificates.domain_covered')}
					</p>
					<p
						class={domain.served === true
							? 'text-emerald-600 dark:text-emerald-400'
							: domain.served === false
								? 'text-destructive'
								: 'text-muted-foreground'}>
						{domain.served === true ? '✓' : domain.served === false ? '✕' : '–'}
						{domain.served === true
							? t('account.certificates.live_match')
							: domain.served === false
								? t('account.certificates.live_mismatch')
								: t('account.certificates.live_unavailable')}
					</p>
					{#if domain.connection_error}<p class="break-words text-xs text-muted-foreground">
							{domain.connection_error}
						</p>{/if}
				</div>
			{/each}
		</div>
	{:else}
		<p class="border-t border-border/60 pt-3 text-xs text-muted-foreground">
			{t('account.certificates.no_assigned_domains')}
		</p>
	{/if}
	<p class="text-xs text-muted-foreground">{t('account.certificates.test_note')}</p>
</div>
