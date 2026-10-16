<script lang="ts">
	import { onDestroy } from 'svelte';
	import type { Snippet } from 'svelte';
	import CopyIcon from '@lucide/svelte/icons/copy';
	import CheckIcon from '@lucide/svelte/icons/check';
	import { toast } from '#lib/utils/toast.js';
	import { cn } from '#lib/utils.js';

	let {
		value,
		label,
		copiedLabel,
		class: className,
		iconClass,
		revealOnHover = true,
		errorMessage,
		resetAfter = 1500,
		children,
		...restProps
	}: {
		/** Text written to the clipboard. */
		value: string;
		/**
		 * Text shown next to the icon. Defaults to `value`; pass `null` for an
		 * icon-only button. Ignored when `children` is provided.
		 */
		label?: string | null;
		/** Replaces `label` while the check is showing, e.g. `copy` → `copied`. */
		copiedLabel?: string;
		class?: string;
		iconClass?: string;
		/** Keep the icon hidden until hover/focus, so dense rows stay quiet. */
		revealOnHover?: boolean;
		/** Toast shown when the clipboard is unavailable. Omit to fail silently. */
		errorMessage?: string;
		/** How long the check stays before reverting to the copy icon. */
		resetAfter?: number;
		children?: Snippet;
		[key: string]: unknown;
	} = $props();

	let copied = $state(false);
	let timer: ReturnType<typeof setTimeout> | undefined;

	const text = $derived(copied && copiedLabel !== undefined ? copiedLabel : (label ?? value));

	async function copy() {
		try {
			await navigator.clipboard.writeText(value);
			copied = true;
			clearTimeout(timer);
			timer = setTimeout(() => (copied = false), resetAfter);
		} catch {
			// A blocked clipboard (insecure origin, denied permission) is otherwise
			// silent, and the click looks like it did nothing.
			if (errorMessage) toast.error(errorMessage);
		}
	}

	onDestroy(() => clearTimeout(timer));
</script>

<!--
	The whole control is the button, so clicking the text copies too — the icon
	is an affordance, not the only target. `font-[inherit]` because buttons do
	not inherit font-family, which would break callers inside `font-mono`.
-->
<button
	{...restProps}
	type="button"
	onclick={copy}
	class={cn(
		'group inline-flex min-w-0 items-center gap-1 rounded text-left font-[inherit] transition-colors',
		'focus-visible:ring-1 focus-visible:ring-ring focus-visible:outline-none',
		'disabled:pointer-events-none disabled:opacity-50',
		className,
	)}
	aria-label={copied ? `Copied ${value}` : `Copy ${value}`}>
	{#if children}
		{@render children()}
	{:else if label !== null}
		<span class="truncate">{text}</span>
	{/if}

	<!-- Both icons are stacked so the swap scales in place instead of reflowing. -->
	<span
		class={cn(
			'relative inline-flex size-3.5 shrink-0 items-center justify-center transition-all duration-200 ease-out',
			revealOnHover && !copied
				? '-translate-x-1 scale-75 opacity-0 group-hover:translate-x-0 group-hover:scale-100 group-hover:opacity-100 group-focus-visible:translate-x-0 group-focus-visible:scale-100 group-focus-visible:opacity-100'
				: 'translate-x-0 scale-100 opacity-100',
			iconClass,
		)}>
		<CopyIcon
			class={cn(
				'absolute size-full transition-all duration-200 ease-out',
				copied ? 'scale-50 opacity-0' : 'scale-100 opacity-100',
			)}
			aria-hidden="true" />
		<CheckIcon
			class={cn(
				'absolute size-full text-success transition-all duration-200 ease-out',
				copied ? 'scale-100 opacity-100' : 'scale-50 opacity-0',
			)}
			aria-hidden="true" />
	</span>

	<span class="sr-only" aria-live="polite">{copied ? 'Copied' : ''}</span>
</button>
