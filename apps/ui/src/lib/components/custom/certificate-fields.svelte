<script lang="ts">
	import { goto } from '$app/navigation';
	import { getMe } from '#lib/clients/auth-client.js';
	import { onMount } from 'svelte';
	import { Label } from '#lib/components/ui/label/index.js';
	import { NativeSelect } from '#lib/components/ui/native-select/index.js';
	import type { CertificateProvider } from '#lib/clients/server-settings-client.js';
	import { listCertificates, type Certificate } from '#lib/clients/certificate-client.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let {
		provider = $bindable<CertificateProvider>('automatic'),
		certificateId = $bindable(''),
		certificateName = '',
		disabled = false,
	}: {
		provider?: CertificateProvider;
		certificateId?: string;
		certificateName?: string;
		disabled?: boolean;
	} = $props();
	const id = $props.id();
	let certificates = $state<Certificate[]>([]);
	let loading = $state(true);
	let error = $state('');
	let canManage = $state(false);
	async function refresh(): Promise<boolean> {
		if (!canManage) return false;
		try {
			certificates = await listCertificates();
			error = '';
			return true;
		} catch (caught) {
			error = caught instanceof Error ? caught.message : t('account.certificates.load_failed');
			return false;
		}
	}
	async function chooseProvider(event: Event) {
		if (
			(event.currentTarget as HTMLSelectElement).value !== 'custom' ||
			!canManage ||
			certificateId
		)
			return;
		if ((await refresh()) && certificates.length === 0) await goto('/settings?tab=certificates');
	}
	onMount(async () => {
		try {
			canManage = (await getMe()).is_sysadmin;
			await refresh();
		} catch (caught) {
			error = caught instanceof Error ? caught.message : t('account.certificates.load_failed');
		} finally {
			loading = false;
		}
	});
</script>

<div class="grid gap-2">
	<Label for={`${id}-provider`}>{t('account.settings.certificate_provider')}</Label>
	<NativeSelect
		id={`${id}-provider`}
		bind:value={provider}
		onchange={chooseProvider}
		{disabled}
		class="w-full">
		<option value="automatic">{t('account.certificates.automatic')}</option>
		<option value="letsencrypt">Let's Encrypt</option>
		<option value="custom" disabled={!canManage && !certificateId}
			>{t('account.certificates.custom')}</option>
	</NativeSelect>
	<p class="text-xs text-muted-foreground">{t('account.certificates.provider_hint')}</p>
</div>
{#if provider === 'custom'}
	<div class="grid gap-2">
		<Label for={`${id}-certificate`}>{t('account.certificates.title')}</Label>
		<NativeSelect
			id={`${id}-certificate`}
			bind:value={certificateId}
			onfocus={refresh}
			disabled={disabled || loading || !canManage}
			required
			class="w-full">
			<option value="">{t('account.certificates.select')}</option>
			{#if certificateId && !certificates.some((certificate) => certificate.id === certificateId)}
				<option value={certificateId}
					>{certificateName || t('account.certificates.saved_certificate')}</option>
			{/if}
			{#each certificates as certificate (certificate.id)}
				<option value={certificate.id}
					>{certificate.name} — {certificate.domains.join(', ')}</option>
			{/each}
		</NativeSelect>
		{#if error}<p role="alert" class="text-xs text-destructive">{error}</p>{/if}
		{#if canManage && !loading && certificates.length === 0 && !certificateId && !error}
			<p class="text-sm text-muted-foreground">
				{t('account.certificates.no_saved_hint')}
				<a
					class="font-medium text-primary underline underline-offset-4"
					href="/settings?tab=certificates">{t('account.certificates.open_settings')}</a>
			</p>
		{/if}
		<p class="text-xs text-muted-foreground">{t('account.certificates.selection_hint')}</p>
	</div>
{/if}
