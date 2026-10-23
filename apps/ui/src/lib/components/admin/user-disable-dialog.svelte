<script lang="ts">
	import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import type { UserWithOrgs } from '#lib/clients/auth-client.js';
	import { setUserDisabled } from '#lib/clients/organization-client.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let {
		user = $bindable<UserWithOrgs | null>(null),
		onChanged,
	}: { user?: UserWithOrgs | null; onChanged: (userId: string, disabled: boolean) => void } =
		$props();

	let busy = $state(false);

	async function confirm() {
		const target = user;
		if (!target) return;
		const disabled = !target.disabled;
		busy = true;
		try {
			await setUserDisabled(target.id, disabled);
			onChanged(target.id, disabled);
			toast.success(
				t(disabled ? 'organizations.members.disabled_done' : 'organizations.members.enabled_done', {
					name: target.name || target.email,
				}),
			);
			user = null;
		} catch (error) {
			toast.error(
				error instanceof Error ? error.message : t('organizations.members.toggle_failed'),
			);
		} finally {
			busy = false;
		}
	}
</script>

<AlertDialog.Root
	open={user !== null}
	onOpenChange={(open) => {
		if (!open && !busy) user = null;
	}}>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title>
				{user?.disabled
					? t('organizations.members.enable_confirm')
					: t('organizations.members.disable_confirm')}
			</AlertDialog.Title>
			<AlertDialog.Description>
				{user
					? t(
							user.disabled
								? 'organizations.members.enable_description'
								: 'organizations.members.disable_description',
							{ name: user.name || user.email },
						)
					: ''}
			</AlertDialog.Description>
		</AlertDialog.Header>
		<AlertDialog.Footer>
			<AlertDialog.Cancel disabled={busy}>{t('organizations.common.cancel')}</AlertDialog.Cancel>
			<Button variant={user?.disabled ? 'default' : 'destructive'} disabled={busy} onclick={confirm}>
				{user?.disabled
					? t('organizations.members.enable_action')
					: t('organizations.members.disable_action')}
			</Button>
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>
