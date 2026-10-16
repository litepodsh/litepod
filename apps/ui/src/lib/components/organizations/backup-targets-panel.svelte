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
		createS3Target,
		deleteS3Target,
		listS3Targets,
		type S3TargetDto,
	} from '#lib/clients/backup-client.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';

	/** Shown only with `backup.targets.manage`; the API enforces the same. */
	let { organizationId }: { organizationId: string } = $props();

	const emptyTarget = () => ({
		name: '',
		endpoint_url: '',
		bucket: '',
		region: 'us-east-1',
		access_key_id: '',
		secret_access_key: '',
	});

	let targets = $state<S3TargetDto[]>([]);
	let loading = $state(true);
	let saving = $state(false);
	let newTarget = $state(emptyTarget());
	let confirmDelete = $state<S3TargetDto | null>(null);
	let deleting = $state(false);

	const complete = $derived(
		[
			newTarget.name,
			newTarget.endpoint_url,
			newTarget.bucket,
			newTarget.region,
			newTarget.access_key_id,
			newTarget.secret_access_key,
		].every((value) => value.trim() !== ''),
	);

	async function load() {
		try {
			targets = await listS3Targets(organizationId);
		} catch (e) {
			toast.error(t('organizations.backup_targets.load_failed', { message: (e as Error).message }));
		} finally {
			loading = false;
		}
	}

	async function addTarget(event: SubmitEvent) {
		event.preventDefault();
		if (!complete) return;
		saving = true;
		try {
			await createS3Target(organizationId, { ...newTarget });
			toast.success(t('organizations.backup_targets.add_success'));
			newTarget = emptyTarget();
			await load();
		} catch (e) {
			toast.error(t('organizations.backup_targets.add_failed', { message: (e as Error).message }));
		} finally {
			saving = false;
		}
	}

	async function removeTarget() {
		if (!confirmDelete) return;
		deleting = true;
		const target = confirmDelete;
		try {
			await deleteS3Target(organizationId, target.id);
			targets = targets.filter((item) => item.id !== target.id);
			toast.success(t('organizations.backup_targets.delete_success'));
			confirmDelete = null;
		} catch (e) {
			toast.error(
				t('organizations.backup_targets.delete_failed', { message: (e as Error).message }),
			);
		} finally {
			deleting = false;
		}
	}

	onMount(load);
</script>

<div class="space-y-6">
	<div>
		<h2 class="text-lg font-semibold tracking-tight">{t('organizations.backup_targets.title')}</h2>
		<p class="mt-1 text-sm text-muted-foreground">
			{t('organizations.backup_targets.description')}
		</p>
	</div>

	<Card.Root class="overflow-hidden">
		<form onsubmit={addTarget}>
			<Card.Content class="grid gap-4 pt-6 sm:grid-cols-2">
				<div class="grid gap-2">
					<Label for="org-target-name">{t('organizations.backup_targets.name')}</Label>
					<Input
						id="org-target-name"
						bind:value={newTarget.name}
						placeholder={t('organizations.backup_targets.name_placeholder')}
						required />
				</div>
				<div class="grid gap-2 sm:col-span-2">
					<Label for="org-target-endpoint">{t('organizations.backup_targets.endpoint_url')}</Label>
					<Input
						id="org-target-endpoint"
						bind:value={newTarget.endpoint_url}
						placeholder="https://s3.example.com"
						required />
					<p class="text-xs text-muted-foreground">
						{t('organizations.backup_targets.endpoint_hint')}
					</p>
				</div>
				<div class="grid gap-2">
					<Label for="org-target-bucket">{t('organizations.backup_targets.bucket')}</Label>
					<Input id="org-target-bucket" bind:value={newTarget.bucket} required />
				</div>
				<div class="grid gap-2">
					<Label for="org-target-region">{t('organizations.backup_targets.region')}</Label>
					<Input
						id="org-target-region"
						bind:value={newTarget.region}
						placeholder="us-east-1"
						required />
				</div>
				<div class="grid gap-2">
					<Label for="org-target-access-key"
						>{t('organizations.backup_targets.access_key_id')}</Label>
					<Input
						id="org-target-access-key"
						bind:value={newTarget.access_key_id}
						autocomplete="off"
						required />
				</div>
				<div class="grid gap-2">
					<Label for="org-target-secret-key"
						>{t('organizations.backup_targets.secret_access_key')}</Label>
					<Input
						id="org-target-secret-key"
						type="password"
						bind:value={newTarget.secret_access_key}
						autocomplete="off"
						required />
				</div>
			</Card.Content>
			<Card.Footer class="justify-end border-t bg-muted/20">
				<Button type="submit" disabled={saving || !complete}>
					{saving
						? t('organizations.backup_targets.verifying')
						: t('organizations.backup_targets.add')}
				</Button>
			</Card.Footer>
		</form>
	</Card.Root>

	{#if loading}
		<SkeletonRows rows={2} />
	{:else if targets.length === 0}
		<p class="text-sm text-muted-foreground">{t('organizations.backup_targets.empty')}</p>
	{:else}
		<div class="lp-raised overflow-x-auto rounded-2xl border bg-card">
			<Table.Root>
				<Table.Header>
					<Table.Row>
						<Table.Head>{t('organizations.backup_targets.name')}</Table.Head>
						<Table.Head>{t('organizations.backup_targets.endpoint_url')}</Table.Head>
						<Table.Head>{t('organizations.backup_targets.bucket')}</Table.Head>
						<Table.Head>{t('organizations.backup_targets.region')}</Table.Head>
						<Table.Head class="w-12"
							><span class="sr-only">{t('organizations.backup_targets.delete')}</span></Table.Head>
					</Table.Row>
				</Table.Header>
				<Table.Body>
					{#each targets as target (target.id)}
						<Table.Row>
							<Table.Cell class="font-medium">{target.name}</Table.Cell>
							<Table.Cell class="text-sm text-muted-foreground">{target.endpoint_url}</Table.Cell>
							<Table.Cell class="text-sm text-muted-foreground">{target.bucket}</Table.Cell>
							<Table.Cell class="text-sm text-muted-foreground">{target.region}</Table.Cell>
							<Table.Cell class="text-right">
								<Button
									variant="ghost"
									size="icon"
									aria-label={t('organizations.backup_targets.delete')}
									onclick={() => (confirmDelete = target)}>
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
			<AlertDialog.Title>{t('organizations.backup_targets.delete_title')}</AlertDialog.Title>
			<AlertDialog.Description>
				{confirmDelete
					? t('organizations.backup_targets.delete_confirm', { name: confirmDelete.name })
					: ''}
			</AlertDialog.Description>
		</AlertDialog.Header>
		<AlertDialog.Footer>
			<AlertDialog.Cancel>{t('organizations.common.cancel')}</AlertDialog.Cancel>
			<DeleteConfirmControl disabled={deleting} onconfirm={removeTarget} />
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>
