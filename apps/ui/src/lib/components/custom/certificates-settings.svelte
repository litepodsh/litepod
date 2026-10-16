<script lang="ts">
	import { onMount } from 'svelte';
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import SettingsSection from '#lib/components/projects/settings-section.svelte';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import ScriptEditor from '#lib/components/scheduled-tasks/script-editor.svelte';
	import {
		listCertificates,
		saveCertificate,
		deleteCertificate,
		testCertificate,
		getCertificateMaterial,
		type Certificate,
		type CertificateTestResult,
	} from '#lib/clients/certificate-client.js';
	import CertificateTestResultView from './certificate-test-result.svelte';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let certificates = $state<Certificate[]>([]);
	let loading = $state(true);
	let error = $state('');
	let busy = $state(false);
	let open = $state(false);
	let editing = $state<Certificate | null>(null);
	let removing = $state<Certificate | null>(null);
	let name = $state('');
	let certificatePem = $state('');
	let privateKeyPem = $state('');
	let originalCertificatePem = '';
	let originalPrivateKeyPem = '';
	let loadingEdit = $state(false);
	let editRequest = 0;
	let testing = $state(false);
	let testResult = $state<CertificateTestResult | null>(null);
	let testDialogOpen = $state(false);
	$effect(() => {
		if (!open) {
			editRequest++;
			certificatePem = '';
			privateKeyPem = '';
		}
	});
	$effect(() => {
		certificatePem;
		privateKeyPem;
		testResult = null;
	});
	async function refresh() {
		loading = true;
		try {
			certificates = await listCertificates();
			error = '';
		} catch (caught) {
			error = caught instanceof Error ? caught.message : t('account.certificates.load_failed');
		} finally {
			loading = false;
		}
	}
	onMount(refresh);
	async function edit(certificate: Certificate | null) {
		const request = ++editRequest;
		editing = certificate;
		name = certificate?.name ?? '';
		certificatePem = '';
		privateKeyPem = '';
		originalCertificatePem = '';
		originalPrivateKeyPem = '';
		testResult = null;
		loadingEdit = false;
		open = true;
		if (!certificate) return;
		loadingEdit = true;
		try {
			const material = await getCertificateMaterial(certificate.id);
			if (request !== editRequest || !open || editing?.id !== certificate.id) return;
			certificatePem = originalCertificatePem = material.certificate_pem;
			privateKeyPem = originalPrivateKeyPem = material.private_key_pem;
		} catch (caught) {
			if (request === editRequest && open) {
				toast.error(
					caught instanceof Error ? caught.message : t('account.certificates.load_failed'),
				);
				open = false;
			}
		} finally {
			if (request === editRequest) loadingEdit = false;
		}
	}
	async function test(certificate: Certificate | null) {
		testing = true;
		testResult = null;
		try {
			const materialChanged =
				certificatePem !== originalCertificatePem || privateKeyPem !== originalPrivateKeyPem;
			testResult = await testCertificate(
				certificate
					? { id: certificate.id }
					: {
							...(editing ? { id: editing.id } : {}),
							...(!editing || materialChanged
								? { certificate_pem: certificatePem, private_key_pem: privateKeyPem }
								: {}),
						},
			);
			if (certificate) testDialogOpen = true;
		} catch (caught) {
			toast.error(caught instanceof Error ? caught.message : t('account.certificates.test_failed'));
		} finally {
			testing = false;
		}
	}
	async function save() {
		busy = true;
		try {
			const materialChanged =
				certificatePem !== originalCertificatePem || privateKeyPem !== originalPrivateKeyPem;
			const saved = await saveCertificate(
				{
					name: name.trim(),
					...(!editing || materialChanged
						? { certificate_pem: certificatePem, private_key_pem: privateKeyPem }
						: {}),
				},
				editing?.id,
			);
			certificates = editing
				? certificates.map((certificate) => (certificate.id === saved.id ? saved : certificate))
				: [...certificates, saved];
			open = false;
		} catch (caught) {
			toast.error(caught instanceof Error ? caught.message : t('account.certificates.save_failed'));
		} finally {
			busy = false;
		}
	}
	async function remove() {
		if (!removing) return;
		busy = true;
		try {
			await deleteCertificate(removing.id);
			certificates = certificates.filter((certificate) => certificate.id !== removing?.id);
			removing = null;
		} catch (caught) {
			toast.error(
				caught instanceof Error ? caught.message : t('account.certificates.delete_failed'),
			);
		} finally {
			busy = false;
		}
	}
</script>

<SettingsSection
	title={t('account.certificates.title')}
	description={t('account.certificates.inventory_hint')}>
	{#if error}<div class="flex flex-wrap items-center gap-3">
			<p role="alert" class="text-sm text-destructive">{error}</p>
			<Button variant="outline" size="sm" onclick={refresh}
				>{t('account.certificates.retry')}</Button>
		</div>
	{:else if loading}<p class="text-sm text-muted-foreground">
			{t('account.certificates.loading')}
		</p>
	{:else if !certificates.length}<p
			class="rounded-[10px] bg-muted/40 px-3 py-4 text-sm text-muted-foreground">
			{t('account.certificates.empty')}
		</p>{/if}
	{#if certificates.length}
		<div class="flex flex-col gap-1.5">
			{#each certificates as certificate (certificate.id)}
				<div
					class="flex flex-col gap-3 rounded-[10px] border border-foreground/9 px-3 py-2.5 sm:flex-row sm:items-center sm:justify-between">
					<div class="min-w-0">
						<p class="font-medium">{certificate.name}</p>
						<p class="break-all font-mono text-xs text-muted-foreground">
							{certificate.domains.join(', ')}
						</p>
						<p class="mt-1 text-xs text-muted-foreground">
							{t('account.certificates.expires')}
							{certificate.expires_at}
							{certificate.in_use ? `· ${t('account.certificates.in_use')}` : ''}
						</p>
					</div>
					<div class="flex shrink-0 gap-2">
						<Button
							variant="outline"
							size="sm"
							disabled={testing}
							onclick={() => void test(certificate)}
							>{testing
								? t('account.certificates.testing')
								: t('account.certificates.test')}</Button
						><Button variant="outline" size="sm" onclick={() => void edit(certificate)}
							>{t('projects.common.edit')}</Button
						><Button
							variant="ghost"
							size="sm"
							disabled={certificate.in_use || busy}
							onclick={() => (removing = certificate)}>{t('projects.common.delete')}</Button>
					</div>
				</div>
			{/each}
		</div>
	{/if}
	<div>
		<Button size="sm" onclick={() => void edit(null)}
			><PlusIcon />{t('account.certificates.add')}</Button>
	</div>
</SettingsSection>
<Dialog.Root bind:open>
	<Dialog.Content
		class="flex max-h-[calc(100dvh-1rem)] w-[calc(100vw-1rem)] max-w-none flex-col gap-0 overflow-hidden p-0 sm:max-h-[92dvh] sm:w-[min(92vw,96rem)] sm:max-w-none">
		<Dialog.Header class="shrink-0 border-b border-border/60 px-4 py-4 pr-14 sm:px-6 sm:py-5"
			><Dialog.Title
				>{t(editing ? 'account.certificates.edit' : 'account.certificates.add')}</Dialog.Title
			><Dialog.Description>{t('account.certificates.custom_hint')}</Dialog.Description
			></Dialog.Header>
		<form
			class="flex min-h-0 flex-1 flex-col"
			onsubmit={(event) => {
				event.preventDefault();
				void save();
			}}>
			<div class="min-h-0 flex-1 space-y-5 overflow-y-auto px-4 py-4 sm:px-6 sm:py-5">
				<div class="grid gap-2">
					<Label for="certificate-name">{t('account.certificates.name')}</Label><Input
						id="certificate-name"
						bind:value={name}
						required
						maxlength={120}
						disabled={busy || loadingEdit}
						class="max-w-xl" />
				</div>
				{#if loadingEdit}<p role="status" class="text-sm text-muted-foreground">
						{t('account.certificates.loading_material')}
					</p>{/if}
				<div class="grid min-w-0 gap-5 lg:grid-cols-2">
					<div class="grid min-w-0 gap-2">
						<Label>{t('account.certificates.chain')}</Label><ScriptEditor
							language="plaintext"
							label={t('account.certificates.chain')}
							height="clamp(9rem, 22dvh, 13rem)"
							bind:value={certificatePem}
							disabled={busy || loadingEdit} />
					</div>
					<div class="grid min-w-0 gap-2">
						<Label>{t('account.certificates.private_key')}</Label><ScriptEditor
							language="plaintext"
							label={t('account.certificates.private_key')}
							height="clamp(9rem, 22dvh, 13rem)"
							bind:value={privateKeyPem}
							disabled={busy || loadingEdit} />
					</div>
				</div>
				{#if editing}<p class="text-xs text-muted-foreground">
						{t('account.certificates.keep_existing')}
					</p>{/if}
				{#if testResult}<CertificateTestResultView result={testResult} />{/if}
			</div>
			<Dialog.Footer
				class="grid shrink-0 grid-cols-2 gap-2 border-t border-border/60 bg-popover px-4 py-3 sm:flex sm:flex-row sm:justify-end sm:px-6"
				><Button
					type="button"
					variant="outline"
					class="col-span-2 w-full sm:mr-auto sm:w-auto"
					disabled={busy || loadingEdit || testing}
					onclick={() => void test(null)}
					>{testing ? t('account.certificates.testing') : t('account.certificates.test')}</Button
				><Button
					type="button"
					variant="outline"
					class="w-full sm:w-auto"
					disabled={busy}
					onclick={() => (open = false)}>{t('projects.projects.cancel')}</Button
				><Button
					type="submit"
					class="w-full sm:w-auto"
					disabled={busy ||
						loadingEdit ||
						testing ||
						!name.trim() ||
						(!editing && (!certificatePem.trim() || !privateKeyPem.trim())) ||
						Boolean(certificatePem.trim()) !== Boolean(privateKeyPem.trim())}
					>{t('account.common.save')}</Button
				></Dialog.Footer>
		</form>
	</Dialog.Content>
</Dialog.Root>
<Dialog.Root bind:open={testDialogOpen}>
	<Dialog.Content class="max-h-[90svh] overflow-y-auto"
		><Dialog.Header
			><Dialog.Title>{t('account.certificates.test_results')}</Dialog.Title></Dialog.Header
		>{#if testResult}<CertificateTestResultView result={testResult} />{/if}</Dialog.Content>
</Dialog.Root>
<Dialog.Root
	open={Boolean(removing)}
	onOpenChange={(value) => {
		if (!value && !busy) removing = null;
	}}>
	<Dialog.Content
		><Dialog.Header
			><Dialog.Title>{t('account.certificates.delete_title')}</Dialog.Title><Dialog.Description
				>{removing?.name}</Dialog.Description
			></Dialog.Header
		><Dialog.Footer
			><Button variant="outline" disabled={busy} onclick={() => (removing = null)}
				>{t('projects.projects.cancel')}</Button
			><Button variant="destructive" disabled={busy} onclick={remove}
				>{t('projects.common.delete')}</Button
			></Dialog.Footer
		></Dialog.Content>
</Dialog.Root>
