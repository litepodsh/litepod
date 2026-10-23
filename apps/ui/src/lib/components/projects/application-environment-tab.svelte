<script module lang="ts">
	type Monaco = typeof import('monaco-editor');

	const SCRAMBLE_COMMAND = 'litepod.scrambleSecret';
	const SCRAMBLE_FRAMES = 16;
	const SCRAMBLE_FRAME_MS = 40;

	let dotenvLanguageRegistered = false;
	// Module-scoped: the command below is registered once for the whole app, so the
	// animation it drives must not close over one component instance.
	let scrambleTimer: ReturnType<typeof setTimeout> | null = null;

	function stopScramble() {
		if (scrambleTimer) clearTimeout(scrambleTimer);
		scrambleTimer = null;
	}

	/**
	 * Decodes the freshly inserted secret in place: the characters churn through the
	 * marker's own alphabet and settle left to right. Frames go through `applyEdits`,
	 * which bypasses the undo stack, so one Ctrl+Z still undoes the whole insertion.
	 */
	function scrambleReveal(
		monaco: Monaco,
		uri: string,
		line: number,
		column: number,
		marker: string,
		value: string,
	) {
		stopScramble();
		const model = monaco.editor.getModel(monaco.Uri.parse(uri));
		if (!model) return;
		const range = {
			startLineNumber: line,
			startColumn: column,
			endLineNumber: line,
			endColumn: column + value.length,
		};
		if (model.getValueInRange(range) !== value) return;
		if (window.matchMedia?.('(prefers-reduced-motion: reduce)').matches) return;

		const alphabet = markerAlphabet(marker);
		const noise = (count: number) =>
			Array.from(
				crypto.getRandomValues(new Uint8Array(count)),
				(byte) => alphabet[byte % alphabet.length],
			).join('');

		const step = (frame: number) => {
			if (model.isDisposed()) {
				stopScramble();
				return;
			}
			// Someone edited inside the secret mid-animation. Land on the real value
			// immediately rather than leaving scrambled noise behind as their password.
			if (model.getValueInRange(range).length !== value.length) {
				model.applyEdits([{ range, text: value }]);
				stopScramble();
				return;
			}
			const revealed = Math.ceil((value.length * frame) / SCRAMBLE_FRAMES);
			model.applyEdits([
				{ range, text: value.slice(0, revealed) + noise(value.length - revealed) },
			]);
			if (frame >= SCRAMBLE_FRAMES) {
				stopScramble();
				return;
			}
			scrambleTimer = setTimeout(() => step(frame + 1), SCRAMBLE_FRAME_MS);
		};
		step(0);
	}
</script>

<script lang="ts">
	import { onDestroy, onMount } from 'svelte';
	import { loadMonaco } from '#lib/utils/load-monaco.js';
	import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
	import DeleteConfirmControl from '#lib/components/custom/delete-confirm-control.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Popover from '#lib/components/ui/popover/index.js';
	import {
		generateSecret,
		markerAlphabet,
		markerStart,
		SECRET_MARKER_DOCS,
		SECRET_MARKER_PRESETS,
	} from '#lib/utils/env-secret-markers.js';
	import { DOMAIN_MARKER_DOCS, domainMarkerPresets } from '#lib/utils/env-domain-markers.js';
	import type { DomainMarkersDto } from '#lib/clients/project-client.js';
	import LinkIcon from '@lucide/svelte/icons/link';
	import { loadNerdFont } from '#lib/utils/lazy-nerd-font.js';
	import EyeIcon from '@lucide/svelte/icons/eye';
	import InfoIcon from '@lucide/svelte/icons/info';
	import EyeOffIcon from '@lucide/svelte/icons/eye-off';
	import PencilIcon from '@lucide/svelte/icons/pencil';
	import CheckIcon from '@lucide/svelte/icons/check';
	import SegmentedSwitch from '#lib/components/custom/segmented-switch.svelte';
	import EnvFileView, { parseEnv } from './env-file-view.svelte';
	import { t } from '#lib/i18n/index.svelte.js';

	type Scope = 'runtime' | 'build';
	type Editor = import('monaco-editor').editor.IStandaloneCodeEditor;

	let {
		runtimeContent = '',
		buildContent = '',
		busy = false,
		onSave,
		onReveal,
		domainMarkers = null,
	}: {
		runtimeContent?: string;
		buildContent?: string;
		busy?: boolean;
		/** `{{url}}`-style markers in the runtime scope and their current addresses. */
		domainMarkers?: DomainMarkersDto | null;
		onSave: (scope: Scope, value: string) => Promise<void>;
		onReveal: (scope: Scope) => Promise<string>;
	} = $props();
	let element = $state<HTMLDivElement | null>(null);
	let editor = $state<Editor | null>(null);
	let fallback = $state('');
	let themeObserver: MutationObserver | null = null;
	let changeListener: { dispose: () => void } | null = null;
	let scope = $state<Scope>('runtime');
	let editing = $state(false);
	let revealed = $state(false);
	let confirmDiscardOpen = $state(false);
	let pendingScope = $state<Scope | null>(null);
	let saving = $state(false);
	let revealing = $state(false);
	let revealRequest = 0;
	let savedRuntimeContent = $state('');
	let savedBuildContent = $state('');
	let currentContent = $derived(scope === 'runtime' ? savedRuntimeContent : savedBuildContent);
	let operationBusy = $derived(busy || saving || revealing);
	let view = $state<'file' | 'table'>('file');
	let runtimeVars = $derived(parseEnv(savedRuntimeContent).vars);
	let buildVars = $derived(parseEnv(savedBuildContent).vars);
	let currentVars = $derived(scope === 'runtime' ? runtimeVars : buildVars);
	// Only runtime values are resolved against domains.
	let linkedDomains = $derived(
		scope === 'runtime'
			? (domainMarkers?.markers ?? []).flatMap((item) =>
					[item.marker, item.resolved].filter((value): value is string => !!value),
				)
			: [],
	);
	let lineErrors = $derived(editing ? parseEnv(fallback).errors : []);
	let dirty = $derived(editing && fallback !== currentContent);
	let decorations: ReturnType<Editor['createDecorationsCollection']> | null = null;

	function maskEnvironmentValues(value: string) {
		return value
			.split('\n')
			.map((line) => {
				const match = line.match(/^(\s*[A-Za-z_][A-Za-z0-9_]*\s*=)(.*)$/);
				if (!match) return line;
				const [, key, rawValue] = match;
				return `${key}${rawValue.replace(/\S/g, '•')}`;
			})
			.join('\n');
	}

	function editorValue() {
		return editor?.getValue() ?? fallback;
	}

	function setEditorValue(value: string) {
		fallback = value;
		if (editor && editor.getValue() !== value) editor.setValue(value);
	}

	function syncPresentation() {
		const value = editing || revealed ? currentContent : maskEnvironmentValues(currentContent);
		setEditorValue(value);
		editor?.updateOptions({ readOnly: !editing });
	}

	onMount(async () => {
		fallback = maskEnvironmentValues(runtimeContent);
		const [, monaco] = await Promise.all([loadNerdFont(), loadMonaco()]);
		if (!dotenvLanguageRegistered) {
			monaco.languages.register({ id: 'litepod-dotenv', extensions: ['.env'] });
			monaco.languages.setMonarchTokensProvider('litepod-dotenv', {
				tokenizer: {
					root: [
						[/^\s*#.*/, 'comment'],
						[/^\s*([A-Za-z_][A-Za-z0-9_]*)(\s*)(=)/, ['variable', '', 'delimiter']],
						[/"(?:[^"\\]|\\.)*"/, 'string'],
						[/'(?:[^'\\]|\\.)*'/, 'string'],
						[/\s+#.*/, 'comment'],
					],
				},
			});
			dotenvLanguageRegistered = true;

			monaco.editor.registerCommand(
				SCRAMBLE_COMMAND,
				(_accessor, uri: string, line: number, column: number, marker: string, value: string) =>
					scrambleReveal(monaco, uri, line, column, marker, value),
			);
			monaco.languages.registerCompletionItemProvider('litepod-dotenv', {
				triggerCharacters: ['{', '_'],
				provideCompletionItems(model, position) {
					const before = model.getValueInRange({
						startLineNumber: position.lineNumber,
						startColumn: 1,
						endLineNumber: position.lineNumber,
						endColumn: position.column,
					});
					const opening = markerStart(before);
					if (opening === null) return { suggestions: [] };

					const range = {
						startLineNumber: position.lineNumber,
						startColumn: opening + 1,
						endLineNumber: position.lineNumber,
						endColumn: position.column,
					};
					// Whatever is half-typed may already name a valid marker with an arbitrary
					// length (`{{openssl_rand_48`), which the fixed preset list cannot cover.
					const typed = `${before.slice(opening)}}}`;
					const markers = SECRET_MARKER_PRESETS.some((preset) => preset.marker === typed)
						? SECRET_MARKER_PRESETS
						: [
								{ marker: typed, detail: t('projects.application_env_tab.generate_secret_detail') },
								...SECRET_MARKER_PRESETS,
							];

					const suggestions = markers.flatMap(({ marker, detail }) => {
						const value = generateSecret(marker);
						if (!value) return [];
						return [
							{
								label: marker,
								kind: monaco.languages.CompletionItemKind.Value,
								detail,
								documentation: t('projects.application_env_tab.generate_secret_doc'),
								insertText: value,
								filterText: marker,
								range,
								command: {
									id: SCRAMBLE_COMMAND,
									title: t('projects.application_env_tab.reveal_command_title'),
									arguments: [
										model.uri.toString(),
										position.lineNumber,
										opening + 1,
										marker,
										value,
									],
								},
							},
						];
					});
					// Domain markers are inserted as written: the API resolves them on every
					// deploy, so there is no value to mint here.
					const domainSuggestions = domainMarkerPresets([]).map((marker) => ({
						label: marker,
						kind: monaco.languages.CompletionItemKind.Reference,
						detail: t('projects.application_env_tab.domain_marker_detail'),
						documentation: t('projects.application_env_tab.domain_marker_doc'),
						insertText: marker,
						filterText: marker,
						range,
					}));
					return { suggestions: [...suggestions, ...domainSuggestions] };
				},
			});
		}
		const applyTheme = () => {
			const dark = document.documentElement.classList.contains('dark');
			const styles = getComputedStyle(document.documentElement);
			const token = (name: string) => styles.getPropertyValue(name).trim();
			const isHex = (value: string) => /^#[0-9a-f]{6}$/i.test(value);
			// Same recipe as the read-only `.env` view: `bg-muted/60` laid over the card.
			const mix = (top: string, bottom: string, amount: number) => {
				const channel = (hex: string, at: number) => parseInt(hex.slice(at, at + 2), 16);
				return (
					'#' +
					[1, 3, 5]
						.map((at) =>
							Math.round(channel(top, at) * amount + channel(bottom, at) * (1 - amount))
								.toString(16)
								.padStart(2, '0'),
						)
						.join('')
				);
			};
			const [surface, deep, ink, muted, info] = [
				'--lp-surface',
				'--lp-deep',
				'--lp-ink',
				'--lp-muted',
				'--lp-info',
			].map(token);
			const palette = [surface, deep, ink, muted, info].every(isHex);
			const well = palette ? mix(deep, surface, 0.6) : undefined;
			monaco.editor.defineTheme('litepod-environment', {
				base: dark ? 'vs-dark' : 'vs',
				inherit: true,
				rules: palette
					? [
						{ token: '', foreground: ink.slice(1) },
						{ token: 'variable', foreground: info.slice(1) },
						{ token: 'delimiter', foreground: muted.slice(1) },
						{ token: 'string', foreground: ink.slice(1) },
						{ token: 'comment', foreground: muted.slice(1), fontStyle: 'italic' },
					]
					: [],
				colors:
					palette && well
						? {
								'editor.background': well,
								'editor.foreground': ink,
								'editorGutter.background': well,
								'editorLineNumber.foreground': mix(muted, well, 0.7),
								'editorLineNumber.activeForeground': ink,
								'editor.lineHighlightBackground': mix(ink, well, 0.04),
								'editor.lineHighlightBorder': well,
								'editorCursor.foreground': ink,
								'editorIndentGuide.background1': well,
							}
						: {},
			});
			monaco.editor.setTheme('litepod-environment');
		};
		applyTheme();
		if (element) {
			editor = monaco.editor.create(element, {
				value: fallback,
				language: 'litepod-dotenv',
				minimap: { enabled: false },
				automaticLayout: true,
				theme: 'litepod-environment',
				fontFamily: "'Commit Mono', 'CommitMono Nerd Font Mono', monospace",
				fontLigatures: false,
				// Geometry of the read-only `.env` view (12.5px / 1.75) so toggling Edit
				// changes the cursor, not the layout.
				fontSize: 12.5,
				lineHeight: 22,
				padding: { top: 12, bottom: 12 },
				lineNumbersMinChars: 3,
				lineDecorationsWidth: 8,
				glyphMargin: false,
				folding: false,
				scrollBeyondLastLine: false,
				renderLineHighlight: 'none',
				overviewRulerLanes: 0,
				overviewRulerBorder: false,
				hideCursorInOverviewRuler: true,
				scrollbar: { useShadows: false, verticalScrollbarSize: 10, horizontalScrollbarSize: 10 },
				ariaLabel: t('projects.application_env_tab.editor_aria'),
				readOnly: true,
			});
			changeListener = editor.onDidChangeModelContent(() => {
				fallback = editor?.getValue() ?? fallback;
			});
			// An action, not `addCommand`: commands bind to the last created editor.
			editor.addAction({
				id: 'litepod.saveEnvironment',
				label: t('projects.application_env_tab.save_scope', { scope }),
				keybindings: [monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS],
				run: trySave,
			});
			decorations = editor.createDecorationsCollection();
			syncPresentation();
		}
		themeObserver = new MutationObserver(applyTheme);
		themeObserver.observe(document.documentElement, {
			attributes: true,
			attributeFilter: ['class', 'data-palette'],
		});
	});

	$effect(() => {
		if (!editor) return;
		decorations?.set(
			lineErrors.map((line) => ({
				range: { startLineNumber: line, startColumn: 1, endLineNumber: line, endColumn: 1 },
				options: {
					isWholeLine: true,
					className: 'env-line-error',
					lineNumberClassName: 'env-ln-error',
				},
			})),
		);
	});

	$effect(() => {
		savedRuntimeContent = runtimeContent;
	});
	$effect(() => {
		savedBuildContent = buildContent;
	});
	$effect(() => {
		if (!editing) syncPresentation();
	});

	onDestroy(() => {
		stopScramble();
		themeObserver?.disconnect();
		changeListener?.dispose();
		editor?.dispose();
	});

	function hasUnsavedEdits() {
		return editing && editorValue() !== currentContent;
	}

	function switchScope(nextScope: Scope) {
		if (operationBusy) return;
		if (nextScope === scope) return;
		if (hasUnsavedEdits()) {
			pendingScope = nextScope;
			confirmDiscardOpen = true;
			return;
		}
		editing = false;
		revealed = false;
		scope = nextScope;
	}

	async function loadRevealedContent(nextEditing: boolean) {
		const requestedScope = scope;
		const request = ++revealRequest;
		revealing = true;
		try {
			const content = await onReveal(requestedScope);
			if (request !== revealRequest || scope !== requestedScope) return;
			if (requestedScope === 'runtime') savedRuntimeContent = content;
			else savedBuildContent = content;
			revealed = true;
			editing = nextEditing;
			setEditorValue(content);
			editor?.updateOptions({ readOnly: !nextEditing });
		} catch {
			// The parent reports the error. Keep the last protected presentation intact.
		} finally {
			if (request === revealRequest) revealing = false;
		}
	}

	function hideValues() {
		revealRequest += 1;
		revealed = false;
		syncPresentation();
	}

	function cancelEditing() {
		editing = false;
		revealed = false;
		syncPresentation();
	}

	function confirmDiscard() {
		if (operationBusy) return;
		const nextScope = pendingScope;
		confirmDiscardOpen = false;
		pendingScope = null;
		if (!nextScope) return;
		editing = false;
		revealed = false;
		scope = nextScope;
	}

	function trySave() {
		if (editing && !operationBusy && lineErrors.length === 0) void save();
	}

	async function save() {
		const savedScope = scope;
		const value = editorValue();
		saving = true;
		try {
			await onSave(savedScope, value);
		} catch {
			return;
		} finally {
			saving = false;
		}
		if (savedScope === 'runtime') savedRuntimeContent = value;
		else savedBuildContent = value;
		if (scope !== savedScope) return;
		editing = false;
		revealed = false;
		syncPresentation();
	}
</script>

<div class="grid max-w-full gap-4">
	<div>
		<div class="flex items-center gap-1.5">
			<p class="font-medium">{t('projects.application_env_tab.title')}</p>
			<Popover.Root>
				<Popover.Trigger
					class="text-muted-foreground hover:text-foreground rounded-full transition-colors"
					aria-label={t('projects.application_env_tab.about_generated_secrets')}
					title={t('projects.application_env_tab.about_generated_secrets')}>
					<InfoIcon class="size-4" />
				</Popover.Trigger>
				<Popover.Content class="w-96 gap-3" align="start">
					<div class="grid gap-1">
						<p class="font-medium">{t('projects.application_env_tab.generate_secret')}</p>
						<p class="text-muted-foreground text-xs">
							{t('projects.application_env_tab.hint_1')} <code class="font-mono">&#123;&#123;</code>
							{t('projects.application_env_tab.hint_2')}
							<code class="font-mono">=</code>
							{t('projects.application_env_tab.hint_3')}
							<code class="font-mono">postgres://user:&#123;&#123;password_32&#125;&#125;@db</code>.
						</p>
					</div>
					<dl class="grid gap-1.5 text-xs">
						{#each SECRET_MARKER_DOCS as entry (entry.syntax)}
							<div class="grid gap-0.5">
								<dt class="font-mono text-foreground">{entry.syntax}</dt>
								<dd class="text-muted-foreground">{entry.detail}</dd>
							</div>
						{/each}
					</dl>
					<p class="text-muted-foreground text-xs">
						<span class="font-mono">N</span>
						{t('projects.application_env_tab.n_any_length')}
					</p>
					<div class="grid gap-1 border-t pt-3">
						<p class="font-medium">{t('projects.application_env_tab.domain_markers_title')}</p>
						<p class="text-muted-foreground text-xs">
							{t('projects.application_env_tab.domain_markers_hint')}
						</p>
						<dl class="grid gap-1.5 text-xs">
							{#each DOMAIN_MARKER_DOCS as entry (entry.syntax)}
								<div class="grid gap-0.5">
									<dt class="font-mono text-foreground">{entry.syntax}</dt>
									<dd class="text-muted-foreground">
										{t(`projects.application_env_tab.${entry.key}`)}
									</dd>
								</div>
							{/each}
						</dl>
					</div>
				</Popover.Content>
			</Popover.Root>
		</div>
	</div>
	<section class="bg-card lp-raised overflow-hidden rounded-xl border">
		<div class="lp-groove-b flex flex-wrap items-center justify-between gap-2 px-3.5 py-3">
			<div class="flex flex-wrap items-center gap-1.5 max-sm:w-full">
				<SegmentedSwitch
					label={t('projects.application_env_tab.scope_label')}
					class="w-60 max-sm:flex-1"
					items={[
						{
							value: 'runtime',
							label: `${t('projects.application_env_tab.runtime')} ${Object.keys(runtimeVars).length}`,
						},
						{
							value: 'build',
							label: `${t('projects.application_env_tab.build_time')} ${Object.keys(buildVars).length}`,
						},
					]}
					bind:value={() => scope, (next: Scope) => switchScope(next)} />
				{#if !editing}
					<SegmentedSwitch
						label={t('projects.application_env_tab.view_label')}
						class="w-36"
						items={[
							{ value: 'file', label: '.env' },
							{ value: 'table', label: t('projects.application_env_tab.table_view') },
						]}
						bind:value={view} />
				{/if}
			</div>
			<div class="flex flex-wrap gap-1.5 max-sm:w-full max-sm:*:flex-1">
				{#if editing}
					<Button size="sm" variant="outline" disabled={operationBusy} onclick={cancelEditing}
						>{t('projects.projects.cancel')}</Button>
					<Button size="sm" disabled={operationBusy || lineErrors.length > 0} onclick={trySave}>
						<CheckIcon class="size-4" />
						{t('projects.application_env_tab.save_scope', { scope })}
					</Button>
				{:else}
					<Button
						size="sm"
						variant="outline"
						disabled={operationBusy}
						onclick={() => (revealed ? hideValues() : loadRevealedContent(false))}
						aria-label={revealed
							? t('projects.application_env_tab.hide_scope_aria', { scope })
							: t('projects.application_env_tab.reveal_scope_aria', { scope })}>
						{#if revealed}<EyeOffIcon class="size-4" />{:else}<EyeIcon class="size-4" />{/if}
						{revealed
							? t('projects.application_env_tab.hide_values')
							: t('projects.application_env_tab.reveal_values')}
					</Button>
					<Button
						size="sm"
						disabled={operationBusy}
						onclick={() => loadRevealedContent(true)}
						aria-label={t('projects.application_env_tab.edit_scope_aria', { scope })}
						title={t('projects.application_env_tab.edit_title')}>
						<PencilIcon class="size-4" />
						{t('projects.common.edit')}
					</Button>
				{/if}
			</div>
		</div>
		{#if !editing}
			<EnvFileView vars={currentVars} reveal={revealed} domains={linkedDomains} {view} />
		{/if}
		<div class:hidden={!editing} class="lp-well relative min-h-80 bg-muted/60">
			<div bind:this={element} class="absolute inset-0"></div>
			{#if !editor}<textarea
					class="absolute inset-0 bg-muted p-3 font-mono text-sm"
					bind:value={fallback}
					readonly={!editing}
					onkeydown={(event) => {
						if ((event.metaKey || event.ctrlKey) && event.key === 's') {
							event.preventDefault();
							trySave();
						}
					}}
					aria-label={t('projects.application_env_tab.editor_aria')}></textarea
				>{/if}
		</div>
		<div
			class="lp-groove-t flex flex-wrap items-center justify-between gap-2.5 px-3.5 py-2.5 text-xs text-muted-foreground">
			<span aria-live="polite">
				{#if !editing}
					{t('projects.application_env_tab.runtime_build_hint')}
				{:else if lineErrors.length}
					<span class="font-medium text-destructive">
						{t('projects.application_env_tab.invalid_lines', { lines: lineErrors.join(', ') })}
					</span>
				{:else if dirty}
					<span class="inline-flex items-center gap-1.5 font-medium text-warning">
						<span class="size-1.5 rounded-full bg-current"></span>
						{t('projects.application_env_tab.unsaved_changes')}
					</span>
				{:else}
					{t('projects.application_env_tab.edit_hint')}
				{/if}
			</span>
			<span class="font-mono">
				{t('projects.application_env_tab.variables_count', {
					count: Object.keys(currentVars).length,
				})}
			</span>
		</div>
		{#if scope === 'runtime' && domainMarkers && domainMarkers.markers.length > 0}
			<div class="lp-groove-t grid gap-2 px-3.5 py-3">
				<p class="text-xs font-medium">{t('projects.application_env_tab.linked_domains_title')}</p>
				<ul class="grid gap-1.5 text-xs">
					{#each domainMarkers.markers as item, index (`${item.name}-${index}`)}
						<li class="flex flex-wrap items-center gap-x-2 gap-y-0.5">
							<span class="font-mono font-medium">{item.name}</span>
							<span class="font-mono text-muted-foreground">{item.marker}</span>
							{#if item.resolved}
								<LinkIcon class="size-3 text-muted-foreground" aria-hidden="true" />
								<span class="font-mono break-all text-success">{item.resolved}</span>
							{:else if item.error}
								<span class="text-destructive">{item.error}</span>
							{/if}
						</li>
					{/each}
				</ul>
				<p class="text-xs text-muted-foreground">
					{t('projects.application_env_tab.linked_domains_unbind')}
				</p>
			</div>
		{/if}
	</section>
</div>

<AlertDialog.Root
	open={confirmDiscardOpen}
	onOpenChange={(open) => {
		confirmDiscardOpen = open;
		if (!open) pendingScope = null;
	}}>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title>{t('projects.application_env_tab.discard_title')}</AlertDialog.Title>
			<AlertDialog.Description>
				{t('projects.application_env_tab.discard_description')}
			</AlertDialog.Description>
		</AlertDialog.Header>
		<AlertDialog.Footer>
			<AlertDialog.Cancel disabled={operationBusy}
				>{t('projects.application_env_tab.keep_editing')}</AlertDialog.Cancel>
			<DeleteConfirmControl
				label={t('projects.application_env_tab.discard_and_switch')}
				pendingLabel={t('projects.application_env_tab.switch_pending')}
				variant="primary"
				disabled={operationBusy}
				onconfirm={confirmDiscard} />
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>

<style>
	/* Monaco decoration classes live outside this component's markup. */
	:global(.env-line-error) {
		background: color-mix(in oklch, var(--destructive) 10%, transparent);
	}
	:global(.env-ln-error) {
		color: var(--destructive) !important;
		font-weight: 600;
	}
</style>
