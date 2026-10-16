<script lang="ts">
	import { onDestroy, onMount } from 'svelte';
	import { Terminal } from '@xterm/xterm';
	import { FitAddon } from '@xterm/addon-fit';
	import { WebLinksAddon } from '@xterm/addon-web-links';
	import '@xterm/xterm/css/xterm.css';
	import { Button } from '#lib/components/ui/button/index.js';
	import ShellSelect from '#lib/components/shell-select.svelte';
	import { loadNerdFont } from '#lib/utils/lazy-nerd-font.js';
	import { toast } from '#lib/utils/toast.js';
	import {
		createContainerExec,
		openContainerExecSocket,
	} from '#lib/clients/podman-console-client.js';
	import type { TerminalSocket } from '#lib/clients/terminal-client.js';
	import { t } from '#lib/i18n/index.svelte.js';

	const CONTAINER_SHELLS = ['/bin/sh', '/bin/bash', '/bin/zsh', '/bin/nu', '/bin/fish'] as const;

	let {
		organizationId,
		serverId,
		containerId,
	}: { organizationId: string; serverId: string; containerId: string } = $props();

	type Status = 'connecting' | 'connected' | 'error' | 'disconnected';
	let status = $state<Status>('connecting');
	let shell = $state('/bin/sh');

	let hostEl = $state<HTMLDivElement | null>(null);
	let term: Terminal | null = null;
	let fit: FitAddon | null = null;
	let socket: TerminalSocket | null = null;
	let resizeObserver: ResizeObserver | null = null;
	let themeObserver: MutationObserver | null = null;
	let dataDisposable: { dispose: () => void } | null = null;
	let sendControlByte: (data: string) => void = () => {};

	const DARK = {
		background: '#1e1e1e',
		foreground: '#e4e4e4',
		cursor: '#fbbf24',
		selectionBackground: '#516d9dcc',
		selectionForeground: '#ffffff',
	};
	const LIGHT = {
		background: '#ffffff',
		foreground: '#1e1e1e',
		cursor: '#1e3a8a',
		selectionBackground: '#add6ffcc',
	};
	const isDark = () =>
		typeof document !== 'undefined' && document.documentElement.classList.contains('dark');

	function fitNow() {
		if (!fit || !term) return;
		try {
			fit.fit();
		} catch {
			/* not laid out yet */
		}
		if (status === 'connected' && socket) {
			const dims = fit.proposeDimensions();
			if (dims && dims.cols > 0) socket.sendResize(dims.cols, dims.rows);
		}
	}

	function buildTerminal() {
		if (!hostEl) return;
		term = new Terminal({
			fontFamily: "'Commit Mono', 'CommitMono Nerd Font Mono', monospace",
			fontSize: 13,
			cursorBlink: true,
			convertEol: true,
			rightClickSelectsWord: true,
			theme: isDark() ? DARK : LIGHT,
		});
		// Drag to select copies straight to the clipboard (best-effort; blocked
		// contexts just leave the selection for a manual Ctrl/Cmd+C).
		term.onSelectionChange(() => {
			const text = term?.getSelection();
			if (text) navigator.clipboard?.writeText(text).catch(() => {});
		});
		term.attachCustomKeyEventHandler((event) => {
			if (event.type !== 'keydown' || !event.ctrlKey || event.altKey || event.metaKey) return true;
			const key = event.key;
			if (key.length !== 1) return true;
			const lower = key.toLowerCase();
			if (lower === 'v') return true;
			if (lower === 'c' && term?.hasSelection()) return true;
			const code = lower.charCodeAt(0);
			if (code < 97 || code > 122) return true;
			sendControlByte(String.fromCharCode(code - 96));
			return false;
		});
		fit = new FitAddon();
		term.loadAddon(fit);
		term.loadAddon(new WebLinksAddon());
		term.open(hostEl);
		themeObserver = new MutationObserver(() => {
			if (term) term.options.theme = isDark() ? DARK : LIGHT;
		});
		themeObserver.observe(document.documentElement, {
			attributes: true,
			attributeFilter: ['class'],
		});
		resizeObserver = new ResizeObserver(() => fitNow());
		if (hostEl.parentElement) resizeObserver.observe(hostEl.parentElement);
		fitNow();
	}

	async function connect() {
		disconnect();
		status = 'connecting';
		term?.clear();
		try {
			const execId = await createContainerExec(
				organizationId,
				serverId,
				containerId,
				[shell, '-l', '-i'],
				true,
			);
			const sock = openContainerExecSocket(organizationId, serverId, containerId, execId);
			socket = sock;
			sock.onData((bytes) => term?.write(bytes));
			sendControlByte = (data) => {
				if (socket === sock) sock.sendStdin(data);
			};
			dataDisposable =
				term?.onData((data) => {
					if (socket === sock) sock.sendStdin(data);
				}) ?? null;
			sock.onOpen(() => {
				status = 'connected';
				fitNow();
				const dims = fit?.proposeDimensions();
				sock.sendResize(dims?.cols ?? 120, dims?.rows ?? 40);
				requestAnimationFrame(() => term?.focus());
			});
			sock.onError((message) => {
				status = 'error';
				toast.error(message);
			});
			sock.onClose(() => {
				dataDisposable?.dispose();
				dataDisposable = null;
				sendControlByte = () => {};
				if (socket === sock) socket = null;
				if (status !== 'error') {
					term?.write('\r\n[process completed]\r\n');
					status = 'disconnected';
				}
			});
		} catch (err) {
			status = 'error';
			toast.error(
				err instanceof Error ? err.message : t('organizations.containers.shell_start_failed'),
			);
		}
	}

	function disconnect() {
		socket?.close();
		socket = null;
		dataDisposable?.dispose();
		dataDisposable = null;
		sendControlByte = () => {};
	}

	onMount(async () => {
		await loadNerdFont();
		buildTerminal();
		void connect();
	});
	onDestroy(() => {
		disconnect();
		themeObserver?.disconnect();
		resizeObserver?.disconnect();
		term?.dispose();
		term = null;
	});
</script>

<div class="flex min-h-0 flex-1 flex-col">
	<div class="flex items-center gap-2 border-b px-4 py-2 text-xs">
		<ShellSelect
			bind:value={shell}
			options={CONTAINER_SHELLS}
			onValueChange={() => void connect()} />
		<Button size="sm" variant="outline" onclick={() => void connect()}
			>{t('organizations.containers.reconnect')}</Button>
		<span class="ml-auto flex items-center gap-1.5 text-muted-foreground">
			<span
				class="inline-block size-2 rounded-full"
				class:bg-success={status === 'connected'}
				class:bg-warning={status === 'connecting'}
				class:bg-destructive={status === 'error'}
				class:bg-muted-foreground={status === 'disconnected'}></span>
			<span class="capitalize">{status}</span>
		</span>
	</div>
	<div class="relative min-h-0 flex-1 bg-white p-2 dark:bg-[#1e1e1e]">
		<div bind:this={hostEl} class="h-full w-full"></div>
	</div>
</div>
