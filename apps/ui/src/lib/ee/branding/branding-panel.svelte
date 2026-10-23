<script lang="ts">
	import { onMount } from 'svelte';
	import SettingsSection from '#lib/components/projects/settings-section.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import { getBranding, updateBranding } from './branding-client';
	import { applyBranding } from './branding.svelte';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let loading = $state(true);
	let saving = $state(false);
	let name = $state('');
	let logoUrl = $state('');
	let faviconUrl = $state('');

	function fill(value: { name: string | null; logo_url: string | null; favicon_url: string | null }) {
		name = value.name ?? '';
		logoUrl = value.logo_url ?? '';
		faviconUrl = value.favicon_url ?? '';
	}

	onMount(async () => {
		try {
			fill(await getBranding());
		} catch (err) {
			toast.error(err instanceof Error ? err.message : t('account.branding.errors.load'));
		} finally {
			loading = false;
		}
	});

	async function save() {
		saving = true;
		try {
			const saved = await updateBranding({
				name: name.trim() || null,
				logo_url: logoUrl.trim() || null,
				favicon_url: faviconUrl.trim() || null,
			});
			fill(saved);
			applyBranding(saved);
			toast.success(t('account.branding.success.saved'));
		} catch (err) {
			toast.error(err instanceof Error ? err.message : t('account.branding.errors.save'));
		} finally {
			saving = false;
		}
	}

	const disabled = $derived(loading || saving);
</script>

<form
	class="flex max-w-[820px] flex-col gap-4"
	onsubmit={(event) => {
		event.preventDefault();
		save();
	}}>
	<SettingsSection
		title={t('account.branding.title')}
		description={t('account.branding.description')}>
		<div class="grid gap-4">
			<div class="grid content-start gap-1.5">
				<Label for="branding-name">{t('account.branding.name')}</Label>
				<Input
					id="branding-name"
					maxlength={40}
					placeholder="litepod"
					bind:value={name}
					{disabled} />
				<p class="text-xs text-muted-foreground">{t('account.branding.name_hint')}</p>
			</div>
			<div class="grid content-start gap-1.5">
				<Label for="branding-logo">{t('account.branding.logo')}</Label>
				<Input
					id="branding-logo"
					type="url"
					class="font-mono"
					placeholder="https://example.com/logo.svg"
					bind:value={logoUrl}
					{disabled} />
				<p class="text-xs text-muted-foreground">{t('account.branding.logo_hint')}</p>
			</div>
			<div class="grid content-start gap-1.5">
				<Label for="branding-favicon">{t('account.branding.favicon')}</Label>
				<Input
					id="branding-favicon"
					type="url"
					class="font-mono"
					placeholder="https://example.com/favicon.png"
					bind:value={faviconUrl}
					{disabled} />
				<p class="text-xs text-muted-foreground">{t('account.branding.favicon_hint')}</p>
			</div>
		</div>
	</SettingsSection>
	<div class="flex justify-end">
		<Button type="submit" {disabled}
			>{saving ? t('account.common.saving') : t('account.common.save')}</Button>
	</div>
</form>
