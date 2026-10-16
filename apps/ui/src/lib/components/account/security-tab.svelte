<script lang="ts">
	import SkeletonRows from '#lib/components/custom/skeleton-rows.svelte';
	import SettingsSection from '#lib/components/projects/settings-section.svelte';
	import { toneBadge, toneDot } from '#lib/utils/status-tone.js';
	import { onMount } from 'svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import { toast, toastError } from '#lib/utils/toast.js';
	import {
		getAccount,
		setupTotp,
		verifyTotp,
		disableTotp,
		regenerateCodes,
	} from '#lib/clients/account-client.js';
	import type { AccountProfile, TotpSetupResponse } from '#lib/clients/account-client.js';
	import QRCode from 'qrcode';
	import { t } from '#lib/i18n/index.svelte.js';

	let profile = $state<AccountProfile | null>(null);
	let loading = $state(true);

	// Setup flow
	let setupState = $state<'idle' | 'setup' | 'enabled'>('idle');
	let setupData = $state<TotpSetupResponse | null>(null);
	let qrDataUrl = $state('');
	let verifyCode = $state('');
	let verifying = $state(false);

	// Disable flow
	let disableDialogOpen = $state(false);
	let disableCode = $state('');
	let disabling = $state(false);

	// Recovery codes reveal
	let showCodes = $state<string[]>([]);
	let codesDialogOpen = $state(false);

	// Regenerate
	let regenDialogOpen = $state(false);
	let regenCode = $state('');
	let regenerating = $state(false);

	onMount(async () => {
		await refreshProfile();
	});

	async function refreshProfile() {
		try {
			profile = await getAccount();
			if (profile.totp_enabled) {
				setupState = 'enabled';
			} else {
				setupState = 'idle';
			}
		} catch {
			// silent
		} finally {
			loading = false;
		}
	}

	async function handleEnable() {
		try {
			setupData = await setupTotp();
			const canvas = document.createElement('canvas');
			await QRCode.toCanvas(canvas, setupData.otpauth_url, { width: 200 });
			qrDataUrl = canvas.toDataURL();
			setupState = 'setup';
			verifyCode = '';
		} catch (err) {
			toastError(err, t('account.security.setup_failed'));
		}
	}

	async function handleVerify() {
		if (!verifyCode || verifyCode.length < 6) {
			toast.error(t('account.security.valid_six_digit_code'));
			return;
		}
		verifying = true;
		try {
			const resp = await verifyTotp(verifyCode);
			showCodes = resp.recovery_codes;
			codesDialogOpen = true;
			setupState = 'enabled';
			await refreshProfile();
		} catch (err) {
			toastError(err, t('account.security.verification_failed'));
		} finally {
			verifying = false;
		}
	}

	function handleCancelSetup() {
		setupState = 'idle';
		setupData = null;
		qrDataUrl = '';
		verifyCode = '';
	}

	async function handleDisable() {
		if (!disableCode) {
			toast.error(t('account.security.totp_or_recovery_required'));
			return;
		}
		disabling = true;
		try {
			await disableTotp(disableCode);
			toast.success(t('account.security.disabled_success'));
			disableDialogOpen = false;
			disableCode = '';
			setupState = 'idle';
			await refreshProfile();
		} catch (err) {
			toastError(err, t('account.security.disable_failed'));
		} finally {
			disabling = false;
		}
	}

	async function handleRegenerate() {
		if (!regenCode || regenCode.length < 6) {
			toast.error(t('account.security.valid_totp_code'));
			return;
		}
		regenerating = true;
		try {
			const resp = await regenerateCodes(regenCode);
			showCodes = resp.recovery_codes;
			codesDialogOpen = true;
			regenDialogOpen = false;
			regenCode = '';
			await refreshProfile();
		} catch (err) {
			toastError(err, t('account.security.regeneration_failed'));
		} finally {
			regenerating = false;
		}
	}

	function handleDownloadCodes() {
		const content = showCodes.join('\n');
		const blob = new Blob([content], { type: 'text/plain' });
		const url = URL.createObjectURL(blob);
		const a = document.createElement('a');
		a.href = url;
		a.download = 'litepod-recovery-codes.txt';
		a.click();
		URL.revokeObjectURL(url);
	}

	function handleCloseCodesDialog() {
		codesDialogOpen = false;
		showCodes = [];
	}
</script>

{#if loading}
	<SkeletonRows rows={3} height="h-20" label={t('account.security.loading')} />
{:else}
	<SettingsSection
		title={t('account.security.title')}
		description={t('account.security.description')}>
		{#snippet aside()}
			<div class="mt-2.5">
				<span
					class="inline-flex items-center gap-1.5 rounded-full border px-2 py-0.5 text-xs font-medium {toneBadge(
						setupState === 'enabled' ? 'ok' : 'neutral',
					)}">
					<span class="size-1.5 rounded-full {toneDot(setupState === 'enabled' ? 'ok' : 'neutral')}"
					></span>
					{setupState === 'enabled' ? t('account.common.enabled') : t('account.common.disabled')}
				</span>
			</div>
		{/snippet}

		{#if setupState === 'idle'}
			<div>
				<Button variant="outline" onclick={handleEnable}>{t('account.common.enable')}</Button>
			</div>
		{:else if setupState === 'setup' && setupData}
			<form
				class="flex flex-col gap-4"
				onsubmit={(event) => {
					event.preventDefault();
					handleVerify();
				}}>
				<p class="font-medium">{t('account.security.scan_qr')}</p>
				<div class="flex flex-col gap-4 sm:flex-row sm:items-start">
					{#if qrDataUrl}
						<div class="lp-well w-fit shrink-0 rounded-[12px] p-2">
							<img
								src={qrDataUrl}
								alt={t('account.security.qr_alt')}
								class="size-[176px] rounded-lg bg-white" />
						</div>
					{/if}
					<div class="flex min-w-0 flex-col gap-4">
						<details class="text-sm">
							<summary class="cursor-pointer text-muted-foreground"
								>{t('account.security.cant_scan')}</summary>
							<code
								class="mt-1.5 block rounded-[8px] bg-muted px-2 py-1.5 font-mono text-xs break-all"
								>{setupData.secret}</code>
						</details>
						<div class="grid gap-1.5">
							<Label for="verify-code">{t('account.security.enter_six_digit_code')}</Label>
							<Input
								id="verify-code"
								bind:value={verifyCode}
								placeholder="000000"
								autocomplete="one-time-code"
								inputmode="numeric"
								maxlength={6}
								class="w-36 font-mono text-lg tracking-[0.3em]" />
						</div>
					</div>
				</div>
				<div class="flex justify-end gap-2">
					<Button type="button" variant="ghost" onclick={handleCancelSetup}
						>{t('account.common.cancel')}</Button>
					<Button type="submit" disabled={verifying}>
						{verifying ? t('account.security.verifying') : t('account.security.verify_enable')}
					</Button>
				</div>
			</form>
		{:else if setupState === 'enabled'}
			<div>
				<Button variant="outline" onclick={() => (disableDialogOpen = true)}
					>{t('account.common.disable')}</Button>
			</div>
		{/if}
	</SettingsSection>

	{#if setupState === 'enabled'}
		<SettingsSection title={t('account.security.recovery_codes')}>
			<div
				class="flex flex-col gap-3 rounded-[10px] border border-foreground/9 px-3 py-2.5 sm:flex-row sm:items-center sm:justify-between">
				<p class="text-muted-foreground">
					{t('account.security.codes_remaining', {
						remaining: profile?.recovery_codes_remaining ?? 0,
					})}
				</p>
				<Button
					variant="outline"
					size="sm"
					class="shrink-0 self-start sm:self-auto"
					onclick={() => (regenDialogOpen = true)}>
					{t('account.security.regenerate_codes')}
				</Button>
			</div>
		</SettingsSection>
	{/if}
{/if}

<!-- Disable TOTP dialog -->
<Dialog.Root bind:open={disableDialogOpen}>
	<Dialog.DialogContent>
		<Dialog.DialogHeader>
			<Dialog.DialogTitle>{t('account.security.disable_title')}</Dialog.DialogTitle>
			<Dialog.DialogDescription>
				{t('account.security.disable_description')}
			</Dialog.DialogDescription>
		</Dialog.DialogHeader>
		<div class="grid gap-2 py-4">
			<Label for="disable-code">{t('account.common.code')}</Label>
			<Input
				id="disable-code"
				name="totp"
				autocomplete="one-time-code"
				maxlength={8}
				bind:value={disableCode}
				placeholder={t('account.security.enter_code')} />
		</div>
		<Dialog.DialogFooter>
			<Button variant="outline" onclick={() => (disableDialogOpen = false)}
				>{t('account.common.cancel')}</Button>
			<Button variant="destructive" onclick={handleDisable} disabled={disabling}>
				{disabling ? t('account.security.disabling') : t('account.security.disable_2fa')}
			</Button>
		</Dialog.DialogFooter>
	</Dialog.DialogContent>
</Dialog.Root>

<!-- Recovery codes reveal dialog -->
<Dialog.Root bind:open={codesDialogOpen}>
	<Dialog.DialogContent>
		<Dialog.DialogHeader>
			<Dialog.DialogTitle>{t('account.security.recovery_codes')}</Dialog.DialogTitle>
			<Dialog.DialogDescription>
				{t('account.security.recovery_codes_description')}
			</Dialog.DialogDescription>
		</Dialog.DialogHeader>
		<div class="grid gap-2 py-4">
			<div class="grid grid-cols-2 gap-2">
				{#each showCodes as code (code)}
					<code class="rounded bg-muted px-3 py-1.5 text-sm font-mono text-center">{code}</code>
				{/each}
			</div>
		</div>
		<Dialog.DialogFooter>
			<Button variant="outline" onclick={handleDownloadCodes}
				>{t('account.common.download')}</Button>
			<Button onclick={handleCloseCodesDialog}>{t('account.common.done')}</Button>
		</Dialog.DialogFooter>
	</Dialog.DialogContent>
</Dialog.Root>

<!-- Regenerate codes dialog -->
<Dialog.Root bind:open={regenDialogOpen}>
	<Dialog.DialogContent>
		<Dialog.DialogHeader>
			<Dialog.DialogTitle>{t('account.security.regenerate_title')}</Dialog.DialogTitle>
			<Dialog.DialogDescription>
				{t('account.security.regenerate_description')}
			</Dialog.DialogDescription>
		</Dialog.DialogHeader>
		<div class="grid gap-2 py-4">
			<Label for="regen-code">{t('account.security.totp_code')}</Label>
			<Input
				id="regen-code"
				name="totp"
				autocomplete="one-time-code"
				inputmode="numeric"
				bind:value={regenCode}
				placeholder="000000"
				maxlength={6}
				class="w-32 font-mono text-lg tracking-[0.3em]" />
		</div>
		<Dialog.DialogFooter>
			<Button variant="outline" onclick={() => (regenDialogOpen = false)}
				>{t('account.common.cancel')}</Button>
			<Button onclick={handleRegenerate} disabled={regenerating}>
				{regenerating ? t('account.security.generating') : t('account.common.generate')}
			</Button>
		</Dialog.DialogFooter>
	</Dialog.DialogContent>
</Dialog.Root>
