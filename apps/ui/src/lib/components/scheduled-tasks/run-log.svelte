<script lang="ts">
	import CheckIcon from '@lucide/svelte/icons/check';
	import CopyIcon from '@lucide/svelte/icons/copy';
	import SquareIcon from '@lucide/svelte/icons/square';
	import {
		cancelScheduledTaskExecution,
		getScheduledTaskExecution,
		streamScheduledTaskExecution,
		type ScheduledTaskExecutionDetail,
		type ScheduledTaskExecutionLog,
		type ScheduledTaskLiveEvent,
	} from '#lib/clients/scheduled-tasks-client.js';
	import { t } from '#lib/i18n/index.svelte.js';
	import { renderTerminalText } from '#lib/utils/terminal-text.js';
	import { toast } from '#lib/utils/toast.js';
	import { cn } from '#lib/utils.js';

	// The log of one execution: the saved rows, then live lines over the push
	// stream while it runs (polling if the stream fails), plus copy and kill.
	let {
		organizationId,
		taskId,
		executionId,
		onFinished,
		intervalMs = 2000,
	}: {
		organizationId: string;
		taskId: string;
		executionId: string;
		/** The run ended or was killed; the owner refreshes its list. */
		onFinished?: () => void;
		intervalMs?: number;
	} = $props();

	let detail = $state<ScheduledTaskExecutionDetail | null>(null);
	let liveLines = $state<Record<number, ScheduledTaskExecutionLog>>({});
	let streamFailed = $state(false);
	let killing = $state(false);
	let copied = $state(false);
	let logBox = $state<HTMLOListElement | null>(null);
	let stickToBottom = true;

	const logs = $derived.by(() => {
		const merged = new Map((detail?.logs ?? []).map((log) => [log.sequence, log]));
		for (const log of Object.values(liveLines)) merged.set(log.sequence, log);
		return [...merged.values()].sort((left, right) => left.sequence - right.sequence);
	});
	const active = $derived(detail?.status === 'running' || detail?.status === 'queued');

	async function refresh(notify = false) {
		try {
			detail = await getScheduledTaskExecution(organizationId, taskId, executionId);
		} catch (error) {
			if (notify)
				toast.error(
					error instanceof Error ? error.message : t('organizations.tasks.execution_load_failed'),
				);
		}
	}

	// a different execution starts with a clean slate
	$effect(() => {
		void executionId;
		detail = null;
		liveLines = {};
		streamFailed = false;
		void refresh(true);
	});

	$effect(() => {
		if (!active || streamFailed) return;
		let source: EventSource;
		try {
			source = streamScheduledTaskExecution(organizationId, taskId, executionId);
		} catch {
			streamFailed = true;
			return;
		}
		source.onmessage = (message) => {
			let event: ScheduledTaskLiveEvent;
			try {
				event = JSON.parse(message.data);
			} catch {
				return;
			}
			if (event.type === 'line') {
				liveLines[event.sequence] = {
					sequence: event.sequence,
					stream: event.stream,
					message: event.message,
					created_at: Date.now(),
				};
			} else {
				// finished: stop before the browser reconnects, then read the final detail
				source.close();
				void refresh().then(() => {
					liveLines = {};
					onFinished?.();
				});
			}
		};
		source.onerror = () => {
			source.close();
			streamFailed = true;
		};
		return () => source.close();
	});

	// polling: the fallback when the stream failed
	$effect(() => {
		if (!active || !streamFailed) return;
		const timer = setInterval(() => void refresh(), intervalMs);
		return () => clearInterval(timer);
	});

	// follow the tail as new lines arrive, unless the reader scrolled up
	$effect(() => {
		const count = logs.length + (logs.at(-1)?.message.length ?? 0);
		if (!logBox || !stickToBottom || count === 0) return;
		queueMicrotask(() => logBox?.scrollTo({ top: logBox.scrollHeight }));
	});

	async function kill() {
		if (killing) return;
		killing = true;
		try {
			await cancelScheduledTaskExecution(organizationId, taskId, executionId);
			toast.success(t('organizations.tasks.execution_killed'));
			await refresh();
			onFinished?.();
		} catch (error) {
			toast.error(
				error instanceof Error ? error.message : t('organizations.tasks.execution_kill_failed'),
			);
		} finally {
			killing = false;
		}
	}

	async function copyLogs() {
		const text = logs
			.map((log) => (log.stream === 'stdout' ? log.message : `[${log.stream}] ${log.message}`))
			.join('\n');
		try {
			await navigator.clipboard.writeText(text);
			copied = true;
			setTimeout(() => (copied = false), 1500);
		} catch {
			copied = false;
		}
	}
</script>

<div
	class="overflow-hidden rounded-[10px] border bg-[#0f1217] text-[#c6ccd7] shadow-[inset_1px_1px_0_rgb(255_255_255/.04),inset_-1px_-1px_0_rgb(0_0_0/.4)] dark:bg-[#07080a]">
	<div
		class="flex items-center justify-between gap-2 border-b border-black/50 py-1.5 ps-3 pe-1.5 font-mono text-xs text-[#8a93a3]">
		<span class="min-w-0 truncate">
			{executionId.slice(0, 8)}.log
			{#if detail}
				<span class="text-[#6c7482]"
					>· {t('organizations.tasks.log_lines', { count: logs.length })}{detail.logs_truncated
						? `, ${t('organizations.tasks.output_truncated')}`
						: ''}</span>
			{/if}
		</span>
		<span class="flex items-center gap-1">
			{#if active}
				<button
					type="button"
					class="inline-flex h-6 items-center gap-1 rounded-md px-2 text-[#ff8f88] hover:bg-white/10 disabled:opacity-50"
					disabled={killing}
					onclick={() => void kill()}
					><SquareIcon class="size-3" />{killing
						? t('organizations.tasks.killing')
						: t('organizations.tasks.kill_run')}</button>
			{/if}
			<button
				type="button"
				class="grid size-6 place-items-center rounded-md hover:bg-white/10 hover:text-white disabled:opacity-40"
				disabled={logs.length === 0}
				aria-label={t('organizations.task_ui.copy_log')}
				title={copied ? t('organizations.tasks.copied') : t('organizations.task_ui.copy_log')}
				onclick={() => void copyLogs()}>
				{#if copied}<CheckIcon class="size-3.5" />{:else}<CopyIcon class="size-3.5" />{/if}
			</button>
		</span>
	</div>
	{#if detail?.error}<p class="bg-[#ff6d64]/10 px-3 py-2 font-mono text-xs text-[#ff8f88]">
			{detail.error}
		</p>{/if}
	<ol
		bind:this={logBox}
		onscroll={() => {
			if (logBox) stickToBottom = logBox.scrollHeight - logBox.scrollTop - logBox.clientHeight < 40;
		}}
		class="run-log max-h-80 overflow-auto py-2 font-mono text-xs leading-[1.7]">
		{#each logs as log (log.sequence)}
			<li
				class={cn(
					log.stream === 'stderr' && 'bg-[#ff6d64]/10 text-[#ff8f88]',
					log.stream === 'system' && 'text-[#8a93a3]',
				)}>
				{renderTerminalText(log.message)}
			</li>
		{:else}
			<li class="text-[#8a93a3]">
				{!detail || active
					? t('organizations.tasks.waiting_for_output')
					: t('organizations.tasks.no_execution_output')}
			</li>
		{/each}
		{#if active}<li>
				<span
					class="inline-block h-3 w-[7px] translate-y-0.5 animate-pulse bg-[#c6ccd7] motion-reduce:animate-none"
				></span>
			</li>{/if}
	</ol>
</div>

<style>
	.run-log {
		counter-reset: ln;
	}
	.run-log li {
		counter-increment: ln;
		display: grid;
		grid-template-columns: 34px minmax(0, 1fr);
		padding-right: 12px;
		white-space: pre-wrap;
		word-break: break-word;
	}
	.run-log li::before {
		content: counter(ln);
		color: #8a93a3;
		text-align: right;
		padding-right: 12px;
		opacity: 0.6;
	}
</style>
