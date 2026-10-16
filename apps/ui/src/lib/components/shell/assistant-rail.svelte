<script lang="ts">
	/**
	 * The assistant without a sidebar to live in: a floating rail on the right,
	 * under the header. Non-modal on purpose, so it stays usable next to the
	 * logs or settings it is answering about. Any caller that asks for the
	 * assistant (`setAssistantMode('chat' | 'detached')`) opens it.
	 */
	import { fly } from 'svelte/transition';
	import AssistantPanel from '#lib/components/assistant/assistant-panel.svelte';
	import ResizeGrip from '#lib/components/assistant/resize-grip.svelte';
	import {
		ASSISTANT_WIDTH_MAX,
		ASSISTANT_WIDTH_MIN,
		assistantState,
		setAssistantMode,
		setAssistantWidth,
	} from '#lib/state/assistant.svelte.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let { organizationId, scope }: { organizationId: string | null; scope?: string } = $props();

	function closeOnEscape(event: KeyboardEvent) {
		if (event.key === 'Escape' && assistantState.mode !== 'nav') setAssistantMode('nav');
	}
</script>

<svelte:window onkeydown={closeOnEscape} />

{#if assistantState.available && assistantState.mode !== 'nav'}
	<aside
		aria-label={t('menu.header.assistant')}
		class="fixed inset-x-2 top-[calc(var(--app-header-height)+0.5rem)] bottom-2 z-40 sm:inset-x-auto sm:right-3 sm:w-(--rail-width)"
		style="--rail-width: {assistantState.assistantWidth}rem"
		transition:fly={{ x: 24, duration: 260 }}>
		<ResizeGrip
			value={assistantState.assistantWidth}
			min={ASSISTANT_WIDTH_MIN}
			max={ASSISTANT_WIDTH_MAX}
			sign={-1}
			class="start-0 max-sm:hidden"
			label="Resize assistant panel"
			onresize={setAssistantWidth} />
		<div
			class="lp-lifted flex size-full min-w-0 flex-col overflow-hidden rounded-[1.5rem] border bg-card">
			<AssistantPanel docked={false} {organizationId} {scope} />
		</div>
	</aside>
{/if}
