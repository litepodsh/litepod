<script lang="ts">
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import {
		updateOrganization,
		uploadOrganizationIcon,
		type OrganizationDto,
	} from '#lib/clients/organization-client.js';
	import { toast } from '#lib/utils/toast.js';
	import { ORGANIZATION_ICON_MAP } from '#lib/utils/organization-icons.js';
	import OrganizationIcon from '#lib/components/organization-icon.svelte';
	import ColorPicker from '#lib/components/color-picker.svelte';
	import IconUploadButton from '#lib/components/icon-upload-button.svelte';
	import { t } from '#lib/i18n/index.svelte.js';

	let {
		open = $bindable(false),
		organization,
		onSaved,
	}: {
		open?: boolean;
		/** The organization being edited; `null` keeps the dialog inert. */
		organization: OrganizationDto | null;
		/** Receives the updated row so each caller can refresh its own copy. */
		onSaved?: (organization: OrganizationDto) => void;
	} = $props();

	const iconOptions = Object.keys(ORGANIZATION_ICON_MAP);

	let name = $state('');
	let plan = $state('');
	let icon = $state('');
	let iconBg = $state('');
	// Typing a URL updates `<img src>` on every keystroke, which fires a
	// browser request per character for a partial/invalid URL — the preview
	// swatch only follows `icon` after a pause.
	let previewIcon = $state('');
	let saving = $state(false);
	let wasOpen = $state(false);

	// Reseed the fields on the open transition only (`!wasOpen`), so a
	// cancelled edit is not still sitting in the inputs the next time it is
	// opened — and so this doesn't re-fire mid-edit on some later reactive
	// nudge and stomp whatever the user is typing back to the saved value.
	$effect(() => {
		if (open && !wasOpen && organization) {
			name = organization.name;
			plan = organization.plan;
			icon = organization.icon || 'shield';
			iconBg = organization.icon_bg || '';
			previewIcon = icon;
		}
		wasOpen = open;
	});

	$effect(() => {
		const value = icon;
		const timer = window.setTimeout(() => {
			previewIcon = value;
		}, 400);
		return () => window.clearTimeout(timer);
	});

	async function save() {
		if (!organization) return;
		const trimmed = name.trim();
		if (!trimmed) {
			toast.error(t('organizations.add.name_required'));
			return;
		}

		saving = true;
		try {
			// The slug is the organization's stable identity and the API refuses to
			// change it, so a rename never invalidates an existing reference.
			const updated = await updateOrganization(organization.id, {
				name: trimmed,
				plan: plan.trim() || trimmed,
				icon,
				icon_bg: iconBg,
			});
			onSaved?.(updated);
			toast.success(t('organizations.edit.updated'));
			open = false;
		} catch (err) {
			toast.error(err instanceof Error ? err.message : t('organizations.edit.update_failed'));
		} finally {
			saving = false;
		}
	}
</script>

<Dialog.Root bind:open>
	<Dialog.Content>
		<Dialog.Header>
			<Dialog.Title>{t('organizations.edit.title')}</Dialog.Title>
			<Dialog.Description>{t('organizations.edit.description')}</Dialog.Description>
		</Dialog.Header>
		<form
			onsubmit={(event) => {
				event.preventDefault();
				save();
			}}>
			<div class="grid gap-4 py-4">
				<div class="grid gap-2">
					<Label for="organization-name">{t('organizations.common.name')}</Label>
					<Input id="organization-name" bind:value={name} maxlength={100} autofocus />
				</div>
				<div class="grid gap-2">
					<Label for="organization-plan">{t('organizations.add.subtitle')}</Label>
					<Input
						id="organization-plan"
						bind:value={plan}
						placeholder={t('organizations.edit.subtitle_placeholder')} />
				</div>
				<div class="grid gap-2">
					<Label for="organization-icon">{t('organizations.common.icon')}</Label>
					<div class="flex items-center gap-2">
						<div
							class="flex size-9 shrink-0 items-center justify-center rounded-md border overflow-hidden">
							<OrganizationIcon icon={previewIcon} bg={iconBg} class="size-4" />
						</div>
						<Input
							id="organization-icon"
							bind:value={icon}
							placeholder={t('organizations.edit.icon_placeholder')} />
						<IconUploadButton
							onUpload={(file) =>
								uploadOrganizationIcon(organization!.id, file).then((o) => o.icon)}
							onUploaded={(url) => {
								icon = url;
								previewIcon = url;
							}} />
						<ColorPicker bind:value={iconBg} />
						{#if iconBg}
							<Button type="button" variant="ghost" size="sm" onclick={() => (iconBg = '')}
								>{t('organizations.common.clear')}</Button>
						{/if}
					</div>
					<div class="flex flex-wrap gap-1.5">
						{#each iconOptions as option}
							<button
								type="button"
								class="flex size-8 items-center justify-center rounded-md border transition-colors hover:bg-accent {icon ===
								option
									? 'border-primary bg-accent'
									: ''}"
								onclick={() => (icon = option)}
								title={option}>
								<OrganizationIcon icon={option} class="size-4" />
							</button>
						{/each}
					</div>
				</div>
			</div>
			<Dialog.Footer>
				<Button type="button" variant="outline" onclick={() => (open = false)}
					>{t('organizations.common.cancel')}</Button>
				<Button type="submit" disabled={saving}>
					{saving ? t('organizations.common.saving') : t('organizations.common.save')}
				</Button>
			</Dialog.Footer>
		</form>
	</Dialog.Content>
</Dialog.Root>
