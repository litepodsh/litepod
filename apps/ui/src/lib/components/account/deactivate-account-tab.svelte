<script lang="ts">
	import { goto } from '$app/navigation';
	import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import DeleteConfirmControl from '#lib/components/custom/delete-confirm-control.svelte';
	import SettingsSection from '#lib/components/projects/settings-section.svelte';
	import { deactivateAccount } from '#lib/clients/account-client.js';
	import { broadcastLogout } from '#lib/auth-broadcast.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let open = $state(false);
	let currentPassword = $state('');
	let submitting = $state(false);
	let subscriptionBlocked = $state(false);

	function reset() {
		currentPassword = '';
		subscriptionBlocked = false;
	}

	async function handleDeactivate() {
		if (submitting) return;
		subscriptionBlocked = false;
		if (!currentPassword) {
			toast.error(t('account.deactivate.password_required'));
			return;
		}

		submitting = true;
		try {
			await deactivateAccount(currentPassword, 'ELIMINAR');
			broadcastLogout();
			toast.success(t('account.deactivate.success'));
			goto('/login');
		} catch (error) {
			const message = error instanceof Error ? error.message : t('account.deactivate.failed');
			subscriptionBlocked = /cancel your active subscription/i.test(message);
			toast.error(message);
		} finally {
			submitting = false;
		}
	}
</script>

<SettingsSection
	title={t('account.deactivate.danger_zone')}
	description={t('account.deactivate.description')}>
	<div>
		<AlertDialog.Root
			bind:open
			onOpenChange={(next) => {
				if (!next) reset();
			}}>
			<AlertDialog.Trigger>
				{#snippet child({ props })}
					<Button {...props} variant="destructive" size="sm"
						>{t('account.deactivate.button')}</Button>
				{/snippet}
			</AlertDialog.Trigger>
			<AlertDialog.Content>
				<AlertDialog.Header>
					<AlertDialog.Title>{t('account.deactivate.dialog_title')}</AlertDialog.Title>
					<AlertDialog.Description>
						{t('account.deactivate.dialog_description')}
					</AlertDialog.Description>
				</AlertDialog.Header>
				<div class="grid gap-4 py-3">
					<div class="grid gap-2">
						<Label for="deactivate-password">{t('account.deactivate.current_password')}</Label>
						<Input
							id="deactivate-password"
							type="password"
							autocomplete="current-password"
							bind:value={currentPassword}
							disabled={submitting} />
					</div>
					{#if subscriptionBlocked}
						<p class="text-sm text-destructive">
							{t('account.deactivate.cancel_subscription')}
						</p>
						<Button variant="outline" onclick={() => goto('/billing')}
							>{t('account.deactivate.go_to_subscription')}</Button>
					{/if}
				</div>
				<AlertDialog.Footer>
					<AlertDialog.Cancel disabled={submitting}
						>{t('account.common.cancel')}</AlertDialog.Cancel>
					<DeleteConfirmControl
						label={t('account.deactivate.button')}
						pendingLabel={t('account.deactivate.deactivating')}
						disabled={submitting || !currentPassword}
						onconfirm={handleDeactivate} />
				</AlertDialog.Footer>
			</AlertDialog.Content>
		</AlertDialog.Root>
	</div>
</SettingsSection>
