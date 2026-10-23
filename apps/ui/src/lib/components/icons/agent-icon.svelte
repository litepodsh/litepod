<script lang="ts">
	import Bot from '@lucide/svelte/icons/bot';

	let {
		agent,
		class: className = 'size-4'
	}: { agent: string | null; class?: string } = $props();
</script>

{#if agent === 'claude'}
	<!-- Claude's starburst mark -->
	<svg class={className} viewBox="0 0 24 24" fill="none" aria-hidden="true">
		<g stroke="#D97757" stroke-width="2" stroke-linecap="round">
			<path d="M12 2v20M2 12h20M4.9 4.9l14.2 14.2M19.1 4.9 4.9 19.1" />
			<path d="M12 6.2 17.8 12 12 17.8 6.2 12z" stroke-width="1.4" opacity=".45" />
		</g>
	</svg>
{:else if agent === 'chatgpt'}
	<!-- OpenAI rosette, simplified -->
	<svg class={className} viewBox="0 0 24 24" fill="none" aria-hidden="true">
		<g stroke="#10A37F" stroke-width="1.7">
			<ellipse cx="12" cy="12" rx="4.3" ry="9.2" />
			<ellipse cx="12" cy="12" rx="4.3" ry="9.2" transform="rotate(60 12 12)" />
			<ellipse cx="12" cy="12" rx="4.3" ry="9.2" transform="rotate(120 12 12)" />
		</g>
	</svg>
{:else if agent === 'hermes'}
	<!-- Hermes: the wing -->
	<svg class={className} viewBox="0 0 24 24" fill="none" aria-hidden="true">
		<g stroke="#6366F1" stroke-width="1.8" stroke-linecap="round">
			<path d="M21 4c-7.5.6-13 3.4-16 8.2-1.6 2.6-2.3 5.3-2 8.3 2.8-3.6 6-5.7 9.7-6.5" />
			<path d="M21 4c-3.1 3.5-6.4 6-10 7.6" />
			<path d="M21 4c-1.4 4.3-3.7 7.6-6.9 10" />
			<path d="M21 4c-.7 3.6-2.2 6.6-4.5 9.1" />
		</g>
	</svg>
{:else}
	<Bot class="{className} text-muted-foreground" />
{/if}
