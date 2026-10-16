<script lang="ts">
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import { Textarea } from '#lib/components/ui/textarea/index.js';
	import type { EnvironmentDto } from '#lib/clients/project-client.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let {
		open = $bindable(false),
		environment = null,
		onSubmit,
	}: {
		open?: boolean;
		environment?: EnvironmentDto | null;
		onSubmit: (input: { name: string; description: string }) => Promise<void>;
	} = $props();

	let name = $state('');
	let description = $state('');
	let saving = $state(false);
	let wasOpen = $state(false);

	$effect(() => {
		if (open && !wasOpen) {
			name = environment?.name ?? '';
			description = environment?.description ?? '';
		}
		wasOpen = open;
	});

	async function submit() {
		const trimmedName = name.trim();
		if (!trimmedName) {
			toast.error(t('projects.environment.name_required'));
			return;
		}

		saving = true;
		try {
			await onSubmit({ name: trimmedName, description: description.trim() });
			open = false;
		} catch (error) {
			toast.error(error instanceof Error ? error.message : t('projects.environment.save_failed'));
		} finally {
			saving = false;
		}
	}
</script>

<Dialog.Root bind:open>
	<Dialog.Content>
		<Dialog.Header>
			<Dialog.Title
				>{environment
					? t('projects.environment.edit_environment')
					: t('projects.environment.create')}</Dialog.Title>
			<Dialog.Description>
				{environment ? t('projects.environment.edit_hint') : t('projects.environment.create_hint')}
			</Dialog.Description>
		</Dialog.Header>

		<form
			class="grid gap-4"
			onsubmit={(event) => {
				event.preventDefault();
				void submit();
			}}>
			<div class="grid gap-2">
				<Label for="environment-name">{t('projects.environment.name')}</Label>
				<Input
					id="environment-name"
					bind:value={name}
					maxlength={100}
					placeholder={t('projects.environment.name_placeholder')}
					autofocus />
			</div>
			<div class="grid gap-2">
				<Label for="environment-description"
					>{t('projects.environment.description')}
					<span class="text-muted-foreground">({t('projects.environment.optional')})</span></Label>
				<Textarea
					id="environment-description"
					bind:value={description}
					maxlength={500}
					placeholder={t('projects.environment.description_placeholder')} />
			</div>
			<Dialog.Footer>
				<Button type="button" variant="outline" onclick={() => (open = false)}
					>{t('projects.projects.cancel')}</Button>
				<Button type="submit" disabled={saving}
					>{saving
						? t('projects.projects.saving')
						: environment
							? t('projects.projects.save_changes')
							: t('projects.environment.create')}</Button>
			</Dialog.Footer>
		</form>
	</Dialog.Content>
</Dialog.Root>
