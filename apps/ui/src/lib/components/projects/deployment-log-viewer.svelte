<script module lang="ts">
	/** Button look for the log header toolbar, so callers' actions match it. */
	export const LOG_BUTTON =
		'h-7 gap-1.5 border bg-background px-2 text-xs shadow-none lp-raised hover:bg-muted';
</script>

<script lang="ts">
	import type { Snippet } from 'svelte';
	import type { DeploymentRunLogDto } from '#lib/clients/project-client.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import PreviewBadge from '#lib/components/custom/preview-badge.svelte';
	import SparklesIcon from '@lucide/svelte/icons/sparkles';
	import CopyIcon from '@lucide/svelte/icons/copy';
	import ClockIcon from '@lucide/svelte/icons/clock';
	import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';
	import PauseIcon from '@lucide/svelte/icons/pause';
	import PlayIcon from '@lucide/svelte/icons/play';
	import { deploymentLogTone, type DeploymentLogTone } from '#lib/utils/deployment-log-tone.js';
	import type { DeploymentStep } from '#lib/utils/deployment-steps.js';
	import { formatLogTimestamp } from '#lib/utils/log-timestamp.js';
	import { formatDuration } from '#lib/utils/format-duration.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';
	import { loadNerdFont } from '#lib/utils/lazy-nerd-font.js';

	const TS_STORAGE_KEY = 'litepod-log-timestamp-enabled';
	const MAX_VISIBLE_LINES = 1000;

	let {
		logs,
		label,
		title,
		subtitle,
		steps,
		stepsPreview = false,
		actions,
		empty,
		showLineNumbers = true,
		onRefresh,
		livePaused,
		onToggleLive,
		onAskAssistant,
	}: {
		logs: DeploymentRunLogDto[];
		label: string;
		/** Header line; falls back to `label`. */
		title?: string;
		subtitle?: string;
		steps?: DeploymentStep[];
		/** True while `steps` is the example fallback rather than recorded steps. */
		stepsPreview?: boolean;
		/** Extra buttons for the header; style them with `LOG_BUTTON`. */
		actions?: Snippet;
		/** Shown in the log body while there are no lines yet. */
		empty?: Snippet;
		showLineNumbers?: boolean;
		onRefresh?: () => void | Promise<void>;
		livePaused?: boolean;
		onToggleLive?: () => void | Promise<void>;
		/** Given the rendered log text. Omitted when the assistant is off. */
		onAskAssistant?: (text: string) => void;
	} = $props();

	let showTimestamps = $state(readTimestampPreference());
	let box = $state<HTMLDivElement | null>(null);
	let stick = true;

	function readTimestampPreference() {
		try {
			return localStorage.getItem(TS_STORAGE_KEY) !== 'false';
		} catch {
			return true;
		}
	}

	function toggleTimestamps() {
		showTimestamps = !showTimestamps;
		try {
			localStorage.setItem(TS_STORAGE_KEY, String(showTimestamps));
		} catch {}
	}

	type Line = { ts: string; text: string; tone: DeploymentLogTone };

	const lines = $derived<Line[]>(
		logs
			.flatMap((log) =>
				log.message.split(/\r?\n/).map((text, i) => ({
					ts: i === 0 ? formatLogTimestamp(log.created_at) : '',
					text,
					tone: deploymentLogTone(log.level, text),
				})),
			)
			.slice(-MAX_VISIBLE_LINES),
	);

	const TONE: Record<DeploymentLogTone, string> = {
		error: 'text-destructive',
		warning: 'text-warning',
		success: 'text-success',
		stage: 'text-info',
		neutral: '',
	};

	function stepLabel(name: string) {
		const key = `projects.deployment_log_viewer.step_${name}`;
		const value = t(key);
		return value === key ? name.replace(/_/g, ' ') : value;
	}

	// Follow the tail only while the reader is already at the bottom.
	$effect.pre(() => {
		void lines;
		if (box) stick = box.scrollTop + box.clientHeight >= box.scrollHeight - 24;
	});
	$effect(() => {
		void lines;
		if (box && stick) box.scrollTop = box.scrollHeight;
	});

	$effect(() => void loadNerdFont());

	const text = () =>
		lines
			.map((line) => (showTimestamps && line.ts ? `${line.ts} ${line.text}` : line.text))
			.join('\n');

	function copyLogs() {
		const value = text();
		if (!value) return;
		navigator.clipboard
			.writeText(value)
			.then(() => toast.success(t('projects.dialogs_common.logs_copied')))
			.catch(() => toast.error(t('projects.dialogs_common.copy_logs_failed')));
	}
</script>

<div class="min-w-0 overflow-hidden rounded-xl border bg-card text-card-foreground lp-raised">
	<div class="flex flex-wrap items-center justify-between gap-2.5 px-4 py-3 lp-groove-b">
		<div class="min-w-0">
			<p class="truncate text-[13px] font-medium">{title ?? label}</p>
			{#if subtitle}<small class="block truncate font-mono text-xs text-muted-foreground"
					>{subtitle}</small
				>{/if}
		</div>
		<div class="flex flex-wrap items-center gap-1.5">
			{@render actions?.()}
			<Button
				variant="ghost"
				size="sm"
				class="{LOG_BUTTON} {showTimestamps ? '' : 'opacity-60'}"
				aria-pressed={showTimestamps}
				onclick={toggleTimestamps}
				><ClockIcon class="size-3.5" />{t('projects.dialogs_common.timestamps')}</Button>
			{#if onRefresh}
				<Button variant="ghost" size="sm" class={LOG_BUTTON} onclick={onRefresh}
					><RefreshCwIcon class="size-3.5" />{t('projects.application_general.refresh')}</Button>
			{/if}
			{#if onToggleLive}
				<Button variant="ghost" size="sm" class={LOG_BUTTON} onclick={onToggleLive}>
					{#if livePaused}<PlayIcon class="size-3.5" />{t(
							'projects.deployment_log_viewer.resume_live',
						)}{:else}<PauseIcon class="size-3.5" />{t(
							'projects.deployment_log_viewer.stop_live',
						)}{/if}
				</Button>
			{/if}
			{#if onAskAssistant}
				<Button
					variant="ghost"
					size="sm"
					class={LOG_BUTTON}
					onclick={() => {
						const value = text();
						if (value) onAskAssistant?.(value);
					}}><SparklesIcon class="size-3.5" />{t('projects.dialogs_common.ask_ai')}</Button>
			{/if}
			<Button
				variant="ghost"
				size="sm"
				class={LOG_BUTTON}
				aria-label={t('projects.application_compose.copy')}
				onclick={copyLogs}><CopyIcon class="size-3.5" /></Button>
		</div>
	</div>

	{#if steps?.length}
		<div class="flex items-start gap-3 px-4 py-3 lp-groove-b">
			<ol
				class="steps grid min-w-0 flex-1 gap-1"
				style="--n: {steps.length}"
				aria-label={t('projects.deployment_log_viewer.steps')}>
				{#each steps as step (step.name)}
					<li
						class="flex min-w-0 flex-col gap-1.5 text-[11.5px] {step.state === 'done'
							? 'text-foreground'
							: step.state === 'run'
								? 'text-warning'
								: step.state === 'failed'
									? 'text-destructive'
									: 'text-muted-foreground'}"
						aria-current={step.state === 'run' ? 'step' : undefined}>
						<i
							class="relative block h-[3px] overflow-hidden rounded-sm {step.state === 'done'
								? 'bg-success'
								: step.state === 'failed'
									? 'bg-destructive'
									: 'bg-muted-foreground/20'} {step.state === 'run'
								? 'step-run'
								: ''} {step.state === 'skipped' ? 'opacity-50' : ''}"></i>
						<span class="flex min-w-0 items-baseline gap-1.5">
							<span class="truncate {step.state === 'skipped' ? 'line-through' : ''}"
								>{stepLabel(step.name)}</span>
							{#if step.durationMs}<span
									class="shrink-0 font-mono text-[10.5px] text-muted-foreground"
									>{formatDuration(step.durationMs)}</span
								>{/if}
							<span class="sr-only">{t(`projects.deployment_log_viewer.state_${step.state}`)}</span>
						</span>
					</li>
				{/each}
			</ol>
			{#if stepsPreview}<span class="shrink-0"><PreviewBadge /></span>{/if}
		</div>
	{/if}

	<div
		bind:this={box}
		role="log"
		aria-label={label}
		class="h-96 overflow-auto bg-muted/50 py-3 text-xs leading-[1.75] lp-well max-sm:h-72"
		style="font-family: 'Commit Mono', 'CommitMono Nerd Font Mono', monospace">
		{#if lines.length === 0 && empty}<div class="px-4 text-muted-foreground">
				{@render empty()}
			</div>{/if}
		{#each lines as line, index}
			<div
				class="grid gap-3 px-4 break-words whitespace-pre-wrap {showTimestamps
					? 'grid-cols-[6.5rem_minmax(0,1fr)]'
					: ''} {showLineNumbers ? 'pl-12' : ''} relative">
				{#if showLineNumbers}<span
						class="absolute left-0 w-9 text-right text-muted-foreground opacity-60 select-none"
						>{index + 1}</span
					>{/if}
				{#if showTimestamps}<span class="text-muted-foreground select-none">{line.ts}</span>{/if}
				<span class={TONE[line.tone]}>{line.text}</span>
			</div>
		{/each}
	</div>
</div>

<style>
	.steps {
		grid-template-columns: repeat(var(--n), minmax(0, 1fr));
	}
	@media (max-width: 640px) {
		.steps {
			grid-template-columns: repeat(3, minmax(0, 1fr));
			row-gap: 10px;
		}
	}
	.step-run::after {
		content: '';
		position: absolute;
		inset: 0;
		background: var(--warning);
		transform-origin: left;
		animation: fill 1.6s var(--lp-ease) infinite;
	}
	@keyframes fill {
		from {
			transform: scaleX(0);
		}
		to {
			transform: scaleX(1);
			opacity: 0.4;
		}
	}
	@media (prefers-reduced-motion: reduce) {
		.step-run::after {
			animation: none;
			transform: scaleX(0.5);
		}
	}
</style>
