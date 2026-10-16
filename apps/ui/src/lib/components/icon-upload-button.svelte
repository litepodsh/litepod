<script lang="ts">
	import { Button } from '#lib/components/ui/button/index.js';
	import UploadIcon from '@lucide/svelte/icons/upload';
	import LoaderCircleIcon from '@lucide/svelte/icons/loader-circle';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';

	const MAX_BYTES = 2 * 1024 * 1024;
	const ACCEPT = 'image/png,image/jpeg,image/webp,image/svg+xml';

	let {
		onUpload,
		onUploaded,
	}: {
		/** Performs the real upload; returns the new icon URL on success. */
		onUpload: (file: File) => Promise<string>;
		onUploaded: (url: string) => void;
	} = $props();

	let input = $state<HTMLInputElement | null>(null);
	let uploading = $state(false);

	async function handleChange(event: Event) {
		const target = event.currentTarget as HTMLInputElement;
		const file = target.files?.[0];
		target.value = '';
		if (!file) return;
		if (file.size > MAX_BYTES) {
			toast.error(t('common.common.icon_too_large'));
			return;
		}
		uploading = true;
		try {
			onUploaded(await onUpload(file));
		} catch (err) {
			toast.error(err instanceof Error ? err.message : t('common.common.icon_upload_failed'));
		} finally {
			uploading = false;
		}
	}
</script>

<input bind:this={input} class="sr-only" type="file" accept={ACCEPT} onchange={handleChange} />
<Button
	type="button"
	variant="outline"
	size="sm"
	disabled={uploading}
	onclick={() => input?.click()}>
	{#if uploading}
		<LoaderCircleIcon class="size-4 animate-spin" />
	{:else}
		<UploadIcon class="size-4" />
	{/if}
	{t('common.common.upload')}
</Button>
