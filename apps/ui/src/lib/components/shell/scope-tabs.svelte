<script lang="ts">
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { tick, type Component } from 'svelte';
	import { cubicOut } from 'svelte/easing';
	import { fade } from 'svelte/transition';
	import * as Tooltip from '#lib/components/ui/tooltip/index.js';
	import { activeTab, orgTabs, scopeOf } from '#lib/navigation/scope-tabs.js';
	import { scopeHero, shellTabs } from '#lib/state/shell.svelte.js';
	import { t } from '#lib/i18n/index.svelte.js';
	import { cn } from '#lib/utils.js';

	/**
	 * The glass capsule under the header: organization sections as links, or the
	 * tabs a project or application page pushed. Sticks under the header; on
	 * phones it becomes a bottom bar in the thumb zone.
	 */
	let {
		organizationId,
		isSysadmin = false,
		isCloud = false,
	}: { organizationId: string | null; isSysadmin?: boolean; isCloud?: boolean } = $props();

	type Tab = {
		id: string;
		label: string;
		icon?: Component;
		href?: string;
		disabled?: boolean;
		disabledTooltip?: string;
	};

	let path = $derived(page.url.pathname);
	let pushed = $derived(shellTabs.path === path && shellTabs.items.length > 0);

	let tabs = $derived.by((): Tab[] => {
		if (pushed) return shellTabs.items.map((item) => ({ ...item, id: item.value }));
		if (scopeOf(path) !== 'org' || !organizationId) return [];
		return orgTabs(organizationId, isSysadmin, isCloud).map((tab) => ({
			id: tab.id,
			label: t(tab.labelKey),
			icon: tab.icon,
			href: tab.href,
		}));
	});

	let current = $derived(
		pushed
			? shellTabs.active
			: organizationId
				? activeTab(path, orgTabs(organizationId, isSysadmin, isCloud))
				: undefined,
	);

	let capsule = $state<HTMLElement | null>(null);
	let indicator = $state({ x: 0, w: 0, ready: false });

	function measure() {
		const on = capsule?.querySelector<HTMLElement>('[aria-selected="true"]');
		if (!on) {
			indicator.w = 0;
			return;
		}
		indicator.x = on.offsetLeft;
		indicator.w = on.offsetWidth;
		// Keep the active tab in view when the capsule scrolls sideways.
		on.scrollIntoView?.({ block: 'nearest', inline: 'nearest' });
	}

	$effect(() => {
		void current;
		void tabs.length;
		if (!capsule) return;
		measure();
		// The first placement should not slide in from the left edge.
		requestAnimationFrame(() => (indicator.ready = true));
		const observer = new ResizeObserver(measure);
		observer.observe(capsule);
		return () => observer.disconnect();
	});

	// Once the hero has scrolled away, the bar shows who you are looking at.
	let sentinel = $state<HTMLElement | null>(null);
	let stuck = $state(false);
	$effect(() => {
		if (!sentinel) return;
		const observer = new IntersectionObserver(
			([entry]) => (stuck = !entry.isIntersecting && entry.boundingClientRect.top < 80),
			{ rootMargin: '-57px 0px 0px 0px' },
		);
		observer.observe(sentinel);
		return () => observer.disconnect();
	});

	/**
	 * iOS-style hold-to-select on the phone bar: hold a tab, the indicator lifts
	 * into a lens that follows the finger, and letting go picks the tab under it.
	 */
	let lens = $state<{ x: number; w: number; h: number; row: number } | null>(null);
	let hovered = $state<string>();
	let hold: ReturnType<typeof setTimeout> | undefined;
	let startX = 0;
	let lastX = 0;
	let edgeFrame = 0;
	let swallowClick = false;

	/**
	 * One axis of the lens' displacement map: neutral through the middle, easing
	 * (smoothstep-like) into a gentle inward bend at the rims. Red drives x,
	 * green drives y. The lens is short, so y bends far less than x or the top
	 * and bottom smear.
	 */
	function rimMap(axis: 'x' | 'y', w: number, h: number) {
		const c = (v: number) => (axis === 'x' ? `rgb(${v},0,0)` : `rgb(0,${v},0)`);
		const amp = axis === 'x' ? 72 : 18;
		const stops = [
			[0, 1],
			[0.1, 0.61],
			[0.22, 0.24],
			[0.34, 0.04],
			[0.42, 0],
			[0.58, 0],
			[0.66, -0.04],
			[0.78, -0.24],
			[0.9, -0.61],
			[1, -1],
		]
			.map(([o, f]) => `<stop offset='${o}' stop-color='${c(Math.round(128 + amp * f))}'/>`)
			.join('');
		const end = axis === 'x' ? "x2='1' y2='0'" : "x2='0' y2='1'";
		const svg =
			`<svg xmlns='http://www.w3.org/2000/svg' width='${w}' height='${h}'>` +
			`<linearGradient id='g' ${end}>${stops}</linearGradient>` +
			`<rect width='100%' height='100%' fill='url(#g)'/></svg>`;
		return `data:image/svg+xml,${encodeURIComponent(svg)}`;
	}

	// Holding near either end scrolls the bar, so one drag reaches the last tab.
	function edgeScroll() {
		if (!lens || !capsule) return;
		const box = capsule.getBoundingClientRect();
		const zone = 48;
		const push =
			lastX > box.right - zone
				? (lastX - box.right + zone) / zone
				: lastX < box.left + zone
					? -(box.left + zone - lastX) / zone
					: 0;
		if (push) {
			capsule.scrollLeft += push * 14;
			follow(lastX);
		}
		edgeFrame = requestAnimationFrame(edgeScroll);
	}

	function follow(clientX: number) {
		if (!capsule) return;
		const tabEls = [...capsule.querySelectorAll<HTMLElement>('[data-tab]')];
		const on = tabEls.find((el) => {
			const r = el.getBoundingClientRect();
			return clientX >= r.left && clientX < r.right;
		});
		const w = on?.offsetWidth ?? lens?.w ?? 0;
		const box = capsule.getBoundingClientRect();
		const x = clientX - box.left + capsule.scrollLeft - w / 2;
		lens = {
			x: Math.min(Math.max(x, 0), capsule.scrollWidth - w),
			w,
			h: capsule.clientHeight - 8,
			row: capsule.scrollWidth,
		};
		if (on && on.dataset.tab !== hovered) {
			hovered = on.dataset.tab;
			navigator.vibrate?.(8);
		}
	}

	function pressStart(event: PointerEvent) {
		if (event.pointerType !== 'touch' || !matchMedia('(max-width: 640px)').matches) return;
		startX = lastX = event.clientX;
		hold = setTimeout(() => {
			follow(lastX);
			edgeFrame = requestAnimationFrame(edgeScroll);
		}, 220);
	}

	function pressMove(event: PointerEvent) {
		lastX = event.clientX;
		if (lens) follow(event.clientX);
		// Moving before the hold lands is a sideways scroll, not a grab.
		else if (Math.abs(event.clientX - startX) > 8) clearTimeout(hold);
	}

	function pressEnd(event: PointerEvent) {
		clearTimeout(hold);
		cancelAnimationFrame(edgeFrame);
		if (!lens) return;
		const tab = event.type === 'pointerup' ? tabs.find((t) => t.id === hovered) : undefined;
		lens = null;
		hovered = undefined;
		// The browser may still fire a click on the tab the press began on.
		swallowClick = true;
		setTimeout(() => (swallowClick = false), 400);
		if (tab) activate(tab);
	}

	/** Switch tabs and slide the new content in from the side the tab sits on. */
	async function activate(tab: Tab) {
		if (tab.disabled || tab.id === current) return;
		const from = tabs.findIndex((t) => t.id === current);
		const dir = tabs.findIndex((t) => t.id === tab.id) < from ? -1 : 1;
		if (tab.href) await goto(tab.href, { reset: false });
		else shellTabs.onSelect?.(tab.id);
		await tick();
		if (reduced()) return;
		document
			.querySelector('main')
			?.animate(
				{ transform: [`translateX(${dir * 48}px)`, 'none'], opacity: [0, 1] },
				{ duration: 340, easing: 'cubic-bezier(0.32, 0.72, 0, 1)' },
			);
	}

	// Pointer listeners can't stop the page from panning; a non-passive touchmove can.
	$effect(() => {
		if (!capsule) return;
		const block = (event: TouchEvent) => lens && event.preventDefault();
		capsule.addEventListener('touchmove', block, { passive: false });
		return () => capsule?.removeEventListener('touchmove', block);
	});

	const reduced = () => !!window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;

	// The title grows its own width in, so the capsule glides over instead of jumping.
	function dock(node: HTMLElement) {
		const w = node.offsetWidth;
		return {
			duration: reduced() ? 0 : 380,
			easing: cubicOut,
			css: (t: number, u: number) =>
				`width:${t * w}px;margin-inline-end:${t * 12}px;opacity:${t};overflow:hidden;` +
				`transform:translateY(${u * 6}px) scale(${0.92 + 0.08 * t});filter:blur(${u * 4}px)`,
		};
	}

	function select(event: MouseEvent, tab: Tab) {
		// Let the browser handle opening a link in a new tab or window.
		if (tab.href && (event.metaKey || event.ctrlKey || event.shiftKey || event.button)) return;
		event.preventDefault();
		activate(tab);
	}
</script>

{#if tabs.length}
	<div bind:this={sentinel} aria-hidden="true" class="h-px"></div>
	<div class="scope-tabs" data-stuck={stuck}>
		<div class="shell-col flex items-center px-3 sm:px-6">
			{#if stuck && scopeHero.current?.path === path}
				<span
					transition:dock
					class="mini me-3 flex min-w-0 shrink items-center gap-2 text-sm font-medium whitespace-nowrap">
					{#if scopeHero.current.avatar}
						<span class="flex size-5.5 shrink-0 overflow-hidden rounded-md"
							>{@render scopeHero.current.avatar()}</span>
					{/if}
					<span class="max-w-40 truncate">{scopeHero.current.title}</span>
				</span>
			{/if}
			<div
				bind:this={capsule}
				class="capsule lp-well"
				role="tablist"
				tabindex="-1"
				aria-label={t('menu.tabs.label')}
				onpointerdown={pressStart}
				onpointermove={pressMove}
				onpointerup={pressEnd}
				onpointercancel={pressEnd}
				oncontextmenu={(event) => lens && event.preventDefault()}
				onclickcapture={(event) => {
					if (!swallowClick) return;
					swallowClick = false;
					event.preventDefault();
					event.stopPropagation();
				}}>
				{#if lens}
					<!-- Edge refraction for the lens: a rim-only displacement map, sized to the lens. -->
					<svg width="0" height="0" class="absolute" aria-hidden="true">
						<filter
							id="lp-lens-glass"
							x="0"
							y="0"
							width="100%"
							height="100%"
							color-interpolation-filters="sRGB">
							<feImage
								href={rimMap('x', lens.w, lens.h)}
								x="0"
								y="0"
								width={lens.w}
								height={lens.h}
								preserveAspectRatio="none"
								result="rx" />
							<feImage
								href={rimMap('y', lens.w, lens.h)}
								x="0"
								y="0"
								width={lens.w}
								height={lens.h}
								preserveAspectRatio="none"
								result="gy" />
							<feComposite in="rx" in2="gy" operator="arithmetic" k2="1" k3="1" result="map" />
							<feDisplacementMap
								in="SourceGraphic"
								in2="map"
								scale="10"
								xChannelSelector="R"
								yChannelSelector="G" />
						</filter>
					</svg>
				{/if}
				<span
					class="ind"
					class:ready={indicator.ready}
					class:lens={!!lens}
					style:transform="translateX({(lens ?? indicator).x}px) scale({lens ? 1.08 : 1})"
					style:width="{(lens ?? indicator).w}px"
					style:opacity={(lens ?? indicator).w ? 1 : 0}>
					{#if lens}
						<!-- A copy of the row seen through the glass: magnified by the lift, bent at the rims. -->
						<span class="lens-view" aria-hidden="true">
							<span class="lens-row" style:left="{-lens.x}px" style:width="{lens.row}px">
								{#each tabs as tab (tab.id)}
									<span class="ttab" data-on={tab.id === hovered}>
										{#if tab.icon}<tab.icon />{/if}
										<span>{tab.label}</span>
									</span>
								{/each}
							</span>
						</span>
					{/if}
				</span>
				{#each tabs as tab (tab.id)}
					{@const selected = tab.id === current}
					<Tooltip.Provider>
						<Tooltip.Root delayDuration={300} disabled={!tab.disabled || !tab.disabledTooltip}>
							<Tooltip.Trigger>
								{#snippet child({ props })}
									<svelte:element
										this={tab.href ? 'a' : 'button'}
										{...props}
										href={tab.href}
										type={tab.href ? undefined : 'button'}
										role="tab"
										data-tab={tab.id}
										aria-selected={selected}
										aria-disabled={tab.disabled || undefined}
										data-sveltekit-noscroll
										class={cn('ttab', tab.disabled && 'opacity-45')}
										onclick={(event: MouseEvent) => select(event, tab)}>
										{#if tab.icon}<tab.icon />{/if}
										<span>{tab.label}</span>
									</svelte:element>
								{/snippet}
							</Tooltip.Trigger>
							<Tooltip.Content>{tab.disabledTooltip}</Tooltip.Content>
						</Tooltip.Root>
					</Tooltip.Provider>
				{/each}
			</div>
			{#if stuck && scopeHero.current?.path === path && scopeHero.current.actions}
				<span
					transition:fade={{ duration: reduced() ? 0 : 200 }}
					class="mini ms-auto flex shrink-0 items-center gap-2 ps-3"
					>{@render scopeHero.current.actions()}</span>
			{/if}
		</div>
	</div>
{/if}

<style>
	.scope-tabs {
		position: sticky;
		top: var(--app-header-height);
		z-index: 20;
		padding: 0.625rem 0;
		border-bottom: 1px solid var(--lp-emboss);
		background: color-mix(in srgb, var(--background) 78%, transparent);
		box-shadow: 0 1px 0 var(--lp-bevel);
		backdrop-filter: blur(14px);
	}
	.capsule {
		position: relative;
		display: inline-flex;
		gap: 2px;
		max-width: 100%;
		padding: 4px;
		overflow-x: auto;
		scrollbar-width: none;
		border: 1px solid color-mix(in srgb, var(--foreground) 9%, transparent);
		border-radius: 999px;
		background: color-mix(in srgb, var(--lp-deep) 70%, transparent);
	}
	.ind {
		position: absolute;
		top: 4px;
		bottom: 4px;
		left: 0;
		border-radius: 999px;
		background: var(--lp-raised);
		box-shadow:
			inset 1px 1px 0 var(--lp-bevel),
			inset -1px -1px 0 var(--lp-emboss),
			0 4px 12px -4px
				color-mix(in srgb, var(--lp-shadow) calc(18% * var(--lp-shadow-strength)), transparent);
		pointer-events: none;
	}
	/* Spring: overshoots a touch, then settles on the tab. */
	.ind.ready {
		transition:
			transform 0.45s cubic-bezier(0.3, 1.25, 0.4, 1),
			width 0.45s cubic-bezier(0.3, 1.25, 0.4, 1),
			opacity 0.2s;
	}
	.ttab {
		position: relative;
		z-index: 1;
		display: inline-flex;
		align-items: center;
		gap: 7px;
		height: 32px;
		padding: 0 14px;
		border-radius: 999px;
		color: var(--muted-foreground);
		font-size: 13px;
		font-weight: 500;
		white-space: nowrap;
		outline: none;
		transition:
			color 0.2s var(--lp-ease),
			transform 0.15s var(--lp-ease);
	}
	.ttab:hover,
	.ttab[aria-selected='true'] {
		color: var(--foreground);
	}
	.ttab:focus-visible {
		box-shadow: 0 0 0 3px color-mix(in srgb, var(--ring) 30%, transparent);
	}
	.ttab:active {
		transform: scale(0.96);
	}
	.ttab :global(svg) {
		width: 15px;
		height: 15px;
		opacity: 0.75;
	}
	.ttab[aria-selected='true'] :global(svg) {
		color: var(--primary);
		opacity: 1;
	}

	@media (max-width: 640px) {
		/* A bottom bar in the thumb zone: icon over label. */
		.scope-tabs {
			position: fixed;
			top: auto;
			right: 0;
			bottom: 0;
			left: 0;
			z-index: 30;
			padding: 8px 10px calc(8px + env(safe-area-inset-bottom));
			border-top: 1px solid var(--lp-emboss);
			border-bottom: 0;
			box-shadow:
				inset 0 1px 0 var(--lp-bevel),
				0 -10px 30px -18px color-mix(in srgb, var(--lp-shadow) 40%, transparent);
		}
		.scope-tabs .shell-col {
			padding: 0;
		}
		.mini {
			display: none;
		}
		.capsule {
			display: flex;
			width: 100%;
			gap: 0;
			border-radius: 20px;
		}
		.ind {
			border-radius: 16px;
		}
		.ttab {
			flex: 1 0 auto;
			flex-direction: column;
			justify-content: center;
			gap: 3px;
			min-width: 64px;
			height: 52px;
			padding: 0 8px;
			border-radius: 16px;
			font-size: 10.5px;
		}
		.ttab {
			-webkit-touch-callout: none;
			user-select: none;
		}
		.ttab :global(svg) {
			width: 19px;
			height: 19px;
		}
	}
	/*
	 * Liquid glass while held: the lens lifts above the row and shows a copy of it,
	 * magnified by the lift and bent at the rims by #lp-lens-glass (CSS `filter`
	 * on a real element, so Safari renders it too). No blur: clear glass, not frost.
	 */
	.ind.lens {
		z-index: 2;
		overflow: hidden;
		background: none;
		box-shadow:
			0 10px 28px -8px
				color-mix(in srgb, var(--lp-shadow) calc(35% * var(--lp-shadow-strength)), transparent),
			0 2px 6px -2px color-mix(in srgb, var(--lp-shadow) 25%, transparent);
		transition:
			transform 0.16s cubic-bezier(0.3, 1.25, 0.4, 1),
			width 0.16s ease-out;
	}
	.lens-view {
		position: absolute;
		inset: 0;
		overflow: hidden;
		border-radius: inherit;
		background: color-mix(in srgb, var(--lp-raised) 55%, var(--background));
		filter: url(#lp-lens-glass) saturate(1.1);
	}
	.lens-row {
		position: absolute;
		top: -4px;
		display: flex;
		padding: 4px;
	}
	.lens-row .ttab[data-on='true'] {
		color: var(--foreground);
	}
	.lens-row .ttab[data-on='true'] :global(svg) {
		color: var(--primary);
		opacity: 1;
	}
	/* A soft band of light just inside the rim, a touch brighter top-left. */
	.ind.lens::after {
		position: absolute;
		inset: 0;
		border-radius: inherit;
		background: radial-gradient(
			140% 90% at 25% 0%,
			color-mix(in srgb, white 12%, transparent),
			transparent 55%
		);
		box-shadow:
			inset 1px 1px 1px -0.5px color-mix(in srgb, white 40%, transparent),
			inset -1px -1px 1px -0.5px color-mix(in srgb, white 18%, transparent),
			inset 0 0 5px 1px color-mix(in srgb, white 10%, transparent),
			inset 0 0 0 0.5px color-mix(in srgb, var(--foreground) 8%, transparent);
		content: '';
		mix-blend-mode: screen;
		pointer-events: none;
	}
	@media (prefers-reduced-motion: reduce) {
		.ind.ready,
		.ttab {
			transition: none;
		}
	}
</style>
