<script lang="ts">
	import CheckIcon from '@lucide/svelte/icons/check';
	import LockIcon from '@lucide/svelte/icons/lock-keyhole';
	import RotateCcwIcon from '@lucide/svelte/icons/rotate-ccw';
	import { Button } from '#lib/components/ui/button/index.js';
	import type { OnboardingTask } from '#lib/clients/onboarding-client.js';
	import { runOnboardingAction } from '#lib/state/onboarding.svelte.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let { task }: { task: OnboardingTask } = $props();
	let busy = $state(false);
	let complete = $derived(task.status === 'detected' || task.status === 'manual');

	async function update(action: 'mark_task' | 'reopen_task') {
		busy = true;
		try {
			await runOnboardingAction({ action, task: task.key });
		} catch (error) {
			toast.error(
				error instanceof Error ? error.message : t('common.onboarding.task_update_failed'),
			);
		} finally {
			busy = false;
		}
	}
</script>

<div
	class="grid grid-cols-[1.5rem_minmax(0,1fr)] gap-2.5 py-2.5 sm:grid-cols-[1.5rem_minmax(0,1fr)_auto]">
	<div
		class="mt-0.5 flex size-6 items-center justify-center rounded-[.6rem] border transition-[transform,opacity] duration-300 ease-[var(--lp-ease)] {complete
			? 'border-transparent bg-[var(--lp-ok)] text-white'
			: task.status === 'unavailable'
				? 'border-foreground/10 bg-muted text-muted-foreground'
				: 'border-foreground/15 bg-card text-transparent'}"
		aria-hidden="true">
		{#if complete}
			<CheckIcon class="size-3.5" strokeWidth={2} />
		{:else if task.status === 'unavailable'}
			<LockIcon class="size-3.5" strokeWidth={1.75} />
		{/if}
	</div>
	<div class="min-w-0">
		<div class="flex flex-wrap items-center gap-2">
			<p class="text-[13px] font-medium leading-tight">{task.title}</p>
			{#if task.completion_source}
				<span class="font-mono text-[10px] tracking-wide text-muted-foreground">
					{task.completion_source === 'auto'
						? t('common.onboarding.auto')
						: t('common.onboarding.manual')}
				</span>
			{/if}
		</div>
		<p class="mt-0.5 max-w-[60ch] text-[11px] leading-[1.45] text-muted-foreground">
			{task.blocked_reason ?? task.description}
		</p>
	</div>
	<div class="col-start-2 flex flex-wrap items-center gap-1.5 sm:col-start-3 sm:justify-end">
		{#if task.href && task.status === 'pending'}
			<Button href={task.href} size="xs">{t('common.onboarding.open_task')}</Button>
		{/if}
		{#if task.can_mark_done}
			<Button variant="outline" size="xs" disabled={busy} onclick={() => update('mark_task')}>
				{t('common.onboarding.mark_task_done')}
			</Button>
		{:else if task.status === 'manual'}
			<Button variant="ghost" size="icon-xs" disabled={busy} onclick={() => update('reopen_task')}>
				<RotateCcwIcon />
				<span class="sr-only">{t('common.onboarding.reopen_task', { title: task.title })}</span>
			</Button>
		{/if}
	</div>
</div>
