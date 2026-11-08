<script lang="ts">
	/**
	 * The page visitors get when the application's domain exists but the
	 * application does not answer (Caddy's 502/503/504): Litepod's default page,
	 * the user's own HTML (pasted, or loaded from an `.html` file), or none.
	 */
	import * as Card from '#lib/components/ui/card/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import SoftSwitch from '#lib/components/custom/soft-switch.svelte';
	import ServerCrashIcon from '@lucide/svelte/icons/server-crash';
	import UploadIcon from '@lucide/svelte/icons/upload';
	import Maximize2Icon from '@lucide/svelte/icons/maximize-2';
	import RotateCwIcon from '@lucide/svelte/icons/rotate-cw';
	import ScriptEditor from '#lib/components/scheduled-tasks/script-editor.svelte';
	import {
		getErrorPage,
		updateErrorPage,
		MAINTENANCE_MAX_PAGE_BYTES,
		type ErrorPageDto,
	} from '#lib/clients/project-client.js';
	import { toast, toastError } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let {
		organizationId,
		projectId,
		environmentId,
		applicationId,
		canWrite = true,
	}: {
		organizationId: string;
		projectId: string;
		environmentId: string;
		applicationId: string;
		canWrite?: boolean;
	} = $props();

	type Source = 'default' | 'html';

	let data = $state<ErrorPageDto | null>(null);
	let source = $state<Source>('default');
	let draft = $state('');
	let saving = $state(false);
	let htmlInput = $state<HTMLInputElement | null>(null);
	let expanded = $state(false);
	/** Bumped by the reload button: re-creating the frame reloads the page from scratch. */
	let previewRun = $state(0);

	function adopt(next: ErrorPageDto) {
		data = next;
		source = next.page_html !== null ? 'html' : 'default';
		draft = next.page_html ?? '';
	}

	$effect(() => {
		getErrorPage(organizationId, projectId, environmentId, applicationId)
			.then(adopt)
			.catch((err) => toastError(err, t('projects.error_page.load_failed')));
	});

	const savedSource = $derived<Source>(data?.page_html != null ? 'html' : 'default');
	const dirty = $derived(
		data !== null &&
			(source !== savedSource || (source === 'html' && draft !== (data.page_html ?? ''))),
	);
	const previewHtml = $derived(
		source === 'html' && draft.trim() ? draft : (data?.default_html ?? ''),
	);

	async function setEnabled(enabled: boolean) {
		saving = true;
		try {
			adopt(
				await updateErrorPage(organizationId, projectId, environmentId, applicationId, { enabled }),
			);
			toast.success(
				t(enabled ? 'projects.error_page.turned_on' : 'projects.error_page.turned_off'),
			);
		} catch (err) {
			toastError(err, t('projects.error_page.update_failed'));
		} finally {
			saving = false;
		}
	}

	async function savePage() {
		if (source === 'html' && !draft.trim()) {
			toast.error(t('projects.error_page.empty_page'));
			return;
		}
		saving = true;
		try {
			adopt(
				await updateErrorPage(organizationId, projectId, environmentId, applicationId, {
					page_html: source === 'html' ? draft : '',
				}),
			);
			toast.success(t('projects.error_page.page_saved'));
		} catch (err) {
			toastError(err, t('projects.error_page.update_failed'));
		} finally {
			saving = false;
		}
	}

	async function onHtmlFile(event: Event) {
		const input = event.currentTarget as HTMLInputElement;
		const file = input.files?.[0];
		input.value = '';
		if (!file) return;
		if (file.size > MAINTENANCE_MAX_PAGE_BYTES) {
			toast.error(t('projects.error_page.too_large', { kib: MAINTENANCE_MAX_PAGE_BYTES / 1024 }));
			return;
		}
		draft = await file.text();
		source = 'html';
	}
</script>

<Card.Root class="max-w-[820px] overflow-hidden">
	<Card.Header>
		<div class="flex items-center gap-2">
			<ServerCrashIcon class="size-4 text-muted-foreground" />
			<Card.Title>{t('projects.error_page.title')}</Card.Title>
		</div>
		<Card.Description>{t('projects.error_page.description')}</Card.Description>
	</Card.Header>
	<Card.Content class="grid gap-4 pt-2">
		{#if data}
			<div class="flex items-center justify-between gap-4">
				<div class="min-w-0">
					<Label for="error-page-enabled">{t('projects.error_page.switch')}</Label>
					<p class="mt-1 text-sm text-muted-foreground">
						{data.enabled ? t('projects.error_page.on_hint') : t('projects.error_page.off_hint')}
					</p>
				</div>
				<SoftSwitch
					id="error-page-enabled"
					checked={data.enabled}
					disabled={saving || !canWrite}
					onCheckedChange={setEnabled} />
			</div>

			{#if data.enabled}
				<div class="grid gap-2">
					<p class="text-sm font-medium">{t('projects.error_page.page')}</p>
					<div class="flex flex-wrap gap-2">
						{#each [['default', 'use_default'], ['html', 'use_custom']] as [value, label] (value)}
							<Button
								type="button"
								size="sm"
								variant={source === value ? 'default' : 'outline'}
								disabled={!canWrite || saving}
								onclick={() => {
									source = value as Source;
									// Start a custom page from the default one instead of an empty box.
									if (source === 'html' && !draft.trim()) draft = data?.default_html ?? '';
								}}>
								{t(`projects.error_page.${label}`)}
							</Button>
						{/each}
					</div>

					{#if source === 'default' && !data.default_enabled}
						<p class="text-sm text-amber-600 dark:text-amber-400">
							{t('projects.error_page.default_off')}
						</p>
					{/if}

					{#if source === 'html'}
						<ScriptEditor
							bind:value={draft}
							language="html"
							height="16rem"
							filename="error.html"
							disabled={!canWrite}
							label={t('projects.error_page.html_label')}>
							{#snippet actions()}
								<Button
									type="button"
									size="sm"
									variant="ghost"
									class="h-6 gap-1.5 px-2 text-[11.5px]"
									onclick={() => (expanded = true)}>
									<Maximize2Icon class="size-3.5" />
									{t('projects.error_page.expand')}
								</Button>
							{/snippet}
						</ScriptEditor>
						<div class="flex flex-wrap items-center gap-2">
							<input
								bind:this={htmlInput}
								type="file"
								accept=".html,.htm,text/html"
								class="hidden"
								onchange={onHtmlFile} />
							<Button
								type="button"
								size="sm"
								variant="outline"
								disabled={!canWrite}
								onclick={() => htmlInput?.click()}>
								<UploadIcon class="size-4" />
								{t('projects.error_page.upload')}
							</Button>
							<p class="text-xs text-muted-foreground">{t('projects.error_page.assets_hint')}</p>
						</div>
					{/if}

					{@render preview('h-64')}

					{#if canWrite}
						<div class="flex justify-end">
							<Button type="button" disabled={saving || !dirty} onclick={savePage}>
								{t('projects.error_page.save_page')}
							</Button>
						</div>
					{/if}
				</div>
			{/if}
		{/if}
	</Card.Content>
</Card.Root>

{#snippet preview(height: string)}
	<!-- Scripts are off and the frame has no origin: a custom page is the user's own
	     HTML, and the preview must not be a way to run it inside Litepod. -->
	<div class="lp-inset min-w-0 overflow-hidden rounded-xl border bg-background">
		<div
			class="lp-groove-b flex items-center justify-between gap-2 bg-muted/60 py-1.5 ps-3 pe-2 font-mono text-[11.5px] text-muted-foreground">
			<span class="truncate">{t('projects.error_page.preview')}</span>
			<Button
				type="button"
				size="icon"
				variant="ghost"
				class="-my-1.5 size-6"
				aria-label={t('projects.error_page.reload_preview')}
				title={t('projects.error_page.reload_preview')}
				onclick={() => previewRun++}>
				<RotateCwIcon class="size-3.5" />
			</Button>
		</div>
		{#key previewRun}
			<iframe
				title={t('projects.error_page.preview')}
				sandbox=""
				srcdoc={previewHtml}
				class="block {height} w-full bg-white"></iframe>
		{/key}
	</div>
{/snippet}

<Dialog.Root bind:open={expanded}>
	<Dialog.Content class="max-w-6xl sm:max-w-6xl">
		<Dialog.Header>
			<Dialog.Title>{t('projects.error_page.editor_title')}</Dialog.Title>
			<Dialog.Description>{t('projects.error_page.editor_description')}</Dialog.Description>
		</Dialog.Header>
		<div class="grid min-w-0 gap-4 lg:grid-cols-2">
			<ScriptEditor
				bind:value={draft}
				language="html"
				height="min(65vh, 44rem)"
				filename="error.html"
				disabled={!canWrite}
				label={t('projects.error_page.html_label')} />
			{@render preview('h-[min(65vh,44rem)]')}
		</div>
		<Dialog.Footer>
			{#if canWrite}
				<Button disabled={saving || !dirty} onclick={savePage}>
					{t('projects.error_page.save_page')}
				</Button>
			{/if}
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>
