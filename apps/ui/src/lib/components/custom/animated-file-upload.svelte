<script lang="ts">
	import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
	import DeleteConfirmControl from '#lib/components/custom/delete-confirm-control.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import { tick, untrack } from 'svelte';
	import CheckIcon from '@lucide/svelte/icons/check';
	import DownloadIcon from '@lucide/svelte/icons/download';
	import FileArchiveIcon from '@lucide/svelte/icons/file-archive';
	import FileTextIcon from '@lucide/svelte/icons/file-text';
	import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';
	import Trash2Icon from '@lucide/svelte/icons/trash-2';
	import UploadIcon from '@lucide/svelte/icons/upload';
	import { t } from '#lib/i18n/index.svelte.js';

	type UploadState = 'idle' | 'uploading' | 'ready' | 'confirming-delete' | 'removing';
	type ValidatedFile = { file: File; kind: string };
	type DisplayFile = { name: string; size: number; kind: string };

	let {
		accept,
		heading,
		description,
		validate,
		onMultipleFiles,
		onUpload,
		currentFile = null,
		allowDelete = true,
		onDelete,
		downloadUrl = null,
	}: {
		accept: string;
		heading: string;
		description: string;
		validate: (file: File) => Promise<ValidatedFile | null>;
		onMultipleFiles?: () => void;
		/** Perform the real upload. When omitted, the dropzone fakes a local upload animation. */
		onUpload?: (file: File) => Promise<void>;
		/** Already-uploaded file to show in the ready state, kept in sync with the caller's source of truth. */
		currentFile?: DisplayFile | null;
		allowDelete?: boolean;
		/** Perform the real removal. When omitted, deleting only clears the local selection. */
		onDelete?: () => Promise<void>;
		/** Where the uploaded `currentFile` can be downloaded from. Omit to hide the button. */
		downloadUrl?: string | null;
	} = $props();

	const progressSteps = [8, 22, 41, 63, 79, 91, 100];
	let input = $state<HTMLInputElement | null>(null);
	// Local state mirrors `currentFile` only at known checkpoints (post-upload,
	// post-delete); the prop drives re-sync, the local state drives the UI.
	let uploadState = $state<UploadState>(untrack(() => (currentFile ? 'ready' : 'idle')));
	let selectedFile = $state<DisplayFile | null>(untrack(() => currentFile));
	let progress = $state(0);
	let dragDepth = $state(0);
	let deleteDialogOpen = $state(false);
	let isDragging = $derived(dragDepth > 0);
	let isBusy = $derived(uploadState === 'uploading' || uploadState === 'removing');

	function prefersReducedMotion() {
		return (
			typeof window !== 'undefined' && window.matchMedia('(prefers-reduced-motion: reduce)').matches
		);
	}

	function wait(milliseconds: number) {
		return new Promise<void>((resolve) => window.setTimeout(resolve, milliseconds));
	}

	async function runFakeUpload(validFile: ValidatedFile) {
		const stepDuration = prefersReducedMotion() ? 0 : 360;
		for (const nextProgress of progressSteps) {
			await wait(stepDuration);
			progress = nextProgress;
		}
		if (!prefersReducedMotion()) await wait(300);
		selectedFile = { name: validFile.file.name, size: validFile.file.size, kind: validFile.kind };
		uploadState = 'ready';
	}

	async function runRealUpload(file: File) {
		let timer: number | null = null;
		if (!prefersReducedMotion()) {
			timer = window.setInterval(() => {
				progress = Math.min(90, progress + Math.max(1, (90 - progress) * 0.12));
			}, 180);
		}
		await onUpload!(file);
		if (timer !== null) window.clearInterval(timer);
		progress = 100;
		await tick();
		if (!prefersReducedMotion()) await wait(200);
		// `currentFile` reflects the caller's source of truth, so a failed upload
		// (which leaves it unchanged) correctly falls back instead of showing success.
		if (currentFile) {
			selectedFile = currentFile;
			uploadState = 'ready';
		} else {
			selectedFile = null;
			uploadState = 'idle';
		}
	}

	async function selectFile(file: File | undefined) {
		if (!file || isBusy) return;
		const validFile = await validate(file);
		if (!validFile) return;

		uploadState = 'uploading';
		progress = 0;
		if (onUpload) {
			await runRealUpload(file);
		} else {
			await runFakeUpload(validFile);
		}
	}

	function handleInputChange(event: Event) {
		const target = event.currentTarget as HTMLInputElement;
		void selectFile(target.files?.[0]);
		target.value = '';
	}

	function handleDrop(event: DragEvent) {
		event.preventDefault();
		dragDepth = 0;
		const files = event.dataTransfer?.files;
		if (!files?.length) return;
		if (files.length > 1) {
			onMultipleFiles?.();
			return;
		}
		void selectFile(files[0]);
	}

	function openDeleteConfirmation() {
		if (!selectedFile || isBusy) return;
		uploadState = 'confirming-delete';
		deleteDialogOpen = true;
	}

	function handleDeleteDialogChange(open: boolean) {
		deleteDialogOpen = open;
		if (!open && uploadState === 'confirming-delete') uploadState = 'ready';
	}

	async function confirmDelete() {
		deleteDialogOpen = false;
		uploadState = 'removing';
		if (onDelete) {
			await onDelete();
			await tick();
			// `currentFile` reflects the caller's source of truth, so a failed removal
			// (which leaves it unchanged) correctly falls back instead of showing success.
			if (currentFile) {
				selectedFile = currentFile;
				uploadState = 'ready';
			} else {
				selectedFile = null;
				progress = 0;
				uploadState = 'idle';
			}
			return;
		}
		await wait(prefersReducedMotion() ? 0 : 360);
		selectedFile = null;
		progress = 0;
		uploadState = 'idle';
	}

	function formatFileSize(size: number) {
		if (size < 1024) return `${size} B`;
		if (size < 1024 ** 2) return `${(size / 1024).toFixed(1)} KB`;
		if (size < 1024 ** 3) return `${(size / 1024 ** 2).toFixed(1)} MB`;
		return `${(size / 1024 ** 3).toFixed(1)} GB`;
	}
</script>

<div class="flex flex-col gap-6">
	<div>
		<h2 class="text-lg font-semibold">{heading}</h2>
		<p class="mt-1 text-sm text-muted-foreground">{description}</p>
	</div>

	<div class={`upload-stage relative min-h-72 ${`state-${uploadState}`}`}>
		<div
			class={`upload-layer idle-layer upload-dropzone flex min-h-72 flex-col items-center justify-center rounded-2xl border-2 border-dashed p-6 text-center sm:p-10 ${isDragging ? 'border-primary bg-primary/5' : 'border-muted-foreground/25'}`}
			role="region"
			aria-label={t('common.upload.dropzone')}
			aria-hidden={uploadState !== 'idle'}
			inert={uploadState !== 'idle'}
			ondragenter={(event) => {
				event.preventDefault();
				dragDepth += 1;
			}}
			ondragover={(event) => event.preventDefault()}
			ondragleave={(event) => {
				event.preventDefault();
				dragDepth = Math.max(0, dragDepth - 1);
			}}
			ondrop={handleDrop}>
			<div class="flex max-w-sm flex-col items-center gap-3">
				<div class="bg-muted upload-icon flex size-12 items-center justify-center rounded-full">
					<UploadIcon class="size-5" />
				</div>
				<div>
					<p class="font-medium">{t('common.upload.drop_title')}</p>
					<p class="mt-1 text-sm text-muted-foreground">{t('common.upload.drop_description')}</p>
				</div>
				<Button onclick={() => input?.click()}
					><UploadIcon class="size-4" />{t('common.upload.choose_file')}</Button>
			</div>
		</div>

		<div
			class="upload-layer uploading-panel flex min-h-72 flex-col items-center justify-center rounded-2xl border p-6 text-center sm:p-10"
			aria-hidden={uploadState !== 'uploading'}
			aria-live={uploadState === 'uploading' ? 'polite' : 'off'}>
			<div
				class="upload-rings relative mb-6 flex size-20 items-center justify-center rounded-full bg-primary/10 text-primary">
				<UploadIcon class="uploading-icon size-8" />
			</div>
			<p class="font-medium">{t('common.upload.preparing')}</p>
			<p class="mt-1 text-sm text-muted-foreground">{t('common.upload.validating')}</p>
			<div class="mt-6 w-full max-w-sm">
				<div class="mb-2 flex items-center justify-between text-xs text-muted-foreground">
					<span>{t('common.upload.uploading_preview')}</span><span>{Math.round(progress)}%</span>
				</div>
				<div class="h-2 overflow-hidden rounded-full bg-muted">
					<div class="progress-fill h-full rounded-full bg-primary" style={`width: ${progress}%`}>
					</div>
				</div>
			</div>
		</div>

		<div
			class="upload-layer ready-layer"
			aria-hidden={uploadState !== 'ready' && uploadState !== 'confirming-delete'}
			inert={uploadState !== 'ready' && uploadState !== 'confirming-delete'}>
			{#if selectedFile}
				<div class="file-card flex h-full w-full flex-col overflow-hidden rounded-2xl border">
					<div class="flex flex-1 items-center gap-3 p-4 sm:p-10">
						<div class="bg-muted flex size-11 shrink-0 items-center justify-center rounded-xl">
							{#if selectedFile.kind.toUpperCase() === 'ZIP'}<FileArchiveIcon
									class="size-5" />{:else}<FileTextIcon class="size-5" />{/if}
						</div>
						<div class="min-w-0 flex-1">
							<p class="truncate font-medium">{selectedFile.name}</p>
							<p class="text-sm text-muted-foreground">
								{selectedFile.kind.toUpperCase()} · {formatFileSize(selectedFile.size)}
							</p>
							<p class="mt-1 inline-flex items-center gap-1 text-xs font-medium text-success">
								<CheckIcon class="size-3.5" />{t('common.upload.ready')}
							</p>
						</div>
					</div>
					<div class="flex items-center justify-end gap-2 border-t bg-muted/30 px-4 py-3">
						{#if downloadUrl && currentFile}
							<Button variant="outline" size="sm" disabled={isBusy} href={downloadUrl} download
								><DownloadIcon class="size-4" />{t('common.upload.download')}</Button>
						{/if}
						<Button variant="outline" size="sm" disabled={isBusy} onclick={() => input?.click()}
							><RefreshCwIcon class="size-4" />{t('common.upload.replace')}</Button>
						{#if allowDelete}
							<Button
								variant="outline"
								size="sm"
								class="text-destructive hover:text-destructive"
								disabled={isBusy}
								onclick={openDeleteConfirmation}
								><Trash2Icon class="size-4" />{t('common.common.delete')}</Button>
						{/if}
					</div>
				</div>
			{/if}
		</div>
	</div>

	<input bind:this={input} class="sr-only" type="file" {accept} onchange={handleInputChange} />
</div>

<AlertDialog.Root open={deleteDialogOpen} onOpenChange={handleDeleteDialogChange}>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title>{t('common.upload.delete_title')}</AlertDialog.Title>
			<AlertDialog.Description>{t('common.upload.delete_description')}</AlertDialog.Description>
		</AlertDialog.Header>
		<AlertDialog.Footer>
			<AlertDialog.Cancel>{t('common.common.cancel')}</AlertDialog.Cancel>
			<DeleteConfirmControl onconfirm={confirmDelete} />
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>

<style>
	.upload-stage {
		isolation: isolate;
	}
	.upload-layer {
		position: absolute;
		inset: 0;
		transition:
			opacity 520ms cubic-bezier(0.22, 1, 0.36, 1),
			transform 520ms cubic-bezier(0.22, 1, 0.36, 1),
			filter 420ms ease;
	}
	.idle-layer {
		z-index: 1;
		opacity: 0;
		transform: translateX(-32px) scale(0.96);
		filter: blur(2px);
	}
	.uploading-panel {
		z-index: 1;
		opacity: 0;
		transform: translateY(24px) scale(0.96);
		filter: blur(2px);
	}
	.ready-layer {
		z-index: 1;
		display: flex;
		align-items: stretch;
		opacity: 0;
		transform: translateX(32px) scale(0.96);
		filter: blur(2px);
	}
	.state-idle .idle-layer {
		z-index: 2;
		opacity: 1;
		transform: translateX(0) scale(1);
		filter: blur(0);
	}
	.state-uploading .idle-layer {
		opacity: 0;
		transform: translateX(-32px) scale(0.96);
		filter: blur(2px);
	}
	.state-uploading .uploading-panel {
		z-index: 2;
		opacity: 1;
		transform: translateY(0) scale(1);
		filter: blur(0);
	}
	.state-ready .uploading-panel,
	.state-confirming-delete .uploading-panel {
		opacity: 0;
		transform: translateY(-24px) scale(0.96);
		filter: blur(2px);
	}
	.state-ready .ready-layer,
	.state-confirming-delete .ready-layer {
		z-index: 2;
		opacity: 1;
		transform: translateX(0) scale(1);
		filter: blur(0);
	}
	.state-removing .ready-layer {
		z-index: 2;
		opacity: 0;
		transform: translateX(36px) scale(0.94);
		filter: blur(3px);
	}
	.state-removing .idle-layer {
		opacity: 0.3;
		transform: translateX(-8px) scale(0.98);
		filter: blur(1px);
	}
	.upload-dropzone {
		transition:
			border-color 220ms ease,
			background-color 220ms ease;
	}
	.upload-dropzone:hover .upload-icon {
		animation: lift 1.6s ease-in-out infinite;
	}
	.upload-rings::before,
	.upload-rings::after {
		content: '';
		position: absolute;
		inset: -10px;
		border: 1px solid color-mix(in oklab, var(--primary) 35%, transparent);
		border-radius: 999px;
		animation: ring 1.8s ease-out infinite;
	}
	.upload-rings::after {
		animation-delay: 600ms;
	}
	.uploading-icon {
		animation: lift 1.2s ease-in-out infinite;
	}
	.progress-fill {
		transition: width 360ms cubic-bezier(0.22, 1, 0.36, 1);
	}
	@keyframes lift {
		0%,
		100% {
			transform: translateY(0);
		}
		50% {
			transform: translateY(-5px);
		}
	}
	@keyframes ring {
		from {
			transform: scale(0.55);
			opacity: 0.9;
		}
		to {
			transform: scale(1.22);
			opacity: 0;
		}
	}
	@media (prefers-reduced-motion: reduce) {
		.upload-layer,
		.upload-dropzone,
		.progress-fill {
			transition-duration: 1ms;
		}
		.upload-dropzone:hover .upload-icon,
		.upload-rings::before,
		.upload-rings::after,
		.uploading-icon {
			animation: none;
		}
	}
</style>
