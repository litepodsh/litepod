<script lang="ts">
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import * as Select from '#lib/components/ui/select/index.js';
	import { Textarea } from '#lib/components/ui/textarea/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import { AnimatedTabs } from '#lib/components/custom/animated-tabs/index.js';
	import SegmentedSwitch from '#lib/components/custom/segmented-switch.svelte';
	import ReadOnlyCodeEditor from '#lib/components/read-only-code-editor.svelte';
	import type { ApplicationDto, ServerSummaryDto } from '#lib/clients/project-client.js';
	import { listServerGroups, type ServerGroupSummary } from '#lib/clients/server-group-client.js';
	import {
		catalogTemplateDocsUrl,
		getCatalogTemplate,
		listCatalogTemplates,
		type CatalogTemplate,
	} from '#lib/clients/template-client.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';
	import { tabIndexFromKeydown } from '#lib/utils/tab-shortcut.js';
	import IconUploadButton from '#lib/components/icon-upload-button.svelte';
	import { marked } from 'marked';
	import DOMPurify from 'dompurify';
	import ArrowLeftIcon from '@lucide/svelte/icons/arrow-left';
	import BookOpenIcon from '@lucide/svelte/icons/book-open';
	import BoxesIcon from '@lucide/svelte/icons/boxes';
	import ExternalLinkIcon from '@lucide/svelte/icons/external-link';
	import LoaderCircleIcon from '@lucide/svelte/icons/loader-circle';
	import PackageIcon from '@lucide/svelte/icons/package';
	import SearchIcon from '@lucide/svelte/icons/search';
	import ServerIcon from '@lucide/svelte/icons/server';

	type CatalogView = 'catalog' | 'detail';
	type FileTab = { value: string; label: string; language: string; content: string };
	type SetupChunk =
		| { kind: 'html'; content: string }
		| { kind: 'code'; code: string; language: string };

	let {
		open = $bindable(false),
		application = null,
		servers = [],
		organizationId = null,
		onSubmit,
		onCreateFromTemplate,
		onUploadIcon,
		initialCreateMode = 'blank',
	}: {
		open?: boolean;
		application?: ApplicationDto | null;
		servers?: ServerSummaryDto[];
		organizationId?: string | null;
		onSubmit: (input: {
			name: string;
			description: string;
			server_id?: string;
			server_group_id?: string;
			icon_url?: string | null;
		}) => Promise<void>;
		onCreateFromTemplate?: (templateId: string, serverId: string) => Promise<void>;
		/** Only ever called while editing an existing application. */
		onUploadIcon?: (file: File) => Promise<string>;
		initialCreateMode?: 'blank' | 'catalog';
	} = $props();

	let name = $state('');
	let description = $state('');
	let iconUrl = $state('');
	// Typing a URL updates `<img src>` on every keystroke, which fires a
	// browser request per character for a partial/invalid URL — the preview
	// swatch only follows `iconUrl` after a pause.
	let previewIconUrl = $state('');
	let serverId = $state('');
	const serverTriggerLabel = $derived.by(() => {
		const match = servers.find((server) => server.id === serverId);
		return match
			? `${match.name} · ${match.ingress_mode.replace('_', ' ')}`
			: t('projects.application_form.choose_server');
	});
	// A group target fans the app out to every member server, so the picker
	// toggles between the two rather than showing them in one list.
	let targetMode = $state<'server' | 'group'>('server');
	let groupId = $state('');
	let groups = $state<ServerGroupSummary[]>([]);
	const groupTriggerLabel = $derived(
		groups.find((group) => group.id === groupId)?.name ??
			t('projects.application_form.choose_group'),
	);
	const selectedGroup = $derived(groups.find((group) => group.id === groupId) ?? null);
	let saving = $state(false);
	let wasOpen = $state(false);
	let createMode = $state<'blank' | 'catalog'>('blank');
	let catalogView = $state<CatalogView>('catalog');
	let search = $state('');
	let searchInput = $state<HTMLElement | null>(null);
	let category = $state('');
	let templates = $state<CatalogTemplate[]>([]);
	let templatesLoading = $state(false);
	let templatesError = $state<string | null>(null);
	let catalogRequestId = 0;
	let selectedTemplate = $state<CatalogTemplate | null>(null);
	let detailLoading = $state(false);
	let detailError = $state<string | null>(null);
	let detailRequestId = 0;
	let activeFileId = $state('compose');

	const categories = $derived(
		[...new Set(templates.flatMap((template) => template.categories))].sort(),
	);
	const fileTabs = $derived.by<FileTab[]>(() => {
		if (!selectedTemplate) return [];
		const tabs: FileTab[] = [];
		if (selectedTemplate.compose)
			tabs.push({
				value: 'compose',
				label: selectedTemplate.compose.filename,
				language: 'yaml',
				content: selectedTemplate.compose.content,
			});
		if (selectedTemplate.env)
			tabs.push({
				value: 'env',
				label: selectedTemplate.env.filename,
				language: 'ini',
				content: selectedTemplate.env.content,
			});
		if (selectedTemplate.manifest)
			tabs.push({
				value: 'manifest',
				label: selectedTemplate.manifest.filename,
				language: 'ini',
				content: selectedTemplate.manifest.content,
			});
		return tabs;
	});
	const activeFile = $derived(
		fileTabs.find((file) => file.value === activeFileId) ?? fileTabs[0] ?? null,
	);

	function handleFileTabShortcut(event: KeyboardEvent) {
		if (!open || !fileTabs.length) return;
		const index = tabIndexFromKeydown(event, fileTabs.length);
		if (index === null) return;
		event.preventDefault();
		activeFileId = fileTabs[index].value;
	}

	function decodeHtmlEntities(s: string): string {
		return s
			.replace(/&lt;/g, '<')
			.replace(/&gt;/g, '>')
			.replace(/&quot;/g, '"')
			.replace(/&#39;/g, "'")
			.replace(/&amp;/g, '&');
	}

	/**
	 * Splits `marked`'s output so each `<pre><code>` block becomes its own
	 * code chunk and the surrounding prose stays a single HTML string per
	 * chunk. The single regex mirrors the reference renderer; `marked` always
	 * emits `<pre><code class="language-xxx">…</code></pre>` for fenced code.
	 */
	function splitCodeBlocks(html: string): SetupChunk[] {
		const chunks: SetupChunk[] = [];
		const re = /<pre><code(?:\s+class="language-([^"]*)")?>([\s\S]*?)<\/code><\/pre>/g;
		let cursor = 0;
		let match: RegExpExecArray | null;
		while ((match = re.exec(html)) !== null) {
			if (match.index > cursor) {
				chunks.push({ kind: 'html', content: html.slice(cursor, match.index) });
			}
			chunks.push({
				kind: 'code',
				code: decodeHtmlEntities(match[2]).replace(/\n+$/, ''),
				language: match[1] || 'plaintext',
			});
			cursor = match.index + match[0].length;
		}
		if (cursor < html.length) chunks.push({ kind: 'html', content: html.slice(cursor) });
		return chunks;
	}

	/**
	 * Sanitizes prose HTML before `{@html}`. `dompurify` needs a DOM and would
	 * throw without one, so there has to be a no-window branch.
	 *
	 * That branch escapes instead of passing the string through. Returning the
	 * raw HTML was harmless only because `ssr = false` in `routes/+layout.ts`
	 * means this never runs without a DOM — which makes it a landmine rather than
	 * a bug: the day anyone turns SSR on, remote catalog content would reach
	 * `{@html}` unsanitized. Failing closed costs nothing today and removes the
	 * dependency on a setting in an unrelated file.
	 */
	function sanitizeProse(html: string): string {
		if (!html) return html;
		if (typeof window === 'undefined') {
			return html.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
		}
		return DOMPurify.sanitize(html, {
			USE_PROFILES: { html: true },
			FORBID_TAGS: ['style', 'iframe', 'object', 'embed', 'form', 'input', 'button'],
			FORBID_ATTR: ['style', 'onerror', 'onload', 'onclick'],
		});
	}

	const setupChunks = $derived.by<SetupChunk[]>(() => {
		const md = selectedTemplate?.setup?.content;
		if (!md) return [];
		const html = marked.parse(md, { gfm: true, async: false }) as string;
		return splitCodeBlocks(html).map((chunk) =>
			chunk.kind === 'html' ? { kind: 'html', content: sanitizeProse(chunk.content) } : chunk,
		);
	});

	/**
	 * Maps a markdown fence language to a Monaco language id. Monaco ships
	 * `shell` (not `sh`/`bash`) and has no `toml` grammar, so we normalise the
	 * common aliases the templates catalog uses — otherwise Monaco silently
	 * falls back to plaintext and the snippet renders uncoloured.
	 */
	function monacoLanguageFor(mdLang: string): string {
		const lang = mdLang.toLowerCase();
		const aliases: Record<string, string> = {
			sh: 'shell',
			bash: 'shell',
			zsh: 'shell',
			fish: 'shell',
			shell: 'shell',
			console: 'shell',
			yml: 'yaml',
			yaml: 'yaml',
			js: 'javascript',
			ts: 'typescript',
			md: 'markdown',
			py: 'python',
			rb: 'ruby',
			toml: 'ini',
			dockerfile: 'dockerfile',
			env: 'ini',
			dotenv: 'ini',
		};
		return aliases[lang] ?? lang;
	}

	/**
	 * Exact Monaco height. At fontSize 14, each line is ~19px and the editor
	 * adds ~4px of internal padding; the outer wrapper contributes 2px of
	 * border. We pass the sum as an exact `height` (not minHeight) so a 1-line
	 * shell snippet renders a 1-line box, not a 32rem floor.
	 */
	function editorHeightFor(code: string): string {
		const lineHeight = 19;
		const padding = 6;
		const maxLines = 60;
		const lines = Math.min(maxLines, Math.max(1, code.split('\n').length));
		return `${lines * lineHeight + padding}px`;
	}

	const setupUrl = $derived(
		selectedTemplate
			? (selectedTemplate.ui_url ?? catalogTemplateDocsUrl(selectedTemplate.id))
			: null,
	);

	function resetCatalog() {
		catalogView = 'catalog';
		search = '';
		category = '';
		templates = [];
		templatesError = null;
		selectedTemplate = null;
		detailError = null;
		activeFileId = 'compose';
	}

	$effect(() => {
		if (open && !wasOpen) {
			name = application?.name ?? '';
			description = application?.description ?? '';
			iconUrl = application?.icon_url ?? '';
			previewIconUrl = iconUrl;
			serverId =
				application?.server?.id ??
				(servers.length === 1
					? servers[0].id
					: (servers.find((s) => s.name.toLowerCase().includes('local'))?.id ?? ''));
			targetMode = application?.server_group ? 'group' : 'server';
			groupId = application?.server_group?.id ?? '';
			createMode = initialCreateMode;
			resetCatalog();
		}
		wasOpen = open;
	});

	// Server groups are only needed for the create picker, but they are cheap to
	// fetch and let the toggle reflect the current membership on every open.
	$effect(() => {
		if (!open || !organizationId) return;
		void listServerGroups(organizationId)
			.then((items) => (groups = items))
			.catch(() => (groups = []));
	});

	$effect(() => {
		const value = iconUrl;
		const timer = window.setTimeout(() => {
			previewIconUrl = value;
		}, 400);
		return () => window.clearTimeout(timer);
	});

	$effect(() => {
		if (open && !application && createMode === 'catalog' && catalogView === 'catalog') {
			searchInput?.focus();
		}
	});

	$effect(() => {
		if (!open || application || createMode !== 'catalog' || catalogView !== 'catalog') return;
		const requestId = ++catalogRequestId;
		const catalogSearch = { q: search, category, limit: 24 };
		templatesLoading = true;
		templatesError = null;
		const timer = window.setTimeout(() => {
			void listCatalogTemplates(catalogSearch)
				.then((items) => {
					if (requestId === catalogRequestId) templates = items;
				})
				.catch((error) => {
					if (requestId === catalogRequestId)
						templatesError =
							error instanceof Error
								? error.message
								: t('projects.application_form.failed_load_templates');
				})
				.finally(() => {
					if (requestId === catalogRequestId) templatesLoading = false;
				});
		}, 300);
		return () => window.clearTimeout(timer);
	});

	function showBlankApplication() {
		createMode = 'blank';
	}

	function showCatalog() {
		createMode = 'catalog';
		resetCatalog();
	}

	function backToCatalog() {
		catalogView = 'catalog';
		selectedTemplate = null;
		detailError = null;
		detailLoading = false;
		activeFileId = 'compose';
	}

	async function openDetail(templateId: string) {
		const requestId = ++detailRequestId;
		catalogView = 'detail';
		selectedTemplate = null;
		detailError = null;
		detailLoading = true;
		activeFileId = 'compose';
		try {
			const template = await getCatalogTemplate(templateId);
			if (requestId === detailRequestId) selectedTemplate = template;
		} catch (error) {
			if (requestId === detailRequestId)
				detailError =
					error instanceof Error
						? error.message
						: t('projects.application_form.failed_load_template_details');
		} finally {
			if (requestId === detailRequestId) detailLoading = false;
		}
	}

	async function createFromTemplate() {
		if (!selectedTemplate || !onCreateFromTemplate) return;
		if (!serverId) {
			toast.error(t('projects.application_form.choose_server_first'));
			return;
		}
		saving = true;
		try {
			await onCreateFromTemplate(selectedTemplate.id, serverId);
			open = false;
		} catch (error) {
			toast.error(
				error instanceof Error
					? error.message
					: t('projects.application_form.failed_create_application'),
			);
		} finally {
			saving = false;
		}
	}

	async function submit() {
		const trimmedName = name.trim();
		if (!trimmedName) {
			toast.error(t('projects.application_form.name_required'));
			return;
		}
		saving = true;
		try {
			const target: { server_id?: string; server_group_id?: string } = {};
			if (!application) {
				if (targetMode === 'group') {
					if (!groupId) throw new Error(t('projects.application_form.choose_group'));
					target.server_group_id = groupId;
				} else {
					if (!serverId) throw new Error(t('projects.application_form.choose_server_first'));
					target.server_id = serverId;
				}
			}
			await onSubmit({
				name: trimmedName,
				description: description.trim(),
				...target,
				...(application ? { icon_url: iconUrl.trim() || null } : {}),
			});
			open = false;
		} catch (error) {
			toast.error(
				error instanceof Error
					? error.message
					: t('projects.application_form.failed_save_application'),
			);
		} finally {
			saving = false;
		}
	}
</script>

<svelte:window onkeydown={handleFileTabShortcut} />

<Dialog.Root bind:open>
	<Dialog.Content
		onOpenAutoFocus={(event) => {
			if (!application && createMode === 'catalog' && catalogView === 'catalog') {
				event.preventDefault();
				searchInput?.focus();
			}
		}}
		class={createMode === 'catalog' && !application
			? 'flex max-h-[90vh] flex-col overflow-hidden sm:max-w-5xl'
			: undefined}>
		<Dialog.Header class={createMode === 'catalog' && !application ? 'shrink-0' : undefined}>
			<Dialog.Title
				>{application
					? t('projects.application.edit')
					: createMode === 'catalog'
						? catalogView === 'detail'
							? t('projects.application_form.template_details')
							: t('projects.application_form.template_catalog')
						: t('projects.application_form.create_application')}</Dialog.Title>
			<Dialog.Description
				>{application
					? t('projects.application_form.desc_edit')
					: createMode === 'catalog'
						? t('projects.application_form.desc_catalog')
						: t('projects.application_form.desc_create')}</Dialog.Description>
		</Dialog.Header>

		{#if !application && catalogView !== 'detail'}
			<div class="grid shrink-0 grid-cols-2 gap-2">
				<Button
					type="button"
					variant={createMode === 'blank' ? 'default' : 'outline'}
					onclick={showBlankApplication}>{t('projects.application_form.blank_application')}</Button>
				<Button
					type="button"
					variant={createMode === 'catalog' ? 'default' : 'outline'}
					onclick={showCatalog}>{t('projects.application_form.template_catalog')}</Button>
			</div>
		{/if}

		{#if !application && createMode === 'catalog'}
			{#if catalogView === 'catalog'}
				<div class="grid min-h-0 flex-1 auto-rows-min gap-4 overflow-y-auto pr-1">
					<div class="flex flex-col gap-3 sm:flex-row sm:items-center">
						<div class="relative flex-1">
							<SearchIcon
								class="absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />{#if templatesLoading}<LoaderCircleIcon
									class="absolute right-3 top-1/2 size-4 -translate-y-1/2 animate-spin text-muted-foreground" />{/if}<Input
								bind:ref={searchInput}
								bind:value={search}
								class="pl-9 pr-9"
								placeholder={t('projects.application_form.search_templates')} />
						</div>
						{#if category}<Button
								type="button"
								variant="ghost"
								size="sm"
								onclick={() => (category = '')}
								>{t('projects.application_form.clear_filter')}</Button
							>{/if}
					</div>
					{#if categories.length}<div
							class="no-scrollbar flex flex-nowrap gap-2 overflow-x-auto sm:flex-wrap">
							{#each categories as item (item)}<Button
									type="button"
									size="sm"
									class="shrink-0"
									variant={category === item ? 'secondary' : 'outline'}
									onclick={() => (category = category === item ? '' : item)}>{item}</Button
								>{/each}
						</div>{/if}
					{#if templatesError}<p
							class="rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive">
							{templatesError}
						</p>{/if}
					{#if !templatesLoading && !templatesError && templates.length === 0}<div
							class="flex flex-col items-center gap-2 rounded-xl border border-dashed p-10 text-center">
							<PackageIcon class="size-7 text-muted-foreground" />
							<p class="font-medium">{t('projects.application_form.no_templates')}</p>
							<p class="text-sm text-muted-foreground">
								{t('projects.application_form.try_other_search')}
							</p>
						</div>{:else}<div class="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
							{#each templates as template (template.id)}<button
									type="button"
									class="group rounded-xl border p-4 text-left transition-colors hover:border-primary/50 hover:bg-accent/50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
									onclick={() => void openDetail(template.id)}
									><div class="flex gap-3">
										{#if template.icon_url}<img
												src={template.icon_url}
												alt=""
												class="size-10 shrink-0 object-contain" />{:else}<div
												class="flex size-10 shrink-0 items-center justify-center rounded-lg bg-muted">
												<PackageIcon class="size-5 text-muted-foreground" />
											</div>{/if}
										<div class="min-w-0">
											<p class="truncate font-medium">
												{template.name}
												<span class="text-sm font-normal text-muted-foreground"
													>v{template.version}</span>
											</p>
											<p class="mt-1 line-clamp-2 min-h-10 text-sm text-muted-foreground">
												{template.description}
											</p>
										</div>
									</div>
									{#if template.categories.length}<div
											class="mt-3 hidden flex-wrap gap-1.5 sm:flex">
											{#each template.categories as item (item)}<Badge variant="secondary"
													>{item}</Badge
												>{/each}
										</div>{/if}{#if template.ports.length}<p
											class="mt-3 text-xs text-muted-foreground">
											{template.ports.map((port) => port.container).join(' · ')}
										</p>{/if}</button
								>{/each}
						</div>{/if}
				</div>
			{:else if detailLoading}<div
					class="flex min-h-72 items-center justify-center gap-2 text-sm text-muted-foreground">
					<LoaderCircleIcon class="size-5 animate-spin" />{t(
						'projects.application_form.loading_template_details',
					)}
				</div>
			{:else if detailError}<div class="grid gap-4">
					<p
						class="rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive">
						{detailError}
					</p>
					<Button type="button" variant="outline" class="w-fit" onclick={backToCatalog}
						><ArrowLeftIcon class="size-4" />{t(
							'projects.application_form.back_to_catalog',
						)}</Button>
				</div>
			{:else if selectedTemplate}
				<div class="grid min-h-0 flex-1 auto-rows-min gap-5 overflow-y-auto pr-1">
					<section class="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
						<div class="flex gap-4">
							{#if selectedTemplate.icon_url}
								<img
									src={selectedTemplate.icon_url}
									alt=""
									class="size-12 shrink-0 object-contain" />
							{:else}
								<div class="flex size-12 shrink-0 items-center justify-center rounded-xl bg-muted">
									<PackageIcon class="size-6 text-muted-foreground" />
								</div>
							{/if}
							<div>
								<h3 class="text-xl font-semibold">
									{selectedTemplate.name}
									<span class="text-base font-normal text-muted-foreground"
										>v{selectedTemplate.version}</span>
								</h3>
								<p class="mt-1 text-sm text-muted-foreground">{selectedTemplate.description}</p>
								{#if selectedTemplate.requires_https}
									<p class="mt-2 text-xs text-warning">
										{t('projects.application_form.requires_https')}
									</p>
								{/if}
								<div class="mt-3 flex flex-wrap gap-1.5">
									{#each selectedTemplate.categories as item (item)}<Badge variant="secondary"
											>{item}</Badge
										>{/each}
									{#each selectedTemplate.tags as item (item)}<Badge variant="outline">{item}</Badge
										>{/each}
									{#if selectedTemplate.license}<Badge variant="outline"
											>{selectedTemplate.license}</Badge
										>{/if}
								</div>
							</div>
						</div>
						<div class="flex flex-wrap gap-2">
							{#if selectedTemplate.website}
								<Button
									variant="outline"
									size="sm"
									href={selectedTemplate.website}
									target="_blank"
									rel="noreferrer">
									<ExternalLinkIcon class="size-4" />{t('projects.application_form.website')}
								</Button>
							{/if}
							{#if selectedTemplate.docs}
								<Button
									variant="outline"
									size="sm"
									href={selectedTemplate.docs}
									target="_blank"
									rel="noreferrer">
									<ExternalLinkIcon class="size-4" />{t('projects.application_form.docs')}
								</Button>
							{/if}
							{#if selectedTemplate.source}
								<Button
									variant="outline"
									size="sm"
									href={selectedTemplate.source}
									target="_blank"
									rel="noreferrer">
									<ExternalLinkIcon class="size-4" />{t('projects.application_form.source')}
								</Button>
							{/if}
						</div>
					</section>

					{#if setupChunks.length > 0}
						<Card.Root>
							<Card.Header>
								<div class="flex items-start justify-between gap-3">
									<div class="grid gap-1">
										<Card.Title class="flex items-center gap-2"
											><BookOpenIcon class="size-4" />{t(
												'projects.application_form.setup',
											)}</Card.Title>
										<Card.Description
											>{t('projects.application_form.setup_description')}</Card.Description>
									</div>
									{#if setupUrl}
										<Button
											variant="outline"
											size="sm"
											href={setupUrl}
											target="_blank"
											rel="noreferrer">
											<ExternalLinkIcon class="size-4" />{t(
												'projects.application_form.view_in_catalog',
											)}
										</Button>
									{/if}
								</div>
							</Card.Header>
							<Card.Content>
								<!--
									`{@html}` is safe here: the catalog decodes base64 content that upstream already sanitizes,
									and we additionally run DOMPurify on every prose chunk before render. Code blocks are
									split out and rendered as ReadOnlyCodeEditor (no `{@html}` for them).
								-->
								<div class="setup-prose flex flex-col gap-3">
									{#each setupChunks as chunk, i (i)}
										{#if chunk.kind === 'html'}
											{@html chunk.content}
										{:else}
											{#key i}
												<ReadOnlyCodeEditor
													content={chunk.code}
													language={monacoLanguageFor(chunk.language)}
													label={`${selectedTemplate.name} setup ${chunk.language}`}
													height={editorHeightFor(chunk.code)} />
											{/key}
										{/if}
									{/each}
								</div>
							</Card.Content>
						</Card.Root>
					{/if}

					{#if activeFile && fileTabs.length}
						<section class="grid gap-3">
							<AnimatedTabs
								items={fileTabs.map((file) => ({ value: file.value, label: file.label }))}
								bind:value={activeFileId} />
							<ReadOnlyCodeEditor
								content={activeFile.content}
								language={activeFile.language}
								label={`${selectedTemplate.name} ${activeFile.label}`}
								minHeight="22rem" />
						</section>
					{/if}

					{#if selectedTemplate.ports.length}
						<section class="rounded-xl border p-4">
							<p class="font-medium">{t('projects.application_form.ports')}</p>
							<div class="mt-3 grid gap-2 sm:grid-cols-2">
								{#each selectedTemplate.ports as port (port.container)}
									<div
										class="flex items-center justify-between rounded-lg bg-muted/50 px-3 py-2 text-sm">
										<span class="font-mono">{port.container}</span>
										<span class="text-muted-foreground"
											>{port.description || '—'}{port.primary
												? t('projects.application_form.primary_marker')
												: ''}</span>
									</div>
								{/each}
							</div>
						</section>
					{/if}
				</div>
				<div class="grid gap-2 border-t pt-4">
					<Label for="catalog-application-server">{t('projects.application_form.server')}</Label>
					<Select.Root type="single" bind:value={serverId}>
						<Select.Trigger id="catalog-application-server" class="w-full"
							>{serverTriggerLabel}</Select.Trigger>
						<Select.Content>
							{#each servers as server (server.id)}
								<Select.Item value={server.id}
									>{server.name} · {server.ingress_mode.replace('_', ' ')}</Select.Item>
							{/each}
						</Select.Content>
					</Select.Root>
					{#if servers.length === 0}<p class="text-sm text-destructive">
							{t('projects.application_form.no_ready_servers')}
						</p>{/if}
				</div>

				<Dialog.Footer class="shrink-0 border-t border-border bg-popover pt-4">
					<Button type="button" variant="outline" onclick={backToCatalog}>
						<ArrowLeftIcon class="size-4" />{t('projects.application_form.back_to_catalog')}
					</Button>
					<Button
						type="button"
						disabled={saving || !serverId}
						onclick={() => void createFromTemplate()}>
						{saving
							? t('projects.application_form.creating')
							: t('projects.application_form.create_from_template')}
					</Button>
				</Dialog.Footer>
			{/if}
		{:else}
			<form
				class="grid gap-4"
				onsubmit={(event) => {
					event.preventDefault();
					void submit();
				}}>
				<div class="grid gap-2">
					<Label for="application-name">{t('projects.projects.name')}</Label><Input
						id="application-name"
						bind:value={name}
						maxlength={100}
						placeholder={t('projects.application_form.name_placeholder')}
						autofocus />
				</div>
				<div class="grid gap-2">
					<Label for="application-description"
						>{t('projects.projects.description')}
						<span class="text-muted-foreground">({t('projects.projects.optional')})</span></Label
					><Textarea
						id="application-description"
						bind:value={description}
						maxlength={1000}
						placeholder={t('projects.application_form.app_description_placeholder')} />
				</div>
				{#if application}<div class="grid gap-2">
						<Label for="application-icon">{t('projects.projects.icon')}</Label>
						<div class="flex items-center gap-2">
							<div
								class="flex size-9 shrink-0 items-center justify-center rounded-md border overflow-hidden bg-muted">
								{#if previewIconUrl.trim()}
									<img
										src={previewIconUrl}
										alt=""
										loading="lazy"
										class="size-full object-contain" />
								{:else}
									<PackageIcon class="size-4 text-muted-foreground" />
								{/if}
							</div>
							<Input
								id="application-icon"
								bind:value={iconUrl}
								placeholder={t('projects.projects.icon_placeholder')} />
							{#if onUploadIcon}
								<IconUploadButton
									onUpload={onUploadIcon}
									onUploaded={(url) => {
										iconUrl = url;
										previewIconUrl = url;
									}} />
							{/if}
						</div>
					</div>{/if}
				{#if !application}<div class="grid gap-2">
						<Label>{t('projects.application_form.deploy_target')}</Label>
						<SegmentedSwitch
							items={[
								{
									value: 'server',
									label: t('projects.application_form.server'),
									icon: ServerIcon,
								},
								{
									value: 'group',
									label: t('projects.application_form.server_group'),
									icon: BoxesIcon,
								},
							]}
							bind:value={targetMode}
							label={t('projects.application_form.deploy_target')} />
						{#if targetMode === 'server'}
							<Select.Root type="single" bind:value={serverId}>
								<Select.Trigger id="application-server" class="w-full"
									>{serverTriggerLabel}</Select.Trigger>
								<Select.Content>
									{#each servers as server (server.id)}
										<Select.Item value={server.id}
											>{server.name} · {server.ingress_mode.replace('_', ' ')}</Select.Item>
									{/each}
								</Select.Content>
							</Select.Root>
							{#if servers.length === 0}<p class="text-sm text-destructive">
									{t('projects.application_form.no_ready_servers')}
								</p>{/if}
						{:else}
							<Select.Root type="single" bind:value={groupId}>
								<Select.Trigger id="application-server-group" class="w-full"
									>{groupTriggerLabel}</Select.Trigger>
								<Select.Content>
									{#each groups as group (group.id)}
										<Select.Item value={group.id}>{group.name}</Select.Item>
									{/each}
								</Select.Content>
							</Select.Root>
							{#if selectedGroup}<p class="text-xs text-muted-foreground">
									{t('projects.application_form.server_group_hint', {
										count: selectedGroup.member_count,
									})}
								</p>{:else if groups.length === 0}<p class="text-sm text-muted-foreground">
									{t('projects.application_form.no_server_groups')}
								</p>{/if}
						{/if}
					</div>{/if}<Dialog.Footer
					><Button type="button" variant="outline" onclick={() => (open = false)}
						>{t('projects.projects.cancel')}</Button
					><Button
						type="submit"
						disabled={saving ||
							(!application &&
								(targetMode === 'server' ? !serverId || servers.length === 0 : !groupId))}
						>{saving
							? t('projects.projects.saving')
							: application
								? t('projects.projects.save_changes')
								: t('projects.application_form.create_application')}</Button
					></Dialog.Footer>
			</form>
		{/if}
	</Dialog.Content>
</Dialog.Root>

<style>
	/*
	 * Scoped styles for the sanitised setup markdown. Code blocks are split
	 * out and rendered as ReadOnlyCodeEditor, so this only needs to cover the
	 * prose around them.
	 */
	.setup-prose :global(h2) {
		font-size: 1.05rem;
		font-weight: 600;
		margin-top: 0.75rem;
		margin-bottom: 0.35rem;
		letter-spacing: -0.01em;
	}
	.setup-prose :global(h2):first-child {
		margin-top: 0;
	}
	.setup-prose :global(h3) {
		font-size: 0.92rem;
		font-weight: 600;
		margin-top: 0.65rem;
		margin-bottom: 0.3rem;
	}
	.setup-prose :global(p) {
		line-height: 1.55;
		margin-block: 0.3rem;
	}
	.setup-prose :global(ul),
	.setup-prose :global(ol) {
		padding-left: 1.2rem;
		line-height: 1.55;
	}
	.setup-prose :global(ul) {
		list-style: disc;
	}
	.setup-prose :global(ol) {
		list-style: decimal;
	}
	.setup-prose :global(li) {
		margin-block: 0.15rem;
	}
	.setup-prose :global(a) {
		color: var(--primary);
		text-underline-offset: 3px;
	}
	.setup-prose :global(a:hover) {
		text-decoration: underline;
	}
	.setup-prose :global(code) {
		font-family: 'Commit Mono', ui-monospace, SFMono-Regular, monospace;
		font-size: 0.85em;
		background: var(--muted);
		padding: 0.1em 0.35em;
		border-radius: 0.3rem;
	}
	.setup-prose :global(table) {
		width: 100%;
		border-collapse: collapse;
		font-size: 0.85rem;
		margin-block: 0.4rem;
	}
	.setup-prose :global(th),
	.setup-prose :global(td) {
		text-align: left;
		padding: 0.4rem 0.6rem;
		border-bottom: 1px solid var(--border);
	}
	.setup-prose :global(th) {
		font-weight: 600;
		background: var(--muted);
	}
	.setup-prose :global(strong) {
		font-weight: 600;
	}
	.setup-prose :global(blockquote) {
		border-left: 3px solid var(--border);
		padding-left: 0.8rem;
		color: var(--muted-foreground);
		margin-block: 0.4rem;
	}
</style>
