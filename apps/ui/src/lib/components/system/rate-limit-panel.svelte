<script lang="ts">
	import { onMount } from 'svelte';
	import SoftSwitch from '#lib/components/custom/soft-switch.svelte';
	import SettingsSection from '#lib/components/projects/settings-section.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import {
		getRateLimitSettings,
		updateRateLimitSettings,
		type RateLimitSettings,
	} from '#lib/clients/rate-limit-client.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let loading = $state(true);
	let saving = $state(false);
	let rateLimit = $state<RateLimitSettings>({
		enabled: true,
		global_per_second: 20,
		global_burst: 40,
		auth_per_second: 5,
		auth_burst: 10,
		auth_max_attempts: 5,
		auth_ban_minutes: 15,
	});

	onMount(async () => {
		try {
			rateLimit = await getRateLimitSettings();
		} catch (err) {
			toast.error(
				err instanceof Error ? err.message : t('account.settings.errors.save_rate_limit'),
			);
		} finally {
			loading = false;
		}
	});

	async function save() {
		saving = true;
		try {
			rateLimit = await updateRateLimitSettings(rateLimit);
			toast.success(t('account.settings.success.rate_limit_saved'));
		} catch (err) {
			toast.error(
				err instanceof Error ? err.message : t('account.settings.errors.save_rate_limit'),
			);
		} finally {
			saving = false;
		}
	}

	const disabled = $derived(loading || saving);
</script>

{#snippet numberField(id: string, label: string, key: Exclude<keyof RateLimitSettings, 'enabled'>)}
	<div class="grid content-start gap-1.5">
		<Label for={id}>{label}</Label>
		<Input
			{id}
			type="number"
			min="1"
			class="font-mono tabular-nums"
			bind:value={rateLimit[key]}
			{disabled} />
	</div>
{/snippet}

<form
	class="flex max-w-[820px] flex-col gap-4"
	onsubmit={(event) => {
		event.preventDefault();
		save();
	}}>
	<SettingsSection
		title={t('account.settings.tabs.rate_limit')}
		description={t('account.settings.rate_limit_description')}>
		<div class="flex items-center justify-between gap-4">
			<Label for="rate-limit-enabled"
				>{rateLimit.enabled ? t('account.common.enabled') : t('account.common.disabled')}</Label>
			<SoftSwitch id="rate-limit-enabled" bind:checked={rateLimit.enabled} {disabled} />
		</div>
	</SettingsSection>
	<SettingsSection
		title={t('account.settings.global_requests')}
		description={t('account.settings.global_requests_hint')}>
		<div class="grid gap-4 sm:grid-cols-2">
			{@render numberField(
				'global-per-second',
				t('account.settings.requests_per_second'),
				'global_per_second',
			)}
			{@render numberField('global-burst', t('account.settings.burst'), 'global_burst')}
		</div>
	</SettingsSection>
	<SettingsSection
		title={t('account.settings.auth_requests')}
		description={t('account.settings.auth_requests_hint')}>
		<div class="grid gap-4 sm:grid-cols-2">
			{@render numberField(
				'auth-per-second',
				t('account.settings.requests_per_second'),
				'auth_per_second',
			)}
			{@render numberField('auth-burst', t('account.settings.burst'), 'auth_burst')}
		</div>
	</SettingsSection>
	<SettingsSection
		title={t('account.settings.login_ban')}
		description={t('account.settings.login_ban_hint')}>
		<div class="grid gap-4 sm:grid-cols-2">
			{@render numberField(
				'auth-max-attempts',
				t('account.settings.failed_attempts_before_ban'),
				'auth_max_attempts',
			)}
			{@render numberField(
				'auth-ban-minutes',
				t('account.settings.ban_duration_minutes'),
				'auth_ban_minutes',
			)}
		</div>
	</SettingsSection>
	<div class="flex justify-end">
		<Button type="submit" {disabled}
			>{saving ? t('account.common.saving') : t('account.common.save')}</Button>
	</div>
</form>
