<script lang="ts">
	import SkeletonRows from '#lib/components/custom/skeleton-rows.svelte';
	import SettingsSection from '#lib/components/projects/settings-section.svelte';
	import { onMount } from 'svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import { toast, toastError } from '#lib/utils/toast.js';
	import { UseAsyncResource } from '#lib/hooks/use-async-resource.svelte.js';
	import { getAccount, updateAccount, changePassword } from '#lib/clients/account-client.js';
	import { t } from '#lib/i18n/index.svelte.js';

	const profileResource = new UseAsyncResource(
		() => getAccount(),
		t('account.profile.load_failed'),
	);
	let profile = $derived(profileResource.data ?? null);

	// Name edit
	let editName = $state('');
	let nameSaving = $state(false);

	// Password change
	let currentPw = $state('');
	let newPw = $state('');
	let confirmPw = $state('');
	let pwSaving = $state(false);

	onMount(async () => {
		await profileResource.load();
		if (profileResource.data) editName = profileResource.data.name;
	});

	async function handleSaveName() {
		if (!editName.trim()) {
			toast.error(t('account.profile.name_required'));
			return;
		}
		nameSaving = true;
		try {
			profileResource.data = await updateAccount(editName.trim());
			toast.success(t('account.profile.name_updated'));
		} catch (err) {
			toastError(err, t('account.profile.update_failed'));
		} finally {
			nameSaving = false;
		}
	}

	async function handleChangePassword() {
		if (!currentPw || !newPw || !confirmPw) {
			toast.error(t('account.profile.password_fields_required'));
			return;
		}
		if (newPw !== confirmPw) {
			toast.error(t('account.profile.passwords_mismatch'));
			return;
		}
		pwSaving = true;
		try {
			await changePassword(currentPw, newPw, confirmPw);
			toast.success(t('account.profile.password_changed'));
			currentPw = '';
			newPw = '';
			confirmPw = '';
		} catch (err) {
			toastError(err, t('account.profile.password_change_failed'));
		} finally {
			pwSaving = false;
		}
	}
</script>

{#if profileResource.loading}
	<SkeletonRows rows={4} height="h-16" label={t('account.profile.loading')} />
{:else if profile}
	<SettingsSection title={t('account.profile.title')}>
		<form
			class="flex flex-col gap-4"
			onsubmit={(event) => {
				event.preventDefault();
				handleSaveName();
			}}>
			<div class="grid gap-4 sm:grid-cols-2">
				<div class="grid content-start gap-1.5">
					<Label for="name">{t('account.profile.name')}</Label>
					<Input id="name" bind:value={editName} autocomplete="name" disabled={nameSaving} />
				</div>
				<div class="grid content-start gap-1.5">
					<Label for="email-display">{t('account.profile.email')}</Label>
					<Input id="email-display" value={profile.email} disabled class="opacity-60" />
					<p class="text-xs text-muted-foreground">{t('account.profile.email_unchangeable')}</p>
				</div>
			</div>
			<div class="flex justify-end">
				<Button type="submit" disabled={nameSaving}>
					{nameSaving ? t('account.profile.saving') : t('account.profile.save_changes')}
				</Button>
			</div>
		</form>
	</SettingsSection>

	<SettingsSection title={t('account.profile.change_password')}>
		<form
			class="flex flex-col gap-4"
			onsubmit={(event) => {
				event.preventDefault();
				handleChangePassword();
			}}>
			<div class="grid content-start gap-1.5 sm:max-w-[calc(50%-0.5rem)]">
				<Label for="current-pw">{t('account.profile.current_password')}</Label>
				<Input
					id="current-pw"
					type="password"
					autocomplete="current-password"
					bind:value={currentPw}
					disabled={pwSaving} />
			</div>
			<div class="grid gap-4 sm:grid-cols-2">
				<div class="grid content-start gap-1.5">
					<Label for="new-pw">{t('account.profile.new_password')}</Label>
					<Input
						id="new-pw"
						type="password"
						autocomplete="new-password"
						bind:value={newPw}
						disabled={pwSaving} />
				</div>
				<div class="grid content-start gap-1.5">
					<Label for="confirm-pw">{t('account.profile.confirm_new_password')}</Label>
					<Input
						id="confirm-pw"
						type="password"
						autocomplete="new-password"
						bind:value={confirmPw}
						disabled={pwSaving} />
				</div>
			</div>
			<div class="flex justify-end">
				<Button type="submit" disabled={pwSaving}>
					{pwSaving ? t('account.profile.changing') : t('account.profile.change_password')}
				</Button>
			</div>
		</form>
	</SettingsSection>
{/if}
