<script lang="ts">
	import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
	import DeleteConfirmControl from '#lib/components/custom/delete-confirm-control.svelte';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let {
		open = $bindable(false),
		port = null,
		ondeploy,
	}: {
		open?: boolean;
		port?: number | null;
		ondeploy: () => Promise<void>;
	} = $props();

	let deploying = $state(false);

	// The dialog closes on failure too: the amber banner still shows the stale
	// state and deployment.last_error carries the detail.
	async function handleDeploy() {
		deploying = true;
		try {
			await ondeploy();
		} catch (error) {
			toast.error(error instanceof Error ? error.message : t('projects.projects.redeploy_failed'));
		} finally {
			deploying = false;
			open = false;
		}
	}
</script>

<AlertDialog.Root bind:open>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title>{t('projects.projects.redeploy_required')}</AlertDialog.Title>
			<AlertDialog.Description>
				{#if port}{t('projects.projects.port_changed', { port })}{/if}
				{t('projects.projects.redeploy_port_hint')}
			</AlertDialog.Description>
		</AlertDialog.Header>
		<AlertDialog.Footer>
			<AlertDialog.Cancel disabled={deploying}>{t('projects.projects.later')}</AlertDialog.Cancel>
			<DeleteConfirmControl
				label={t('projects.environment.redeploy')}
				pendingLabel={t('projects.projects.redeploying')}
				variant="primary"
				disabled={deploying}
				onconfirm={handleDeploy} />
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>
