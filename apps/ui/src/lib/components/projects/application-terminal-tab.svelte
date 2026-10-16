<script lang="ts">
	import { onDestroy, onMount } from 'svelte';
	import { page } from '$app/state';
	import { Terminal } from '@xterm/xterm';
	import { FitAddon } from '@xterm/addon-fit';
	import { WebLinksAddon } from '@xterm/addon-web-links';
	import '@xterm/xterm/css/xterm.css';

	import { Button } from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import ShellSelect from '#lib/components/shell-select.svelte';
	import {
		listApplicationLogTargets,
		type ApplicationLogTargetDto,
	} from '#lib/clients/project-client.js';
	import {
		createExec,
		listApplicationTerminalSessions,
		TerminalSocket,
	} from '#lib/clients/terminal-client.js';
	import TerminalSessionHistory from './terminal-session-history.svelte';
	import { isRunning } from '#lib/utils/container-status.js';
	import { loadNerdFont } from '#lib/utils/lazy-nerd-font.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';

	import RotateCcwIcon from '@lucide/svelte/icons/rotate-ccw';
	import TerminalIcon from '@lucide/svelte/icons/terminal';
	import SquareTerminalIcon from '@lucide/svelte/icons/square-terminal';
	import ServerIcon from '@lucide/svelte/icons/server';
	import PlugIcon from '@lucide/svelte/icons/plug';
	import PlugZapIcon from '@lucide/svelte/icons/plug-zap';
	import ZoomInIcon from '@lucide/svelte/icons/zoom-in';
	import ZoomOutIcon from '@lucide/svelte/icons/zoom-out';

	let {
		organizationId,
		projectId,
		environmentId,
		applicationId,
		hostServerName,
		onOpenHostShell,
	}: {
		organizationId: string;
		projectId: string;
		environmentId: string;
		applicationId: string;
		/** Server the app runs on; with `onOpenHostShell` it adds the host shell row. */
		hostServerName?: string;
		onOpenHostShell?: () => void;
	} = $props();

	// The exec endpoint runs whatever argv it gets, so every shell is allowed;
	// one missing from the image fails fast with the exited-early toast.
	const SHELLS = ['sh', 'bash', 'zsh', 'fish', 'nu', 'pwsh'] as const;
	type Shell = (typeof SHELLS)[number];
	let shellPick = $state<Record<string, Shell>>({});

	type Status = 'disconnected' | 'connecting' | 'connected' | 'error';

	let targets = $state<ApplicationLogTargetDto[]>([]);
	let selectedContainer = $state<string | null>(null);
	let selectedShell = $state<Shell>('bash');
	let status = $state<Status>('disconnected');
	let notified = false;

	let host = $state<HTMLDivElement | null>(null);
	let container = $state<HTMLDivElement | null>(null);
	let term: Terminal | null = null;
	let fit: FitAddon | null = null;
	let socket: TerminalSocket | null = null;
	let resizeObserver: ResizeObserver | null = null;
	let resizeTimer: ReturnType<typeof setTimeout> | null = null;
	let connectGraceTimer: ReturnType<typeof setTimeout> | null = null;
	let termDataDisposable: { dispose: () => void } | null = null;
	let themeObserver: MutationObserver | null = null;
	let focused = $state(false);
	let sendControlByte: (data: string) => void = () => {};
	const CONNECT_GRACE_MS = 1500;

	const DARK_THEME = {
		background: '#1e1e1e',
		foreground: '#e4e4e4',
		cursor: '#fbbf24',
		selectionBackground: '#516d9dcc',
	};
	const LIGHT_THEME = {
		background: '#ffffff',
		foreground: '#1e1e1e',
		cursor: '#1e3a8a',
		selectionBackground: '#add6ffcc',
	};

	let runningTargets = $derived(targets.filter((t) => isRunning(t.status)));
	let active = $derived(status === 'connected' || status === 'connecting');
	const targetLabel = (target: ApplicationLogTargetDto) =>
		target.service ?? t('projects.application_volumes.application_container');
	let canConnect = $derived(
		!!selectedContainer && !!selectedShell && status !== 'connecting' && status !== 'connected',
	);
	const FONT_SIZE_MIN = 8;
	const FONT_SIZE_MAX = 24;
	let fontSize = $state(13);

	function isDarkMode() {
		return typeof document !== 'undefined' && document.documentElement.classList.contains('dark');
	}

	function applyTheme() {
		if (!term) return;
		term.options.theme = isDarkMode() ? DARK_THEME : LIGHT_THEME;
	}

	function buildTerminal() {
		if (!host) return;
		term = new Terminal({
			fontFamily: "'Commit Mono', 'CommitMono Nerd Font Mono', monospace",
			fontSize,
			cursorBlink: true,
			cursorStyle: 'block',
			rightClickSelectsWord: true,
			theme: isDarkMode() ? DARK_THEME : LIGHT_THEME,
			convertEol: true,
			disableStdin: false,
		});
		// Drag-select copies to the clipboard immediately.
		term.onSelectionChange(() => {
			const text = term?.getSelection();
			if (text) navigator.clipboard?.writeText(text).catch(() => {});
		});
		// Intercept Ctrl+letter in capture so we always send the raw control
		// byte (0x01..0x1a). xterm.js v5's default key handling sometimes
		// emits an escape sequence instead, which nano/vim/htop don't accept
		// for control commands.
		// Ctrl+V is excluded so the browser's native paste (and xterm's own
		// paste handling) still works. Ctrl+C is excluded only when there's
		// an active selection, so copy works — otherwise it still sends SIGINT.
		term.attachCustomKeyEventHandler((event) => {
			if (event.type !== 'keydown') return true;
			if (!event.ctrlKey || event.altKey || event.metaKey) return true;
			const key = event.key;
			if (key.length !== 1) return true;
			const lower = key.toLowerCase();
			if (lower === 'v') return true;
			if (lower === 'c' && term?.hasSelection()) return true;
			const code = lower.charCodeAt(0);
			if (code < 97 || code > 122) return true; // a-z only
			const controlCode = code - 96; // Ctrl+A=1, Ctrl+Z=26
			sendControlByte(String.fromCharCode(controlCode));
			return false;
		});
		fit = new FitAddon();
		term.loadAddon(fit);
		term.loadAddon(new WebLinksAddon());
		term.open(host);
		term.textarea?.addEventListener('focus', () => (focused = true));
		term.textarea?.addEventListener('blur', () => (focused = false));

		// Track light/dark mode toggles so the running terminal repaints
		// with the right colors without losing scrollback.
		themeObserver = new MutationObserver(applyTheme);
		themeObserver.observe(document.documentElement, {
			attributes: true,
			attributeFilter: ['class'],
		});

		// Observe the parent container, not the xterm host. The xterm host
		// changes size when we call fit.fit(), which would feed back into
		// the observer and trigger a runaway resize loop. The parent's size
		// only changes from real layout changes (window resize, font zoom).
		resizeObserver = new ResizeObserver(() => scheduleFit());
		if (container) resizeObserver.observe(container);
		scheduleFit();
	}

	function scheduleFit() {
		if (resizeTimer) clearTimeout(resizeTimer);
		resizeTimer = setTimeout(() => {
			resizeTimer = null;
			if (!fit || !term) return;
			try {
				fit.fit();
			} catch {
				// Host not laid out yet — try again on next resize.
			}
			if (status === 'connected' && socket && term) {
				const dims = fit.proposeDimensions();
				if (dims) socket.sendResize(dims.cols, dims.rows);
			}
		}, 100);
	}

	async function loadTargets() {
		try {
			targets = await listApplicationLogTargets(
				organizationId,
				projectId,
				environmentId,
				applicationId,
			);
		} catch {
			targets = [];
		}
	}

	function waitForOpen(s: TerminalSocket): Promise<void> {
		return new Promise((resolve, reject) => {
			const sock = s.socket;
			const start = Date.now();
			const tick = () => {
				if (sock.readyState === WebSocket.OPEN) return resolve();
				if (sock.readyState === WebSocket.CLOSED) {
					return reject(new Error(t('projects.application_terminal.socket_closed')));
				}
				if (Date.now() - start > 10_000)
					return reject(new Error(t('projects.application_terminal.socket_timed_out')));
				setTimeout(tick, 50);
			};
			tick();
		});
	}

	async function connect() {
		if (!canConnect || !selectedContainer || !selectedShell) return;
		notified = false;
		status = 'connecting';
		try {
			const { execId } = await createExec({
				organizationId,
				projectId,
				environmentId,
				applicationId,
				containerId: selectedContainer,
				// pwsh installs to /usr/bin (or /opt) and takes -Login, not -l -i.
				command:
					selectedShell === 'pwsh'
						? ['/usr/bin/env', 'pwsh', '-Login', '-NoLogo']
						: [`/bin/${selectedShell}`, '-l', '-i'],
				tty: true,
			});
			const sock = TerminalSocket.open({
				organizationId,
				projectId,
				environmentId,
				applicationId,
				containerId: selectedContainer,
				execId,
			});
			sock.onData((bytes) => term?.write(bytes));
			sendControlByte = (data: string) => {
				if (socket === sock) sock.sendStdin(data);
			};
			const termDataDisposable = term?.onData((data) => {
				if (socket === sock) sock.sendStdin(data);
			});
			sock.onOpen(() => {
				// Send an initial resize immediately so the running PTY matches
				// the visible terminal before any TUI (btop, htop, vim, less)
				// queries its size. The backend pre-sizes to 120x40 as a
				// fallback, so this overrides with the real layout.
				try {
					fit?.fit();
				} catch {
					// Host not laid out yet — use backend default.
				}
				const dims = fit?.proposeDimensions();
				if (dims && dims.cols > 0 && dims.rows > 0) {
					sock.sendResize(dims.cols, dims.rows);
				} else {
					sock.sendResize(120, 40);
				}
				// Give the exec a moment to start; if the WS dies before any
				// data arrives, treat it as a connect failure (e.g. shell not
				// in the container) instead of a normal disconnect.
				if (connectGraceTimer) clearTimeout(connectGraceTimer);
				connectGraceTimer = setTimeout(() => {
					connectGraceTimer = null;
					if (sock.hasReceivedData() || sock.hasReceivedError()) return;
					if (status === 'connected' || status === 'connecting') {
						status = 'connected';
					}
				}, CONNECT_GRACE_MS);
			});
			sock.onError((message) => {
				if (!notified) {
					toast.error(message);
					notified = true;
				}
				status = 'error';
			});
			sock.onClose(() => {
				termDataDisposable?.dispose();
				if (socket === sock) {
					sendControlByte = () => {};
					socket = null;
				}
				if (connectGraceTimer) {
					clearTimeout(connectGraceTimer);
					connectGraceTimer = null;
				}
				const wasFastFail =
					!sock.hasReceivedData() && (status === 'connecting' || status === 'error');
				if (wasFastFail) {
					status = 'error';
					if (!notified) {
						toast.error(t('projects.application_terminal.exited_early'));
						notified = true;
					}
				} else {
					term?.write('\r\n[process completed]\r\n');
					status = 'disconnected';
				}
				if (socket === sock) socket = null;
			});
			socket = sock;
			await waitForOpen(sock);
			status = 'connected';
			requestAnimationFrame(() => term?.focus());
			scheduleFit();
		} catch (err) {
			status = 'error';
			if (!notified) {
				toast.error(
					err instanceof Error ? err.message : t('projects.application_terminal.failed_to_start'),
				);
				notified = true;
			}
		}
	}

	function connectTo(containerName: string) {
		selectedContainer = containerName;
		selectedShell = shellPick[containerName] ?? 'bash';
		void connect();
	}

	function clearScreen() {
		term?.clear();
		term?.focus();
	}

	function clearControlBinding() {
		sendControlByte = () => {};
	}

	function disconnect() {
		socket?.close();
		socket = null;
		termDataDisposable?.dispose();
		termDataDisposable = null;
		clearControlBinding();
		status = 'disconnected';
	}

	function zoomIn() {
		if (fontSize >= FONT_SIZE_MAX) return;
		fontSize += 1;
		applyFontSize();
	}
	function zoomOut() {
		if (fontSize <= FONT_SIZE_MIN) return;
		fontSize -= 1;
		applyFontSize();
	}
	function applyFontSize() {
		if (!term) return;
		term.options.fontSize = fontSize;
		try {
			fit?.fit();
		} catch {
			// Host not laid out yet.
		}
		if (status === 'connected' && socket && fit) {
			const dims = fit.proposeDimensions();
			if (dims && dims.cols > 0 && dims.rows > 0) {
				socket.sendResize(dims.cols, dims.rows);
			}
		}
	}

	onMount(async () => {
		await loadNerdFont();
		buildTerminal();
		await loadTargets();
		const requested = page.url.searchParams.get('container');
		if (requested && runningTargets.some((t) => t.container_name === requested)) {
			selectedContainer = requested;
		} else if (!selectedContainer && runningTargets.length === 1) {
			selectedContainer = runningTargets[0].container_name;
		}
	});

	onDestroy(() => {
		if (resizeTimer) clearTimeout(resizeTimer);
		if (connectGraceTimer) clearTimeout(connectGraceTimer);
		termDataDisposable?.dispose();
		termDataDisposable = null;
		clearControlBinding();
		themeObserver?.disconnect();
		themeObserver = null;
		resizeObserver?.disconnect();
		socket?.close();
		term?.dispose();
	});
</script>

<div class="flex max-w-full flex-col gap-4">
	<Card.Root class="gap-0 pb-0">
		<Card.Header>
			<Card.Title>{t('projects.application_terminal.container_shells_title')}</Card.Title>
			<Card.Description
				>{t('projects.application_terminal.container_shells_description')}</Card.Description>
		</Card.Header>
		{#if runningTargets.length === 0}
			<p class="px-4 py-5 text-sm text-muted-foreground">
				{t('projects.application_terminal.no_running_containers')}
			</p>
		{:else}
			<div class="flex flex-col">
				{#each runningTargets as target (target.container_name)}
					{@const current = selectedContainer === target.container_name && active}
					<div
						class="trow grid grid-cols-[auto_minmax(0,1fr)] items-center gap-x-3 gap-y-2.5 px-4 py-3 sm:grid-cols-[auto_minmax(0,1fr)_auto_auto]">
						<span class="term-tile"><SquareTerminalIcon class="size-4" /></span>
						<span class="min-w-0">
							<b class="block truncate font-medium">{targetLabel(target)}</b>
							<small class="block truncate font-mono text-[11.5px] text-muted-foreground"
								>{target.container_name} · {target.status}</small>
						</span>
						<div class="col-span-2 sm:col-span-1">
							<ShellSelect
								value={shellPick[target.container_name] ?? 'bash'}
								options={SHELLS}
								onValueChange={(next) => (shellPick[target.container_name] = next)} />
						</div>
						{#if current}
							<Button
								variant="outline"
								size="sm"
								class="col-span-2 sm:col-span-1"
								disabled={status !== 'connected'}
								onclick={disconnect}>
								<PlugIcon class="size-4" />
								{t('projects.application_terminal.disconnect')}
							</Button>
						{:else}
							<Button
								size="sm"
								class="col-span-2 sm:col-span-1"
								disabled={active}
								onclick={() => connectTo(target.container_name)}>
								<PlugZapIcon class="size-4" />
								{t('projects.application_terminal.connect')}
							</Button>
						{/if}
					</div>
				{/each}
			</div>
		{/if}
	</Card.Root>

	<div class="lp-panel overflow-hidden rounded-[14px] border border-foreground/9 bg-card">
		<div class="lp-groove-b flex flex-wrap items-center gap-2 bg-muted/40 px-3 py-2">
			<span
				class="inline-flex h-[22px] items-center gap-1.5 rounded-full px-2 text-[11.5px] capitalize"
				class:status-ok={status === 'connected'}
				class:status-warn={status === 'connecting'}
				class:status-err={status === 'error'}
				class:status-off={status === 'disconnected'}>
				<span class="size-1.5 rounded-full bg-current" class:pip={status === 'connected'}></span>
				{status}
			</span>
			{#if selectedContainer}
				<span class="flex min-w-0 items-center gap-1.5 font-mono text-xs text-muted-foreground">
					<TerminalIcon class="size-3.5 shrink-0" />
					<span class="truncate">{selectedContainer} · {selectedShell}</span>
				</span>
			{/if}
			<div class="ml-auto flex items-center gap-1">
				<Button variant="ghost" size="sm" disabled={status !== 'connected'} onclick={clearScreen}>
					<RotateCcwIcon class="size-4" />
					{t('projects.common.clear')}
				</Button>
				<Button
					variant="ghost"
					size="icon"
					disabled={fontSize <= FONT_SIZE_MIN}
					onclick={zoomOut}
					aria-label={t('projects.application_terminal.zoom_out')}
					title={t('projects.application_terminal.zoom_out')}>
					<ZoomOutIcon class="size-4" />
				</Button>
				<span class="min-w-9 text-center font-mono text-xs tabular-nums text-muted-foreground"
					>{fontSize}px</span>
				<Button
					variant="ghost"
					size="icon"
					disabled={fontSize >= FONT_SIZE_MAX}
					onclick={zoomIn}
					aria-label={t('projects.application_terminal.zoom_in')}
					title={t('projects.application_terminal.zoom_in')}>
					<ZoomInIcon class="size-4" />
				</Button>
			</div>
		</div>
		<div
			bind:this={container}
			class="relative min-h-96 overflow-hidden bg-white dark:bg-[#1e1e1e]"
			class:ring-2={status === 'connected' && !focused}
			class:ring-inset={status === 'connected' && !focused}
			class:ring-amber-500={status === 'connected' && !focused}>
			<!--
				xterm.js owns the element's keyboard input, so the host must be
				focusable and capture mousedown for cursor positioning. role="application"
				is the ARIA-documented pattern for terminal-style widgets.
			-->
			<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
			<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
			<div
				bind:this={host}
				class="absolute inset-0 p-2"
				style="cursor: text"
				onmousedown={() => term?.focus()}
				role="application"
				tabindex="0">
			</div>
			{#if status === 'connected' && !focused}
				<button
					type="button"
					class="absolute inset-0 flex items-center justify-center bg-background/60 text-sm font-medium text-foreground backdrop-blur-sm transition-opacity hover:bg-background/50"
					onclick={() => term?.focus()}>
					{t('projects.application_terminal.click_to_focus')}
				</button>
			{/if}
			{#if runningTargets.length === 0}
				<div
					class="absolute inset-0 flex items-center justify-center bg-background/95 p-4 text-center text-sm text-muted-foreground">
					{t('projects.application_terminal.no_running_containers')}
				</div>
			{/if}
		</div>
	</div>

	{#if onOpenHostShell}
		<Card.Root class="gap-0 pb-0">
			<Card.Header>
				<Card.Title>{t('projects.application_terminal.host_title')}</Card.Title>
			</Card.Header>
			<div
				class="grid grid-cols-[auto_minmax(0,1fr)] items-center gap-x-3 gap-y-2.5 px-4 py-3 sm:grid-cols-[auto_minmax(0,1fr)_auto]">
				<span class="term-tile"><ServerIcon class="size-4" /></span>
				<span class="min-w-0">
					<b class="block truncate font-medium">{hostServerName}</b>
					<small class="block truncate font-mono text-[11.5px] text-muted-foreground"
						>{t('projects.application_terminal.host_description', {
							server: hostServerName ?? '',
						})}</small>
				</span>
				<Button
					variant="outline"
					size="sm"
					class="col-span-2 sm:col-span-1"
					onclick={onOpenHostShell}>
					<TerminalIcon class="size-4" />
					{t('projects.application_terminal.open_host_shell')}
				</Button>
			</div>
		</Card.Root>
	{/if}

	<TerminalSessionHistory
		refreshOn={status}
		load={(page) =>
			listApplicationTerminalSessions(
				{ organizationId, projectId, environmentId, applicationId },
				page,
			)} />
</div>

<style>
	.trow + .trow {
		border-top: 1px solid var(--lp-emboss);
		box-shadow: inset 0 1px 0 var(--lp-bevel);
	}
	.term-tile {
		display: grid;
		width: 2rem;
		height: 2rem;
		place-items: center;
		border-radius: 9px;
		background: #0c0e12;
		color: #43c983;
		box-shadow:
			inset 1px 1px 0 rgb(255 255 255 / 0.06),
			inset -1px -1px 0 rgb(0 0 0 / 0.4);
	}
	.status-ok {
		background: color-mix(in oklch, var(--success) 14%, transparent);
		color: var(--success);
	}
	.status-warn {
		background: color-mix(in oklch, var(--warning) 14%, transparent);
		color: var(--warning);
	}
	.status-err {
		background: color-mix(in oklch, var(--destructive) 14%, transparent);
		color: var(--destructive);
	}
	.status-off {
		background: var(--muted);
		color: var(--muted-foreground);
	}
	.pip {
		animation: pip 2s var(--lp-ease) infinite;
	}
	@keyframes pip {
		50% {
			opacity: 0.35;
		}
	}
	@media (prefers-reduced-motion: reduce) {
		.pip {
			animation: none;
		}
	}
</style>
