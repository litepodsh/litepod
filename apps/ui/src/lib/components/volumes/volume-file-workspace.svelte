<script lang="ts">
	import { onDestroy, onMount, tick } from 'svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import type {
		WorkspaceClient,
		WorkspaceEntry,
		WorkspaceFile,
	} from '#lib/clients/volume-workspace-client.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';
	import { loadNerdFont } from '#lib/utils/lazy-nerd-font.js';
	import { loadMonaco } from '#lib/utils/load-monaco.js';
	import DownloadIcon from '@lucide/svelte/icons/download';
	import FolderPlusIcon from '@lucide/svelte/icons/folder-plus';
	import FilePlusIcon from '@lucide/svelte/icons/file-plus';
	import SaveIcon from '@lucide/svelte/icons/save';
	import UploadIcon from '@lucide/svelte/icons/upload';
	import WandSparklesIcon from '@lucide/svelte/icons/wand-sparkles';
	import LoaderCircleIcon from '@lucide/svelte/icons/loader-circle';

	let {
		title,
		client,
		canWrite = true,
	}: { title: string; client: WorkspaceClient; canWrite?: boolean } = $props();
	let entries = $state<WorkspaceEntry[]>([]);
	let listLogs = $state<string[]>([]);
	let selected = $state<WorkspaceFile | null>(null);
	let loading = $state(true);
	let fileLoading = $state(false);
	let pendingPath = $state<string | null>(null);
	let saving = $state(false);
	let formatting = $state(false);
	let search = $state('');
	let selectionVersion = 0;
	let treeHost = $state<HTMLDivElement | null>(null);
	let editorHost = $state<HTMLDivElement | null>(null);
	let editor = $state<import('monaco-editor').editor.IStandaloneCodeEditor | null>(null);
	let monacoApi = $state<typeof import('monaco-editor') | null>(null);
	let tree: {
		cleanUp: () => void;
		resetPaths: (paths: string[]) => void;
		render: (options: { containerWrapper: HTMLElement }) => void;
	} | null = null;
	let themeObserver: MutationObserver | null = null;

	function language(path: string) {
		const ext = path.split('.').pop()?.toLowerCase();
		return (
			(
				{
					html: 'html',
					css: 'css',
					js: 'javascript',
					mjs: 'javascript',
					ts: 'typescript',
					json: 'json',
					md: 'markdown',
					yml: 'yaml',
					yaml: 'yaml',
					rs: 'rust',
					toml: 'ini',
					sh: 'shell',
				} as Record<string, string>
			)[ext ?? ''] ?? 'plaintext'
		);
	}
	function normalizedPath(path: string) {
		return path.replace(/^\.\//, '').replace(/\/$/, '');
	}
	function pathsForTree() {
		const query = search.trim().toLowerCase();
		const matches = query
			? entries.filter((entry) => normalizedPath(entry.path).toLowerCase().includes(query))
			: entries;
		const paths = new Set<string>();
		for (const entry of matches) {
			const path = normalizedPath(entry.path);
			paths.add(entry.kind === 'directory' ? `${path}/` : path);
			const parts = path.split('/');
			for (let index = 1; index < parts.length; index += 1)
				paths.add(`${parts.slice(0, index).join('/')}/`);
		}
		return [...paths];
	}
	function updateTree() {
		tree?.resetPaths(pathsForTree());
	}
	function mergeEntries(nextEntries: WorkspaceEntry[]) {
		const merged = new Map(entries.map((entry) => [normalizedPath(entry.path), entry]));
		for (const entry of nextEntries) merged.set(normalizedPath(entry.path), entry);
		entries = [...merged.values()].sort((left, right) => left.path.localeCompare(right.path));
	}
	async function refresh(path?: string) {
		loading = true;
		try {
			const listing = await client.list(path);
			listLogs = listing.logs;
			if (path) mergeEntries(listing.entries);
			else entries = listing.entries;
			updateTree();
		} catch (error) {
			toast.error(error instanceof Error ? error.message : t('projects.volume_files.list_failed'));
		} finally {
			loading = false;
		}
	}
	async function select(path: string) {
		const entry = entries.find((item) => normalizedPath(item.path) === normalizedPath(path));
		if (!entry) return;
		if (entry.kind === 'directory') {
			await refresh(entry.path);
			return;
		}
		const version = ++selectionVersion;
		pendingPath = entry.path;
		fileLoading = true;
		try {
			const file = await client.read(entry.path);
			if (version !== selectionVersion) return;
			selected = file;
			if (file.is_text && editor) editor.setValue(file.content ?? '');
			await tick();
			if (version === selectionVersion && file.is_text) await formatEditor(file.path, false);
		} catch (error) {
			if (version === selectionVersion)
				toast.error(
					error instanceof Error ? error.message : t('projects.volume_files.open_failed'),
				);
		} finally {
			if (version === selectionVersion) {
				pendingPath = null;
				fileLoading = false;
			}
		}
	}
	onMount(async () => {
		await refresh();
		const { FileTree } = await import('@pierre/trees');
		if (treeHost) {
			tree = new FileTree({
				paths: pathsForTree(),
				initialExpansion: 'open',
				flattenEmptyDirectories: false,
				icons: 'standard',
				onSelectionChange: (paths) => void select(String(paths[0] ?? '')),
			});
			tree.render({ containerWrapper: treeHost });
		}
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
	});
	$effect(() => {
		if (selected?.is_text && editorHost && monacoApi && !editor)
			editor = monacoApi.editor.create(editorHost, {
				value: selected.content ?? '',
				language: language(selected.path),
				minimap: { enabled: false },
				bracketPairColorization: { enabled: true },
				guides: {
					bracketPairs: true,
					bracketPairsHorizontal: true,
					highlightActiveBracketPair: true,
				},
				automaticLayout: true,
				fontFamily: "'Commit Mono', 'CommitMono Nerd Font Mono', monospace",
				fontLigatures: false,
				ariaLabel: t('projects.volume_files.editor_aria'),
				readOnly: !canWrite,
			});
		if (selected?.is_text && editor && monacoApi) {
			editor.setValue(selected.content ?? '');
			const model = editor.getModel();
			if (model) monacoApi.editor.setModelLanguage(model, language(selected.path));
		}
	});
	onDestroy(() => {
		tree?.cleanUp();
		editor?.dispose();
		themeObserver?.disconnect();
	});
	async function save() {
		if (!selected?.is_text || !editor) return;
		saving = true;
		try {
			selected = await client.save(selected.path, editor.getValue());
			toast.success(t('projects.volume_files.saved_toast', { path: selected.path }));
		} catch (error) {
			toast.error(error instanceof Error ? error.message : t('projects.volume_files.save_failed'));
		} finally {
			saving = false;
		}
	}
	async function formatEditor(path: string, announce: boolean) {
		if (!editor) return;
		const extension = path.split('.').pop()?.toLowerCase();
		const parser = (
			{
				json: 'json',
				jsonc: 'json',
				js: 'babel',
				mjs: 'babel',
				cjs: 'babel',
				ts: 'typescript',
				tsx: 'typescript',
				html: 'html',
				htm: 'html',
				svg: 'html',
				css: 'css',
				scss: 'css',
				less: 'css',
				md: 'markdown',
				mdx: 'mdx',
				yml: 'yaml',
				yaml: 'yaml',
			} as Record<string, string>
		)[extension ?? ''];
		if (!parser) {
			if (announce) toast.error(t('projects.volume_files.format_unavailable'));
			return;
		}
		formatting = true;
		try {
			const source = editor.getValue();
			const prettier = await import('prettier/standalone');
			const [babel, estree, html, markdown, postcss, typescript, yaml] = await Promise.all([
				import('prettier/plugins/babel'),
				import('prettier/plugins/estree'),
				import('prettier/plugins/html'),
				import('prettier/plugins/markdown'),
				import('prettier/plugins/postcss'),
				import('prettier/plugins/typescript'),
				import('prettier/plugins/yaml'),
			]);
			const formatted = await prettier.format(source, {
				parser,
				plugins: [
					babel.default,
					estree.default,
					html.default,
					markdown.default,
					postcss.default,
					typescript.default,
					yaml.default,
				],
			});
			if (selected?.path === path) editor.setValue(formatted);
			if (announce) toast.success(t('projects.volume_files.formatted_toast'));
		} catch (error) {
			if (announce)
				toast.error(
					error instanceof Error ? error.message : t('projects.volume_files.format_failed'),
				);
		} finally {
			formatting = false;
		}
	}
	async function format() {
		if (selected?.is_text) await formatEditor(selected.path, true);
	}
	async function create(kind: 'file' | 'directory') {
		const path = prompt(
			kind === 'file'
				? t('projects.volume_files.prompt_new_file')
				: t('projects.volume_files.prompt_new_folder'),
		);
		if (!path) return;
		try {
			await client.create(path, kind, kind === 'file' ? '' : undefined);
			await refresh();
			toast.success(t('projects.volume_files.created_toast', { path }));
		} catch (error) {
			toast.error(
				error instanceof Error ? error.message : t('projects.volume_files.create_failed'),
			);
		}
	}
	async function upload(event: Event) {
		const file = (event.currentTarget as HTMLInputElement).files?.[0];
		if (!file) return;
		const path = prompt(t('projects.volume_files.prompt_upload'), file.name);
		if (!path) return;
		try {
			await client.upload(path, file);
			await refresh();
			toast.success(t('projects.volume_files.uploaded_toast', { path }));
		} catch (error) {
			toast.error(
				error instanceof Error ? error.message : t('projects.volume_files.upload_failed'),
			);
		}
	}
</script>

<section
	class="grid h-full min-h-[24rem] overflow-hidden rounded-2xl border bg-card lg:grid-cols-[17rem_minmax(0,1fr)]">
	<aside class="flex min-h-0 flex-col border-b bg-muted/20 p-3 lg:border-e lg:border-b-0">
		<div class="mb-3 flex items-center justify-between">
			<div>
				<p class="font-medium">{title}</p>
				<p class="text-xs text-muted-foreground">
					{t('projects.volume_files.entries_count', { count: entries.length })}
				</p>
			</div>
			<Button
				size="icon-sm"
				variant="ghost"
				onclick={() => refresh()}
				disabled={loading}
				aria-label={t('projects.volume_files.refresh_tree_aria')}
				>{#if loading}<LoaderCircleIcon class="animate-spin" />{:else}↻{/if}</Button>
		</div>
		<Input
			bind:value={search}
			oninput={updateTree}
			placeholder={t('projects.volume_files.search_files_placeholder')}
			aria-label={t('projects.volume_files.search_files_aria')}
			class="mb-3" />
		<div
			bind:this={treeHost}
			class="min-h-[16rem] flex-1 overflow-auto [--trees-bg-override:var(--background)] [--trees-fg-override:var(--foreground)] [--trees-fg-muted-override:var(--muted-foreground)] [--trees-border-color-override:var(--border)] [--trees-bg-muted-override:var(--muted)] [--trees-selected-bg-override:var(--accent)] [--trees-selected-fg-override:var(--accent-foreground)] [--trees-input-bg-override:var(--input)] [--trees-focus-ring-color-override:var(--ring)]">
		</div>
		{#if listLogs.length}<p class="mt-2 text-[11px] leading-4 text-muted-foreground">
				{listLogs.join(' · ')}
			</p>{/if}
	</aside>
	<div class="flex min-h-0 min-w-0 flex-col">
		<div class="flex flex-wrap items-center justify-between gap-2 border-b p-3">
			<div class="min-w-0">
				<p class="truncate font-mono text-sm">
					{pendingPath ?? selected?.path ?? t('projects.volume_files.select_file')}
				</p>
				<p class="text-xs text-muted-foreground">
					{#if fileLoading}{t('projects.volume_files.loading_file')}{:else if selected}{t(
							'projects.volume_files.bytes',
							{ count: selected.byte_size },
						)} · {selected.is_text
							? t('projects.volume_files.text_type')
							: t('projects.volume_files.binary_type')}{:else}{t(
							'projects.volume_files.browse_tree',
						)}{/if}
				</p>
			</div>
			{#if canWrite}<div class="flex gap-1">
					<Button size="sm" variant="outline" onclick={() => create('file')}
						><FilePlusIcon />{t('projects.volume_files.new_file')}</Button
					><Button size="sm" variant="outline" onclick={() => create('directory')}
						><FolderPlusIcon />{t('projects.volume_files.folder')}</Button
					><label
						><Button size="sm" variant="outline"
							><UploadIcon />{t('projects.volume_files.upload')}</Button
						><Input type="file" class="sr-only" onchange={upload} /></label>
				</div>{/if}
		</div>
		{#if selected?.is_text}<div class="relative min-h-[16rem] flex-1">
				<div bind:this={editorHost} class="absolute inset-0 bg-white dark:bg-[#1e1e1e]"></div>
				{#if fileLoading}<div
						class="absolute inset-0 flex items-center justify-center gap-2 bg-background/65 text-sm text-muted-foreground backdrop-blur-sm">
						<LoaderCircleIcon class="size-4 animate-spin" />{t(
							'projects.volume_files.loading_file',
						)}
					</div>{/if}
			</div>
			<div class="sticky bottom-0 z-10 flex justify-end gap-2 border-t bg-card p-3">
				<Button variant="outline" onclick={format} disabled={!canWrite || formatting || fileLoading}
					><WandSparklesIcon />{formatting
						? t('projects.volume_files.formatting')
						: t('projects.volume_files.format')}</Button
				><Button onclick={save} disabled={!canWrite || saving || fileLoading}
					><SaveIcon />{saving
						? t('projects.volume_files.saving')
						: t('projects.volume_files.save')}</Button>
			</div>{:else if fileLoading}<div
				class="flex flex-1 items-center justify-center gap-2 p-8 text-muted-foreground">
				<LoaderCircleIcon class="size-4 animate-spin" />{t('projects.volume_files.loading_file')}
			</div>{:else if selected}<div
				class="flex flex-1 flex-col items-center justify-center gap-3 p-8 text-center">
				<p class="font-medium">{t('projects.volume_files.binary_file')}</p>
				<p class="text-sm text-muted-foreground">{t('projects.volume_files.binary_not_opened')}</p>
				<Button href={client.downloadUrl(selected.path)}
					><DownloadIcon />{t('projects.volume_files.download')}</Button>
			</div>{:else}<div class="flex flex-1 items-center justify-center p-8 text-muted-foreground">
				{t('projects.volume_files.choose_file_hint')}
			</div>{/if}
	</div>
</section>
