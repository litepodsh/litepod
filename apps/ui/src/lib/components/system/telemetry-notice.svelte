<script lang="ts">
	import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
	import { goto } from '$app/navigation';
	import {
		acknowledgeTelemetryNotice,
		getTelemetrySettings,
		updateTelemetrySettings,
	} from '#lib/clients/telemetry-client.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let { isSysadmin }: { isSysadmin: boolean } = $props();

	let open = $state(false);
	let busy = $state(false);

	// Told once, to the sysadmin only: the setting is instance-wide and only they can change it.
	$effect(() => {
		if (!isSysadmin) return;
		getTelemetrySettings().then(
			(settings) => (open = !settings.notice_acknowledged && !settings.env_locked),
			() => {},
		);
	});

	async function resolve(enabled: boolean) {
		busy = true;
		try {
			if (enabled) await acknowledgeTelemetryNotice();
			else await updateTelemetrySettings(false);
			open = false;
		} catch (err) {
			toast.error(err instanceof Error ? err.message : t('account.telemetry.errors.save'));
		} finally {
			busy = false;
		}
	}
</script>

<AlertDialog.Root bind:open>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title>{t('account.telemetry.notice_title')}</AlertDialog.Title>
			<AlertDialog.Description>
				{t('account.telemetry.notice_description')}
			</AlertDialog.Description>
		</AlertDialog.Header>
		<AlertDialog.Footer>
			<AlertDialog.Cancel disabled={busy} onclick={() => resolve(false)}>
				{t('account.telemetry.turn_off')}
			</AlertDialog.Cancel>
			<AlertDialog.Action disabled={busy} onclick={() => resolve(true)}>
				{t('account.telemetry.keep_on')}
			</AlertDialog.Action>
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>
