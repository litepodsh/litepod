<script lang="ts">
	import { onMount } from 'svelte';
	import { formatDuration } from '#lib/utils/format-duration.js';

	let { startedAt }: { startedAt: number } = $props();

	let now = $state(Date.now());

	onMount(() => {
		const tick = window.setInterval(() => (now = Date.now()), 1000);
		return () => window.clearInterval(tick);
	});

	let elapsed = $derived(Math.max(0, now - startedAt));
</script>

<span>{formatDuration(elapsed) ?? '0s'}</span>
