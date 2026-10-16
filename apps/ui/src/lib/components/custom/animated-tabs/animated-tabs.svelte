<script lang="ts">
	import { Tabs as TabsPrimitive } from 'bits-ui';
	import * as Tooltip from '#lib/components/ui/tooltip/index.js';
	import { Kbd, KbdGroup } from '#lib/components/ui/kbd/index.js';
	import { isMacPlatform, tabShortcutDigit } from '#lib/utils/tab-shortcut.js';
	import { prefersReducedMotion } from 'svelte/motion';
	import { cn } from '#lib/utils.js';

	type Item = { value: string; label: string; disabled?: boolean; disabledTooltip?: string };

	let {
		items,
		value = $bindable(),
		onValueChange,
		class: className,
		...restProps
	}: {
		items: Item[];
		value: string;
		onValueChange?: (value: string) => void;
		class?: string;
	} & Omit<TabsPrimitive.RootProps, 'value' | 'onValueChange' | 'children'> = $props();

	let listEl = $state<HTMLElement | null>(null);
	let triggerRefs = $state<Record<string, HTMLElement | null>>({});
	// Collect each trigger's button element without `bind:ref`, which would pass
	// `undefined` into bits-ui's `ref = $bindable(null)` on the first render and
	// throw props_invalid_value. The teardown also drops refs for removed items so
	// stale elements don't leak and `triggerRefs[value]` lookups stay bounded.
	function trackTrigger(value: string) {
		return (node: HTMLElement) => {
			triggerRefs[value] = node;
			return () => {
				delete triggerRefs[value];
			};
		};
	}
	let indicatorStyle = $state('');

	function updateIndicator() {
		const el = triggerRefs[value];
		if (!el || !listEl) return;
		indicatorStyle = `left:${el.offsetLeft}px; width:${el.offsetWidth}px;`;
	}

	$effect(() => {
		// tracked deps: value + whichever trigger element is currently active
		triggerRefs[value];
		updateIndicator();
	});

	$effect(() => {
		// Keep the active tab visible when the pill row overflows on narrow screens.
		triggerRefs[value]?.scrollIntoView({
			inline: 'nearest',
			block: 'nearest',
			behavior: prefersReducedMotion.current ? 'auto' : 'smooth',
		});
	});

	$effect(() => {
		if (!listEl) return;
		const observer = new ResizeObserver(updateIndicator);
		observer.observe(listEl);
		return () => observer.disconnect();
	});

	// Pointer drag-to-scroll for the pill row (mouse/pen). Touch scrolls natively.
	let drag = { down: false, moved: false, startX: 0, startLeft: 0 };

	function onPointerDown(e: PointerEvent) {
		if (!listEl || e.pointerType === 'touch') return;
		drag = { down: true, moved: false, startX: e.clientX, startLeft: listEl.scrollLeft };
		listEl.setPointerCapture(e.pointerId);
	}

	function onPointerMove(e: PointerEvent) {
		if (!drag.down || !listEl) return;
		const dx = e.clientX - drag.startX;
		if (Math.abs(dx) > 3) drag.moved = true;
		listEl.scrollLeft = drag.startLeft - dx;
	}

	function onPointerUp(e: PointerEvent) {
		drag.down = false;
		listEl?.releasePointerCapture?.(e.pointerId);
	}

	// Swallow the click that ends a drag so it doesn't switch tabs.
	function onClickCapture(e: MouseEvent) {
		if (!drag.moved) return;
		e.preventDefault();
		e.stopPropagation();
		drag.moved = false;
	}
</script>

<TabsPrimitive.Root
	bind:value
	onValueChange={(v) => {
		value = v;
		onValueChange?.(v);
	}}
	class={cn('gap-2 flex min-w-0 flex-col', className)}
	{...restProps}>
	<TabsPrimitive.List
		bind:ref={listEl}
		onpointerdown={onPointerDown}
		onpointermove={onPointerMove}
		onpointerup={onPointerUp}
		onpointercancel={onPointerUp}
		onclickcapture={onClickCapture}
		class="bg-muted text-muted-foreground no-scrollbar relative inline-flex h-[40px] w-fit max-w-full touch-pan-x items-center justify-start overflow-x-auto rounded-full p-1 select-none">
		<!-- ponytail: animates left/width, not transform, against DESIGN.md §6. A
		     translateX+scaleX indicator would stay off the layout path but visibly
		     distorts the rounded-full ends, and this is one absolutely-positioned
		     element outside any scroll loop. Revisit if the pill row ever gets long
		     enough for the per-frame layout to show up in a profile. -->
		<span
			class="bg-background dark:bg-input/30 absolute inset-y-1 rounded-full shadow-sm transition-[left,width] duration-300 ease-[var(--lp-ease)]"
			style={indicatorStyle}></span>
		{#each items as item, index (item.value)}
			{@const shortcutHint = item.disabled ? undefined : tabShortcutDigit(index)}
			{@const tooltipText = item.disabled
				? (item.disabledTooltip ?? 'This tab is currently unavailable.')
				: undefined}
			{@const triggerClass =
				'text-foreground/60 hover:text-foreground data-active:text-foreground relative z-0 inline-flex h-[calc(100%-1px)] flex-none items-center justify-center gap-2 rounded-full px-3 py-1 text-sm font-medium whitespace-nowrap transition-colors focus-visible:ring-[3px] focus-visible:ring-ring/50 focus-visible:outline-1 focus-visible:outline-ring disabled:pointer-events-none disabled:opacity-50'}
			{#if tooltipText || shortcutHint}
				<Tooltip.Root delayDuration={200}>
					<Tooltip.Trigger>
						{#snippet child({ props })}
							<TabsPrimitive.Trigger
								{...props}
								{@attach trackTrigger(item.value)}
								value={item.value}
								disabled={item.disabled}
								class={triggerClass}>
								{item.label}
							</TabsPrimitive.Trigger>
						{/snippet}
					</Tooltip.Trigger>
					<Tooltip.Content>
						{#if tooltipText}
							{tooltipText}
						{:else}
							<KbdGroup>
								<Kbd>{isMacPlatform ? '⌘' : 'Ctrl'}</Kbd>
								<Kbd>{shortcutHint}</Kbd>
							</KbdGroup>
						{/if}
					</Tooltip.Content>
				</Tooltip.Root>
			{:else}
				<TabsPrimitive.Trigger
					{@attach trackTrigger(item.value)}
					value={item.value}
					disabled={item.disabled}
					class={triggerClass}>
					{item.label}
				</TabsPrimitive.Trigger>
			{/if}
		{/each}
	</TabsPrimitive.List>
</TabsPrimitive.Root>
