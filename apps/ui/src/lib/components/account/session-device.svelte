<script lang="ts">
	import { parseDevice } from '#lib/utils/device-info.js';
	import AppleIcon from '#lib/components/icons/apple-icon.svelte';
	import LinuxIcon from '#lib/components/icons/linux-icon.svelte';
	import WindowsIcon from '#lib/components/icons/windows-icon.svelte';
	import AndroidIcon from '#lib/components/icons/android-icon.svelte';
	import Smartphone from '@lucide/svelte/icons/smartphone';
	import Monitor from '@lucide/svelte/icons/monitor';
	import Compass from '@lucide/svelte/icons/compass';
	import Flame from '@lucide/svelte/icons/flame';
	import Globe from '@lucide/svelte/icons/globe';
	import { t } from '#lib/i18n/index.svelte.js';

	let { userAgent }: { userAgent: string | null } = $props();

	const device = $derived(parseDevice(userAgent));
</script>

<div class="flex items-center gap-2">
	<div class="flex items-center" aria-label={device.osLabel}>
		{#if device.osKey === 'macos' || device.osKey === 'ios'}
			<AppleIcon class="size-4" />
		{:else if device.osKey === 'windows'}
			<WindowsIcon class="size-4" />
		{:else if device.osKey === 'linux'}
			<LinuxIcon class="size-4" />
		{:else if device.osKey === 'android'}
			<AndroidIcon class="size-4" />
		{:else if device.deviceType === 'mobile' || device.deviceType === 'tablet'}
			<Smartphone class="text-muted-foreground size-4" />
		{:else}
			<Monitor class="text-muted-foreground size-4" />
		{/if}
	</div>
	<div class="text-muted-foreground flex items-center" aria-label={device.browserLabel}>
		{#if device.browserKey === 'chrome' || device.browserKey === 'safari'}
			<Compass class="size-3.5" />
		{:else if device.browserKey === 'firefox'}
			<Flame class="size-3.5" />
		{:else if device.browserKey === 'edge' || device.browserKey === 'opera'}
			<Globe class="size-3.5" />
		{:else}
			<Globe class="size-3.5" />
		{/if}
	</div>
	<span class="text-sm">
		{t('account.sessions.device_browser', { browser: device.browserLabel, os: device.osLabel })}
	</span>
</div>
