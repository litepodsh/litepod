<script lang="ts">
	/**
	 * The assistant. One component, two homes: docked it fills the sidebar's
	 * column, detached it fills its own rail.
	 *
	 * It renders `thread.items` and nothing else. Sending a message returns the
	 * whole turn from the API, including the changes it wants to make; those
	 * arrive as cards, and nothing is written until one is confirmed.
	 */
	import PlusIcon from '@lucide/svelte/icons/plus';
	import PanelRightOpenIcon from '@lucide/svelte/icons/panel-right-open';
	import PanelLeftCloseIcon from '@lucide/svelte/icons/panel-left-close';
	import XIcon from '@lucide/svelte/icons/x';
	import ArrowUpIcon from '@lucide/svelte/icons/arrow-up';
	import CheckIcon from '@lucide/svelte/icons/check';
	import SparklesIcon from '@lucide/svelte/icons/sparkles';
	import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Textarea } from '#lib/components/ui/textarea/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import { setAssistantMode } from '#lib/state/assistant.svelte.js';
	import type { ActionDto } from '#lib/clients/assistant-client.js';
	import SquareIcon from '@lucide/svelte/icons/square';
	import HistoryIcon from '@lucide/svelte/icons/history';
	import { onDestroy, tick } from 'svelte';
	import AssistantHistoryPopover from './assistant-history-popover.svelte';
	import ModelPicker from './model-picker.svelte';
	import AssistantRun from './assistant-run.svelte';
	import {
		ask,
		cancelTurn,
		resetThread,
		restore,
		restoreLast,
		settle,
		thread,
		type ThreadItem,
	} from './thread.svelte';
	import { renderMarkdown } from './markdown';
	import { enhanceCodeBlocks } from '#lib/utils/code-copy.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let {
		/** Docked shares the sidebar's column; detached owns its own rail. */
		docked = true,
		organizationId,
		scope = t('common.assistant.this_organization'),
		onclose,
	}: {
		docked?: boolean;
		organizationId: string | null;
		scope?: string;
		onclose?: () => void;
	} = $props();

	let draft = $state('');
	let composer = $state<HTMLTextAreaElement | null>(null);
	let transcript = $state<HTMLDivElement | null>(null);
	let historyOpen = $state(false);

	async function reopen(conversationId: string) {
		if (!organizationId) return;
		try {
			await restore(organizationId, conversationId);
		} catch (error) {
			thread.error =
				error instanceof Error ? error.message : t('common.assistant.conversation_gone');
		}
	}

	const suggestions = $derived([
		t('common.assistant.suggestion_apps'),
		t('common.assistant.suggestion_dsn'),
		t('common.assistant.suggestion_env'),
	]);

	const title = $derived(
		thread.items.find((item) => item.kind === 'user')?.text ?? t('common.assistant.title'),
	);

	type Step = Extract<ThreadItem, { kind: 'step' }>;
	type Block = ThreadItem | { kind: 'run'; steps: Step[] };

	// Consecutive tool steps read as one run, not as a column of loose rows.
	const blocks = $derived(
		thread.items.reduce<Block[]>((out, item) => {
			if (item.kind !== 'step') return [...out, item];
			const last = out.at(-1);
			if (last?.kind === 'run') last.steps.push(item);
			else out.push({ kind: 'run', steps: [item] });
			return out;
		}, []),
	);

	// Follow the newest item while a turn is running.
	$effect(() => {
		void thread.items.length;
		if (transcript) transcript.scrollTop = transcript.scrollHeight;
	});

	$effect(() => {
		const pendingDraft = thread.pendingDraft;
		if (pendingDraft === null) return;
		draft = pendingDraft;
		thread.pendingDraft = null;
		void tick().then(() => composer?.focus());
	});

	function send(text: string) {
		if (!organizationId) return;
		draft = '';
		thread.pendingDraftNotice = false;
		void ask(organizationId, text);
	}

	function submit(event: KeyboardEvent) {
		if (event.key !== 'Enter' || event.shiftKey) return;
		event.preventDefault();
		send(draft);
	}

	function decide(action: ActionDto, decision: 'confirm' | 'reject') {
		if (!organizationId) return;
		void settle(organizationId, action, decision).then(() => {
			// After a confirmed apply, tell any open editor of this application
			// to refresh from disk. `rejects` and `pending` are intentionally
			// silent: nothing about the application changed.
			//
			// `settle` navigates to the action's route first, which on a fresh
			// mount registers the listener in the next microtask. Defer with
			// queueMicrotask so the dispatch runs after the listener is alive.
			if (decision !== 'confirm') return;
			if (typeof window === 'undefined') return;
			queueMicrotask(() => {
				// A project-scoped change has no application to refresh; the
				// project list is what moved.
				window.dispatchEvent(
					action.application_id
						? new CustomEvent('application-updated', {
								detail: { applicationId: action.application_id },
							})
						: new CustomEvent('projects-updated'),
				);
			});
		});
	}

	// A turn left running after the panel closes would keep writing into a
	// transcript nobody is looking at.
	onDestroy(cancelTurn);

	// Reopen whatever was on screen before the reload.
	$effect(() => {
		if (organizationId) void restoreLast(organizationId);
	});
</script>

{#snippet actionCard(action: ActionDto)}
	<div
		class={action.status === 'pending'
			? 'lp-lifted bg-card rounded-xl border p-3'
			: action.status === 'applied'
				? 'border-success/30 bg-success/8 rounded-xl border p-3'
				: 'bg-card rounded-xl border p-3'}>
		<div class="text-label-ink text-[.5625rem] font-medium tracking-[.16em] uppercase">
			{action.status === 'pending' ? t('common.assistant.proposed_change') : action.status}
		</div>
		<h4 class="mt-1 mb-1 text-[.8125rem] font-semibold tracking-tight">{action.title}</h4>
		<p class="text-muted-foreground m-0 text-xs leading-relaxed">{action.summary}</p>
		{#if action.diff}
			<pre
				class="bg-card mt-2 max-h-72 overflow-auto rounded-[.6rem] border p-2 font-mono text-[.6875rem] leading-tight">{#each action.diff.split('\n') as line, index (index)}<span
						class="block px-1 {line.startsWith('+')
							? 'bg-success/12 text-success'
							: line.startsWith('-')
								? 'bg-destructive/10 text-destructive'
								: line.startsWith(' ')
									? 'text-muted-foreground'
									: ''}">{line || ' '}</span
					>{/each}</pre>
		{:else if action.added || action.removed}
			<div class="mt-2 overflow-hidden rounded-[.6rem] border font-mono text-[.6875rem]">
				{#if action.removed}
					<div class="bg-destructive/10 text-destructive overflow-x-auto px-2 whitespace-nowrap">
						- {action.removed}
					</div>
				{/if}
				{#if action.added}
					<div class="bg-success/12 text-success overflow-x-auto px-2 whitespace-nowrap">
						+ {action.added}
					</div>
				{/if}
			</div>
		{/if}
		{#if action.status === 'pending'}
			<div class="mt-2.5 flex flex-wrap items-center gap-1.5">
				<Button size="xs" onclick={() => decide(action, 'confirm')}
					>{t('common.assistant.apply_change')}</Button>
				<Button size="xs" variant="ghost" onclick={() => decide(action, 'reject')}
					>{t('common.assistant.dismiss')}</Button>
			</div>
		{:else if action.status === 'applied'}
			<div class="text-success mt-2 flex items-center gap-1.5 text-[.6875rem] font-medium">
				<CheckIcon class="size-3" />
				{t('common.assistant.confirmed_by_you')}
			</div>
		{:else if action.failure}
			<p class="text-destructive mt-2 text-[.6875rem]">{action.failure}</p>
		{/if}
	</div>
{/snippet}

{#snippet threadItem(item: ThreadItem)}
	{#if item.kind === 'user'}
		<div class="bg-secondary max-w-[88%] self-end rounded-[.875rem] px-3 py-2 text-[.8125rem]">
			{item.text}
		</div>
	{:else if item.kind === 'assistant'}
		<!-- eslint-disable-next-line svelte/no-at-html-tags -- sanitised in renderMarkdown -->
		<div
			class="assistant-prose border-primary border-s-2 ps-3 text-[.8125rem] leading-relaxed"
			use:enhanceCodeBlocks>
			{@html renderMarkdown(item.text)}
		</div>
	{:else if item.kind === 'choices'}
		<!-- A miss the user can fix by picking, instead of retyping the sentence. -->
		<div class="flex flex-wrap gap-1.5">
			{#each item.options as option (option)}
				<button
					type="button"
					class="bg-card hover:bg-secondary rounded-full border px-2.5 py-1 font-mono text-[.6875rem] transition-[background-color,transform] duration-300 ease-[var(--lp-ease)] active:scale-[.98] disabled:opacity-50"
					disabled={thread.running}
					onclick={() => send(option)}>
					{option}
				</button>
			{/each}
		</div>
	{:else if item.kind === 'stopped'}
		<!-- The turn hit its step ceiling. Said plainly, so the answer above is
		     not read as a conclusion. -->
		<div
			class="border-warning/40 text-muted-foreground flex items-start gap-1.5 border-s-2 ps-3 text-[.75rem] leading-relaxed">
			<TriangleAlertIcon class="text-warning mt-0.5 size-3 shrink-0" />
			<span>{item.text}</span>
		</div>
	{:else if item.kind === 'action'}
		{@render actionCard(item.action)}
	{/if}
{/snippet}

<div class="flex min-h-0 flex-1 flex-col">
	<div class="flex items-center gap-1 border-b p-2">
		<div class="min-w-0 flex-1 ps-1.5">
			<b class="block truncate text-[.8125rem] font-medium tracking-tight">{title}</b>
			<small class="text-muted-foreground block truncate font-mono text-[.6875rem]">{scope}</small>
		</div>
		<Button
			variant="ghost"
			size="icon-sm"
			title={t('common.assistant.new_chat')}
			onclick={resetThread}>
			<PlusIcon />
			<span class="sr-only">{t('common.assistant.new_chat')}</span>
		</Button>
		<AssistantHistoryPopover
			bind:open={historyOpen}
			{organizationId}
			onOpenChange={(value) => (historyOpen = value)}
			onSelect={reopen}>
			{#snippet icon()}
				<HistoryIcon />
			{/snippet}
		</AssistantHistoryPopover>
		{#if docked}
			<Button
				variant="ghost"
				size="icon-sm"
				title={t('common.assistant.detach')}
				onclick={() => setAssistantMode('detached')}>
				<PanelRightOpenIcon />
				<span class="sr-only">{t('common.assistant.detach')}</span>
			</Button>
		{:else}
			<Button
				variant="ghost"
				size="icon-sm"
				title={t('common.assistant.dock')}
				onclick={() => setAssistantMode('chat')}>
				<PanelLeftCloseIcon />
				<span class="sr-only">{t('common.assistant.dock')}</span>
			</Button>
		{/if}
		<Button
			variant="ghost"
			size="icon-sm"
			title={t('common.assistant.close')}
			onclick={() => (onclose ? onclose() : setAssistantMode('nav'))}>
			<XIcon />
			<span class="sr-only">{t('common.assistant.close')}</span>
		</Button>
	</div>

	{#if thread.items.length === 0}
		<div class="flex min-h-0 flex-1 flex-col justify-center gap-2.5 overflow-y-auto px-3.5 py-5">
			<span
				class="bg-secondary text-primary grid size-9 place-items-center rounded-[.875rem]"
				aria-hidden="true">
				<SparklesIcon class="size-4" />
			</span>
			<h4 class="mt-1 mb-0 text-sm font-semibold tracking-tight">
				{t('common.assistant.ask_about_projects')}
			</h4>
			<p class="text-muted-foreground m-0 text-xs leading-relaxed">
				{t('common.assistant.empty_description')}
			</p>
			<div class="mt-1 flex flex-col gap-1.5">
				{#each suggestions as suggestion (suggestion)}
					<button
						type="button"
						class="bg-card hover:bg-secondary rounded-[.875rem] border px-2.5 py-2 text-start text-xs leading-snug transition-[background-color,transform] duration-300 ease-[var(--lp-ease)] active:scale-[.98]"
						onclick={() => send(suggestion)}>
						{suggestion}
					</button>
				{/each}
			</div>
		</div>
	{:else}
		<div
			bind:this={transcript}
			class="flex min-h-0 flex-1 flex-col gap-3.5 overflow-y-auto px-3 pt-3.5 pb-4">
			{#each blocks as block, index (index)}
				{#if block.kind === 'run'}
					<AssistantRun
						steps={block.steps}
						live={thread.running && index === blocks.length - 1} />
				{:else}
					{@render threadItem(block)}
				{/if}
			{/each}
			{#if thread.running && !(blocks.at(-1)?.kind === 'run')}
				<div
					class="border-primary/40 flex flex-col gap-1.5 border-s-2 ps-3"
					role="status"
					aria-busy="true"
					aria-label={t('common.assistant.working')}>
					<Skeleton class="h-2 w-[82%] rounded-full" />
					<Skeleton class="h-2 w-[64%] rounded-full [animation-delay:180ms]" />
					<Skeleton class="h-2 w-[44%] rounded-full [animation-delay:360ms]" />
				</div>
			{/if}
		</div>
	{/if}

	{#if thread.error}
		<div class="text-destructive flex items-start gap-1.5 border-t px-3 py-2 text-[.6875rem]">
			<TriangleAlertIcon class="mt-px size-3 shrink-0" />
			<span>{thread.error}</span>
		</div>
	{/if}

	<div class="flex flex-col gap-1.5 border-t p-2.5">
		<label
			class="text-label-ink text-[.5625rem] font-medium tracking-[.16em] uppercase"
			for="assistant-composer">
			{t('common.assistant.ask_about_scope', { scope })}
		</label>
		<Textarea
			id="assistant-composer"
			bind:ref={composer}
			bind:value={draft}
			rows={1}
			placeholder={t('common.assistant.placeholder')}
			class="min-h-14 text-[.8125rem]"
			disabled={!organizationId}
			onkeydown={submit} />
		{#if thread.pendingDraftNotice}
			<p class="text-muted-foreground m-0 text-xs">{t('common.assistant.context_added')}</p>
		{/if}
		<div class="flex items-center gap-1">
			<ModelPicker {organizationId} />
			{#if thread.running}
				<Button
					size="icon-sm"
					variant="outline"
					class="ms-auto rounded-full"
					title={t('common.assistant.stop')}
					onclick={cancelTurn}>
					<SquareIcon />
					<span class="sr-only">{t('common.assistant.stop')}</span>
				</Button>
			{:else}
				<Button
					size="icon-sm"
					class="ms-auto rounded-full"
					title={t('common.assistant.send')}
					disabled={!draft.trim() || !organizationId}
					onclick={() => send(draft)}>
					<ArrowUpIcon />
					<span class="sr-only">{t('common.assistant.send')}</span>
				</Button>
			{/if}
		</div>
		<p class="text-muted-foreground m-0 pe-1 text-end text-[.625rem]">
			{t('common.assistant.changes_ask_before_run')}
		</p>
	</div>
</div>

<style>
	/* The panel is 16rem wide at its narrowest, so a reply's own structure has
	   to stay compact: tight lists, no runaway code blocks, and headings that
	   read as emphasis rather than as new sections. */
	.assistant-prose :global(p) {
		margin: 0 0 0.5rem;
	}
	.assistant-prose :global(p:last-child) {
		margin-bottom: 0;
	}
	.assistant-prose :global(ul),
	.assistant-prose :global(ol) {
		margin: 0 0 0.5rem;
		padding-inline-start: 1.1rem;
	}
	.assistant-prose :global(li) {
		margin-bottom: 0.125rem;
	}
	.assistant-prose :global(code) {
		background: var(--secondary);
		border-radius: 0.35rem;
		padding: 0.05rem 0.3rem;
		font-family: var(--font-mono);
		font-size: 0.75rem;
	}
	.assistant-prose :global(pre) {
		background: var(--secondary);
		border-radius: 0.6rem;
		padding: 0.5rem 0.625rem;
		margin: 0 0 0.5rem;
		overflow-x: auto;
		font-family: var(--font-mono);
		font-size: 0.75rem;
		line-height: 1.45;
	}
	.assistant-prose :global(pre code) {
		background: none;
		padding: 0;
		font-family: inherit;
		font-size: inherit;
	}
	.assistant-prose :global(h1),
	.assistant-prose :global(h2),
	.assistant-prose :global(h3) {
		margin: 0 0 0.25rem;
		font-size: 0.8125rem;
		font-weight: 600;
		letter-spacing: -0.01em;
	}
	.assistant-prose :global(a) {
		color: var(--primary);
		text-decoration: underline;
		text-underline-offset: 2px;
	}
	.assistant-prose :global(table) {
		display: block;
		overflow-x: auto;
		font-size: 0.75rem;
	}
</style>
