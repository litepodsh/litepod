<script lang="ts">
	import CompassIcon from '@lucide/svelte/icons/compass';
	import { Button } from '#lib/components/ui/button/index.js';
	import SettingsSection from '#lib/components/projects/settings-section.svelte';
	import { onboardingState, runOnboardingAction } from '#lib/state/onboarding.svelte.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let busy = $state(false);

	async function start() {
		busy = true;
		try {
			await runOnboardingAction({ action: 'reset' });
			toast.success(t('account.getting_started.ready'));
		} catch (error) {
			toast.error(
				error instanceof Error ? error.message : t('account.getting_started.start_failed'),
			);
		} finally {
			busy = false;
		}
	}
</script>

<SettingsSection
	title={t('account.getting_started.title')}
	description={t('account.getting_started.description')}>
	<div class="flex flex-col gap-2">
		<div>
			<Button disabled={busy || !onboardingState.organizationId} onclick={start}>
				<CompassIcon />
				{onboardingState.summary?.dismissed
					? t('account.getting_started.start')
					: t('account.getting_started.restart')}
			</Button>
		</div>
		{#if !onboardingState.organizationId}
			<p class="text-xs text-muted-foreground">
				{t('account.getting_started.select_organization')}
			</p>
		{/if}
	</div>
</SettingsSection>
