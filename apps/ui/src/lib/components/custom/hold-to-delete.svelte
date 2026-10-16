<script lang="ts">
	import { onDestroy, onMount } from 'svelte';
	import type { Component } from 'svelte';
	import CheckIcon from '@lucide/svelte/icons/check';
	import Trash2Icon from '@lucide/svelte/icons/trash-2';
	import { cn } from '#lib/utils.js';

	let {
		label = 'Hold to delete',
		doneLabel = 'Delete confirmed',
		holdMs = 1100,
		disabled = false,
		variant = 'destructive',
		icon = undefined,
		onconfirm,
		class: className,
	}: {
		label?: string;
		doneLabel?: string;
		holdMs?: number;
		disabled?: boolean;
		variant?: 'destructive' | 'primary';
		icon?: Component;
		onconfirm: () => void;
		class?: string;
	} = $props();

	let holding = $state(false);
	let confirmed = $state(false);
	let timer: ReturnType<typeof setTimeout> | null = null;
	let button: HTMLButtonElement | null = null;
	let dialog: HTMLElement | null = null;
	const idleTone = $derived(
		variant === 'destructive' ? 'bg-destructive/10 text-destructive' : 'bg-primary/10 text-primary',
	);
	const fillTone = $derived(variant === 'destructive' ? 'bg-destructive' : 'bg-primary');
	const ActionIcon = $derived(icon ?? (variant === 'destructive' ? Trash2Icon : CheckIcon));

	function cancel() {
		if (timer) clearTimeout(timer);
		timer = null;
		holding = false;
	}

	function confirm() {
		cancel();
		confirmed = true;
		onconfirm();
	}

	function start() {
		if (disabled || confirmed || timer) return;
		holding = true;
		timer = setTimeout(confirm, holdMs);
	}

	function handleKeydown(event: KeyboardEvent) {
		if (event.repeat || (event.key !== ' ' && event.key !== 'Enter')) return;
		event.preventDefault();
		start();
	}

	function handleKeyup(event: KeyboardEvent) {
		if (event.key === ' ' || event.key === 'Enter') cancel();
	}

	// Inside a dialog, bits-ui focuses the dialog container on open (alert
	// dialogs) or leaves the first tabbable focused. Pull that initial focus
	// onto the hold button so a held Enter/Space works without tabbing over.
	// Inline (non-dialog) usages keep the page's own focus target.
	function grabDialogFocus(event: FocusEvent) {
		if (event.target === dialog && button && !disabled && !confirmed) button.focus();
	}

	onMount(() => {
		dialog = button?.closest<HTMLElement>('[role="alertdialog"], [role="dialog"]') ?? null;
		if (!dialog || !button) return;
		dialog.addEventListener('focusin', grabDialogFocus);
		// The dialog may have focused its container before we mounted.
		if (document.activeElement === dialog) button.focus();
	});

	onDestroy(() => {
		dialog?.removeEventListener('focusin', grabDialogFocus);
		cancel();
	});
</script>

<button
	bind:this={button}
	type="button"
	data-slot="hold-to-delete"
	data-holding={holding || undefined}
	data-confirmed={confirmed || undefined}
	aria-label={confirmed ? doneLabel : label}
	{disabled}
	onpointerdown={start}
	onpointerup={cancel}
	onpointerleave={cancel}
	onpointercancel={cancel}
	onkeydown={handleKeydown}
	onkeyup={handleKeyup}
	oncontextmenu={(event) => event.preventDefault()}
	class={cn(
		'relative flex h-11 w-full touch-none items-center justify-center overflow-hidden rounded-full px-5 text-sm font-medium outline-none transition-[box-shadow,transform] duration-300 ease-[var(--lp-ease)] lp-raised focus-visible:ring-3 focus-visible:ring-ring/30 active:scale-[.98] disabled:pointer-events-none disabled:opacity-50 sm:w-56',
		idleTone,
		className,
	)}>
	<span class="relative flex items-center gap-2">
		<ActionIcon class="size-4" strokeWidth={2} aria-hidden="true" />
		{label}
	</span>
	<span
		aria-hidden="true"
		class={cn(
			'absolute inset-0 z-10 overflow-hidden transition-transform motion-reduce:transition-none',
			fillTone,
			holding || confirmed
				? 'translate-x-0 duration-[1100ms] ease-linear'
				: '-translate-x-full duration-300 ease-[var(--lp-ease)]',
		)}
		style:transition-duration={holding || confirmed ? `${holdMs}ms` : undefined}>
		<span
			class={cn(
				'absolute inset-0 flex items-center justify-center gap-2 text-[var(--lp-accent-ink)] transition-transform motion-reduce:transition-none',
				holding || confirmed
					? 'translate-x-0 duration-[1100ms] ease-linear'
					: 'translate-x-full duration-300 ease-[var(--lp-ease)]',
			)}
			style:transition-duration={holding || confirmed ? `${holdMs}ms` : undefined}>
			{#if confirmed}<CheckIcon class="size-4" strokeWidth={2.5} />{:else}<ActionIcon
					class="size-4"
					strokeWidth={2} />{/if}
			{confirmed ? doneLabel : label}
		</span>
	</span>
	<span
		aria-hidden="true"
		class={cn(
			'pointer-events-none absolute inset-0 z-20 rounded-[inherit] transition-shadow duration-300 ease-[var(--lp-ease)]',
			holding || confirmed
				? 'shadow-[inset_0_1px_0_color-mix(in_oklch,var(--lp-accent-ink)_24%,transparent),inset_0_2px_4px_color-mix(in_oklch,var(--lp-shadow)_28%,transparent)]'
				: 'shadow-[inset_0_-1px_2px_color-mix(in_oklch,var(--lp-shadow)_10%,transparent)]',
		)}></span>
</button>
