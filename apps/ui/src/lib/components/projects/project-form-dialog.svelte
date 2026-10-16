<script lang="ts">
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import { Textarea } from '#lib/components/ui/textarea/index.js';
	import IconUploadButton from '#lib/components/icon-upload-button.svelte';
	import FolderKanbanIcon from '@lucide/svelte/icons/folder-kanban';
	import type { ProjectDto } from '#lib/clients/project-client.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';

	type ProjectInput = { name: string; description?: string; icon_url?: string | null };

	let {
		open = $bindable(false),
		project = null,
		onsave,
		onUploadIcon,
	}: {
		open?: boolean;
		project?: ProjectDto | null;
		onsave: (input: ProjectInput) => Promise<void>;
		/** Only ever called while editing an existing project. */
		onUploadIcon?: (file: File) => Promise<string>;
	} = $props();

	let name = $state('');
	let description = $state('');
	let iconUrl = $state('');
	// Typing a URL updates `<img src>` on every keystroke, which fires a
	// browser request per character for a partial/invalid URL — the actual
	// preview only follows `iconUrl` after a pause, same debounce shape as
	// the catalog search in application-form-dialog.svelte.
	let previewIconUrl = $state('');
	let saving = $state(false);
	let wasOpen = $state(false);

	// `!wasOpen` matters: without it this effect re-fires on any later
	// reactive nudge while the dialog stays open (e.g. a refreshed `project`
	// reference), stomping whatever the user is mid-typing back to the saved
	// value — reseed once, on the open transition, not on every run.
	$effect(() => {
		if (open && !wasOpen) {
			name = project?.name ?? '';
			description = project?.description ?? '';
			iconUrl = project?.icon_url ?? '';
			previewIconUrl = iconUrl;
		}
		wasOpen = open;
	});

	$effect(() => {
		const value = iconUrl;
		const timer = window.setTimeout(() => {
			previewIconUrl = value;
		}, 400);
		return () => window.clearTimeout(timer);
	});

	async function handleSubmit() {
		const trimmedName = name.trim();
		if (!trimmedName) {
			toast.error(t('projects.projects.name_required'));
			return;
		}

		saving = true;
		try {
			await onsave({
				name: trimmedName,
				description: description.trim() || undefined,
				...(project ? { icon_url: iconUrl.trim() || null } : {}),
			});
			open = false;
		} catch (error) {
			toast.error(error instanceof Error ? error.message : t('projects.projects.save_failed'));
		} finally {
			saving = false;
		}
	}
</script>

<Dialog.Root bind:open>
	<Dialog.Content>
		<Dialog.Header>
			<Dialog.Title
				>{project
					? t('projects.projects.edit_project_named', { name: project.name })
					: t('projects.projects.create')}</Dialog.Title>
			<Dialog.Description>
				{project ? t('projects.projects.edit_hint') : t('projects.projects.create_hint')}
			</Dialog.Description>
		</Dialog.Header>

		<form
			onsubmit={(event) => {
				event.preventDefault();
				void handleSubmit();
			}}>
			<div class="grid gap-4 py-4">
				<div class="grid gap-2">
					<Label for="project-name">{t('projects.projects.name')}</Label>
					<Input id="project-name" bind:value={name} maxlength={100} autofocus disabled={saving} />
				</div>
				<div class="grid gap-2">
					<Label for="project-description"
						>{t('projects.projects.description')}
						<span class="text-muted-foreground">({t('projects.projects.optional')})</span></Label>
					<Textarea
						id="project-description"
						bind:value={description}
						maxlength={2000}
						placeholder={t('projects.projects.description_placeholder')}
						disabled={saving} />
				</div>
				{#if project}
					<div class="grid gap-2">
						<Label for="project-icon">{t('projects.projects.icon')}</Label>
						<div class="flex items-center gap-2">
							<div
								class="flex size-9 shrink-0 items-center justify-center rounded-md border overflow-hidden bg-muted">
								{#if previewIconUrl.trim()}
									<img
										src={previewIconUrl}
										alt=""
										loading="lazy"
										class="size-full object-contain" />
								{:else}
									<FolderKanbanIcon class="size-4 text-muted-foreground" />
								{/if}
							</div>
							<Input
								id="project-icon"
								bind:value={iconUrl}
								disabled={saving}
								placeholder={t('projects.projects.icon_placeholder')} />
							{#if onUploadIcon}
								<IconUploadButton
									onUpload={onUploadIcon}
									onUploaded={(url) => {
										iconUrl = url;
										previewIconUrl = url;
									}} />
							{/if}
						</div>
					</div>
				{/if}
			</div>
			<Dialog.Footer>
				<Button type="button" variant="outline" disabled={saving} onclick={() => (open = false)}
					>{t('projects.projects.cancel')}</Button>
				<Button type="submit" disabled={saving}
					>{saving
						? t('projects.projects.saving')
						: project
							? t('projects.projects.save_changes')
							: t('projects.projects.create')}</Button>
			</Dialog.Footer>
		</form>
	</Dialog.Content>
</Dialog.Root>
