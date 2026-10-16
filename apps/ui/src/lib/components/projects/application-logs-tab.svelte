<script lang="ts">
	import { onDestroy, onMount } from 'svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import SearchableDropdown, {
		type SearchableDropdownItem,
	} from '#lib/components/custom/searchable-dropdown.svelte';
	import {
		streamDeploymentLogs,
		fetchDeploymentLogs,
		listApplicationLogTargets,
		type ApplicationLogTargetDto,
	} from '#lib/clients/project-client.js';
	import { isRunning } from '#lib/utils/container-status.js';
	import { loadNerdFont } from '#lib/utils/lazy-nerd-font.js';
	import { loadMonaco } from '#lib/utils/load-monaco.js';
	import SoftCheckbox from '#lib/components/custom/soft-checkbox.svelte';
	import * as Label from '#lib/components/ui/label/index.js';
	import PlayIcon from '@lucide/svelte/icons/play';
	import PauseIcon from '@lucide/svelte/icons/pause';
	import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';
	import PaletteIcon from '@lucide/svelte/icons/palette';
	import CopyIcon from '@lucide/svelte/icons/copy';
	import SparklesIcon from '@lucide/svelte/icons/sparkles';
	import { deploymentLogDecoration, deploymentLogTone } from '#lib/utils/deployment-log-tone.js';
	import { formatLogTimestamp } from '#lib/utils/log-timestamp.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let {
		organizationId,
		projectId,
		environmentId,
		applicationId,
		onAskAssistant,
	}: {
		organizationId: string;
		projectId: string;
		environmentId: string;
		applicationId: string;
		/** Given the rendered log text. Omitted when the assistant is off. */
		onAskAssistant?: (text: string) => void;
	} = $props();

	let editorHost = $state<HTMLDivElement | null>(null);
	let editor = $state<import('monaco-editor').editor.IStandaloneCodeEditor | null>(null);
	let monacoApi = $state<typeof import('monaco-editor') | null>(null);
	let eventSource = $state<EventSource | null>(null);
	let status = $state<'connected' | 'paused' | 'disconnected' | 'error'>('disconnected');
	let logTargets = $state<ApplicationLogTargetDto[]>([]);
	let selectedContainer = $state<string | null>(null);
	let loadingTargets = $state(true);
	let targetGeneration = 0;
	type LogLine = { text: string; ts: string | null };
	let buffer = $state<LogLine[]>([]);
	let allLines = $state<LogLine[]>([]);
	let renderedLines: string[] = [];
	let showTimestamps = $state(true);
	const TS_STORAGE_KEY = 'litepod-log-timestamp-enabled';
	let tailLines = $state(200);
	const MAX_VISIBLE_LINES = 1000;
	let retryCount = $state(0);
	const maxRetries = 3;
	let retryTimer: ReturnType<typeof setTimeout> | null = null;
	let flushTimer: ReturnType<typeof setTimeout> | null = null;
	let themeObserver: MutationObserver | null = null;
	let logDecorations: import('monaco-editor').editor.IEditorDecorationsCollection | null = null;
	let colorEnabled = $state(false);
	let selectedTarget = $derived(
		logTargets.find((target) => target.container_name === selectedContainer) ?? null,
	);
	let targetItems = $derived<SearchableDropdownItem[]>(
		logTargets.map((target) => ({
			value: target.container_name,
			label: target.service ?? t('projects.application_volumes.application_container'),
			description: `${target.status} · ${target.container_name}`,
		})),
	);

	const STORAGE_KEY = 'litepod-log-color-enabled';

	function loadPreference() {
		try {
			const stored = localStorage.getItem(STORAGE_KEY);
			if (stored !== null) colorEnabled = stored === 'true';
		} catch {}
	}

	function savePreference() {
		try {
			localStorage.setItem(STORAGE_KEY, String(colorEnabled));
		} catch {}
	}

	function toggleColor() {
		colorEnabled = !colorEnabled;
		savePreference();
		applyDecorations();
	}

	function loadTimestampPref() {
		try {
			const s = localStorage.getItem(TS_STORAGE_KEY);
			if (s !== null) showTimestamps = s === 'true';
		} catch {}
	}
	function saveTimestampPref() {
		try {
			localStorage.setItem(TS_STORAGE_KEY, String(showTimestamps));
		} catch {}
	}

	function toggleTimestamps() {
		showTimestamps = !showTimestamps;
		saveTimestampPref();
		rebuildEditor();
	}

	function formatLogLines(lines: LogLine[]): string[] {
		return lines.map((line) =>
			showTimestamps && line.ts ? `${formatLogTimestamp(line.ts)} ${line.text}` : line.text,
		);
	}

	function replaceEditor(lines: string[]) {
		if (!editor) return;
		editor.setValue(lines.join('\n'));
		renderedLines = lines;
		applyDecorations();
	}

	function matchingSuffixLength(previous: string[], next: string[]) {
		const maximum = Math.min(previous.length, next.length);
		for (let length = maximum; length > 0; length--) {
			let matches = true;
			for (let index = 0; index < length; index++) {
				if (previous[previous.length - length + index] !== next[index]) {
					matches = false;
					break;
				}
			}
			if (matches) return length;
		}
		return 0;
	}

	function updateEditor(lines: string[]) {
		if (!editor || !monacoApi) return;
		const model = editor.getModel();
		if (!model) return;

		if (renderedLines.length === 0 || lines.length === 0) {
			replaceEditor(lines);
			return;
		}

		const overlap = matchingSuffixLength(renderedLines, lines);
		if (overlap === 0) {
			replaceEditor(lines);
			return;
		}

		const dropped = renderedLines.length - overlap;
		const appended = lines.slice(overlap);
		const edits: import('monaco-editor').editor.IIdentifiedSingleEditOperation[] = [];
		if (dropped > 0) {
			edits.push({
				range: new monacoApi.Range(1, 1, dropped + 1, 1),
				text: '',
				forceMoveMarkers: true,
			});
		}
		if (appended.length > 0) {
			const lastLine = model.getLineCount();
			edits.push({
				range: new monacoApi.Range(
					lastLine,
					model.getLineMaxColumn(lastLine),
					lastLine,
					model.getLineMaxColumn(lastLine),
				),
				text: `\n${appended.join('\n')}`,
				forceMoveMarkers: true,
			});
		}
		if (edits.length > 0) model.applyEdits(edits);
		renderedLines = lines;
		applyDecorations();
	}

	function rebuildEditor() {
		if (!editor || !monacoApi) return;
		const model = editor.getModel();
		if (!model) return;
		if (allLines.length > tailLines) {
			allLines = allLines.slice(-tailLines);
		}
		replaceEditor(formatLogLines(allLines));
		editor.revealLine(model.getLineCount());
	}

	onMount(async () => {
		loadPreference();
		loadTimestampPref();
		const [, monaco] = await Promise.all([loadNerdFont(), loadMonaco()]);
		monacoApi = monaco;
		const applyTheme = () =>
			monacoApi?.editor.setTheme(
				document.documentElement.classList.contains('dark') ? 'vs-dark' : 'vs',
			);
		applyTheme();

		themeObserver = new MutationObserver(applyTheme);
		themeObserver.observe(document.documentElement, {
			attributes: true,
			attributeFilter: ['class'],
		});

		if (editorHost && monacoApi) {
			editor = monacoApi.editor.create(editorHost, {
				value: '',
				language: 'plaintext',
				minimap: { enabled: false },
				automaticLayout: true,
				theme: document.documentElement.classList.contains('dark') ? 'vs-dark' : 'vs',
				readOnly: true,
				wordWrap: 'on',
				lineNumbers: 'on',
				scrollBeyondLastLine: false,
				fontFamily: "'Commit Mono', 'CommitMono Nerd Font Mono', monospace",
				fontLigatures: "'ss01', 'ss02'",
				ariaLabel: t('projects.application_logs_tab.viewer_aria'),
			});
			logDecorations = editor.createDecorationsCollection();
		}

		await loadLogTargets();
	});

	onDestroy(() => {
		themeObserver?.disconnect();
		logDecorations?.clear();
		editor?.dispose();
		disconnect();
	});

	function clearLogs() {
		buffer = [];
		allLines = [];
		renderedLines = [];
		editor?.setValue('');
	}

	async function loadLogTargets() {
		loadingTargets = true;
		try {
			logTargets = await listApplicationLogTargets(
				organizationId,
				projectId,
				environmentId,
				applicationId,
			);
			const currentTargetStillExists =
				selectedContainer &&
				logTargets.some((target) => target.container_name === selectedContainer);
			selectedContainer = currentTargetStillExists
				? selectedContainer
				: (logTargets[0]?.container_name ?? null);
			if (selectedContainer) await openTarget(selectedContainer, true, false);
		} catch {
			logTargets = [];
			selectedContainer = null;
			status = 'error';
		} finally {
			loadingTargets = false;
		}
	}

	function connect() {
		const container = selectedContainer;
		const target = selectedTarget;
		if (!container || !target || !isRunning(target.status)) {
			status = 'disconnected';
			return;
		}
		disconnect();
		status = 'disconnected';
		retryCount = 0;

		const es = streamDeploymentLogs(
			organizationId,
			projectId,
			environmentId,
			applicationId,
			tailLines,
			container,
		);
		eventSource = es;

		es.onopen = () => {
			status = 'connected';
			retryCount = 0;
		};

		es.onmessage = (event: MessageEvent) => {
			try {
				const parsed = JSON.parse(event.data);
				buffer.push({ text: parsed.text, ts: parsed.ts ?? null });
			} catch {
				buffer.push({ text: event.data, ts: null });
			}
			if (buffer.length > MAX_VISIBLE_LINES) buffer = buffer.slice(-MAX_VISIBLE_LINES);
			scheduleFlush();
		};

		es.onerror = () => {
			if (eventSource !== es) return;
			disconnect();
			if (retryCount < maxRetries) {
				status = 'error';
				retryCount++;
				retryTimer = setTimeout(connect, 2000 * retryCount);
			} else {
				status = 'error';
			}
		};
	}

	async function openTarget(container: string, follow: boolean, reset = true) {
		const generation = ++targetGeneration;
		disconnect();
		status = 'disconnected';
		if (reset) clearLogs();
		try {
			const result = await fetchDeploymentLogs(
				organizationId,
				projectId,
				environmentId,
				applicationId,
				tailLines,
				container,
			);
			if (generation !== targetGeneration || container !== selectedContainer) return;
			allLines = result.lines
				.slice(-Math.min(tailLines, MAX_VISIBLE_LINES))
				.map((line) => ({ text: line.text, ts: line.ts ?? null }));
			buffer = [];
			replaceEditor(formatLogLines(allLines));
			if (follow && selectedTarget && isRunning(selectedTarget.status)) connect();
		} catch {
			if (generation === targetGeneration) status = 'error';
		}
	}

	function selectTarget(container: string) {
		if (container === selectedContainer) return;
		selectedContainer = container;
		void openTarget(container, true);
	}

	function disconnect() {
		eventSource?.close();
		eventSource = null;
		if (retryTimer) {
			clearTimeout(retryTimer);
			retryTimer = null;
		}
	}

	function scheduleFlush() {
		if (flushTimer) return;
		flushTimer = setTimeout(flush, 100);
	}

	function isAtBottom() {
		if (!editor) return true;
		const model = editor.getModel();
		const visibleRanges = editor.getVisibleRanges();
		if (!model || visibleRanges.length === 0) return true;
		const lastVisibleLine = visibleRanges[visibleRanges.length - 1].endLineNumber;
		return lastVisibleLine >= model.getLineCount() - 2;
	}

	function flush() {
		if (buffer.length === 0 || !editor || !monacoApi) {
			flushTimer = null;
			return;
		}
		const model = editor.getModel();
		if (!model) {
			flushTimer = null;
			return;
		}

		allLines.push(...buffer);
		buffer = [];
		flushTimer = null;

		// Keep last N lines rolling window
		const visibleLimit = Math.min(tailLines, MAX_VISIBLE_LINES);
		if (allLines.length > visibleLimit) {
			allLines = allLines.slice(-visibleLimit);
		}

		const wasAtBottom = isAtBottom();
		updateEditor(formatLogLines(allLines));
		if (wasAtBottom) editor.revealLine(model.getLineCount());
	}

	function applyDecorations() {
		if (!editor || !monacoApi || !colorEnabled) {
			logDecorations?.clear();
			return;
		}
		const model = editor.getModel();
		if (!model) return;
		const lineCount = model.getLineCount();
		const decorations = [];
		for (let i = 1; i <= lineCount; i++) {
			const line = model.getLineContent(i);
			const tone = deploymentLogTone('info', line);
			if (tone !== 'neutral') {
				decorations.push(deploymentLogDecoration(monacoApi!, i, tone));
			}
		}
		logDecorations?.set(decorations);
	}

	function pause() {
		disconnect();
		status = 'paused';
		flush();
	}

	function resume() {
		connect();
	}

	function fetchLogs() {
		if (selectedContainer) void openTarget(selectedContainer, true, false);
	}

	function copyLogs() {
		if (!editor) return;
		const text = editor.getValue();
		if (!text) return;
		navigator.clipboard
			.writeText(text)
			.then(() => toast.success(t('projects.dialogs_common.logs_copied')))
			.catch(() => toast.error(t('projects.dialogs_common.copy_logs_failed')));
	}

	function onTailChange(event: Event) {
		const value = parseInt((event.target as HTMLInputElement).value, 10);
		if (!isNaN(value) && value >= 10 && value <= MAX_VISIBLE_LINES) {
			tailLines = value;
			allLines = [];
			buffer = [];
			renderedLines = [];
			if (editor) editor.setValue('');
			if (selectedContainer) void openTarget(selectedContainer, status !== 'paused');
		}
	}
</script>

<div class="grid gap-4 max-w-full">
	<div class="flex flex-wrap items-center gap-2">
		<SearchableDropdown
			items={targetItems}
			selectedValue={selectedContainer}
			label={loadingTargets
				? t('projects.application_logs_tab.loading_containers')
				: t('projects.dialogs_common.select_container')}
			searchPlaceholder={t('projects.dialogs_common.search_containers')}
			emptyText={t('projects.application_logs_tab.no_containers')}
			disabled={loadingTargets || targetItems.length === 0}
			onSelect={selectTarget} />
		<Button
			variant="outline"
			size="sm"
			onclick={status === 'paused' ? resume : pause}
			disabled={!selectedTarget || (status === 'error' && retryCount >= maxRetries)}>
			{#if status === 'paused'}
				<PlayIcon class="h-4 w-4" /> {t('projects.application_logs_tab.resume')}
			{:else}
				<PauseIcon class="h-4 w-4" /> {t('projects.application_logs_tab.pause')}
			{/if}
		</Button>
		<Button variant="outline" size="sm" onclick={fetchLogs} disabled={!selectedTarget}>
			<RefreshCwIcon class="h-4 w-4" />
			{t('projects.application_logs_tab.fetch')}
		</Button>
		{#if onAskAssistant}
			<Button
				variant="outline"
				size="sm"
				onclick={() => {
					const text = editor?.getValue();
					if (text) onAskAssistant(text);
				}}>
				<SparklesIcon class="h-4 w-4" />
				{t('projects.dialogs_common.ask_ai')}
			</Button>
		{/if}
		<Button variant="outline" size="sm" onclick={copyLogs}>
			<CopyIcon class="h-4 w-4" />
			{t('projects.application_compose.copy')}
		</Button>
		<Button variant={colorEnabled ? 'default' : 'outline'} size="sm" onclick={toggleColor}>
			<PaletteIcon class="h-4 w-4" />
			{colorEnabled
				? t('projects.application_logs_tab.color_on')
				: t('projects.application_logs_tab.color_off')}
		</Button>
		<div class="flex items-center gap-1.5">
			<SoftCheckbox
				checked={showTimestamps}
				onCheckedChange={(c) => {
					showTimestamps = c ?? false;
					saveTimestampPref();
					rebuildEditor();
				}} />
			<Label.Root
				class="text-xs cursor-pointer"
				onclick={() => {
					showTimestamps = !showTimestamps;
					saveTimestampPref();
					rebuildEditor();
				}}>{t('projects.dialogs_common.timestamps')}</Label.Root>
		</div>
		<input
			type="number"
			value={tailLines}
			min="10"
			max={MAX_VISIBLE_LINES}
			class="w-20 rounded-md border bg-background px-2 py-1 text-sm"
			onchange={onTailChange}
			aria-label={t('projects.application_logs_tab.lines_aria')} />
		<span class="text-xs text-muted-foreground">{t('projects.application_logs_tab.lines')}</span>
		<div class="ml-auto flex items-center gap-1.5">
			<span
				class="inline-block h-2 w-2 rounded-full"
				class:bg-success={status === 'connected'}
				class:bg-warning={status === 'paused'}
				class:bg-destructive={status === 'error'}
				class:bg-muted-foreground={status === 'disconnected'}></span>
			<span class="text-xs text-muted-foreground capitalize">{status}</span>
		</div>
	</div>
	<div class="relative min-h-96 overflow-hidden rounded-lg border bg-white dark:bg-[#1e1e1e]">
		<div bind:this={editorHost} class="absolute inset-0"></div>
		{#if loadingTargets || !selectedTarget}
			<div
				class="absolute inset-0 flex items-center justify-center bg-background/95 text-sm text-muted-foreground">
				{loadingTargets
					? t('projects.application_logs_tab.loading_app_containers')
					: t('projects.application_logs_tab.no_containers_for_app')}
			</div>
		{/if}
	</div>
</div>
