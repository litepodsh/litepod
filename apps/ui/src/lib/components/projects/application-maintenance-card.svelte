<script lang="ts">
	/**
	 * Maintenance mode for one application: while on, every domain answers with a
	 * 503 page and the containers keep running. The page is the default template,
	 * the user's own HTML (pasted, or loaded from an `.html` file), or a small site
	 * uploaded as a ZIP with its own CSS and images.
	 */
	import * as Card from '#lib/components/ui/card/index.js';
	import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Textarea } from '#lib/components/ui/textarea/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import AnimatedFileUpload from '#lib/components/custom/animated-file-upload.svelte';
	import SoftSwitch from '#lib/components/custom/soft-switch.svelte';
	import ConstructionIcon from '@lucide/svelte/icons/construction';
	import UploadIcon from '@lucide/svelte/icons/upload';
	import {
		deleteMaintenanceSite,
		getMaintenance,
		updateMaintenance,
		uploadMaintenanceSite,
		MAINTENANCE_MAX_PAGE_BYTES,
		MAINTENANCE_MAX_SITE_BYTES,
		type MaintenanceDto,
	} from '#lib/clients/project-client.js';
	import { toast, toastError } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let {
		organizationId,
		projectId,
		environmentId,
		applicationId,
		canWrite = true,
		onChange,
	}: {
		organizationId: string;
		projectId: string;
		environmentId: string;
		applicationId: string;
		canWrite?: boolean;
		/** Called with the mode's state whenever it is loaded or changed, so the page header can show it. */
		onChange?: (enabled: boolean) => void;
	} = $props();

	type Source = 'default' | 'html' | 'site';

	let data = $state<MaintenanceDto | null>(null);
	let source = $state<Source>('default');
	let draft = $state('');
	let saving = $state(false);
	let confirmEnable = $state(false);
	let htmlInput = $state<HTMLInputElement | null>(null);

	/** What the application actually serves, as the server has it. */
	function savedSource(from: MaintenanceDto): Source {
		if (from.site) return 'site';
		return from.custom_html !== null ? 'html' : 'default';
	}

	function adopt(next: MaintenanceDto) {
		data = next;
		source = savedSource(next);
		draft = next.custom_html ?? '';
		onChange?.(next.enabled);
	}

	$effect(() => {
		getMaintenance(organizationId, projectId, environmentId, applicationId)
			.then(adopt)
			.catch((err) => toastError(err, t('projects.maintenance.load_failed')));
	});

	// The Railpack build runs in the background; follow it until it settles.
	$effect(() => {
		if (data?.companion_status !== 'building') return;
		const timer = window.setInterval(() => {
			getMaintenance(organizationId, projectId, environmentId, applicationId)
				.then(adopt)
				.catch(() => {});
		}, 3000);
		return () => window.clearInterval(timer);
	});

	/** A site is saved the moment it is uploaded; only the other two sources wait for "Save". */
	const dirty = $derived(
		data !== null &&
			source !== 'site' &&
			(source !== savedSource(data) || (source === 'html' && draft !== (data.custom_html ?? ''))),
	);
	const previewHtml = $derived(source === 'html' && draft.trim() ? draft : (data?.default_html ?? ''));

	function size(bytes: number) {
		return bytes >= 1024 * 1024
			? `${(bytes / (1024 * 1024)).toFixed(1)} MB`
			: `${Math.max(1, Math.round(bytes / 1024))} KB`;
	}

	async function setEnabled(enabled: boolean) {
		saving = true;
		try {
			adopt(
				await updateMaintenance(organizationId, projectId, environmentId, applicationId, {
					enabled,
				}),
			);
			toast.success(
				t(enabled ? 'projects.maintenance.turned_on' : 'projects.maintenance.turned_off'),
			);
		} catch (err) {
			toastError(err, t('projects.maintenance.update_failed'));
		} finally {
			saving = false;
			confirmEnable = false;
		}
	}

	function onSwitch(checked: boolean) {
		// Turning it on takes the site's visitors off the application, so it asks first.
		if (checked) confirmEnable = true;
		else void setEnabled(false);
	}

	async function savePage() {
		if (source === 'html' && !draft.trim()) {
			toast.error(t('projects.maintenance.empty_page'));
			return;
		}
		saving = true;
		try {
			// Choosing a page that is not the uploaded site drops the site first.
			if (data?.site) {
				await deleteMaintenanceSite(organizationId, projectId, environmentId, applicationId);
			}
			adopt(
				await updateMaintenance(organizationId, projectId, environmentId, applicationId, {
					page_html: source === 'html' ? draft : '',
				}),
			);
			toast.success(t('projects.maintenance.page_saved'));
		} catch (err) {
			toastError(err, t('projects.maintenance.update_failed'));
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
			toast.error(t('projects.maintenance.too_large', { kib: MAINTENANCE_MAX_PAGE_BYTES / 1024 }));
			return;
		}
		draft = await file.text();
		source = 'html';
	}

	/** Cheap client-side checks; the server validates the archive for real. */
	async function validateZip(file: File) {
		if (file.size > MAINTENANCE_MAX_SITE_BYTES) {
			toast.error(
				t('projects.maintenance.zip_too_large', { mb: MAINTENANCE_MAX_SITE_BYTES / (1024 * 1024) }),
			);
			return null;
		}
		// A ZIP starts with "PK" and a local-header, empty-archive or spanned marker.
		const head = new Uint8Array(await file.slice(0, 4).arrayBuffer());
		const isZip =
			head[0] === 0x50 &&
			head[1] === 0x4b &&
			[3, 5, 7].includes(head[2]) &&
			[4, 6, 8].includes(head[3]);
		if (!isZip) {
			toast.error(t('projects.maintenance.invalid_zip'));
			return null;
		}
		return { file, kind: 'zip' };
	}

	async function onZipRemove() {
		saving = true;
		try {
			adopt(await deleteMaintenanceSite(organizationId, projectId, environmentId, applicationId));
		} catch (err) {
			toastError(err, t('projects.maintenance.update_failed'));
		} finally {
			saving = false;
		}
	}

	async function onZipUpload(file: File) {
		saving = true;
		try {
			adopt(
				await uploadMaintenanceSite(organizationId, projectId, environmentId, applicationId, file),
			);
			toast.success(t('projects.maintenance.site_uploaded'));
		} catch (err) {
			toastError(err, t('projects.maintenance.update_failed'));
		} finally {
			saving = false;
		}
	}
</script>

<Card.Root class="max-w-[820px] overflow-hidden">
	<Card.Header>
		<div class="flex items-center gap-2">
			<ConstructionIcon class="size-4 text-muted-foreground" />
			<Card.Title>{t('projects.maintenance.title')}</Card.Title>
			{#if data?.enabled}
				<Badge variant="outline" class="text-amber-600 dark:text-amber-400">
					{t('projects.maintenance.active_badge')}
				</Badge>
			{/if}
		</div>
		<Card.Description>{t('projects.maintenance.description')}</Card.Description>
	</Card.Header>
	<Card.Content class="grid gap-4 pt-2">
		{#if data}
			<div class="flex items-center justify-between gap-4">
				<div class="min-w-0">
					<Label for="maintenance-enabled">{t('projects.maintenance.switch')}</Label>
					<p class="mt-1 text-sm text-muted-foreground">
						{data.enabled
							? t('projects.maintenance.on_hint')
							: t('projects.maintenance.off_hint')}
					</p>
				</div>
				<SoftSwitch
					id="maintenance-enabled"
					checked={data.enabled}
					disabled={saving || !canWrite}
					onCheckedChange={onSwitch} />
			</div>

			<div class="grid gap-2">
				<p class="text-sm font-medium">{t('projects.maintenance.page')}</p>
				<div class="flex flex-wrap gap-2">
					{#each [['default', 'use_default'], ['html', 'use_custom'], ['site', 'use_site']] as [value, label] (value)}
						<Button
							type="button"
							size="sm"
							variant={source === value ? 'default' : 'outline'}
							disabled={!canWrite || saving}
							onclick={() => (source = value as Source)}>
							{t(`projects.maintenance.${label}`)}
						</Button>
					{/each}
				</div>

				{#if source === 'html'}
					<Textarea
						bind:value={draft}
						rows={10}
						spellcheck={false}
						disabled={!canWrite}
						class="font-mono text-xs"
						aria-label={t('projects.maintenance.html_label')}
						placeholder={t('projects.maintenance.html_placeholder')} />
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
							{t('projects.maintenance.upload')}
						</Button>
						<p class="text-xs text-muted-foreground">{t('projects.maintenance.assets_hint')}</p>
					</div>
				{:else if source === 'site'}
					<div class="mt-2">
						<AnimatedFileUpload
							accept=".zip,application/zip"
							heading={t('projects.maintenance.zip_heading')}
							description={t('projects.maintenance.zip_description')}
							validate={validateZip}
							onMultipleFiles={() => toast.error(t('projects.maintenance.one_file_at_a_time'))}
							onUpload={onZipUpload}
							onDelete={onZipRemove}
							allowDelete={canWrite}
							currentFile={data.site
								? { name: 'site.zip', size: data.site.bytes, kind: 'zip' }
								: null} />
					</div>
					{#if data.site}
						<p class="text-sm text-muted-foreground">
							{t('projects.maintenance.site_summary', {
								files: data.site.files,
								size: size(data.site.bytes),
							})}
						</p>
					{:else}
						<p class="text-sm text-amber-600 dark:text-amber-400">
							{t('projects.maintenance.no_site_yet')}
						</p>
					{/if}
					{#if data.companion_status !== 'none'}
						<p
							class="text-sm {data.companion_status === 'failed'
								? 'text-destructive'
								: 'text-muted-foreground'}">
							{#if data.companion_status === 'failed'}
								{t('projects.maintenance.build_failed', { error: data.companion_error ?? '' })}
							{:else}
								{t(`projects.maintenance.build_${data.companion_status}`)}
							{/if}
						</p>
					{/if}
					<p class="text-xs text-muted-foreground">{t('projects.maintenance.zip_hint')}</p>
				{/if}

				{#if source !== 'site'}
					<p class="text-sm font-medium">{t('projects.maintenance.preview')}</p>
					<!-- Scripts are off and the frame has no origin: a custom page is the user's own
					     HTML, and the preview must not be a way to run it inside Litepod. -->
					<iframe
						title={t('projects.maintenance.preview')}
						sandbox=""
						srcdoc={previewHtml}
						class="h-64 w-full rounded-lg border bg-white"></iframe>
				{:else if data.site}
					<!-- An uploaded site is only served to the application's own domains while the
					     mode is on, so there is nothing to frame from here. -->
					<p class="text-xs text-muted-foreground">{t('projects.maintenance.site_no_preview')}</p>
				{/if}

				{#if canWrite && source !== 'site'}
					<div class="flex justify-end">
						<Button type="button" disabled={saving || !dirty} onclick={savePage}>
							{t('projects.maintenance.save_page')}
						</Button>
					</div>
				{/if}
			</div>
		{/if}
	</Card.Content>
</Card.Root>

<AlertDialog.Root
	open={confirmEnable}
	onOpenChange={(open) => {
		if (!open && !saving) confirmEnable = false;
	}}>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title>{t('projects.maintenance.confirm_title')}</AlertDialog.Title>
			<AlertDialog.Description>
				{t('projects.maintenance.confirm_description')}
			</AlertDialog.Description>
		</AlertDialog.Header>
		<AlertDialog.Footer>
			<AlertDialog.Cancel disabled={saving}>{t('projects.maintenance.cancel')}</AlertDialog.Cancel>
			<Button disabled={saving} onclick={() => setEnabled(true)}>
				{t('projects.maintenance.confirm_action')}
			</Button>
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>
