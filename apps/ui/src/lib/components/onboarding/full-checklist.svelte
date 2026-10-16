<script lang="ts">
	import CheckCircle2Icon from '@lucide/svelte/icons/circle-check';
	import GitBranchIcon from '@lucide/svelte/icons/git-branch';
	import FileArchiveIcon from '@lucide/svelte/icons/file-archive';
	import RocketIcon from '@lucide/svelte/icons/rocket';
	import BellIcon from '@lucide/svelte/icons/bell';
	import { Button } from '#lib/components/ui/button/index.js';
	import SegmentedSwitch from '#lib/components/custom/segmented-switch.svelte';
	import type { GuideKey, OnboardingSummary } from '#lib/clients/onboarding-client.js';
	import { runOnboardingAction } from '#lib/state/onboarding.svelte.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';
	import TaskRow from './task-row.svelte';

	let { summary, compact = false }: { summary: OnboardingSummary; compact?: boolean } = $props();
	let guide = $derived(summary.guides.find((item) => item.key === summary.active_guide)!);

	async function act(action: Parameters<typeof runOnboardingAction>[0]) {
		try {
			await runOnboardingAction(action);
		} catch (error) {
			toast.error(
				error instanceof Error ? error.message : t('common.onboarding.save_preference_failed'),
			);
		}
	}

	function selectGuide(guide: GuideKey) {
		if (guide !== summary.active_guide) void act({ action: 'select_guide', guide });
	}
</script>

<section
	aria-label={t('common.onboarding.getting_started_checklist')}
	class={compact ? '' : 'lp-inset rounded-[1.8125rem] p-2'}>
	<div class="rounded-[1.25rem] bg-card {compact ? 'p-4 sm:p-5' : 'p-5 sm:p-6'}">
		<div class="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
			<div>
				<h2 class="text-base font-medium tracking-[-.03em]">
					{t('common.onboarding.getting_started')}
				</h2>
				<p class="mt-1 text-[11px] leading-[1.45] text-muted-foreground">
					{t('common.onboarding.app_intro')}
				</p>
			</div>
			<SegmentedSwitch
				class="w-full sm:w-60"
				label={t('common.onboarding.getting_started_guides')}
				value={summary.active_guide}
				onchange={selectGuide}
				items={[
					{ value: 'deploy', label: t('common.onboarding.deploy'), icon: RocketIcon },
					{ value: 'notifications', label: t('common.onboarding.notifications'), icon: BellIcon },
				]} />
		</div>

		{#if summary.active_guide === 'deploy' && !summary.deploy_route}
			<div class="mt-6 grid gap-3 sm:grid-cols-2">
				<button
					type="button"
					class="lp-raised rounded-2xl bg-background p-4 text-left active:scale-[.99]"
					onclick={() => act({ action: 'select_route', route: 'git' })}>
					<GitBranchIcon class="size-5 text-primary" strokeWidth={1.75} />
					<strong class="mt-4 block text-sm">{t('common.onboarding.deploy_from_git')}</strong>
					<span class="mt-1 block text-xs leading-relaxed text-muted-foreground"
						>{t('common.onboarding.deploy_from_git_description')}</span>
				</button>
				<button
					type="button"
					class="lp-raised rounded-2xl bg-background p-4 text-left active:scale-[.99]"
					onclick={() => act({ action: 'select_route', route: 'static' })}>
					<FileArchiveIcon class="size-5 text-primary" strokeWidth={1.75} />
					<strong class="mt-4 block text-sm">{t('common.onboarding.deploy_static_html')}</strong>
					<span class="mt-1 block text-xs leading-relaxed text-muted-foreground"
						>{t('common.onboarding.deploy_static_html_description')}</span>
				</button>
			</div>
		{:else if guide.is_complete}
			<div class="mt-6 flex items-start gap-3 rounded-2xl bg-muted/60 p-4">
				<CheckCircle2Icon class="mt-0.5 size-5 text-[var(--lp-ok)]" strokeWidth={1.75} />
				<div>
					<p class="text-sm font-medium">
						{t('common.onboarding.guide_complete', { title: guide.title })}
					</p>
					<p class="mt-1 text-xs text-muted-foreground">
						{t('common.onboarding.guide_complete_description')}
					</p>
				</div>
			</div>
		{:else}
			<div class="mt-4 divide-y divide-foreground/8">
				{#each guide.tasks as task (task.key)}<TaskRow {task} />{/each}
			</div>
		{/if}

		<div
			class="mt-5 flex flex-wrap items-center justify-between gap-3 border-t border-foreground/8 pt-4">
			<span class="font-mono text-[10px] tracking-wide text-muted-foreground"
				>{t('common.onboarding.complete_count', {
					completed: guide.completed,
					total: guide.total || 'CHOICE',
				})}</span>
			<Button variant="ghost" size="xs" onclick={() => act({ action: 'dismiss' })}
				>{t('common.onboarding.mark_done')}</Button>
		</div>
	</div>
</section>
