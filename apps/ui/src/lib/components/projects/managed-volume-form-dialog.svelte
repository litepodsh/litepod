<script lang="ts">
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import * as Select from '#lib/components/ui/select/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import type { CreateManagedVolumeInput } from '#lib/clients/project-client.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let {
		open = $bindable(false),
		isCompose,
		services = [],
		onSubmit,
	}: {
		open?: boolean;
		isCompose: boolean;
		services?: string[];
		onSubmit: (input: CreateManagedVolumeInput) => Promise<void>;
	} = $props();

	let name = $state('');
	let mountPath = $state('');
	let composeService = $state('');
	let saving = $state(false);
	let wasOpen = $state(false);

	$effect(() => {
		if (open && !wasOpen) {
			name = '';
			mountPath = '';
			composeService = '';
		}
		wasOpen = open;
	});

	async function submit() {
		const trimmedName = name.trim();
		const trimmedPath = mountPath.trim();
		if (!trimmedName) return toast.error(t('projects.volume.name_required'));
		if (!trimmedPath) return toast.error(t('projects.volume.mount_path_required'));
		if (isCompose && !composeService) return toast.error(t('projects.volume.choose_service'));
		saving = true;
		try {
			await onSubmit({
				name: trimmedName,
				mount_path: trimmedPath,
				compose_service: isCompose ? composeService : null,
			});
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
			<Dialog.Title>{t('projects.volume.add')}</Dialog.Title>
			<Dialog.Description>{t('projects.volume.add_hint')}</Dialog.Description>
		</Dialog.Header>
		<form
			class="grid gap-4"
			onsubmit={(event) => {
				event.preventDefault();
				void submit();
			}}>
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
					>{saving ? t('projects.volume.creating') : t('projects.volume.create')}</Button>
			</Dialog.Footer>
		</form>
	</Dialog.Content>
</Dialog.Root>
