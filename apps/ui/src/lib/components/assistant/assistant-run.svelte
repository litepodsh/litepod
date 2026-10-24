<script lang="ts">
	/**
	 * The tools one turn ran, as a single card: the step in flight on top with
	 * a running clock, a segment per step beneath it, and the finished steps
	 * stacking below. When the turn ends the card settles into a summary.
	 */
	import CheckIcon from '@lucide/svelte/icons/check';
	import XIcon from '@lucide/svelte/icons/x';
	import LoaderCircleIcon from '@lucide/svelte/icons/loader-circle';
	import { t } from '#lib/i18n/index.svelte.js';
	import type { ThreadItem } from './thread.svelte';

	type Step = Extract<ThreadItem, { kind: 'step' }>;

	let { steps, live }: { steps: Step[]; live: boolean } = $props();

	let now = $state(Date.now());
	$effect(() => {
		if (!live) return;
		const timer = setInterval(() => (now = Date.now()), 250);
		return () => clearInterval(timer);
	});

	const current = $derived(steps.findLast((step) => step.running));
	const startedAt = $derived(steps[0]?.startedAt);
	const elapsed = $derived(
		startedAt
			? Math.max(
					0,
					((live ? now : (steps.at(-1)!.startedAt! + (steps.at(-1)!.durationMs ?? 0))) -
						startedAt) /
						1000,
				)
			: null,
	);
	const failed = $derived(steps.some((step) => !step.running && !step.ok));

	function seconds(ms: number | undefined) {
		return ms === undefined ? '' : `${(ms / 1000).toFixed(1)}s`;
	}
</script>

<div
	class="run-card bg-card flex min-w-0 flex-col gap-2 rounded-[.75rem] border px-2.5 py-2"
	role="status"
	aria-busy={live}>
	<div class="flex min-w-0 items-center gap-1.5 text-[.75rem]">
		{#if live}
			<LoaderCircleIcon class="text-primary size-3.5 shrink-0 motion-safe:animate-spin" />
			<span class="shimmer-text min-w-0 flex-1 truncate font-medium">
				{current?.name ?? t('common.assistant.working')}
			</span>
		{:else if failed}
			<XIcon class="text-destructive size-3.5 shrink-0" />
			<span class="min-w-0 flex-1 truncate font-medium">
				{t('common.assistant.run_failed', { count: steps.length })}
			</span>
		{:else}
			<span class="run-done text-success shrink-0"><CheckIcon class="size-3.5" /></span>
			<span class="min-w-0 flex-1 truncate font-medium">
				{t('common.assistant.run_done', { count: steps.length })}
			</span>
		{/if}
		{#if elapsed !== null}
			<span class="text-muted-foreground shrink-0 font-mono text-[.6875rem] tabular-nums">
				{elapsed.toFixed(live ? 0 : 1)}s
			</span>
		{/if}
	</div>

	<div class="flex gap-1" aria-hidden="true">
		{#each steps as step (step.id)}
			<span class="bg-border relative h-0.5 flex-1 overflow-hidden rounded-full">
				<span
					class="segment absolute inset-y-0 start-0 rounded-full {step.running
						? 'bg-primary/60 w-full motion-safe:animate-pulse'
						: step.ok
							? 'bg-primary w-full'
							: 'bg-destructive w-full'}"></span>
			</span>
		{/each}
	</div>

	<ul class="m-0 flex list-none flex-col gap-1 p-0">
		{#each steps as step (step.id)}
			<li
				class="step-row flex min-w-0 items-center gap-1.5 font-mono text-[.6875rem] {step.running
					? 'text-foreground'
					: 'text-muted-foreground'}">
				<span class="shimmer-text-wrap min-w-0 flex-1 truncate">
					<b class="font-medium {step.running ? 'shimmer-text' : ''}">{step.name}</b>
					<span class="opacity-70">{step.arguments}</span>
				</span>
				<span class="shrink-0 tabular-nums">{seconds(step.durationMs)}</span>
			</li>
		{/each}
	</ul>
</div>

<style>
	/* A step slides in from below as the model starts it. */
	.step-row {
		animation: step-in 360ms var(--lp-ease) both;
	}
	.segment {
		transform-origin: left;
		animation: segment-fill 500ms var(--lp-ease) both;
	}
	.run-done {
		animation: pop 420ms var(--lp-ease) both;
	}
	.shimmer-text {
		background: linear-gradient(
			90deg,
			var(--muted-foreground) 0%,
			var(--foreground) 50%,
			var(--muted-foreground) 100%
		);
		background-size: 200% 100%;
		-webkit-background-clip: text;
		background-clip: text;
		color: transparent;
		animation: text-shimmer 1.8s linear infinite;
	}
	@keyframes step-in {
		from {
			opacity: 0;
			transform: translateY(6px);
		}
	}
	@keyframes segment-fill {
		from {
			transform: scaleX(0);
		}
	}
	@keyframes pop {
		from {
			opacity: 0;
			transform: scale(0.4);
		}
	}
	@keyframes text-shimmer {
		from {
			background-position: 100% 0;
		}
		to {
			background-position: -100% 0;
		}
	}
	@media (prefers-reduced-motion: reduce) {
		.step-row,
		.segment,
		.run-done,
		.shimmer-text {
			animation: none;
		}
		.shimmer-text {
			color: inherit;
			background: none;
		}
	}
</style>
