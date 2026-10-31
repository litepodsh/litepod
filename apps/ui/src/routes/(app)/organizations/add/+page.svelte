<script lang="ts">
	import { brandedTitle } from '#lib/ee/branding/branding.svelte.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import { createOrganization } from '#lib/clients/organization-client.js';
	import { toast } from '#lib/utils/toast.js';
	import { ORGANIZATION_ICON_MAP } from '#lib/utils/organization-icons.js';
	import OrganizationIcon from '#lib/components/organization-icon.svelte';
	import ColorPicker from '#lib/components/color-picker.svelte';
	import ArrowLeftIcon from '@lucide/svelte/icons/arrow-left';
	import { t } from '#lib/i18n/index.svelte.js';

	let name = $state('');
	let plan = $state('');
	let icon = $state('shield');
	let iconBg = $state('');
	let loading = $state(false);

	const iconOptions = Object.keys(ORGANIZATION_ICON_MAP);

	function slugify(value: string): string {
		return value
			.toLowerCase()
			.trim()
			.replace(/[^a-z0-9]+/g, '-')
			.replace(/^-+|-+$/g, '');
	}

	async function handleCreate() {
		const trimmed = name.trim();
		if (!trimmed) {
			toast.error(t('organizations.add.name_required'));
			return;
		}
		const slug = slugify(trimmed);
		if (!slug) {
			toast.error(t('organizations.add.name_invalid'));
			return;
		}

		loading = true;
		try {
			await createOrganization({
				name: trimmed,
				slug,
				plan: plan.trim() || trimmed,
				icon,
				icon_bg: iconBg,
			});
			toast.success(t('organizations.add.created'));
			// Full-load nav so the header's org switcher refetches its organization list.
			window.location.assign('/home');
		} catch (err) {
			toast.error(err instanceof Error ? err.message : t('organizations.add.create_failed'));
			loading = false;
		}
	}
</script>

<svelte:head>
	<title>{brandedTitle(t('organizations.add.title'))}</title>
</svelte:head>

<div class="flex flex-col gap-5 px-4 py-6 sm:px-6">
	<div class="min-w-0">
		<a
			href="/organizations"
			class="mb-2 inline-flex items-center gap-1.5 text-sm text-muted-foreground hover:text-foreground">
			<ArrowLeftIcon class="size-4" />
			{t('organizations.list.title')}
		</a>
		<h1 class="text-xl font-semibold tracking-tight">{t('organizations.add.title')}</h1>
		<p class="text-sm text-muted-foreground">{t('organizations.add.description')}</p>
	</div>

	<form
		class="lp-raised grid max-w-xl gap-5 rounded-2xl border bg-card p-5 sm:p-6"
		onsubmit={(event) => {
			event.preventDefault();
			handleCreate();
		}}>
		<div class="grid gap-2">
			<Label for="organization-name">{t('organizations.common.name')}</Label>
			<Input
				id="organization-name"
				bind:value={name}
				maxlength={100}
				placeholder="Acme Inc."
				autofocus />
		</div>

		<div class="grid gap-2">
			<Label for="organization-plan">{t('organizations.add.subtitle')}</Label>
			<Input
				id="organization-plan"
				bind:value={plan}
				placeholder={t('organizations.add.subtitle_placeholder')} />
		</div>

		<div class="grid gap-2">
			<Label for="organization-icon">{t('organizations.common.icon')}</Label>
			<div class="flex items-center gap-2">
				<span
					class="lp-inset grid size-9 shrink-0 place-items-center overflow-hidden rounded-[9px] border">
					<OrganizationIcon {icon} bg={iconBg} class="size-4" />
				</span>
				<Input
					id="organization-icon"
					bind:value={icon}
					placeholder={t('organizations.add.icon_placeholder')} />
				<ColorPicker bind:value={iconBg} />
				{#if iconBg}
					<Button type="button" variant="ghost" size="sm" onclick={() => (iconBg = '')}
						>{t('organizations.common.clear')}</Button>
				{/if}
			</div>
			<div class="flex flex-wrap gap-1.5">
				{#each iconOptions as opt (opt)}
					<button
						type="button"
						class="grid size-8 place-items-center rounded-lg border transition-colors hover:bg-muted {icon ===
						opt
							? 'border-primary/50 bg-primary/10 text-primary'
							: ''}"
						onclick={() => (icon = opt)}
						title={opt}>
						<OrganizationIcon icon={opt} class="size-4" />
					</button>
				{/each}
			</div>
		</div>

		<div class="lp-groove-t flex justify-end gap-2 pt-4">
			<Button type="button" variant="outline" href="/organizations"
				>{t('organizations.common.cancel')}</Button>
			<Button type="submit" disabled={loading}>
				{loading ? t('organizations.add.creating') : t('organizations.add.title')}
			</Button>
		</div>
	</form>
</div>
