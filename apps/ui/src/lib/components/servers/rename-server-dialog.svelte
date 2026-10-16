<script lang="ts">
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import { renameServer, type Server } from '#lib/clients/server-client.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let {
		open = $bindable(false),
		organizationId,
		server,
		onRenamed,
	}: {
		open?: boolean;
		organizationId: string;
		/** The server being renamed; `null` keeps the dialog inert. */
		server: Server | null;
		/** Receives the updated row so the caller can refresh its own copy. */
		onRenamed?: (server: Server) => void;
	} = $props();

	let name = $state('');
	let saving = $state(false);

	// Reseed on every open, so a cancelled rename does not linger in the input.
	$effect(() => {
		if (!open || !server) return;
		name = server.name;
	});

	async function save() {
		if (!server) return;
		const trimmed = name.trim();
		if (!trimmed) {
			toast.error(t('organizations.servers.name_required'));
			return;
		}
		saving = true;
		try {
			const updated = await renameServer(organizationId, server.id, trimmed);
			onRenamed?.(updated);
			toast.success(t('organizations.servers.renamed', { name: updated.name }));
			open = false;
		} catch (err) {
			toast.error(err instanceof Error ? err.message : t('organizations.servers.rename_failed'));
		} finally {
			saving = false;
		}
	}
</script>

<Dialog.Root bind:open>
	<Dialog.Content>
		<Dialog.Header>
			<Dialog.Title>{t('organizations.servers.rename', { name: server?.name ?? '' })}</Dialog.Title>
			<Dialog.Description>{t('organizations.servers.rename_hint')}</Dialog.Description>
		</Dialog.Header>
		<form
			onsubmit={(event) => {
				event.preventDefault();
				save();
			}}>
			<div class="grid gap-2 py-4">
				<Label for="server-name">{t('organizations.common.name')}</Label>
				<Input id="server-name" bind:value={name} maxlength={100} autofocus />
			</div>
			<Dialog.Footer>
				<Button type="button" variant="outline" onclick={() => (open = false)}
					>{t('organizations.common.cancel')}</Button>
				<Button type="submit" disabled={saving}>
					{saving ? t('organizations.common.saving') : t('organizations.common.save')}
				</Button>
			</Dialog.Footer>
		</form>
	</Dialog.Content>
</Dialog.Root>
