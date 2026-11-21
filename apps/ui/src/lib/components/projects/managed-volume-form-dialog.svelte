<script lang="ts">
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import * as Select from '#lib/components/ui/select/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import type {
		CreateManagedVolumeInput,
		ManagedVolumeDto,
		S3TarDto,
		VolumeRestores,
	} from '#lib/clients/project-client.js';
	import { listS3TargetOptions, type S3TargetOption } from '#lib/clients/backup-client.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let {
		open = $bindable(false),
		importing = false,
		organizationId,
		isCompose,
		services = [],
		onSubmit,
		restores,
		onRestored,
	}: {
		open?: boolean;
		/** Create the volume from a `.tar` (in S3 or uploaded) instead of empty. */
		importing?: boolean;
		organizationId: string;
		isCompose: boolean;
		services?: string[];
		onSubmit: (input: CreateManagedVolumeInput) => Promise<void>;
		restores: VolumeRestores;
		/** A volume was created with content (detached): attach it and refresh. */
		onRestored: (volume: ManagedVolumeDto) => Promise<void>;
	} = $props();

	type Source = 'empty' | 's3' | 'upload';
	const IMPORT_SOURCES: Source[] = ['s3', 'upload'];

	let source = $state<Source>('empty');
	let name = $state('');
	let mountPath = $state('');
	let composeService = $state('');
	let saving = $state(false);
	let wasOpen = $state(false);
	let loading = $state(false);

	let targets = $state<S3TargetOption[]>([]);
	/** `''` is the default storage. */
	let targetId = $state('');
	let s3Files = $state<S3TarDto[]>([]);
	let s3Key = $state('');

	let file = $state<File | null>(null);

	$effect(() => {
		if (open && !wasOpen) {
			name = '';
			mountPath = '';
			composeService = '';
			s3Key = '';
			file = null;
			choose(importing ? 's3' : 'empty');
		}
		wasOpen = open;
	});

	async function load(action: () => Promise<void>) {
		loading = true;
		try {
			await action();
		} catch (error) {
			toast.error(error instanceof Error ? error.message : t('projects.volume.backups_failed'));
		} finally {
			loading = false;
		}
	}

	function choose(next: Source) {
		source = next;
		if (next === 's3') {
			void load(async () => {
				targets = await listS3TargetOptions(organizationId).catch(() => []);
				if (!targetId && targets.length) targetId = targets[0].id;
				await loadS3Files();
			});
		}
	}

	async function loadS3Files() {
		s3Key = '';
		s3Files = await restores.listS3Files(targetId || null);
	}

	/** Suggests a volume name from the picked source while the field is empty. */
	function suggestName(base: string) {
		if (!name.trim()) name = `${base}-restore`.slice(0, 100);
	}

	function fileLabel(object: S3TarDto) {
		return `${object.key} · ${new Date(object.last_modified).toLocaleString()}`;
	}

	async function submit() {
		const trimmedName = name.trim();
		const trimmedPath = mountPath.trim();
		if (source === 's3' && !s3Key) return toast.error(t('projects.volume.choose_s3_file'));
		if (source === 'upload' && !file) return toast.error(t('projects.volume.choose_tar'));
		if (!trimmedName) return toast.error(t('projects.volume.name_required'));
		if (!trimmedPath) return toast.error(t('projects.volume.mount_path_required'));
		if (isCompose && !composeService && trimmedPath)
			return toast.error(t('projects.volume.choose_service'));
		const input = {
			name: trimmedName,
			mount_path: trimmedPath,
			compose_service: isCompose && composeService ? composeService : null,
		};
		saving = true;
		try {
			if (source === 'empty') {
				await onSubmit(input);
			} else {
				const volume =
					source === 's3'
						? await restores.fromS3(targetId || null, s3Key, input)
						: await restores.fromUpload(file!, input);
				toast.success(t('projects.volume.restored_from_backup', { name: volume.name }));
				await onRestored(volume);
			}
			open = false;
		} catch (error) {
			toast.error(error instanceof Error ? error.message : t('projects.volume.add_failed'));
		} finally {
			saving = false;
		}
	}
</script>

<Dialog.Root bind:open>
	<Dialog.Content>
		<Dialog.Header>
			<Dialog.Title
				>{importing ? t('projects.volume.import_title') : t('projects.volume.add')}</Dialog.Title>
			<Dialog.Description
				>{importing ? t('projects.volume.import_hint') : t('projects.volume.add_hint')}</Dialog.Description>
		</Dialog.Header>
		<form
			class="grid gap-4"
			onsubmit={(event) => {
				event.preventDefault();
				void submit();
			}}>
			{#if importing}
				<div class="grid grid-cols-2 gap-1 rounded-md bg-muted p-1" role="radiogroup">
					{#each IMPORT_SOURCES as option (option)}
						<Button
							type="button"
							size="sm"
							role="radio"
							aria-checked={source === option}
							variant={source === option ? 'outline' : 'ghost'}
							onclick={() => choose(option)}>{t(`projects.volume.source_${option}`)}</Button>
					{/each}
				</div>
			{/if}

			{#if source === 's3'}
				<div class="grid gap-2">
					<Label for="volume-restore-target">{t('projects.db_backup.destination')}</Label>
					<select
						id="volume-restore-target"
						class="flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm"
						bind:value={targetId}
						onchange={() => void load(loadS3Files)}>
						<option value="">{t('projects.db_backup.destination_default')}</option>
						{#each targets as target (target.id)}
							<option value={target.id}>{target.name}</option>
						{/each}
					</select>
					<Label>{t('projects.volume.s3_file')}</Label>
					{#if loading}
						<p class="text-sm text-muted-foreground">{t('projects.db_backup.loading')}</p>
					{:else if s3Files.length === 0}
						<p class="rounded-md border border-dashed p-3 text-sm text-muted-foreground">
							{t('projects.volume.no_s3_files')}
						</p>
					{:else}
						<Select.Root
							type="single"
							value={s3Key}
							onValueChange={(value) => {
								s3Key = String(value ?? '');
								const base = s3Key.split('/').pop()?.replace(/\.tar$/, '');
								if (base) suggestName(base);
							}}>
							<Select.Trigger class="w-full truncate" aria-label={t('projects.volume.s3_file')}
								>{s3Key || t('projects.volume.choose_s3_file')}</Select.Trigger>
							<Select.Content
								>{#each s3Files as object (object.key)}<Select.Item
										value={object.key}
										label={fileLabel(object)} />{/each}</Select.Content>
						</Select.Root>
					{/if}
				</div>
			{:else if source === 'upload'}
				<div class="grid gap-2">
					<Label for="volume-restore-file">{t('projects.volume.tar_file')}</Label>
					<Input
						id="volume-restore-file"
						type="file"
						accept=".tar,application/x-tar"
						onchange={(event) => {
							file = event.currentTarget.files?.[0] ?? null;
							if (file) suggestName(file.name.replace(/\.tar$/, ''));
						}} />
					<p class="text-xs text-muted-foreground">{t('projects.volume.tar_hint')}</p>
				</div>
			{/if}

			<div class="grid gap-2">
				<Label for="managed-volume-name">{t('projects.volume.name')}</Label>
				<Input
					id="managed-volume-name"
					bind:value={name}
					maxlength={100}
					placeholder="uploads"
					autofocus />
			</div>
			<div class="grid gap-2">
				<Label for="managed-volume-mount-path">{t('projects.volume.mount_path')}</Label>
				<Input id="managed-volume-mount-path" bind:value={mountPath} placeholder="/app/data" />
			</div>
			{#if isCompose}
				<div class="grid gap-2">
					<Label>{t('projects.volume.compose_service')}</Label>
					<Select.Root
						type="single"
						value={composeService}
						onValueChange={(value) => (composeService = String(value ?? ''))}>
						<Select.Trigger class="w-full" aria-label={t('projects.volume.compose_service')}
							>{composeService || t('projects.volume.choose_service')}</Select.Trigger>
						<Select.Content
							>{#each services as service (service)}<Select.Item
									value={service}
									label={service} />{/each}</Select.Content>
					</Select.Root>
				</div>
			{/if}
			<Dialog.Footer>
				<Button type="button" variant="outline" onclick={() => (open = false)} disabled={saving}
					>{t('projects.projects.cancel')}</Button>
				<Button type="submit" disabled={saving}
					>{saving
						? t('projects.volume.creating')
						: importing
							? t('projects.volume.import')
							: t('projects.volume.create')}</Button>
			</Dialog.Footer>
		</form>
	</Dialog.Content>
</Dialog.Root>
