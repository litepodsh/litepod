<script lang="ts">
	import { untrack } from 'svelte';
	import KeyRoundIcon from '@lucide/svelte/icons/key-round';
	import ServerIcon from '@lucide/svelte/icons/server';
	import SquareTerminalIcon from '@lucide/svelte/icons/square-terminal';
	import UserIcon from '@lucide/svelte/icons/user';

	import * as Card from '#lib/components/ui/card/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import { ApiError } from '#lib/clients/base-client.js';
	import type { TerminalSessionDto, TerminalSessionPage } from '#lib/clients/terminal-client.js';
	import { relativeTime } from '#lib/utils/relative-time.js';
	import { i18n, t } from '#lib/i18n/index.svelte.js';

	let {
		load,
		refreshOn,
		hostLabel,
		class: className = '',
	}: {
		load: (page: TerminalSessionPage) => Promise<TerminalSessionDto[]>;
		/** Any value; the first page reloads whenever it changes (e.g. terminal status). */
		refreshOn?: unknown;
		/** Shown for host shell rows (the server name). */
		hostLabel?: string;
		class?: string;
	} = $props();

	const PAGE = 20;
	let sessions = $state<TerminalSessionDto[]>([]);
	let loading = $state(true);
	let failed = $state(false);
	let forbidden = $state(false);
	let hasMore = $state(false);
	let loadingMore = $state(false);

	async function fetchPage(before?: string) {
		const rows = await load({ limit: PAGE, before });
		hasMore = rows.length === PAGE;
		return rows;
	}

	async function reload() {
		failed = false;
		try {
			sessions = await fetchPage();
		} catch (error) {
			if (error instanceof ApiError && error.status === 403) forbidden = true;
			else failed = true;
		} finally {
			loading = false;
		}
	}

	async function loadMore() {
		loadingMore = true;
		try {
			sessions = [...sessions, ...(await fetchPage(sessions.at(-1)?.id))];
		} catch {
			failed = true;
		} finally {
			loadingMore = false;
		}
	}

	$effect(() => {
		void refreshOn;
		untrack(reload);
	});

	// SQLite `datetime('now')` is UTC without a zone marker.
	const utcMs = (value: string) => Date.parse(`${value.replace(' ', 'T')}Z`);

	function duration(seconds: number) {
		if (seconds < 60) return `${seconds}s`;
		const minutes = Math.floor(seconds / 60);
		if (minutes < 60) return seconds % 60 ? `${minutes}m ${seconds % 60}s` : `${minutes}m`;
		return `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
	}

	function actor(session: TerminalSessionDto) {
		if (session.actor_token_name)
			return t('projects.application_terminal.history_token', { name: session.actor_token_name });
		return (
			session.actor_name ||
			session.actor_email ||
			t('projects.application_terminal.history_unknown_actor')
		);
	}

	function resultLabel(session: TerminalSessionDto) {
		switch (session.result) {
			case 'closed':
				return t('projects.application_terminal.history_result_closed');
			case 'disconnected':
				return t('projects.application_terminal.history_result_disconnected');
			case 'failed':
				return t('projects.application_terminal.history_result_failed');
			default:
				return t('projects.application_terminal.history_open');
		}
	}
</script>

{#if !forbidden}
	<Card.Root class="gap-0 pb-0 {className}">
		<Card.Header>
			<Card.Title>{t('projects.application_terminal.recent_sessions')}</Card.Title>
		</Card.Header>
		{#if loading}
			<div class="flex flex-col gap-2 px-4 py-3" aria-busy="true">
				{#each [0, 1, 2] as row (row)}
					<Skeleton class="h-6 w-full" />
				{/each}
			</div>
		{:else if failed && !sessions.length}
			<div class="flex flex-wrap items-center gap-3 px-4 py-4 text-sm text-muted-foreground">
				<span>{t('projects.application_terminal.history_failed')}</span>
				<Button variant="outline" size="sm" onclick={reload}
					>{t('projects.application_terminal.history_retry')}</Button>
			</div>
		{:else if !sessions.length}
			<p class="px-4 py-4 text-sm text-muted-foreground">
				{t('projects.application_terminal.history_empty')}
			</p>
		{:else}
			<ul class="flex flex-col">
				{#each sessions as session (session.id)}
					<li
						class="trow grid grid-cols-[auto_minmax(0,1fr)] items-center gap-x-2.5 gap-y-1 px-4 py-2.5 text-[12.5px] sm:grid-cols-[auto_minmax(0,1fr)_auto]">
						<span
							class="lp-raised grid size-6 place-items-center rounded-full bg-muted text-muted-foreground">
							{#if session.actor_token_name}<KeyRoundIcon class="size-3.5" />{:else}<UserIcon
									class="size-3.5" />{/if}
						</span>
						<span class="min-w-0">
							<span class="flex min-w-0 items-center gap-1.5">
								<b class="truncate font-medium">{actor(session)}</b>
								<span
									class="inline-flex min-w-0 items-center gap-1 font-mono text-[11.5px] text-muted-foreground">
									{#if session.kind === 'host'}
										<ServerIcon class="size-3 shrink-0" />
										<span class="truncate"
											>{hostLabel ?? t('projects.application_terminal.session_host')}</span>
									{:else}
										<SquareTerminalIcon class="size-3 shrink-0" />
										<span class="truncate">{session.container_id}</span>
									{/if}
								</span>
							</span>
							<small
								class="block truncate text-[11.5px]"
								class:text-destructive={session.result === 'failed'}
								class:text-success={!session.result}
								class:text-muted-foreground={session.result && session.result !== 'failed'}
								title={session.failure_reason ?? undefined}
								>{resultLabel(session)}{session.failure_reason
									? ` · ${session.failure_reason}`
									: ''}</small>
						</span>
						<span
							class="col-start-2 font-mono text-xs text-muted-foreground tabular-nums sm:col-start-auto sm:text-right">
							<time datetime={`${session.started_at.replace(' ', 'T')}Z`}
								>{relativeTime(utcMs(session.started_at), Date.now(), i18n.locale)}</time
							>{#if session.duration_seconds !== null}, {duration(session.duration_seconds)}{/if}
						</span>
					</li>
				{/each}
			</ul>
			{#if hasMore}
				<div class="lp-groove-t flex justify-center px-4 py-2">
					<Button variant="ghost" size="sm" disabled={loadingMore} onclick={loadMore}
						>{t('projects.application_terminal.history_load_more')}</Button>
				</div>
			{/if}
		{/if}
	</Card.Root>
{/if}

<style>
	.trow + .trow {
		border-top: 1px solid var(--lp-emboss);
		box-shadow: inset 0 1px 0 var(--lp-bevel);
	}
</style>
