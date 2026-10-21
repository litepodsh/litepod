<script lang="ts">
	import { onMount } from 'svelte';
	import SettingsSection from '#lib/components/projects/settings-section.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import {
		getBuildConfiguration,
		updateBuildConfiguration,
	} from '#lib/clients/build-configuration-client.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let loading = $state(true);
	let saving = $state(false);
	let maxConcurrentBuilds = $state(3);
	let activeBuilds = $state(0);

	onMount(async () => {
		try {
			const configuration = await getBuildConfiguration();
			maxConcurrentBuilds = configuration.max_concurrent_builds;
			activeBuilds = configuration.active_builds;
		} catch (err) {
			toast.error(err instanceof Error ? err.message : t('account.settings.errors.save_builds'));
		} finally {
			loading = false;
		}
	});

	async function save() {
		saving = true;
		try {
			const saved = await updateBuildConfiguration(maxConcurrentBuilds);
			maxConcurrentBuilds = saved.max_concurrent_builds;
			activeBuilds = saved.active_builds;
			toast.success(t('account.settings.success.builds_saved'));
		} catch (err) {
			toast.error(err instanceof Error ? err.message : t('account.settings.errors.save_builds'));
		} finally {
			saving = false;
		}
	}
</script>

<div class="flex max-w-[820px] flex-col gap-4">
	<SettingsSection
		title={t('account.settings.tabs.builds')}
		description={t('account.settings.builds_description')}>
		<form
			class="flex flex-col gap-4"
			onsubmit={(event) => {
				event.preventDefault();
				save();
			}}>
			<div class="grid max-w-xs gap-1.5">
				<Label for="max-builds">{t('account.settings.max_concurrent_builds')}</Label>
				<Input
					id="max-builds"
					type="number"
					min="1"
					max="64"
					class="font-mono tabular-nums"
					bind:value={maxConcurrentBuilds}
					disabled={loading || saving} />
				<p class="text-xs text-muted-foreground">
					{t('account.settings.builds_active_hint', { active: activeBuilds })}
				</p>
			</div>
			<div class="flex justify-end">
				<Button type="submit" disabled={loading || saving}
					>{saving ? t('account.common.saving') : t('account.common.save')}</Button>
			</div>
		</form>
	</SettingsSection>
</div>
