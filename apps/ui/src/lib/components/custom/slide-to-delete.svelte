<script lang="ts">
	import CheckIcon from '@lucide/svelte/icons/check';
	import ChevronRightIcon from '@lucide/svelte/icons/chevron-right';
	import type { Component } from 'svelte';
	import { cn } from '#lib/utils.js';

	const PAD = 4;
	const KNOB = 40;
	let {
		label = 'Slide to delete',
		doneLabel = 'Delete confirmed',
		disabled = false,
		variant = 'destructive',
		icon: Icon = ChevronRightIcon,
		onconfirm,
		class: className,
	}: {
		label?: string;
		doneLabel?: string;
		disabled?: boolean;
		variant?: 'destructive' | 'primary';
		icon?: Component;
		onconfirm: () => void;
		class?: string;
	} = $props();

	let track = $state<HTMLDivElement | null>(null);
	let dragging = $state(false);
	let confirmed = $state(false);
	let x = $state(0);
	const fillTone = $derived(variant === 'destructive' ? 'bg-destructive' : 'bg-primary');
	const iconTone = $derived(variant === 'destructive' ? 'text-destructive' : 'text-primary');

	function maxX() {
		return Math.max((track?.clientWidth ?? 0) - KNOB - PAD * 2, 0);
	}

	function position(clientX: number) {
		if (!track) return 0;
		const rect = track.getBoundingClientRect();
		return Math.min(Math.max(clientX - rect.left - PAD - KNOB / 2, 0), maxX());
	}

	function start(event: PointerEvent) {
		if (disabled || confirmed) return;
		dragging = true;
		(event.currentTarget as HTMLButtonElement).setPointerCapture(event.pointerId);
	}

	function move(event: PointerEvent) {
		if (!dragging || confirmed) return;
		x = position(event.clientX);
	}

	function finish(event: PointerEvent) {
		if (!dragging) return;
		dragging = false;
		x = position(event.clientX);
		if (position(event.clientX) >= maxX() - 4) {
			x = maxX();
			confirmed = true;
			onconfirm();
		} else x = 0;
	}
</script>

<div data-slot="slide-to-delete" class={cn('w-full select-none sm:w-56', className)}>
	<div
		bind:this={track}
		class={cn(
			'relative isolate h-12 overflow-hidden rounded-full bg-muted p-1 lp-inset transition-colors duration-300 ease-[var(--lp-ease)]',
			confirmed && fillTone,
		)}>
		<span
			aria-hidden="true"
			class={cn(
				'absolute inset-0 flex items-center justify-center text-sm font-medium transition-opacity duration-200 ease-[var(--lp-ease)]',
				confirmed ? 'text-[var(--lp-accent-ink)]' : 'text-muted-foreground',
			)}
			style:opacity={confirmed ? 1 : Math.max(0, 1 - (x / Math.max(maxX(), 1)) * 1.4)}>
			{confirmed ? doneLabel : label}
		</span>
		<button
			type="button"
			aria-label={label}
			{disabled}
			onpointerdown={start}
			onpointermove={move}
			onpointerup={finish}
			onpointercancel={finish}
			class={cn(
				'absolute top-1 left-1 grid size-10 touch-none place-items-center rounded-full bg-card text-foreground outline-none lp-raised transition-[transform,box-shadow] duration-300 ease-[var(--lp-ease)] will-change-transform focus-visible:ring-3 focus-visible:ring-destructive/20 disabled:pointer-events-none disabled:opacity-50',
				dragging ? 'cursor-grabbing' : 'cursor-grab',
			)}
			style:transform={`translate3d(${x}px, 0, 0)`}>
			{#if confirmed}<CheckIcon
					class={cn('size-4', iconTone)}
					strokeWidth={2.5}
					aria-hidden="true" />{:else}<Icon
					class="size-5"
					strokeWidth={2.25}
					aria-hidden="true" />{/if}
		</button>
	</div>
</div>
