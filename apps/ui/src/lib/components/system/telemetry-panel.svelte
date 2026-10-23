<script lang="ts">
	import { onMount } from 'svelte';
	import SoftSwitch from '#lib/components/custom/soft-switch.svelte';
	import SettingsSection from '#lib/components/projects/settings-section.svelte';
	import { Label } from '#lib/components/ui/label/index.js';
	import {
		getTelemetrySettings,
		updateTelemetrySettings,
		type TelemetrySettings,
	} from '#lib/clients/telemetry-client.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let loading = $state(true);
	let saving = $state(false);
	let telemetry = $state<TelemetrySettings | null>(null);

	onMount(async () => {
		try {
			telemetry = await getTelemetrySettings();
		} catch (err) {
			toast.error(err instanceof Error ? err.message : t('account.telemetry.errors.save'));
		} finally {
			loading = false;
		}
	});

	async function toggle(enabled: boolean) {
		saving = true;
		try {
			telemetry = await updateTelemetrySettings(enabled);
			toast.success(t('account.telemetry.success.saved'));
		} catch (err) {
			toast.error(err instanceof Error ? err.message : t('account.telemetry.errors.save'));
		} finally {
			saving = false;
		}
	}

	const disabled = $derived(loading || saving || !telemetry || telemetry.env_locked);
</script>

<div class="flex max-w-[820px] flex-col gap-4">
	<SettingsSection
		title={t('account.telemetry.title')}
		description={t('account.telemetry.description')}>
		<div class="flex items-center justify-between gap-4">
			<Label for="telemetry-enabled"
				>{telemetry?.enabled && !telemetry.env_locked
					? t('account.common.enabled')
					: t('account.common.disabled')}</Label>
			<SoftSwitch
				id="telemetry-enabled"
				checked={telemetry?.enabled && !telemetry.env_locked}
				onCheckedChange={toggle}
				{disabled} />
		</div>
		{#if telemetry?.env_locked}
			<p class="text-sm text-muted-foreground">{t('account.telemetry.env_locked')}</p>
		{/if}
		<p class="text-sm text-muted-foreground">{t('account.telemetry.what_is_sent')}</p>
		{#if telemetry}
			<p class="font-mono text-xs text-muted-foreground">
				{t('account.telemetry.instance_id')}: {telemetry.instance_id}
			</p>
		{/if}
	</SettingsSection>
</div>
