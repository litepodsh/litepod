<script lang="ts">
	import { Button } from '#lib/components/ui/button/index.js';
	import PlayIcon from '@lucide/svelte/icons/play';
	import RotateCwIcon from '@lucide/svelte/icons/rotate-cw';
	import SquareIcon from '@lucide/svelte/icons/square';

	let {
		status,
		busy = false,
		onStart,
		onStop,
		onRestart,
	}: {
		status: string;
		busy?: boolean;
		onStart: () => Promise<void>;
		onStop: () => Promise<void>;
		onRestart: () => Promise<void>;
	} = $props();

	const running = $derived(status === 'running');
	const startable = $derived(['stopped', 'paused', 'created'].includes(status));
</script>

{#if running || startable}
	<div class="flex items-center gap-1 rounded-full bg-muted/70 p-1 ring-1 ring-foreground/10">
		{#if running}
			<Button
				variant="ghost"
				size="sm"
				class="rounded-full transition-transform duration-500 ease-[cubic-bezier(0.32,0.72,0,1)] active:scale-[0.98]"
				disabled={busy}
				onclick={() => void onStop()}><SquareIcon class="size-3.5" />Stop</Button>
			<Button
				variant="ghost"
				size="sm"
				class="rounded-full transition-transform duration-500 ease-[cubic-bezier(0.32,0.72,0,1)] active:scale-[0.98]"
				disabled={busy}
				onclick={() => void onRestart()}><RotateCwIcon class="size-3.5" />Restart</Button>
		{:else}
			<Button
				variant="ghost"
				size="sm"
				class="rounded-full transition-transform duration-500 ease-[cubic-bezier(0.32,0.72,0,1)] active:scale-[0.98]"
				disabled={busy}
				onclick={() => void onStart()}><PlayIcon class="size-3.5" />Start</Button>
		{/if}
	</div>
{/if}
