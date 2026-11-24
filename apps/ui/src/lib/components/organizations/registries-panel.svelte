<script lang="ts">
	import SkeletonRows from '#lib/components/custom/skeleton-rows.svelte';
	import { onMount } from 'svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import * as Table from '#lib/components/ui/table/index.js';
	import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
	import DeleteConfirmControl from '#lib/components/custom/delete-confirm-control.svelte';
	import TrashIcon from '@lucide/svelte/icons/trash-2';
	import {
		deleteRegistryCredential,
		listRegistryCredentials,
		saveRegistryCredential,
		type RegistryCredential,
	} from '#lib/clients/server-client.js';
	import { toastError, toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';

	/** Shown only with `server.manage`; the API enforces the same. */
	let { organizationId }: { organizationId: string } = $props();

	let credentials = $state<RegistryCredential[]>([]);
	let loading = $state(true);
	let saving = $state(false);
	let registry = $state('');
	let username = $state('');
	let password = $state('');
	let confirmDelete = $state<RegistryCredential | null>(null);
	let deleting = $state(false);

	const complete = $derived(!!registry.trim() && !!username.trim() && !!password);

	async function load() {
		try {
			credentials = await listRegistryCredentials(organizationId);
		} catch (err) {
			toastError(err, t('organizations.registries.load_failed'));
		} finally {
			loading = false;
		}
	}

	async function save(event: SubmitEvent) {
		event.preventDefault();
		if (!complete) return;
		saving = true;
		try {
			await saveRegistryCredential(organizationId, {
				registry: registry.trim(),
				username: username.trim(),
				password,
			});
			toast.success(t('organizations.registries.saved'));
			registry = '';
			username = '';
			password = '';
			await load();
		} catch (err) {
			toastError(err, t('organizations.registries.save_failed'));
		} finally {
			saving = false;
		}
	}

	async function remove() {
		if (!confirmDelete) return;
		deleting = true;
		const credential = confirmDelete;
		try {
			await deleteRegistryCredential(organizationId, credential.id);
			credentials = credentials.filter((item) => item.id !== credential.id);
			confirmDelete = null;
		} catch (err) {
			toastError(err, t('organizations.registries.delete_failed'));
		} finally {
			deleting = false;
		}
	}

	onMount(load);
</script>

<div class="space-y-6">
	<div>
		<h2 class="text-lg font-semibold tracking-tight">{t('organizations.registries.title')}</h2>
		<p class="mt-1 text-sm text-muted-foreground">{t('organizations.registries.description')}</p>
	</div>

	<Card.Root class="overflow-hidden">
		<form onsubmit={save}>
			<Card.Content class="grid gap-4 pt-6 sm:grid-cols-3">
				<div class="grid gap-2">
					<Label for="registry-host">{t('organizations.registries.registry')}</Label>
					<Input
						id="registry-host"
						bind:value={registry}
						placeholder="ghcr.io"
						autocomplete="off"
						spellcheck="false"
						required />
				</div>
				<div class="grid gap-2">
					<Label for="registry-username">{t('organizations.registries.username')}</Label>
					<Input id="registry-username" bind:value={username} autocomplete="off" required />
				</div>
				<div class="grid gap-2">
					<Label for="registry-password">{t('organizations.registries.password')}</Label>
					<Input
						id="registry-password"
						type="password"
						bind:value={password}
						autocomplete="new-password"
						required />
				</div>
				<p class="text-xs text-muted-foreground sm:col-span-3">
					{t('organizations.registries.hint')}
				</p>
			</Card.Content>
			<Card.Footer class="justify-end border-t bg-muted/20">
				<Button type="submit" disabled={saving || !complete}>
					{t('organizations.registries.save')}
				</Button>
			</Card.Footer>
		</form>
	</Card.Root>

	{#if loading}
		<SkeletonRows rows={2} />
	{:else if credentials.length === 0}
		<p class="text-sm text-muted-foreground">{t('organizations.registries.empty')}</p>
	{:else}
		<div class="lp-raised overflow-x-auto rounded-2xl border bg-card">
			<Table.Root>
				<Table.Header>
					<Table.Row>
						<Table.Head>{t('organizations.registries.registry')}</Table.Head>
						<Table.Head>{t('organizations.registries.username')}</Table.Head>
						<Table.Head
							><span class="sr-only">{t('organizations.registries.delete')}</span></Table.Head>
					</Table.Row>
				</Table.Header>
				<Table.Body>
					{#each credentials as credential (credential.id)}
						<Table.Row>
							<Table.Cell class="font-mono text-sm">{credential.registry}</Table.Cell>
							<Table.Cell class="text-sm text-muted-foreground">{credential.username}</Table.Cell>
							<Table.Cell class="text-right">
								<Button
									variant="ghost"
									size="icon"
									aria-label={t('organizations.registries.delete')}
									onclick={() => (confirmDelete = credential)}>
									<TrashIcon class="size-4" />
								</Button>
							</Table.Cell>
						</Table.Row>
					{/each}
				</Table.Body>
			</Table.Root>
		</div>
	{/if}
</div>

<AlertDialog.Root
	open={confirmDelete !== null}
	onOpenChange={(open) => !open && (confirmDelete = null)}>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title>{t('organizations.registries.delete_title')}</AlertDialog.Title>
			<AlertDialog.Description>
				{confirmDelete
					? t('organizations.registries.delete_confirm', { registry: confirmDelete.registry })
					: ''}
			</AlertDialog.Description>
		</AlertDialog.Header>
		<AlertDialog.Footer>
			<AlertDialog.Cancel>{t('organizations.common.cancel')}</AlertDialog.Cancel>
			<DeleteConfirmControl disabled={deleting} onconfirm={remove} />
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>
