<script lang="ts">
	import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
	import DeleteConfirmControl from '#lib/components/custom/delete-confirm-control.svelte';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let {
		open = $bindable(false),
		volumeName,
		ondelete,
	}: { open?: boolean; volumeName: string; ondelete: () => Promise<void> } = $props();
	let deleting = $state(false);
	async function handleDelete() {
		deleting = true;
		try {
			await ondelete();
			open = false;
		} catch (error) {
			toast.error(error instanceof Error ? error.message : t('projects.volume_delete.failed'));
		} finally {
			deleting = false;
		}
	}
</script>

<AlertDialog.Root bind:open>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title>{t('projects.volume_delete.title')}</AlertDialog.Title>
			<AlertDialog.Description
				>{t('projects.volume_delete.description', { name: volumeName })}</AlertDialog.Description>
		</AlertDialog.Header>
		<AlertDialog.Footer>
			<AlertDialog.Cancel disabled={deleting}>{t('projects.projects.cancel')}</AlertDialog.Cancel>
			<DeleteConfirmControl disabled={deleting} onconfirm={handleDelete} />
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>
