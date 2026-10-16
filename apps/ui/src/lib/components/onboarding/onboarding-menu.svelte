<script lang="ts">
	import EllipsisIcon from '@lucide/svelte/icons/ellipsis';
	import PanelLeftIcon from '@lucide/svelte/icons/panel-left';
	import PanelRightIcon from '@lucide/svelte/icons/panel-right';
	import EyeOffIcon from '@lucide/svelte/icons/eye-off';
	import CheckCheckIcon from '@lucide/svelte/icons/check-check';
	import * as DropdownMenu from '#lib/components/ui/dropdown-menu/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { onboardingState, runOnboardingAction } from '#lib/state/onboarding.svelte.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';

	async function run(action: Parameters<typeof runOnboardingAction>[0]) {
		try {
			await runOnboardingAction(action);
		} catch (error) {
			toast.error(
				error instanceof Error ? error.message : t('common.onboarding.save_preference_failed'),
			);
		}
	}
</script>

<DropdownMenu.Root>
	<DropdownMenu.Trigger>
		{#snippet child({ props })}
			<Button
				{...props}
				variant="ghost"
				size="icon-xs"
				aria-label={t('common.onboarding.getting_started_options')}>
				<EllipsisIcon />
			</Button>
		{/snippet}
	</DropdownMenu.Trigger>
	<DropdownMenu.Content align="end" class="w-52">
		<DropdownMenu.Item onclick={() => (onboardingState.expanded = true)}
			>{t('common.onboarding.open_full_checklist')}</DropdownMenu.Item>
		<DropdownMenu.Separator />
		<DropdownMenu.Label>{t('common.onboarding.change_position')}</DropdownMenu.Label>
		<DropdownMenu.Item onclick={() => run({ action: 'set_position', position: 'right' })}>
			<PanelRightIcon />
			{t('common.onboarding.right_edge')}
		</DropdownMenu.Item>
		<DropdownMenu.Item onclick={() => run({ action: 'set_position', position: 'sidebar' })}>
			<PanelLeftIcon />
			{t('common.onboarding.sidebar_bottom')}
		</DropdownMenu.Item>
		<DropdownMenu.Item onclick={() => run({ action: 'set_position', position: 'hidden' })}>
			<EyeOffIcon />
			{t('common.onboarding.hide_for_now')}
		</DropdownMenu.Item>
		<DropdownMenu.Separator />
		<DropdownMenu.Item class="text-muted-foreground" onclick={() => run({ action: 'dismiss' })}>
			<CheckCheckIcon />
			{t('common.onboarding.mark_done')}
		</DropdownMenu.Item>
	</DropdownMenu.Content>
</DropdownMenu.Root>
