<script lang="ts">
	import type { ScheduledTaskExecution } from '#lib/clients/scheduled-tasks-client.js';
	import { t } from '#lib/i18n/index.svelte.js';
	import { formatDuration, runDurationMs, runOutcome } from './task-utils.js';

	// One column of cells per run, oldest on the left. Height says how long it
	// took (log buckets); a failed run fills its whole column in red.
	let { executions, limit = 20 }: { executions: ScheduledTaskExecution[]; limit?: number } =
		$props();

	const S = 3.2;
	const G = 1.3;
	const P = S + G;
	const ROWS = 7;
	const STEP = P * 1.35;
	const H = ROWS * P - G;
	// seconds at which a run earns one more cell
	const BUCKETS = [1, 10, 60, 300, 1800];

	const runs = $derived(executions.slice(0, limit).reverse());

	function cells(execution: ScheduledTaskExecution) {
		const outcome = runOutcome(execution);
		if (outcome === 'err') return ROWS;
		if (outcome === 'live') return 1;
		const seconds = (runDurationMs(execution) ?? 0) / 1000;
		return 1 + BUCKETS.filter((limit) => seconds >= limit).length;
	}
</script>

<svg
	class="run-bars block h-auto w-30"
	viewBox="0 0 {limit * STEP} {H}"
	role="img"
	aria-label={t('organizations.task_ui.col_runs')}>
	{#each runs as execution, index (execution.id)}
		{@const outcome = runOutcome(execution)}
		{@const x = index * STEP}
		<g class="run {outcome}" data-testid="run-bar">
			<title
				>{outcome === 'err'
					? t('organizations.task_ui.failed')
					: outcome === 'live'
						? t('organizations.tasks.running')
						: t('organizations.task_ui.succeeded')}
				· {formatDuration(runDurationMs(execution))}</title>
			{#each { length: cells(execution) }, k (k)}
				<rect {x} y={H - S - k * P} width={S} height={S} rx="0.6" />
			{/each}
		</g>
	{/each}
</svg>

<style>
	.run rect {
		fill: var(--success);
		opacity: 0.85;
	}
	.run.err rect {
		fill: var(--destructive);
		opacity: 1;
	}
	.run.live rect {
		fill: var(--warning);
	}
	.run:hover rect {
		opacity: 1;
	}
</style>
