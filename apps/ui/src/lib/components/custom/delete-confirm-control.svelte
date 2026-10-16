<script lang="ts">
	import { IsMobile } from '#lib/hooks/is-mobile.svelte.js';
	import type { Component } from 'svelte';
	import HoldToDelete from './hold-to-delete.svelte';
	import SlideToDelete from './slide-to-delete.svelte';
	import { i18n, t } from '#lib/i18n/index.svelte.js';

	let {
		label = 'Delete',
		pendingLabel = 'Deleting…',
		disabled = false,
		variant = 'destructive',
		icon,
		onconfirm,
		class: className = 'shrink-0 sm:h-8 sm:w-auto',
	}: {
		label?: string;
		pendingLabel?: string;
		disabled?: boolean;
		variant?: 'destructive' | 'primary';
		icon?: Component;
		onconfirm: () => void | Promise<void>;
		class?: string;
	} = $props();

	const isMobile = new IsMobile();
	let pending = $state(false);
	let reset = $state(0);
	const desktopLabel = $derived(
		i18n.locale === 'es'
			? t('common.common.hold_to_confirm')
			: label === 'Delete'
				? 'Hold to delete'
				: `Hold to ${label.toLowerCase()}`,
	);
	const mobileLabel = $derived(
		label === 'Delete' ? 'Slide to delete' : `Slide to ${label.toLowerCase()}`,
	);

	async function confirm() {
		if (disabled || pending) return;
		pending = true;
		try {
			await onconfirm();
		} finally {
			pending = false;
			reset += 1;
		}
	}
</script>

{#key reset}
	{#if isMobile.current}
		<SlideToDelete
			label={mobileLabel}
			doneLabel={pendingLabel}
			{variant}
			{icon}
			disabled={disabled || pending}
			onconfirm={() => void confirm()}
			class={className} />
	{:else}
		<HoldToDelete
			label={desktopLabel}
			doneLabel={pendingLabel}
			{variant}
			{icon}
			disabled={disabled || pending}
			onconfirm={() => void confirm()}
			class={className} />
	{/if}
{/key}
