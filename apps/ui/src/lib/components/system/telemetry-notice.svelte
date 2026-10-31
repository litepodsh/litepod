<script lang="ts">
	import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
	import { goto } from '$app/navigation';
	import {
		acknowledgeTelemetryNotice,
		getTelemetrySettings,
	} from '#lib/clients/telemetry-client.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let { isSysadmin }: { isSysadmin: boolean } = $props();

	let open = $state(false);
	let busy = $state(false);

	// Told once, to the sysadmin only: the setting is instance-wide and only they can change it.
	$effect(() => {
		// Dev instances are throwaway; the notice would reappear on every fresh database.
		if (import.meta.env.DEV || !isSysadmin) return;
		getTelemetrySettings().then(
			(settings) => (open = !settings.notice_acknowledged && !settings.env_locked),
			() => {},
		);
	});

	async function acknowledge() {
		busy = true;
		try {
			await acknowledgeTelemetryNotice();
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
			<AlertDialog.Action disabled={busy} onclick={acknowledge}>
				{t('account.telemetry.ok')}
			</AlertDialog.Action>
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>
