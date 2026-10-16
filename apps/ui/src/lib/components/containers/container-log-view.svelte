<script lang="ts">
	import { onDestroy, onMount } from 'svelte';
	import { Terminal } from '@xterm/xterm';
	import { FitAddon } from '@xterm/addon-fit';
	import { WebLinksAddon } from '@xterm/addon-web-links';
	import '@xterm/xterm/css/xterm.css';
	import { Button } from '#lib/components/ui/button/index.js';
	import { loadNerdFont } from '#lib/utils/lazy-nerd-font.js';
	import { toast } from '#lib/utils/toast.js';
	import { getContainerLogs, streamContainerLogs } from '#lib/clients/podman-console-client.js';
	import { t } from '#lib/i18n/index.svelte.js';
	import PlayIcon from '@lucide/svelte/icons/play';
	import PauseIcon from '@lucide/svelte/icons/pause';
	import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';
	import CopyIcon from '@lucide/svelte/icons/copy';
	import EraserIcon from '@lucide/svelte/icons/eraser';

	let {
		organizationId,
		serverId,
		containerId,
	}: { organizationId: string; serverId: string; containerId: string } = $props();

	let live = $state(true);
	let busy = $state(false);
	let hostEl = $state<HTMLDivElement | null>(null);
	let term: Terminal | null = null;
	let fit: FitAddon | null = null;
	let resizeObserver: ResizeObserver | null = null;
	let themeObserver: MutationObserver | null = null;

	const DARK = {
		background: '#1e1e1e',
		foreground: '#e4e4e4',
		cursor: '#1e1e1e',
		selectionBackground: '#516d9dcc',
		selectionForeground: '#ffffff',
	};
	const LIGHT = {
		background: '#ffffff',
		foreground: '#1e1e1e',
		cursor: '#ffffff',
		selectionBackground: '#add6ffcc',
	};
	const isDark = () =>
		typeof document !== 'undefined' && document.documentElement.classList.contains('dark');

	function fitNow() {
		try {
			fit?.fit();
		} catch {
			/* not laid out yet */
		}
	}

	async function fetchTail() {
		busy = true;
		try {
			const { logs } = await getContainerLogs(organizationId, serverId, containerId);
			term?.clear();
			term?.write(
				(logs || t('organizations.containers.log_lines_placeholder')).replace(/\n/g, '\r\n') +
					'\r\n',
			);
		} catch (err) {
			toast.error(
				err instanceof Error ? err.message : t('organizations.containers.logs_fetch_failed'),
			);
		} finally {
			busy = false;
		}
	}

	function copyAll() {
		if (!term) return;
		let text = term.getSelection();
		if (!text) {
			term.selectAll();
			text = term.getSelection();
			term.clearSelection();
		}
		if (text) {
			navigator.clipboard?.writeText(text).catch(() => {});
			toast.success(t('common.common.copied'));
		}
	}

	// Live tail over SSE while `live` is on.
	$effect(() => {
		if (!live || !term) return;
		const stream = streamContainerLogs(organizationId, serverId, containerId);
		const onLine = (event: Event) => {
			term?.write((event as MessageEvent<string>).data.replace(/\r?\n/g, '') + '\r\n');
		};
		stream.addEventListener('log', onLine);
		return () => {
			stream.removeEventListener('log', onLine);
			stream.close();
		};
	});

	onMount(async () => {
		await loadNerdFont();
		term = new Terminal({
			fontFamily: "'Commit Mono', 'CommitMono Nerd Font Mono', monospace",
			fontSize: 13,
			convertEol: true,
			disableStdin: true,
			cursorStyle: 'bar',
			cursorInactiveStyle: 'none',
			scrollback: 5000,
			rightClickSelectsWord: true,
			theme: isDark() ? DARK : LIGHT,
		});
		term.onSelectionChange(() => {
			const text = term?.getSelection();
			if (text) navigator.clipboard?.writeText(text).catch(() => {});
		});
		fit = new FitAddon();
		term.loadAddon(fit);
		term.loadAddon(new WebLinksAddon());
		if (hostEl) term.open(hostEl);
		themeObserver = new MutationObserver(() => {
			if (term) term.options.theme = isDark() ? DARK : LIGHT;
		});
		themeObserver.observe(document.documentElement, {
			attributes: true,
			attributeFilter: ['class'],
		});
		resizeObserver = new ResizeObserver(() => fitNow());
		if (hostEl?.parentElement) resizeObserver.observe(hostEl.parentElement);
		fitNow();
		await fetchTail();
	});
	onDestroy(() => {
		themeObserver?.disconnect();
		resizeObserver?.disconnect();
		term?.dispose();
		term = null;
	});
</script>

<div class="flex min-h-0 flex-1 flex-col">
	<div class="flex flex-wrap items-center gap-2 border-b px-4 py-2">
		<Button size="sm" variant="outline" onclick={() => (live = !live)}>
			{#if live}<PauseIcon class="size-3.5" />{t('organizations.containers.live')}{:else}<PlayIcon
					class="size-3.5" />{t('organizations.containers.paused')}{/if}
		</Button>
		<Button size="sm" variant="outline" onclick={fetchTail} disabled={busy}>
			<RefreshCwIcon class={`size-3.5 ${busy ? 'animate-spin' : ''}`} />{t(
				'organizations.containers.fetch',
			)}
		</Button>
		<Button size="sm" variant="outline" onclick={copyAll}
			><CopyIcon class="size-3.5" />{t('common.common.copy')}</Button>
		<Button size="sm" variant="outline" onclick={() => term?.clear()}>
			<EraserIcon class="size-3.5" />{t('organizations.common.clear')}
		</Button>
		<span class="ml-auto flex items-center gap-1.5 text-xs text-muted-foreground">
			<span
				class="inline-block size-2 rounded-full"
				class:bg-success={live}
				class:bg-muted-foreground={!live}></span>
			{live ? 'streaming' : 'paused'}
		</span>
	</div>
	<div class="min-h-0 flex-1 bg-white p-2 dark:bg-[#1e1e1e]">
		<div bind:this={hostEl} class="h-full w-full"></div>
	</div>
</div>
