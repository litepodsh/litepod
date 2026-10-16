<script lang="ts">
	import ArrowRightIcon from '@lucide/svelte/icons/arrow-right';
	import CheckCircle2Icon from '@lucide/svelte/icons/circle-check';
	import { Button } from '#lib/components/ui/button/index.js';
	import type { OnboardingSummary } from '#lib/clients/onboarding-client.js';
	import { onboardingState, runOnboardingAction } from '#lib/state/onboarding.svelte.js';
	import { t } from '#lib/i18n/index.svelte.js';
	import OnboardingMenu from './onboarding-menu.svelte';

	let { summary, embedded = false }: { summary: OnboardingSummary; embedded?: boolean } = $props();
	let guide = $derived(summary.guides.find((item) => item.key === summary.active_guide)!);
	let next = $derived(guide.tasks.find((task) => task.key === guide.next_task_key));
</script>

<aside
	aria-label={t('common.onboarding.getting_started')}
	class="lp-inset w-full rounded-[1.25rem] p-1.5 {embedded ? '' : 'lp-lifted sm:w-72'}">
	<div class="rounded-[.875rem] bg-card p-3.5">
		<div class="flex items-center justify-between gap-2">
			<button
				type="button"
				class="min-w-0 text-left"
				onclick={() => (onboardingState.expanded = true)}>
				<span class="block font-mono text-[9px] tracking-[.1em] text-muted-foreground uppercase">
					{t('common.onboarding.getting_started')}
				</span>
				<span class="mt-0.5 block truncate text-xs font-medium">{guide.title}</span>
			</button>
			<OnboardingMenu />
		</div>

		<div
			class="mt-3 flex gap-1"
			aria-label={t('common.onboarding.complete_progress', {
				completed: guide.completed,
				total: guide.total,
			})}>
			{#each Array(Math.max(guide.total, 1)) as _, index}
				<span
					class="h-1 flex-1 rounded-full {index < guide.completed
						? 'bg-[var(--lp-ok)]'
						: 'bg-muted'}"></span>
			{/each}
		</div>

		{#if summary.active_guide === 'deploy' && !summary.deploy_route}
			<p class="mt-3 text-xs leading-relaxed text-muted-foreground">
				{t('common.onboarding.choose_deployment')}
			</p>
			<Button class="mt-3 w-full" size="sm" onclick={() => (onboardingState.expanded = true)}>
				{t('common.onboarding.choose_route')}
				<ArrowRightIcon />
			</Button>
		{:else if guide.is_complete}
			<div class="mt-3 flex items-center gap-2 text-xs">
				<CheckCircle2Icon class="size-4 text-[var(--lp-ok)]" /><span
					>{t('common.onboarding.complete')}</span>
			</div>
		{:else if next}
			<p class="mt-3 text-[13px] font-medium">{next.title}</p>
			<p class="mt-1 line-clamp-2 text-[11px] leading-[1.45] text-muted-foreground">
				{next.description}
			</p>
			<Button class="mt-3 w-full" size="sm" onclick={() => (onboardingState.expanded = true)}>
				{t('common.onboarding.open_full_checklist')}
				<ArrowRightIcon />
			</Button>
		{/if}

		<div class="mt-3 grid grid-cols-2 gap-1 rounded-full bg-muted p-1">
			{#each summary.guides as option}
				<button
					type="button"
					class="min-h-7 rounded-full px-2 text-[10px] font-medium {option.key ===
					summary.active_guide
						? 'lp-raised bg-card'
						: 'text-muted-foreground'}"
					onclick={() => runOnboardingAction({ action: 'select_guide', guide: option.key })}>
					{option.key === 'deploy'
						? t('common.onboarding.deploy')
						: t('common.onboarding.notifications')}
				</button>
			{/each}
		</div>
	</div>
</aside>
