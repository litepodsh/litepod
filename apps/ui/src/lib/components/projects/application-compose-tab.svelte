<script lang="ts">
	import { onDestroy, onMount } from 'svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import RocketIcon from '@lucide/svelte/icons/rocket';
	import CheckCircleIcon from '@lucide/svelte/icons/check-circle';
	import XCircleIcon from '@lucide/svelte/icons/x-circle';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import Trash2Icon from '@lucide/svelte/icons/trash-2';
	import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';
	import PencilIcon from '@lucide/svelte/icons/pencil';
	import Undo2Icon from '@lucide/svelte/icons/undo-2';
	import ExternalLinkIcon from '@lucide/svelte/icons/external-link';
	import AlignLeftIcon from '@lucide/svelte/icons/align-left';
	import * as prettier from 'prettier/standalone';
	import * as yamlPlugin from 'prettier/plugins/yaml';
	import { loadNerdFont } from '#lib/utils/lazy-nerd-font.js';
	import { loadMonaco } from '#lib/utils/load-monaco.js';
	import { IsMobile } from '#lib/hooks/is-mobile.svelte.js';
	import SearchableDropdown, {
		type SearchableDropdownItem,
	} from '#lib/components/custom/searchable-dropdown.svelte';
	import ComposeVariablesDialog from '#lib/components/projects/compose-variables-dialog.svelte';
	import CopyButton from '#lib/components/custom/copy-button.svelte';
	import {
		diffMissingVariables,
		extractComposeVariableRefs,
	} from '#lib/utils/compose-variables.js';
	import {
		extractHostBindMounts,
		extractUndeclaredVolumes,
		hasRootVolumesBlock,
	} from '#lib/utils/compose-volumes.js';
	import { hasDeprecatedComposeVersion } from '#lib/utils/compose-version.js';
	import { findReservedServices } from '#lib/utils/compose-reserved-services.js';
	import { deployLabel } from '#lib/utils/deploy-label.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';
	import * as Select from '#lib/components/ui/select/index.js';
	import type {
		ComposeContent,
		PortMapping,
		ValidationResponse,
	} from '#lib/clients/project-client.js';

	let {
		busy = false,
		composeContent = '',
		composePortMappings = [],
		runtimeVariableNames = [],
		onSave,
		onDeploy,
		onValidate,
		onContentChange,
		onCreateRuntimeVariables,
		readOnly = false,
		editWarning = null,
		showPortMappings = true,
		running = false,
		templateDocsUrl = null,
	}: {
		busy?: boolean;
		composeContent?: string;
		composePortMappings?: PortMapping[];
		runtimeVariableNames?: string[];
		onSave: (content: string, portMappings: PortMapping[]) => Promise<ComposeContent>;
		onDeploy: () => Promise<void>;
		onValidate: (content: string) => Promise<ValidationResponse>;
		onContentChange: (content: string) => void;
		onCreateRuntimeVariables: (created: { name: string; value: string }[]) => Promise<void>;
		/** Start as a highlighted, non-editable Compose view. */
		readOnly?: boolean;
		/** Non-blocking guidance shown while a read-only caller is editing. */
		editWarning?: string | null;
		/** Hide generic host-port controls when another UI owns ports. */
		showPortMappings?: boolean;
		/** Live containers exist, so the deploy button recreates instead of creating. */
		running?: boolean;
		/** Catalog setup page, when the application was created from a template. */
		templateDocsUrl?: string | null;
	} = $props();

	let content = $state('');
	let portMappings = $state<PortMapping[]>([]);
	let validationResult = $state<ValidationResponse | null>(null);
	let validating = $state(false);
	let editorContainer = $state<HTMLDivElement | null>(null);
	let editor = $state<import('monaco-editor').editor.IStandaloneCodeEditor | null>(null);
	let monacoModule = $state<typeof import('monaco-editor') | null>(null);
	let formatDialogOpen = $state(false);
	let formatPreview = $state<string | null>(null);
	let formatOriginal = $state('');
	let diffContainer = $state<HTMLDivElement | null>(null);
	let themeObserver: MutationObserver | null = null;
	const isMobile = new IsMobile();
	let variablesDialogOpen = $state(false);
	let editing = $state(true);
	let savedContent = $state('');
	let savedPortMappings = $state<PortMapping[]>([]);

	const VARIABLE_MARKER_OWNER = 'litepod-compose-vars';
	const VOLUME_MARKER_OWNER = 'litepod-compose-volumes';
	const VERSION_MARKER_OWNER = 'litepod-compose-version';
	const RESERVED_MARKER_OWNER = 'litepod-compose-reserved-services';

	let missingVariables = $derived(
		diffMissingVariables(extractComposeVariableRefs(content), new Set(runtimeVariableNames)),
	);
	let hostBindMounts = $derived(extractHostBindMounts(content));
	let undeclaredVolumes = $derived(extractUndeclaredVolumes(content));
	let hasVolumesBlock = $derived(hasRootVolumesBlock(content));
	let deprecatedVersion = $derived(hasDeprecatedComposeVersion(content));
	let reservedServices = $derived(findReservedServices(content));
	// Swapping a bind for a named volume is only half the fix — the name also has
	// to be declared at the root, and the block may not exist yet.
	let volumeDeclaration = $derived.by(() => {
		const names = [
			...new Set([
				...hostBindMounts.map((mount) => mount.suggestion),
				...undeclaredVolumes.map((volume) => volume.name),
			]),
		];
		if (names.length === 0) return '';
		const entries = names.map((name) => `  ${name}:`).join('\n');
		return hasVolumesBlock ? entries : `volumes:\n${entries}`;
	});
	let creatableVariables = $derived(missingVariables.filter((variable) => variable.creatable));

	let availableServices = $derived.by(() => {
		const lines = (content || '').split('\n');
		const services: string[] = [];
		let inServices = false;
		for (const line of lines) {
			if (line.trim().startsWith('services:')) {
				inServices = true;
				continue;
			}
			if (inServices) {
				const match = line.trim().match(/^([\w-]+):/);
				if (match && line.startsWith('  ')) services.push(match[1]);
				else if (line.trim() !== '' && !line.startsWith(' ')) inServices = false;
			}
		}
		return services;
	});
	let serviceItems = $derived<SearchableDropdownItem[]>(
		availableServices.map((service) => ({
			value: service,
			label: service,
			description: t('projects.volume.compose_service'),
		})),
	);

	onMount(async () => {
		editing = !readOnly;
		content = composeContent;
		portMappings = composePortMappings;
		savedContent = composeContent;
		savedPortMappings = composePortMappings;
		const [, monaco] = await Promise.all([loadNerdFont(), loadMonaco()]);
		monacoModule = monaco;
		const applyTheme = () => {
			const dark = document.documentElement.classList.contains('dark');
			monaco.editor.defineTheme('litepod-compose', {
				base: dark ? 'vs-dark' : 'vs',
				inherit: true,
				rules: [],
				colors: {},
			});
			monaco.editor.setTheme('litepod-compose');
		};
		applyTheme();
		if (editorContainer) {
			editor = monaco.editor.create(editorContainer, {
				value: composeContent,
				language: 'yaml',
				minimap: { enabled: !isMobile.current },
				automaticLayout: true,
				theme: 'litepod-compose',
				lineNumbers: isMobile.current ? 'off' : 'on',
				wordWrap: isMobile.current ? 'on' : 'off',
				scrollBeyondLastLine: false,
				fontSize: 13,
				tabSize: 2,
				fontFamily: "'Commit Mono', 'CommitMono Nerd Font Mono', monospace",
				fontLigatures: false,
				ariaLabel: t('projects.application_compose.compose_editor_aria'),
				readOnly: !editing,
			});
			editor.onDidChangeModelContent(() => {
				if (!editing) return;
				content = editor!.getValue();
				onContentChange(content);
			});
		}
		themeObserver = new MutationObserver(applyTheme);
		themeObserver.observe(document.documentElement, {
			attributes: true,
			attributeFilter: ['class'],
		});
	});

	onDestroy(() => {
		themeObserver?.disconnect();
		const model = editor?.getModel();
		if (monacoModule && model) {
			monacoModule.editor.setModelMarkers(model, VARIABLE_MARKER_OWNER, []);
			monacoModule.editor.setModelMarkers(model, VOLUME_MARKER_OWNER, []);
			monacoModule.editor.setModelMarkers(model, VERSION_MARKER_OWNER, []);
		}
		editor?.dispose();
	});

	$effect(() => {
		if (editor && editor.getValue() !== composeContent) {
			editor.setValue(composeContent);
		}
	});

	$effect(() => {
		editor?.updateOptions({ readOnly: !editing });
	});

	$effect(() => {
		if (!formatDialogOpen || !formatPreview || !diffContainer || !monacoModule) return;
		const original = monacoModule.editor.createModel(formatOriginal, 'yaml');
		const modified = monacoModule.editor.createModel(formatPreview, 'yaml');
		const diffEditor = monacoModule.editor.createDiffEditor(diffContainer, {
			automaticLayout: true,
			readOnly: true,
			originalEditable: false,
			renderSideBySide: !isMobile.current,
			ignoreTrimWhitespace: false,
			renderIndicators: true,
			renderOverviewRuler: true,
			fontSize: 13,
			fontFamily: "'Commit Mono', 'CommitMono Nerd Font Mono', monospace",
			scrollBeyondLastLine: false,
		});
		diffEditor.setModel({ original, modified });
		return () => {
			diffEditor.dispose();
			original.dispose();
			modified.dispose();
		};
	});

	// Narrow screens: drop line numbers + minimap for code width, wrap long lines.
	$effect(() => {
		if (!editor) return;
		editor.updateOptions({
			lineNumbers: isMobile.current ? 'off' : 'on',
			wordWrap: isMobile.current ? 'on' : 'off',
			minimap: { enabled: !isMobile.current },
		});
		editor.layout();
	});

	// Squiggle every `${VAR}` the runtime scope does not define. Podman would
	// substitute a blank string for these at deploy time without complaining.
	$effect(() => {
		const model = editor?.getModel();
		if (!monacoModule || !model) return;
		monacoModule.editor.setModelMarkers(
			model,
			VARIABLE_MARKER_OWNER,
			missingVariables.map((variable) => ({
				severity: monacoModule!.MarkerSeverity.Warning,
				message: variable.creatable
					? t('projects.application_compose.runtime_variable_missing', { name: variable.name })
					: t('projects.application_compose.runtime_variable_missing_uppercase', {
							name: variable.name,
						}),
				startLineNumber: variable.line,
				startColumn: variable.column,
				endLineNumber: variable.line,
				endColumn: variable.column + variable.length,
			})),
		);
	});

	// litepod keeps Compose projects inside the `litepod_data` named volume, so a
	// host bind source never exists on the host Podman resolves it against.
	$effect(() => {
		const model = editor?.getModel();
		if (!monacoModule || !model) return;
		monacoModule.editor.setModelMarkers(model, VOLUME_MARKER_OWNER, [
			...hostBindMounts.map((mount) => ({
				severity: monacoModule!.MarkerSeverity.Warning,
				message: t('projects.application_compose.host_bind_marker', {
					source: mount.source,
					suggestion: mount.suggestion,
					target: mount.target || '/data',
					note: hasVolumesBlock ? '' : ', which this file does not have yet',
					declaration: hasVolumesBlock
						? `  ${mount.suggestion}:`
						: `volumes:\n  ${mount.suggestion}:`,
				}),
				startLineNumber: mount.line,
				startColumn: mount.column,
				endLineNumber: mount.line,
				endColumn: mount.column + mount.length,
			})),
			...undeclaredVolumes.map((volume) => ({
				severity: monacoModule!.MarkerSeverity.Warning,
				message: t('projects.application_compose.named_volume_not_declared', { name: volume.name }),
				startLineNumber: volume.line,
				startColumn: volume.column,
				endLineNumber: volume.line,
				endColumn: volume.column + volume.length,
			})),
		]);
	});

	// Squiggle the deprecated `version:` key — Compose V2+ ignores it and Podman warns.
	$effect(() => {
		const model = editor?.getModel();
		if (!monacoModule || !model) return;
		monacoModule.editor.setModelMarkers(
			model,
			VERSION_MARKER_OWNER,
			deprecatedVersion.found
				? [
						{
							severity: monacoModule!.MarkerSeverity.Error,
							message: t('projects.application_compose.version_marker_message'),
							startLineNumber: deprecatedVersion.line,
							startColumn: deprecatedVersion.column,
							endLineNumber: deprecatedVersion.line,
							endColumn: deprecatedVersion.column + deprecatedVersion.length,
						},
					]
				: [],
		);
	});

	// Squiggle service names reserved by litepod's own containers on the shared network.
	$effect(() => {
		const model = editor?.getModel();
		if (!monacoModule || !model) return;
		monacoModule.editor.setModelMarkers(
			model,
			RESERVED_MARKER_OWNER,
			reservedServices.map((service) => ({
				severity: monacoModule!.MarkerSeverity.Error,
				message: t('projects.application_compose.reserved_service_marker', { name: service.name }),
				startLineNumber: service.line,
				startColumn: service.column,
				endLineNumber: service.line,
				endColumn: service.column + service.length,
			})),
		);
	});

	function addPortMapping() {
		const service = availableServices[0] || '';
		portMappings = [...portMappings, { service, host_port: 0, container_port: 0, protocol: 'tcp' }];
	}

	async function formatCompose() {
		try {
			const formatted = await prettier.format(content, {
				parser: 'yaml',
				plugins: [yamlPlugin],
			});
			if (formatted === content) {
				toast.info(t('projects.application_compose.already_formatted'));
				return;
			}
			formatOriginal = content;
			formatPreview = formatted;
			formatDialogOpen = true;
		} catch (error) {
			toast.error(error instanceof Error ? error : t('projects.application_compose.format_failed'));
		}
	}

	function applyFormat() {
		const activeEditor = editor;
		const model = activeEditor?.getModel();
		if (!activeEditor || !model || !formatPreview) return;
		activeEditor.pushUndoStop();
		activeEditor.executeEdits('format-compose', [
			{ range: model.getFullModelRange(), text: formatPreview, forceMoveMarkers: true },
		]);
		activeEditor.pushUndoStop();
		formatDialogOpen = false;
	}

	function removePortMapping(index: number) {
		portMappings = portMappings.filter((_, i) => i !== index);
	}

	async function handleValidate() {
		validating = true;
		try {
			validationResult = await onValidate(content);
		} finally {
			validating = false;
		}
	}

	async function handleSave() {
		const saved = await onSave(
			content,
			portMappings.filter((m) => m.service && m.host_port > 0 && m.container_port > 0),
		);
		content = saved.content;
		portMappings = saved.port_mappings;
		savedContent = saved.content;
		savedPortMappings = saved.port_mappings;
		if (readOnly) editing = false;
	}

	function discard() {
		content = savedContent;
		portMappings = savedPortMappings;
		onContentChange(content);
		validationResult = null;
		editing = false;
	}

	async function handleDeploy() {
		await handleSave();
		await onDeploy();
	}
</script>

<div class="grid min-w-0 gap-4 [&>*]:min-w-0">
	<div class="flex flex-wrap items-center justify-between gap-2">
		<p class="font-medium">{t('projects.application_compose.compose_file')}</p>
		<div class="flex flex-wrap gap-2">
			{#if templateDocsUrl}
				<Button variant="outline" size="sm" href={templateDocsUrl} target="_blank" rel="noreferrer">
					<ExternalLinkIcon class="size-4" />
					{t('projects.application_compose.template_docs')}
				</Button>
			{/if}
			<CopyButton
				value={content}
				label={t('projects.application_compose.copy')}
				copiedLabel={t('projects.application_compose.copied')}
				revealOnHover={false}
				disabled={!content.trim()}
				errorMessage={t('projects.application_compose.copy_compose_failed')}
				class="h-8 flex-row-reverse gap-2 rounded-md border px-3 text-sm font-medium hover:bg-accent hover:text-accent-foreground"
				iconClass="size-4" />
			{#if !editing}
				<Button variant="outline" size="sm" disabled={busy} onclick={() => (editing = true)}>
					<PencilIcon class="size-4" />
					{t('projects.application_compose.edit_compose')}
				</Button>
			{:else}
				<Button
					variant="outline"
					size="sm"
					disabled={busy || !content.trim()}
					onclick={formatCompose}>
					<AlignLeftIcon class="size-4" />
					{t('projects.application_compose.format')}
				</Button>
				<Button variant="outline" size="sm" disabled={busy || validating} onclick={handleValidate}>
					{validating
						? t('projects.application_compose.validating')
						: t('projects.application_compose.validate')}
				</Button>
				{#if readOnly}
					<Button variant="outline" size="sm" disabled={busy} onclick={discard}>
						<Undo2Icon class="size-4" />
						{t('projects.application_compose.discard')}
					</Button>
				{/if}
				<Button
					variant="outline"
					size="sm"
					disabled={busy || reservedServices.length > 0}
					onclick={handleSave}>{t('projects.application_compose.save')}</Button>
				<Button
					size="sm"
					disabled={busy ||
						!content.trim() ||
						deprecatedVersion.found ||
						reservedServices.length > 0}
					onclick={handleDeploy}>
					<RocketIcon class="size-4" />
					{deployLabel(running)}
				</Button>
			{/if}
		</div>
	</div>

	{#if editing && editWarning}
		<div
			class="flex items-start gap-2 rounded-lg border border-warning/30 bg-warning/5 p-3 text-sm">
			<TriangleAlertIcon class="mt-0.5 size-4 shrink-0 text-warning" />
			<p class="text-muted-foreground">{editWarning}</p>
		</div>
	{/if}

	{#if reservedServices.length > 0}
		<div
			class="flex items-start gap-2 rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-sm">
			<TriangleAlertIcon class="mt-0.5 size-4 shrink-0 text-destructive" />
			<div>
				<p class="font-medium text-destructive">
					{reservedServices.length === 1
						? t('projects.application_compose.reserved_one')
						: t('projects.application_compose.reserved_many')}
					{reservedServices.map((service) => `"${service.name}"`).join(', ')}
					{reservedServices.length === 1
						? t('projects.application_compose.reserved_is')
						: t('projects.application_compose.reserved_are')}
					{t('projects.application_compose.reserved_by')}
				</p>
				<p class="mt-1 text-xs text-muted-foreground">
					{t('projects.application_compose.reserved_hint_1')}
					<code class="font-mono">web</code>
					{t('projects.application_compose.reserved_hint_2')}
					<code class="font-mono">app</code>
					{t('projects.application_compose.reserved_hint_3')}
				</p>
			</div>
		</div>
	{/if}

	{#if deprecatedVersion.found}
		<div
			class="flex items-start gap-2 rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-sm">
			<TriangleAlertIcon class="mt-0.5 size-4 shrink-0 text-destructive" />
			<div>
				<p class="font-medium text-destructive">
					{t('projects.application_compose.version_obsolete_prefix')}
					<code class="font-mono">version</code>
					{t('projects.application_compose.version_obsolete_suffix')}
				</p>
				<p class="mt-1 text-xs text-muted-foreground">
					{t('projects.application_compose.version_delete_prefix')}
					<code class="font-mono">version: {deprecatedVersion.version}</code>
					{t('projects.application_compose.version_delete_suffix')}
				</p>
			</div>
		</div>
	{/if}

	{#if missingVariables.length > 0}
		<div
			class="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-warning/30 bg-warning/5 p-3 text-sm">
			<div class="flex items-start gap-2">
				<TriangleAlertIcon class="mt-0.5 size-4 shrink-0 text-warning" />
				<div>
					<p class="font-medium text-warning">
						{missingVariables.length}
						{missingVariables.length === 1
							? t('projects.application_compose.variable_is')
							: t('projects.application_compose.variables_are')}
						{t('projects.application_compose.not_defined_in_env')}
					</p>
					<p class="mt-1 font-mono text-xs text-muted-foreground">
						{missingVariables.map((variable) => variable.name).join(', ')}
					</p>
				</div>
			</div>
			<Button
				variant="outline"
				size="sm"
				disabled={busy || creatableVariables.length === 0}
				onclick={() => (variablesDialogOpen = true)}>
				{t('projects.application_compose.create_ellipsis')}
			</Button>
		</div>
	{/if}

	{#if hostBindMounts.length > 0 || undeclaredVolumes.length > 0}
		{@const steps = hostBindMounts.length > 0 ? ['1. ', '2. '] : ['', '']}
		<div
			class="flex flex-wrap items-start gap-2 rounded-lg border border-warning/30 bg-warning/5 p-3 text-sm">
			<TriangleAlertIcon class="mt-0.5 size-4 shrink-0 text-warning" />
			<div>
				<p class="font-medium text-warning">
					{#if hostBindMounts.length > 0}
						{hostBindMounts.length === 1
							? t('projects.application_compose.bind_one')
							: t('projects.application_compose.bind_many', { count: hostBindMounts.length })}
						{t('projects.application_compose.bind_fail')}
					{:else}
						{undeclaredVolumes.length === 1
							? t('projects.application_compose.undeclared_one')
							: t('projects.application_compose.undeclared_many', {
									count: undeclaredVolumes.length,
								})}
						{t('projects.application_compose.undeclared_fail')}
					{/if}
				</p>

				{#if hostBindMounts.length > 0}
					<p class="mt-1 text-xs text-muted-foreground">
						{t('projects.application_compose.podman_cannot_resolve')}
					</p>
					<p class="mt-2 text-xs font-medium">
						{steps[0]}{t('projects.application_compose.replace_mounts')}
					</p>
					<ul class="mt-1 space-y-0.5 font-mono text-xs text-muted-foreground">
						{#each hostBindMounts as mount (`${mount.line}:${mount.column}`)}
							{@const replacement = `${mount.suggestion}:${mount.target || '/data'}`}
							<li class="flex items-center gap-1">
								<span class="line-through">{mount.source}:{mount.target}</span>
								<span>→</span>
								<CopyButton
									value={replacement}
									errorMessage={t('projects.application_compose.copy_replacement_failed')}
									class="rounded px-1 py-0.5 font-mono text-foreground hover:bg-warning/20" />
							</li>
						{/each}
					</ul>
				{/if}

				<p class="mt-2 text-xs font-medium">
					{steps[1]}{hasVolumesBlock
						? t('projects.application_compose.declare_volumes_key')
						: t('projects.application_compose.add_volumes_section')}
				</p>
				<CopyButton
					value={volumeDeclaration}
					errorMessage={t('projects.application_compose.copy_declaration_failed')}
					class="mt-1 items-start rounded px-1 py-0.5 hover:bg-warning/20">
					<pre
						class="max-w-full overflow-x-auto font-mono text-xs whitespace-pre text-foreground">{volumeDeclaration}</pre>
				</CopyButton>
			</div>
		</div>
	{/if}

	<div
		class:cursor-not-allowed={!editing}
		class:opacity-60={!editing}
		class="relative min-h-[36rem] w-full min-w-0 overflow-hidden rounded-lg border bg-white transition-opacity dark:bg-[#1e1e1e]"
		aria-disabled={!editing}>
		<div bind:this={editorContainer} class:pointer-events-none={!editing} class="absolute inset-0">
		</div>
		{#if !editor}
			<textarea
				class="absolute inset-0 bg-muted p-3 font-mono text-sm disabled:cursor-not-allowed"
				bind:value={content}
				readonly={!editing}
				disabled={!editing}
				oninput={() => editing && onContentChange(content)}
				aria-label={t('projects.application_compose.compose_editor_aria')}></textarea>
		{/if}
	</div>

	{#if validationResult}
		<div
			class="rounded-lg border p-3 {validationResult.valid
				? 'border-success/30 bg-success/5'
				: 'border-destructive/30 bg-destructive/5'}">
			<div class="flex items-center gap-2 text-sm">
				{#if validationResult.valid}
					<CheckCircleIcon class="size-4 text-success" />
					<span class="text-success">{t('projects.application_compose.valid_compose')}</span>
				{:else}
					<XCircleIcon class="size-4 text-destructive" />
					<span class="font-medium text-destructive"
						>{t('projects.application_compose.validation_errors')}</span>
				{/if}
			</div>
			{#if validationResult.errors.length > 0}
				<ul class="mt-2 space-y-1 text-sm text-destructive">
					{#each validationResult.errors as error}
						<li class="font-mono text-xs">
							{error.line
								? t('projects.application_compose.line_number', { line: error.line })
								: ''}{error.column ? `:${error.column}` : ''}
							{error.message}
						</li>
					{/each}
				</ul>
			{/if}
		</div>
	{/if}

	{#if showPortMappings}
		<section class="rounded-xl border p-5">
			<div class="mb-4 rounded-lg border border-info/30 bg-info/5 p-3 text-sm">
				<p class="font-medium text-info">{t('projects.application_compose.caddy_title')}</p>
				<p class="mt-1 text-muted-foreground">
					{t('projects.application_compose.caddy_hint')}
				</p>
			</div>
			<div class="flex flex-wrap items-center justify-between gap-3">
				<div>
					<p class="font-medium">{t('projects.application_compose.direct_title')}</p>
					<p class="text-xs text-muted-foreground">
						{t('projects.application_compose.direct_hint')}
					</p>
				</div>
				<Button
					variant="outline"
					size="sm"
					disabled={availableServices.length === 0}
					onclick={addPortMapping}>
					<PlusIcon class="size-4" />
					{t('projects.application_compose.add_port')}
				</Button>
			</div>
			{#if portMappings.length === 0}
				<p class="mt-3 text-sm text-muted-foreground">
					{t('projects.application_compose.no_port_mappings')}
				</p>
			{:else}
				<div class="mt-3 grid gap-2">
					{#each portMappings as mapping, i}
						<div class="flex items-end gap-2">
							<div class="grid flex-1 gap-1.5">
								<Label for="service-{i}">{t('projects.application_compose.service')}</Label>
								<SearchableDropdown
									items={serviceItems}
									selectedValue={mapping.service || null}
									label={t('projects.application_compose.select_service')}
									searchPlaceholder={t('projects.application_compose.search_services')}
									emptyText={t('projects.application_compose.no_service_found')}
									disabled={busy || serviceItems.length === 0}
									onSelect={(service) =>
										(portMappings = portMappings.map((m, idx) =>
											idx === i ? { ...m, service } : m,
										))} />
							</div>
							<div class="grid w-28 gap-1.5">
								<Label for="host-port-{i}">{t('projects.application_compose.host_port')}</Label>
								<Input
									id="host-port-{i}"
									type="number"
									min="1"
									max="65535"
									value={mapping.host_port || ''}
									placeholder="8080"
									oninput={(e) => {
										const val = (e.target as HTMLInputElement).value;
										portMappings = portMappings.map((m, idx) =>
											idx === i ? { ...m, host_port: parseInt(val) || 0 } : m,
										);
									}}
									disabled={busy} />
							</div>
							<div class="grid w-28 gap-1.5">
								<Label for="container-port-{i}"
									>{t('projects.application_compose.container_port')}</Label>
								<Input
									id="container-port-{i}"
									type="number"
									min="1"
									max="65535"
									value={mapping.container_port || ''}
									placeholder="80"
									oninput={(e) => {
										const val = (e.target as HTMLInputElement).value;
										portMappings = portMappings.map((m, idx) =>
											idx === i ? { ...m, container_port: parseInt(val) || 0 } : m,
										);
									}}
									disabled={busy} />
							</div>
							<div class="grid w-24 gap-1.5">
								<Label for="protocol-{i}">{t('projects.application_compose.protocol')}</Label>
								<Select.Root
									type="single"
									value={mapping.protocol}
									disabled={busy}
									onValueChange={(value) =>
										(portMappings = portMappings.map((m, idx) =>
											idx === i ? { ...m, protocol: value === 'udp' ? 'udp' : 'tcp' } : m,
										))}>
									<Select.Trigger id="protocol-{i}" class="w-full uppercase"
										>{mapping.protocol}</Select.Trigger>
									<Select.Content>
										<Select.Item value="tcp" label="TCP" />
										<Select.Item value="udp" label="UDP" />
									</Select.Content>
								</Select.Root>
							</div>
							<Button
								variant="ghost"
								size="icon"
								class="mb-0.5"
								disabled={busy}
								onclick={() => removePortMapping(i)}>
								<Trash2Icon class="size-4 text-destructive" />
							</Button>
						</div>
					{/each}
				</div>
			{/if}
		</section>
	{/if}
</div>

<Dialog.Root bind:open={formatDialogOpen}>
	<Dialog.Content class="max-w-6xl sm:max-w-6xl">
		<Dialog.Header>
			<Dialog.Title>{t('projects.application_compose.review_formatting')}</Dialog.Title>
			<Dialog.Description
				>{t('projects.application_compose.review_formatting_hint')}</Dialog.Description>
		</Dialog.Header>
		<div bind:this={diffContainer} class="h-[min(65vh,44rem)] overflow-hidden rounded-md border">
		</div>
		<Dialog.Footer>
			<Button variant="outline" onclick={() => (formatDialogOpen = false)}
				>{t('projects.projects.cancel')}</Button>
			<Button onclick={applyFormat}>{t('projects.application_compose.apply_formatting')}</Button>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>

<ComposeVariablesDialog
	bind:open={variablesDialogOpen}
	variables={missingVariables}
	onCreate={onCreateRuntimeVariables} />
