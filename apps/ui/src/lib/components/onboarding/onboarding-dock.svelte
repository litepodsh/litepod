<script lang="ts">
	import { onMount } from 'svelte';
	import SkeletonRows from '#lib/components/custom/skeleton-rows.svelte';
	import { assistantState } from '#lib/state/assistant.svelte.js';
	import {
		installOnboardingInvalidation,
		loadOnboarding,
		onboardingState,
	} from '#lib/state/onboarding.svelte.js';
	import CompactPanel from './compact-panel.svelte';
	import FullChecklist from './full-checklist.svelte';
	import { t } from '#lib/i18n/index.svelte.js';

	let { organizationId }: { organizationId: string | null } = $props();
	onMount(() => installOnboardingInvalidation());
	$effect(() => {
		void loadOnboarding(organizationId);
	});

	function closeOnEscape(event: KeyboardEvent) {
		if (event.key === 'Escape' && onboardingState.expanded) onboardingState.expanded = false;
	}
</script>

<svelte:window onkeydown={closeOnEscape} />

{#if onboardingState.loading && organizationId}
	<div class="fixed right-4 bottom-5 z-30 hidden w-72 sm:block">
		<SkeletonRows rows={2} height="h-10" label={t('common.onboarding.loading')} />
	</div>
{:else if onboardingState.summary && !onboardingState.summary.dismissed}
	{@const summary = onboardingState.summary}
	<!-- There is no sidebar any more, so a checklist pinned there floats on the right. -->
	{#if summary.panel_position === 'right' || summary.panel_position === 'sidebar'}
		<div
			class="fixed bottom-5 z-30 hidden transition-[transform,opacity] duration-300 ease-[var(--lp-ease)] sm:block {onboardingState.expanded
				? 'w-[30rem]'
				: 'w-72'}"
			style:right={assistantState.mode !== 'nav'
				? `${assistantState.assistantWidth + 1}rem`
				: '1rem'}>
			{#if onboardingState.expanded}
				<div
					class="lp-lifted max-h-[min(42rem,calc(100dvh-2.5rem))] overflow-y-auto rounded-[1.8125rem] bg-background p-2">
					<FullChecklist {summary} compact />
					<button
						type="button"
						class="mt-2 min-h-8 w-full text-xs text-muted-foreground hover:text-foreground"
						onclick={() => (onboardingState.expanded = false)}
						>{t('common.onboarding.collapse_checklist')}</button>
				</div>
			{:else}
				<CompactPanel {summary} />
			{/if}
		</div>
	{/if}
	{#if summary.panel_position !== 'hidden'}
		<div
			class="fixed right-3 bottom-[calc(env(safe-area-inset-bottom)+.75rem)] left-3 z-30 sm:hidden">
			{#if onboardingState.expanded}
				<div
					class="lp-lifted max-h-[min(38rem,calc(100dvh-1.5rem))] overflow-y-auto rounded-[1.5rem] bg-background p-3">
					<FullChecklist {summary} compact />
					<button
						type="button"
						class="mt-2 min-h-10 w-full text-xs text-muted-foreground"
						onclick={() => (onboardingState.expanded = false)}
						>{t('common.onboarding.collapse_checklist')}</button>
				</div>
			{:else}
				<button
					type="button"
					class="lp-lifted min-h-12 w-full rounded-2xl bg-card px-4 text-left text-sm font-medium"
					onclick={() => (onboardingState.expanded = true)}>
					{t('common.onboarding.getting_started')}
					<span class="float-right font-mono text-xs text-muted-foreground"
						>{t('common.onboarding.open')}</span>
				</button>
			{/if}
		</div>
	{/if}
{/if}
