<script lang="ts">
	import type { Snippet } from 'svelte';
	import * as Popover from '#lib/components/ui/popover/index.js';
	import * as Command from '#lib/components/ui/command/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';
	import PencilIcon from '@lucide/svelte/icons/pencil';
	import CheckIcon from '@lucide/svelte/icons/check';
	import XIcon from '@lucide/svelte/icons/x';
	import Fuse from 'fuse.js';
	import type { ConversationSummaryDto } from '#lib/clients/assistant-client.js';
	import { listConversations, renameConversation } from '#lib/clients/assistant-client.js';
	import { highlightMatches, type HighlightSegment } from '#lib/utils/highlight-matches.js';
	import { t } from '#lib/i18n/index.svelte.js';

	type Props = {
		open: boolean;
		organizationId: string | null;
		onOpenChange: (open: boolean) => void;
		onSelect: (conversationId: string) => void;
		icon: Snippet;
	};

	let { open = $bindable(), organizationId, onOpenChange, onSelect, icon }: Props = $props();

	let query = $state('');
	let items = $state<ConversationSummaryDto[]>([]);
	let loading = $state(false);
	let error = $state<string | null>(null);
	let editingId = $state<string | null>(null);
	let editingDraft = $state('');
	let savingTitle = $state(false);

	const isSearching = $derived(query.trim().length > 0);

	// The server already narrowed items to a working candidate set with a
	// multi-token substring filter; Fuse here does the fuzzy re-rank so a
	// typo or transposed letter still surfaces the right conversation.
	const fuse = $derived(
		new Fuse(items, {
			threshold: 0.4,
			ignoreLocation: true,
			keys: [
				{ name: 'title', weight: 2 },
				{ name: 'snippet', weight: 1 },
			],
		}),
	);
	const ranked = $derived(
		isSearching ? fuse.search(query.trim()).map((result) => result.item) : items,
	);

	const titleMatches = $derived(
		ranked.filter((item) => item.match_in === 'title' || item.match_in === 'both'),
	);
	const contentOnlyMatches = $derived(ranked.filter((item) => item.match_in === 'content'));
	// The list endpoint omits `match_in` when there is no query, so the
	// unfiltered rows are neither title nor content matches by the rules
	// above. They still need to render, so they sit in this bucket and the
	// template decides which heading they go under.
	const untagged = $derived(ranked.filter((item) => item.match_in === undefined));

	$effect(() => {
		if (!open) return;
		query = '';
		error = null;
		void refresh('');
	});

	$effect(() => {
		if (!open) return;
		const next = query.trim();
		void refresh(next);
	});

	async function refresh(q: string) {
		if (!organizationId) return;
		loading = true;
		error = null;
		try {
			items = await listConversations(organizationId, q || undefined);
		} catch (err) {
			error = err instanceof Error ? err.message : t('common.assistant.history_load_failed');
		} finally {
			loading = false;
		}
	}

	function relativeAge(updatedAt: number): string {
		const minutes = Math.max(0, Math.round((Date.now() - updatedAt) / 60_000));
		if (minutes < 1) return 'now';
		if (minutes < 60) return `${minutes}m`;
		const hours = Math.round(minutes / 60);
		if (hours < 24) return `${hours}h`;
		return `${Math.round(hours / 24)}d`;
	}

	function segmentsFor(text: string, query: string): HighlightSegment[] {
		if (!query) return [{ text, highlighted: false }];
		const lower = text.toLowerCase();
		const needle = query.toLowerCase();
		const indices: [number, number][] = [];
		let cursor = 0;
		while (cursor <= lower.length - needle.length) {
			const found = lower.indexOf(needle, cursor);
			if (found === -1) break;
			indices.push([found, found + needle.length - 1]);
			cursor = found + needle.length;
		}
		return highlightMatches(text, indices.length ? indices : undefined);
	}

	function pick(conversation: ConversationSummaryDto) {
		onOpenChange(false);
		onSelect(conversation.id);
	}

	function startEdit(conversation: ConversationSummaryDto) {
		editingId = conversation.id;
		editingDraft = conversation.title;
	}

	function cancelEdit() {
		editingId = null;
		editingDraft = '';
	}

	async function commitEdit() {
		if (!organizationId || editingId === null) return;
		const trimmed = editingDraft.trim();
		if (!trimmed) {
			cancelEdit();
			return;
		}
		savingTitle = true;
		try {
			const updated = await renameConversation(organizationId, editingId, trimmed);
			items = items.map((item) => (item.id === updated.id ? updated : item));
			cancelEdit();
		} catch (err) {
			error = err instanceof Error ? err.message : t('common.assistant.history_rename_failed');
		} finally {
			savingTitle = false;
		}
	}

	function onEditKeydown(event: KeyboardEvent) {
		if (event.key === 'Enter') {
			event.preventDefault();
			void commitEdit();
		} else if (event.key === 'Escape') {
			event.preventDefault();
			cancelEdit();
		}
	}
</script>

<Popover.Root bind:open {onOpenChange}>
	<Popover.Trigger>
		{#snippet child({ props })}
			<Button
				{...props}
				variant="ghost"
				size="icon-sm"
				title={t('common.assistant.history_search')}
				aria-label={t('common.assistant.history_search')}>
				{@render icon()}
				<span class="sr-only">{t('common.assistant.history_search')}</span>
			</Button>
		{/snippet}
	</Popover.Trigger>
	<Popover.Content class="w-[22rem] p-0" align="start" sideOffset={6}>
		<Command.Root shouldFilter={false}>
			<Command.Input
				bind:value={query}
				placeholder={t('common.assistant.history_search_placeholder')} />
			<Command.List class="max-h-[22rem]">
				{#if loading && ranked.length === 0}
					<div class="flex flex-col gap-2 p-2">
						<Skeleton class="h-12 rounded-[.875rem]" />
						<Skeleton class="h-12 rounded-[.875rem]" />
						<Skeleton class="h-12 rounded-[.875rem]" />
					</div>
				{:else}
					{#if untagged.length}
						<Command.Group
							heading={isSearching
								? t('common.assistant.history_other')
								: t('common.assistant.history_recent')}>
							{#each untagged as conversation (conversation.id)}
								{@render row(conversation, !isSearching)}
							{/each}
						</Command.Group>
					{/if}
					{#if titleMatches.length}
						<Command.Group heading={t('common.assistant.history_title_matches')}>
							{#each titleMatches as conversation (conversation.id)}
								{@render row(conversation, true)}
							{/each}
						</Command.Group>
					{/if}
					{#if contentOnlyMatches.length}
						<Command.Group heading={t('common.assistant.history_messages')}>
							{#each contentOnlyMatches as conversation (conversation.id)}
								{@render row(conversation, false)}
							{/each}
						</Command.Group>
					{/if}
					{#if ranked.length === 0 && !loading}
						<p class="text-muted-foreground px-3 py-4 text-xs leading-relaxed">
							{isSearching
								? t('common.assistant.history_no_matches')
								: t('common.assistant.history_empty')}
						</p>
					{/if}
				{/if}
			</Command.List>
		</Command.Root>
		{#if error}
			<div class="text-destructive flex items-start gap-1.5 border-t px-3 py-2 text-[.6875rem]">
				<TriangleAlertIcon class="mt-px size-3 shrink-0" />
				<span>{error}</span>
			</div>
		{/if}
	</Popover.Content>
</Popover.Root>

{#snippet row(conversation: ConversationSummaryDto, isTitleRow: boolean)}
	<div
		class="hover:bg-secondary flex flex-col gap-1.5 border-b px-3 py-2 text-start transition-colors duration-300 ease-[var(--lp-ease)] last:border-b-0">
		{#if editingId === conversation.id}
			<div class="flex items-center gap-1.5">
				<input
					type="text"
					class="bg-input/40 min-w-0 flex-1 rounded-md border px-2 py-1 text-[.8125rem] font-medium outline-none focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/30"
					bind:value={editingDraft}
					onkeydown={onEditKeydown}
					onblur={() => void commitEdit()}
					disabled={savingTitle}
					maxlength={80} />
				<Button
					size="icon-sm"
					variant="ghost"
					title={t('common.assistant.history_save_title')}
					onmousedown={(event) => event.preventDefault()}
					onclick={() => void commitEdit()}
					disabled={savingTitle}>
					<CheckIcon class="size-3.5" />
					<span class="sr-only">{t('common.assistant.history_save_title')}</span>
				</Button>
				<Button
					size="icon-sm"
					variant="ghost"
					title={t('common.assistant.history_cancel')}
					onmousedown={(event) => event.preventDefault()}
					onclick={cancelEdit}
					disabled={savingTitle}>
					<XIcon class="size-3.5" />
					<span class="sr-only">{t('common.assistant.history_cancel')}</span>
				</Button>
			</div>
		{:else}
			<div class="flex items-center gap-1.5">
				<button type="button" class="min-w-0 flex-1 text-start" onclick={() => pick(conversation)}>
					<b class="block truncate text-[.8125rem] font-medium">
						{#each segmentsFor(conversation.title, isTitleRow ? query.trim() : '') as segment}
							{#if segment.highlighted}<mark class="bg-primary/20 text-foreground rounded-sm"
									>{segment.text}</mark
								>{:else}{segment.text}{/if}
						{/each}
					</b>
				</button>
				<Button
					size="icon-sm"
					variant="ghost"
					title={t('common.assistant.history_rename')}
					onclick={() => startEdit(conversation)}>
					<PencilIcon class="size-3.5" />
					<span class="sr-only">{t('common.assistant.history_rename')}</span>
				</Button>
			</div>
		{/if}
		<button
			type="button"
			class="text-start disabled:opacity-50"
			disabled={editingId === conversation.id}
			onclick={() => pick(conversation)}>
			<span class="text-muted-foreground flex items-center gap-1.5 font-mono text-[.625rem]">
				<span>{relativeAge(conversation.updated_at)}</span>
				{#if conversation.pending_actions > 0}
					<span aria-hidden="true">·</span>
					<Badge variant="secondary" class="text-warning text-[.5625rem]">
						{t('common.assistant.history_waiting', { count: conversation.pending_actions })}
					</Badge>
				{/if}
			</span>
			{#if conversation.snippet}
				<p class="mt-1 text-xs leading-snug text-foreground/80 italic">
					…{#each segmentsFor(conversation.snippet, query.trim()) as segment}
						{#if segment.highlighted}<mark class="bg-primary/20 text-foreground rounded-sm"
								>{segment.text}</mark
							>{:else}{segment.text}{/if}
					{/each}…
				</p>
			{/if}
		</button>
	</div>
{/snippet}
