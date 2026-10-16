<script lang="ts">
	import { onDestroy, onMount } from 'svelte';
	import { Terminal } from '@xterm/xterm';
	import { FitAddon } from '@xterm/addon-fit';
	import { WebLinksAddon } from '@xterm/addon-web-links';
	import { SearchAddon } from '@xterm/addon-search';
	import { ProgressAddon, type IProgressState } from '@xterm/addon-progress';
	import '@xterm/xterm/css/xterm.css';
	import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
	import CircleDotIcon from '@lucide/svelte/icons/circle-dot';
	import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';
	import SearchIcon from '@lucide/svelte/icons/search';
	import { Button } from '#lib/components/ui/button/index.js';
	import ShellSelect from '#lib/components/shell-select.svelte';
	import TerminalSearch from '#lib/components/terminal/terminal-search.svelte';
	import { t } from '#lib/i18n/index.svelte.js';
	import {
		openServerTerminalSocket,
		SERVER_TERMINAL_SHELLS,
		type ServerTerminalShell,
	} from '#lib/clients/server-terminal-client.js';
	import type { TerminalSocket } from '#lib/clients/terminal-client.js';
	import { loadNerdFont } from '#lib/utils/lazy-nerd-font.js';
	import { loadTerminalRenderer } from '#lib/utils/terminal-renderer.js';

	let {
		active,
		organizationId,
		serverId,
	}: { active: boolean; organizationId: string; serverId: string } = $props();
	type Status = 'connecting' | 'connected' | 'error' | 'disconnected';
	let shell = $state<ServerTerminalShell>('bash');
	let status = $state<Status>('connecting');
	let problem = $state('');
	let host = $state<HTMLDivElement | null>(null);
	// $state.raw so the <TerminalSearch> child re-runs when these get assigned in
	// start(); .raw avoids deep-proxying the xterm instance.
	let terminal = $state.raw<Terminal | null>(null);
	let fit: FitAddon | null = null;
	let socket: TerminalSocket | null = null;
	let resizeObserver: ResizeObserver | null = null;
	let themeObserver: MutationObserver | null = null;
	let disposeInput: (() => void) | null = null;
	let searchAddon = $state.raw<SearchAddon | null>(null);
	let terminalSearch: TerminalSearch | null = null;
	let disposeRenderer: (() => void) | null = null;
	let progress = $state<IProgressState>({ state: 0, value: 0 });
	// OSC 9;4 progress states -> bar colour (1 normal, 2 error, 3 busy, 4 paused).
	const PROGRESS_COLOR: Record<1 | 2 | 3 | 4, string> = {
		1: 'bg-success',
		2: 'bg-destructive',
		3: 'bg-success',
		4: 'bg-warning',
	};
	const dark = {
		background: '#18181b',
		foreground: '#e4e4e7',
		cursor: '#34d399',
		selectionBackground: '#155e75aa',
	};
	const light = {
		background: '#fafafa',
		foreground: '#27272a',
		cursor: '#047857',
		selectionBackground: '#99f6e4aa',
	};
	const isDark = () => document.documentElement.classList.contains('dark');
	function fitNow() {
		if (!fit || !terminal || !active) return;
		try {
			fit.fit();
		} catch {
			return;
		}
		const size = fit.proposeDimensions();
		if (socket?.readyState === WebSocket.OPEN && size?.cols && size.rows)
			socket.sendResize(size.cols, size.rows);
	}
	function disconnect() {
		socket?.close();
		socket = null;
		disposeInput?.();
		disposeInput = null;
	}
	function cleanup() {
		disconnect();
		resizeObserver?.disconnect();
		themeObserver?.disconnect();
		disposeRenderer?.();
		disposeRenderer = null;
		try {
			terminal?.dispose();
		} catch (error) {
			// A half-disposed WebGL addon can throw again here; never break teardown.
			console.warn('[terminal] dispose failed', error);
		}
		terminal = null;
		fit = null;
		searchAddon = null;
	}
	function connect() {
		disconnect();
		status = 'connecting';
		problem = '';
		progress = { state: 0, value: 0 };
		terminal?.clear();
		const current = openServerTerminalSocket(organizationId, serverId, shell);
		socket = current;
		current.onData((data) => terminal?.write(data));
		disposeInput = terminal?.onData((data) => current.sendStdin(data)).dispose ?? null;
		current.onOpen(() => {
			if (socket !== current) return;
			status = 'connected';
			requestAnimationFrame(() => {
				fitNow();
				terminal?.focus();
			});
		});
		current.onError((message) => {
			if (socket !== current) return;
			status = 'error';
			problem = message;
		});
		current.onClose(() => {
			if (socket !== current) return;
			disposeInput?.();
			disposeInput = null;
			socket = null;
			if (status !== 'error') {
				status = 'disconnected';
				terminal?.write('\r\n[connection closed]\r\n');
			}
		});
	}
	async function start() {
		if (!host) return;
		await loadNerdFont();
		if (!host) return;
		terminal = new Terminal({
			fontFamily: "'Commit Mono', 'CommitMono Nerd Font Mono', monospace",
			fontSize: 13,
			cursorBlink: true,
			convertEol: true,
			rightClickSelectsWord: true,
			// SearchAddon match highlighting needs a non-zero overview ruler; without
			// it findNext() draws no decorations and reports no results.
			allowProposedApi: true,
			overviewRulerWidth: 14,
			theme: isDark() ? dark : light,
		});
		terminal.onSelectionChange(() => {
			const selection = terminal?.getSelection();
			if (selection) navigator.clipboard?.writeText(selection).catch(() => {});
		});
		// Ctrl+Shift+F opens the in-terminal search box.
		terminal.attachCustomKeyEventHandler((event) => {
			if (
				event.type === 'keydown' &&
				event.ctrlKey &&
				event.shiftKey &&
				event.key.toLowerCase() === 'f'
			) {
				void terminalSearch?.show();
				return false;
			}
			return true;
		});
		fit = new FitAddon();
		terminal.loadAddon(fit);
		terminal.loadAddon(new WebLinksAddon());
		terminal.open(host);
		// Accelerated renderer (WebGL -> canvas -> DOM). Must come after open().
		disposeRenderer = loadTerminalRenderer(terminal);
		// Search + ConEmu OSC 9;4 progress. No Unicode11/Ligatures/Image yet — the
		// image addon isn't rendering and is parked for later.
		searchAddon = new SearchAddon();
		terminal.loadAddon(searchAddon);
		const progressAddon = new ProgressAddon();
		terminal.loadAddon(progressAddon);
		progressAddon.onChange((state) => (progress = state));
		themeObserver = new MutationObserver(() => {
			if (terminal) terminal.options.theme = isDark() ? dark : light;
		});
		themeObserver.observe(document.documentElement, {
			attributes: true,
			attributeFilter: ['class'],
		});
		resizeObserver = new ResizeObserver(fitNow);
		resizeObserver.observe(host.parentElement ?? host);
		requestAnimationFrame(fitNow);
		connect();
	}
	$effect(() => {
		if (active) requestAnimationFrame(fitNow);
	});
	onMount(() => void start());
	onDestroy(cleanup);
</script>

<div class:invisible={!active} class="absolute inset-0 flex min-h-0 flex-col">
	<div
		class="flex flex-wrap items-center gap-x-3 gap-y-2 border-b bg-background px-5 py-2.5 text-xs">
		<span class="flex items-center gap-2 text-muted-foreground">
			{t('organizations.servers.shell')}
			<ShellSelect
				bind:value={shell}
				options={SERVER_TERMINAL_SHELLS}
				onValueChange={connect}
				id={`server-console-shell-${serverId}`}
				triggerClass="font-mono text-foreground" />
		</span><Button size="sm" variant="outline" onclick={connect} disabled={status === 'connecting'}
			><RefreshCwIcon class={`size-3.5 ${status === 'connecting' ? 'animate-spin' : ''}`} />{t(
				'organizations.servers.reconnect',
			)}</Button
		><Button
			size="sm"
			variant="outline"
			onclick={() => terminalSearch?.show()}
			title={t('organizations.servers.search_terminal')}
			><SearchIcon class="size-3.5" />{t('organizations.servers.search')}</Button
		><span class="ml-auto flex items-center gap-1.5 text-muted-foreground"
			><CircleDotIcon
				class={`size-3.5 ${status === 'connected' ? 'text-success' : status === 'connecting' ? 'text-warning' : status === 'error' ? 'text-destructive' : 'text-muted-foreground'}`} />{status}</span>
	</div>
	<div class="relative min-h-0 flex-1 bg-[#fafafa] dark:bg-[#18181b]">
		{#if progress.state !== 0}
			<div
				class="pointer-events-none absolute inset-x-0 top-0 z-20 h-1 overflow-hidden bg-black/10 dark:bg-white/10"
				role="progressbar"
				aria-label={t('organizations.servers.terminal_progress')}
				aria-valuenow={progress.state === 3 ? undefined : progress.value}
				aria-valuemin={0}
				aria-valuemax={100}>
				{#if progress.state === 3}
					<div class={`progress-bounce h-full w-2/5 rounded-full ${PROGRESS_COLOR[3]}`}></div>
				{:else}
					<div
						class={`progress-fill h-full rounded-full ${PROGRESS_COLOR[progress.state]}`}
						style={`width:${progress.value}%`}>
					</div>
				{/if}
			</div>
		{/if}
		{#if problem}<div
				class="absolute inset-x-3 top-3 z-10 flex items-center gap-2 rounded-lg border border-destructive/30 bg-background/95 px-3 py-2 text-xs text-destructive shadow-sm">
				<CircleAlertIcon class="size-4 shrink-0" />{problem}
			</div>{/if}
		<div bind:this={host} class="absolute inset-3"></div>
		<TerminalSearch bind:this={terminalSearch} addon={searchAddon} {terminal} />
	</div>
</div>

<style>
	/* Indeterminate (OSC 9;4;3) bar: runs wall to wall and bounces off each edge.
	   The back-out easing overshoots slightly and the track clips it, so it reads
	   as a bump. `alternate` returns it without a hard jump. */
	@keyframes progress-bounce {
		from {
			transform: translateX(-25%);
		}
		to {
			transform: translateX(175%);
		}
	}
	.progress-bounce {
		animation: progress-bounce 1.3s cubic-bezier(0.34, 1.56, 0.64, 1) infinite alternate;
	}
	/* Determinate fill: the back-out easing lets the width overshoot the new
	   value and settle back, so every progress bump gets a small rebound. */
	.progress-fill {
		transition: width 450ms cubic-bezier(0.34, 1.56, 0.64, 1);
	}
	@media (prefers-reduced-motion: reduce) {
		.progress-fill {
			transition: width 150ms ease-out;
		}
		.progress-bounce {
			animation: none;
			width: 100%;
			opacity: 0.6;
		}
	}
</style>
