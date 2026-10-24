<script lang="ts">
	import { onDestroy, onMount } from 'svelte';
	import ExternalLinkIcon from '@lucide/svelte/icons/external-link';
	import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';
	import { Button } from '#lib/components/ui/button/index.js';
	import { getSystemVersion, type SystemVersion } from '#lib/clients/system-client.js';
	import { updateState } from '#lib/state/update.svelte.js';
	import { isInstallableUpdate, SYSTEM_UPDATE_CHANGED_EVENT } from '#lib/utils/system-update.js';

	const RELEASES_URL = 'https://github.com/litepodsh/litepod/releases';

	let { isSysadmin }: { isSysadmin: boolean } = $props();
	let info = $state<SystemVersion | null>(null);
	let pollTimer: number | null = null;

	const updateAvailable = $derived(isSysadmin && isInstallableUpdate(info));

	async function refresh() {
		if (!isSysadmin) {
			info = null;
			return;
		}
		try {
			info = await getSystemVersion();
		} catch (error) {
			console.warn('[sidebar-update] could not load update state', error);
		}
	}

	onMount(() => {
		pollTimer = window.setInterval(refresh, 60_000);
		window.addEventListener(SYSTEM_UPDATE_CHANGED_EVENT, refresh);
	});

	onDestroy(() => {
		if (pollTimer) window.clearInterval(pollTimer);
		window.removeEventListener(SYSTEM_UPDATE_CHANGED_EVENT, refresh);
		// reset() is a no-op while an install is running - the grace/watchdog
		// timers own the reload and must survive this component unmounting.
		updateState.reset();
	});

	$effect(() => {
		if (isSysadmin) void refresh();
		else info = null;
	});
</script>

{#if updateAvailable}
	<Button variant="ghost" size="sm" href={RELEASES_URL} target="_blank" rel="noopener noreferrer">
		<ExternalLinkIcon />
		<span class="max-md:sr-only">Release notes</span>
	</Button>
	<Button
		size="sm"
		onclick={() => void updateState.startUpdate()}
		aria-disabled={updateState.applying}>
		<RefreshCwIcon class={updateState.applying ? 'animate-spin' : ''} />
		<span class="max-md:sr-only">{updateState.applying ? 'Installing…' : 'Update available'}</span>
	</Button>
{/if}
